//! Status state (`ConnectionState::Status`) server list ping, MOTD, player count, and favicon packets.

use std::io::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use bytes::{Buf, BufMut};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use minestom_data::{PROTOCOL_VERSION, VERSION_NAME};
use minestom_macros::PacketMeta;
use minestom_text::Component;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_string, write_string,
};

const FAVICON_DATA_URI_PREFIX: &str = "data:image/png;base64,";

fn crc32_ieee(chunks: &[&[u8]]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for slice in chunks {
        for &byte in *slice {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
    }
    !crc
}

fn write_png_chunk(out: &mut Vec<u8>, chunk_type: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&chunk_type);
    out.extend_from_slice(data);
    let crc = crc32_ieee(&[chunk_type.as_slice(), data]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encodes a 64×64 RGBA pixel buffer (`64 * 64` pixels, row-major) into a valid PNG byte vector
/// suitable for [`ServerStatus::favicon`] in the vanilla Minecraft server list.
#[must_use]
pub fn encode_rgba_64x64_png(pixels: &[[u8; 4]; 64 * 64]) -> Vec<u8> {
    let mut raw_scanlines = Vec::with_capacity(64 * (1 + 64 * 4));
    for y in 0..64 {
        raw_scanlines.push(0); // Filter type 0 (None)
        for x in 0..64 {
            raw_scanlines.extend_from_slice(&pixels[y * 64 + x]);
        }
    }
    let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
    let _ = zlib.write_all(&raw_scanlines);
    let idat = zlib.finish().unwrap_or_default();

    let mut png = Vec::with_capacity(64 + idat.len());
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut ihdr = [0_u8; 13];
    ihdr[0..4].copy_from_slice(&64_u32.to_be_bytes());
    ihdr[4..8].copy_from_slice(&64_u32.to_be_bytes());
    ihdr[8] = 8; // 8-bit depth
    ihdr[9] = 6; // RGBA color type
    write_png_chunk(&mut png, *b"IHDR", &ihdr);
    write_png_chunk(&mut png, *b"IDAT", &idat);
    write_png_chunk(&mut png, *b"IEND", &[]);
    png
}

/// Server version block inside [`ServerStatus`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusVersion {
    /// Version display string (defaults to `"26.2"`).
    pub name: String,
    /// Protocol version number (defaults to `776`).
    pub protocol: i32,
}

impl Default for StatusVersion {
    fn default() -> Self {
        Self {
            name: VERSION_NAME.to_owned(),
            protocol: PROTOCOL_VERSION,
        }
    }
}

/// Hover sample entry in [`StatusPlayers`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPlayerSample {
    /// Player name or legacy-formatted line.
    pub name: String,
    /// Player UUID.
    pub id: Uuid,
}

impl StatusPlayerSample {
    /// Creates a new player sample entry.
    #[must_use]
    pub fn new(name: impl Into<String>, id: Uuid) -> Self {
        Self {
            name: name.into(),
            id,
        }
    }
}

/// Online/max player counts and hover sample inside [`ServerStatus`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPlayers {
    /// Maximum player slots.
    pub max: u32,
    /// Currently online players.
    pub online: u32,
    /// Optional hover sample list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample: Vec<StatusPlayerSample>,
}

impl StatusPlayers {
    /// Creates a `StatusPlayers` summary with an empty sample list.
    #[must_use]
    pub const fn new(online: u32, max: u32) -> Self {
        Self {
            max,
            online,
            sample: Vec::new(),
        }
    }
}

#[allow(clippy::ref_option)] // Required by serde `serialize_with` signature
fn serialize_favicon<S: Serializer>(
    favicon: &Option<Vec<u8>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match favicon {
        None => serializer.serialize_none(),
        Some(png_bytes) => {
            let mut uri = String::with_capacity(
                FAVICON_DATA_URI_PREFIX.len() + png_bytes.len().div_ceil(3) * 4,
            );
            uri.push_str(FAVICON_DATA_URI_PREFIX);
            BASE64_STANDARD.encode_string(png_bytes, &mut uri);
            serializer.serialize_some(&uri)
        }
    }
}

fn deserialize_favicon<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<u8>>, D::Error> {
    let opt = Option::<String>::deserialize(deserializer)?;
    let Some(uri) = opt else {
        return Ok(None);
    };
    let b64 = uri.strip_prefix(FAVICON_DATA_URI_PREFIX).ok_or_else(|| {
        serde::de::Error::custom("favicon must start with `data:image/png;base64,`")
    })?;
    let bytes = BASE64_STANDARD
        .decode(b64)
        .map_err(serde::de::Error::custom)?;
    Ok(Some(bytes))
}

/// Complete server list ping payload corresponding to Minestom's `Status` record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerStatus {
    /// Version metadata (`26.2`, protocol `776`).
    pub version: StatusVersion,
    /// Online/max player counts and optional hover sample.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub players: Option<StatusPlayers>,
    /// Rich MOTD text [`Component`].
    pub description: Component,
    /// Optional raw 64×64 PNG bytes serialized as `"data:image/png;base64,..."`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_favicon",
        deserialize_with = "deserialize_favicon"
    )]
    pub favicon: Option<Vec<u8>>,
    /// Whether the server enforces Mojang signed chat messages.
    #[serde(rename = "enforcesSecureChat", default)]
    pub enforces_secure_chat: bool,
}

