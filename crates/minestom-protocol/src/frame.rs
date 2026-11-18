//! Minecraft packet framing (length-prefixing and optional Zlib compression).

use std::io::{Read, Write};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

use crate::{Decode, PacketPayload, ProtocolError, read_var_int, var_int_len, write_var_int};

/// Maximum framed packet size in Play/Configuration states (`2_097_151`, 3-byte `VarInt`).
pub const MAX_PACKET_SIZE: usize = 2_097_151;

/// Maximum incoming framed packet size in unauthenticated states (`8_192`, matching Minestom).
pub const MAX_PACKET_SIZE_PRE_AUTH: usize = 8_192;

/// Decoded raw packet frame consisting of `packet_id` and uncompressed `payload` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPacketFrame {
    /// Numeric wire packet ID.
    pub packet_id: i32,
    /// Uncompressed packet body after `packet_id`.
    pub payload: Bytes,
}

/// Encodes a typed [`PacketPayload`] into `[VarInt packet_id][fields]`.
pub fn encode_packet_payload<P: PacketPayload>(packet: &P) -> Result<Bytes, ProtocolError> {
    let mut body = BytesMut::with_capacity(64);
    write_var_int(&mut body, P::PACKET_ID);
    packet.encode(&mut body)?;
    Ok(body.freeze())
}

/// Decodes `P` from `payload` and verifies zero trailing unread bytes remain.
pub fn decode_packet_exact<P: Decode>(mut payload: Bytes) -> Result<P, ProtocolError> {
    let pkt = P::decode(&mut payload)?;
    if payload.has_remaining() {
        return Err(ProtocolError::TrailingBytes(payload.remaining()));
    }
    Ok(pkt)
}

/// Stateful stream framer handling uncompressed and Zlib-compressed Minecraft TCP frames.
#[derive(Debug, Clone)]
pub struct FrameCodec {
    compression_threshold: Option<usize>,
    max_inbound_size: usize,
}

impl Default for FrameCodec {
    fn default() -> Self {
        Self::new(MAX_PACKET_SIZE_PRE_AUTH)
    }
}

impl FrameCodec {
    /// Creates an uncompressed `FrameCodec` enforcing `max_inbound_size` on incoming frames.
    #[must_use]
    pub const fn new(max_inbound_size: usize) -> Self {
        Self {
            compression_threshold: None,
            max_inbound_size,
        }
    }

    /// Updates the Zlib compression threshold (`None` disables compression).
    pub const fn set_compression_threshold(&mut self, threshold: Option<usize>) {
        self.compression_threshold = threshold;
    }

    /// Returns the active compression threshold, if enabled.
    pub const fn compression_threshold(&self) -> Option<usize> {
        self.compression_threshold
    }

    /// Updates the maximum permitted inbound frame byte size.
    pub const fn set_max_inbound_size(&mut self, max_inbound_size: usize) {
        self.max_inbound_size = max_inbound_size;
    }

    /// Encodes a typed [`PacketPayload`] into `dst` with framing (and compression if active).
    pub fn encode_packet<P: PacketPayload>(
        &self,
        packet: &P,
        dst: &mut BytesMut,
    ) -> Result<(), ProtocolError> {
        let raw = encode_packet_payload(packet)?;
        self.encode_raw_body(&raw, dst)
    }

    /// Wraps an already-encoded `[VarInt packet_id][payload]` body into a wire frame in `dst`.
    pub fn encode_raw_body(
        &self,
        uncompressed_body: &[u8],
        dst: &mut BytesMut,
    ) -> Result<(), ProtocolError> {
        match self.compression_threshold {
            None => {
                if uncompressed_body.len() > MAX_PACKET_SIZE {
                    return Err(ProtocolError::PacketTooLarge {
                        len: uncompressed_body.len(),
                        max: MAX_PACKET_SIZE,
                    });
                }
                dst.reserve(var_int_len(uncompressed_body.len() as i32) + uncompressed_body.len());
                write_var_int(dst, uncompressed_body.len() as i32);
                dst.put_slice(uncompressed_body);
                Ok(())
            }
            Some(threshold) => {
                if uncompressed_body.len() < threshold {
                    let frame_len = 1 + uncompressed_body.len();
                    if frame_len > MAX_PACKET_SIZE {
                        return Err(ProtocolError::PacketTooLarge {
                            len: frame_len,
                            max: MAX_PACKET_SIZE,
                        });
                    }
                    dst.reserve(var_int_len(frame_len as i32) + frame_len);
                    write_var_int(dst, frame_len as i32);
                    write_var_int(dst, 0);
                    dst.put_slice(uncompressed_body);
                    Ok(())
                } else {
                    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
                    encoder
                        .write_all(uncompressed_body)
                        .map_err(|e| ProtocolError::Compression(e.to_string()))?;
                    let compressed = encoder
                        .finish()
                        .map_err(|e| ProtocolError::Compression(e.to_string()))?;
                    let data_len_varint = var_int_len(uncompressed_body.len() as i32);
                    let frame_len = data_len_varint + compressed.len();
                    if frame_len > MAX_PACKET_SIZE {
                        return Err(ProtocolError::PacketTooLarge {
                            len: frame_len,
                            max: MAX_PACKET_SIZE,
                        });
                    }
                    dst.reserve(var_int_len(frame_len as i32) + frame_len);
                    write_var_int(dst, frame_len as i32);
                    write_var_int(dst, uncompressed_body.len() as i32);
                    dst.put_slice(&compressed);
                    Ok(())
                }
            }
        }
    }