impl Default for ServerStatus {
    fn default() -> Self {
        Self {
            version: StatusVersion::default(),
            players: Some(StatusPlayers::new(0, 100)),
            description: Component::text("A minestom-rs Server"),
            favicon: None,
            enforces_secure_chat: false,
        }
    }
}

/// Serverbound `StatusRequestPacket` (`0x00` in [`ConnectionState::Status`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PacketMeta)]
pub struct StatusRequestPacket;

impl PacketPayload for StatusRequestPacket {
    const PACKET_ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Status;
}

impl Encode for StatusRequestPacket {
    fn encode<B: BufMut>(&self, _buf: &mut B) -> Result<(), ProtocolError> {
        Ok(())
    }
}

impl Decode for StatusRequestPacket {
    fn decode<B: Buf>(_buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self)
    }
}

/// Clientbound `ResponsePacket` (`0x00` in [`ConnectionState::Status`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct StatusResponsePacket {
    /// Server status payload.
    pub status: ServerStatus,
}

impl PacketPayload for StatusResponsePacket {
    const PACKET_ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Status;
}

impl Encode for StatusResponsePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        let json = serde_json::to_string(&self.status)?;
        write_string(buf, &json, DEFAULT_MAX_STRING_CHARS)
    }
}

impl Decode for StatusResponsePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let json = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        let status: ServerStatus = serde_json::from_str(&json)?;
        Ok(Self { status })
    }
}

/// Serverbound `ClientPingRequestPacket` (`0x01` in [`ConnectionState::Status`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct StatusPingRequestPacket {
    /// Client timestamp / nonce echoed back in [`StatusPongResponsePacket`].
    pub payload: i64,
}

impl PacketPayload for StatusPingRequestPacket {
    const PACKET_ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Status;
}

impl Encode for StatusPingRequestPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.payload.encode(buf)
    }
}

impl Decode for StatusPingRequestPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            payload: i64::decode(buf)?,
        })
    }
}

/// Clientbound `PingResponsePacket` (`0x01` in [`ConnectionState::Status`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct StatusPongResponsePacket {
    /// Echoed client timestamp / nonce.
    pub payload: i64,
}

impl PacketPayload for StatusPongResponsePacket {
    const PACKET_ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Status;
}

impl Encode for StatusPongResponsePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.payload.encode(buf)
    }
}

impl Decode for StatusPongResponsePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            payload: i64::decode(buf)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use minestom_text::NamedTextColor;
    use proptest::prelude::*;

    use super::*;
    use crate::{decode_packet_exact, encode_packet_payload, read_var_int};

    #[test]
    fn status_response_with_favicon_and_sample_roundtrip() {
        let png_stub = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
        let pkt = StatusResponsePacket {
            status: ServerStatus {
                version: StatusVersion::default(),
                players: Some(StatusPlayers {
                    online: 12,
                    max: 50,
                    sample: vec![StatusPlayerSample::new(
                        "Notch",
                        Uuid::from_u128(0x069a_79f4_44e9_4726_a5be_fca9_0e38_aaf5),
                    )],
                }),
                description: Component::text("minestom-rs ")
                    .color(NamedTextColor::Gold)
                    .append(
                        Component::text("26.2")
                            .color(NamedTextColor::Green)
                            .bold(true),
                    ),
                favicon: Some(png_stub),
                enforces_secure_chat: false,
            },
        };

        let mut body = encode_packet_payload(&pkt).unwrap();
        assert_eq!(
            read_var_int(&mut body).unwrap(),
            StatusResponsePacket::PACKET_ID
        );
        let decoded = decode_packet_exact::<StatusResponsePacket>(body).unwrap();
        assert_eq!(decoded, pkt);
    }

    proptest! {
        #[test]
        fn ping_pong_roundtrip(payload in any::<i64>()) {
            let req = StatusPingRequestPacket { payload };
            let mut req_body = encode_packet_payload(&req).unwrap();
            prop_assert_eq!(read_var_int(&mut req_body).unwrap(), 0x01);
            prop_assert_eq!(decode_packet_exact::<StatusPingRequestPacket>(req_body).unwrap(), req);

            let resp = StatusPongResponsePacket { payload };
            let mut resp_body = encode_packet_payload(&resp).unwrap();
            prop_assert_eq!(read_var_int(&mut resp_body).unwrap(), 0x01);
            prop_assert_eq!(decode_packet_exact::<StatusPongResponsePacket>(resp_body).unwrap(), resp);
        }

        #[test]
        fn arbitrary_bytes_never_panic_status_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let b = bytes::Bytes::from(raw);
            let _ = decode_packet_exact::<StatusRequestPacket>(b.clone());
            let _ = decode_packet_exact::<StatusResponsePacket>(b.clone());
            let _ = decode_packet_exact::<StatusPingRequestPacket>(b.clone());
            let _ = decode_packet_exact::<StatusPongResponsePacket>(b);
        }
    }
}