    /// Attempts to decode the next complete [`RawPacketFrame`] from `src`.
    ///
    /// Returns `Ok(None)` if `src` contains an incomplete frame and needs more socket bytes.
    pub fn try_decode_frame(
        &self,
        src: &mut BytesMut,
    ) -> Result<Option<RawPacketFrame>, ProtocolError> {
        if src.is_empty() {
            return Ok(None);
        }

        // Peek VarInt frame length up to 5 bytes without consuming `src` on partial read.
        let mut frame_len_u32 = 0_u32;
        let mut header_len = 0_usize;
        let mut complete_len = false;
        for (i, &byte) in src.iter().take(5).enumerate() {
            frame_len_u32 |= u32::from(byte & 0x7F) << (i * 7);
            header_len = i + 1;
            if (byte & 0x80) == 0 {
                complete_len = true;
                break;
            }
        }

        if !complete_len {
            if src.len() >= 5 {
                return Err(ProtocolError::VarIntTooLarge);
            }
            return Ok(None);
        }

        let signed_len = frame_len_u32 as i32;
        if signed_len <= 0 {
            return Err(ProtocolError::InvalidLength {
                len: signed_len,
                max: self.max_inbound_size,
            });
        }
        let frame_len = signed_len as usize;
        if frame_len > self.max_inbound_size {
            return Err(ProtocolError::PacketTooLarge {
                len: frame_len,
                max: self.max_inbound_size,
            });
        }

        if src.len() < header_len + frame_len {
            src.reserve(header_len + frame_len - src.len());
            return Ok(None);
        }

        src.advance(header_len);
        let mut frame_bytes = src.split_to(frame_len).freeze();

        let mut uncompressed = if self.compression_threshold.is_some() {
            let data_len = read_var_int(&mut frame_bytes)?;
            if data_len < 0 {
                return Err(ProtocolError::InvalidLength {
                    len: data_len,
                    max: MAX_PACKET_SIZE,
                });
            }
            if data_len == 0 {
                frame_bytes
            } else {
                let expected = data_len as usize;
                if expected > MAX_PACKET_SIZE {
                    return Err(ProtocolError::PacketTooLarge {
                        len: expected,
                        max: MAX_PACKET_SIZE,
                    });
                }
                let mut decoder =
                    ZlibDecoder::new(frame_bytes.as_ref()).take((expected + 1) as u64);
                let mut out = Vec::with_capacity(expected.min(65_536));
                decoder
                    .read_to_end(&mut out)
                    .map_err(|e| ProtocolError::Compression(e.to_string()))?;
                if out.len() != expected {
                    return Err(ProtocolError::Compression(format!(
                        "decompressed length mismatch: expected {expected}, got {}",
                        out.len()
                    )));
                }
                Bytes::from(out)
            }
        } else {
            frame_bytes
        };

        let packet_id = read_var_int(&mut uncompressed)?;
        Ok(Some(RawPacketFrame {
            packet_id,
            payload: uncompressed,
        }))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn frame_codec_uncompressed_and_compressed_roundtrip(
            packet_id in 0_i32..128_i32,
            payload in prop::collection::vec(any::<u8>(), 0..1024),
            threshold in prop_oneof![Just(None), Just(Some(64_usize)), Just(Some(256_usize))],
        ) {
            let mut body = BytesMut::new();
            write_var_int(&mut body, packet_id);
            body.put_slice(&payload);

            let mut codec = FrameCodec::new(MAX_PACKET_SIZE);
            codec.set_compression_threshold(threshold);

            let mut wire = BytesMut::new();
            codec.encode_raw_body(&body, &mut wire).unwrap();

            // Verify partial streaming reads return Ok(None) before full frame arrives
            if wire.len() > 2 {
                let mut partial = wire.clone();
                partial.truncate(wire.len() - 1);
                prop_assert_eq!(codec.try_decode_frame(&mut partial).unwrap(), None);
            }

            let decoded = codec.try_decode_frame(&mut wire).unwrap().expect("complete frame");
            prop_assert_eq!(decoded.packet_id, packet_id);
            prop_assert_eq!(decoded.payload.as_ref(), payload.as_slice());
            prop_assert!(wire.is_empty());
        }

        #[test]
        fn arbitrary_bytes_never_panic_frame_decoder(
            raw in prop::collection::vec(any::<u8>(), 0..512),
            compressed in any::<bool>(),
        ) {
            let mut codec = FrameCodec::new(MAX_PACKET_SIZE_PRE_AUTH);
            if compressed {
                codec.set_compression_threshold(Some(64));
            }
            let mut buf = BytesMut::from(raw.as_slice());
            let _ = codec.try_decode_frame(&mut buf);
        }
    }
}
