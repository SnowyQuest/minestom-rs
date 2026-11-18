//! Wire-level primitive codecs (`Encode`, `Decode`, `VarInt`, `VarLong`, strings, collections).

use bytes::{Buf, BufMut};
use minestom_core::{BlockVec, Key};
use minestom_nbt::{NbtCompound, NbtTag};
use minestom_text::Component;
use uuid::Uuid;

use crate::{ConnectionState, ProtocolError};

/// Default maximum character count for unbounded protocol strings (`32_767`).
pub const DEFAULT_MAX_STRING_CHARS: usize = 32_767;

/// Serializes `self` onto a Minecraft network byte buffer.
pub trait Encode {
    /// Appends the wire representation of `self` into `buf`.
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError>;
}

/// Deserializes `Self` from a Minecraft network byte buffer without panicking on malformed input.
pub trait Decode: Sized {
    /// Reads `Self` from `buf`, advancing the cursor.
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError>;
}

/// Associates a packet struct with its numeric ID and protocol state.
pub trait PacketPayload: Encode + Decode {
    /// Numeric wire packet ID within [`Self::STATE`].
    const PACKET_ID: i32;
    /// Protocol state to which this packet belongs.
    const STATE: ConnectionState;
}

fn ensure_rem<B: Buf>(buf: &B, n: usize) -> Result<(), ProtocolError> {
    if buf.remaining() < n {
        Err(ProtocolError::UnexpectedEof)
    } else {
        Ok(())
    }
}

/// Returns the encoded byte length (`1..=5`) of a 32-bit [`VarInt`].
#[must_use]
pub const fn var_int_len(value: i32) -> usize {
    let u = value as u32;
    if (u & !0x7F) == 0 {
        1
    } else if (u & !0x3FFF) == 0 {
        2
    } else if (u & !0x1F_FFFF) == 0 {
        3
    } else if (u & !0x0FFF_FFFF) == 0 {
        4
    } else {
        5
    }
}

/// Writes a 32-bit LEB128 `VarInt` (`1..=5` bytes).
pub fn write_var_int<B: BufMut>(buf: &mut B, value: i32) {
    let mut u = value as u32;
    loop {
        if (u & !0x7F) == 0 {
            buf.put_u8(u as u8);
            return;
        }
        buf.put_u8(((u & 0x7F) as u8) | 0x80);
        u >>= 7;
    }
}

/// Reads a 32-bit LEB128 `VarInt` (`1..=5` bytes).
pub fn read_var_int<B: Buf>(buf: &mut B) -> Result<i32, ProtocolError> {
    let mut result = 0_u32;
    for shift in (0..=28).step_by(7) {
        ensure_rem(buf, 1)?;
        let byte = buf.get_u8();
        result |= u32::from(byte & 0x7F) << shift;
        if (byte & 0x80) == 0 {
            return Ok(result as i32);
        }
    }
    Err(ProtocolError::VarIntTooLarge)
}

/// Writes a 64-bit LEB128 `VarLong` (`1..=10` bytes).
pub fn write_var_long<B: BufMut>(buf: &mut B, value: i64) {
    let mut u = value as u64;
    loop {
        if (u & !0x7F) == 0 {
            buf.put_u8(u as u8);
            return;
        }
        buf.put_u8(((u & 0x7F) as u8) | 0x80);
        u >>= 7;
    }
}

/// Reads a 64-bit LEB128 `VarLong` (`1..=10` bytes).
pub fn read_var_long<B: Buf>(buf: &mut B) -> Result<i64, ProtocolError> {
    let mut result = 0_u64;
    for shift in (0..=63).step_by(7) {
        ensure_rem(buf, 1)?;
        let byte = buf.get_u8();
        if shift == 63 && (byte & 0xFE) != 0 {
            return Err(ProtocolError::VarLongTooLarge);
        }
        result |= u64::from(byte & 0x7F) << shift;
        if (byte & 0x80) == 0 {
            return Ok(result as i64);
        }
    }
    Err(ProtocolError::VarLongTooLarge)
}

/// Wrapper type encoding an `i32` as a Minecraft `VarInt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VarInt(pub i32);

impl Encode for VarInt {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.0);
        Ok(())
    }
}

impl Decode for VarInt {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        read_var_int(buf).map(Self)
    }
}

/// Wrapper type encoding an `i64` as a Minecraft `VarLong`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VarLong(pub i64);

impl Encode for VarLong {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_long(buf, self.0);
        Ok(())
    }
}

impl Decode for VarLong {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        read_var_long(buf).map(Self)
    }
}

/// `OPTIONAL_VAR_INT` wire type (`0` = `None`, `v + 1` = `Some(v)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct OptionalVarInt(pub Option<i32>);

impl Encode for OptionalVarInt {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        match self.0 {
            None => write_var_int(buf, 0),
            Some(v) => write_var_int(buf, v.wrapping_add(1)),
        }
        Ok(())
    }
}

impl Decode for OptionalVarInt {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let raw = read_var_int(buf)?;
        Ok(Self(if raw == 0 {
            None
        } else {
            Some(raw.wrapping_sub(1))
        }))
    }
}

/// Writes a UTF-8 string prefixed by its `VarInt` byte length, enforcing `max_chars`.
pub fn write_string<B: BufMut>(
    buf: &mut B,
    value: &str,
    max_chars: usize,
) -> Result<(), ProtocolError> {
    let char_count = value.chars().count();
    if char_count > max_chars {
        return Err(ProtocolError::StringTooLong {
            len: char_count,
            max: max_chars,
        });
    }
    let bytes = value.as_bytes();
    write_var_int(buf, bytes.len() as i32);
    buf.put_slice(bytes);
    Ok(())
}

/// Reads a `VarInt`-byte-prefixed UTF-8 string, enforcing `max_chars` (max UTF-8 bytes `max_chars * 4`)
/// before allocating memory.
pub fn read_string<B: Buf>(buf: &mut B, max_chars: usize) -> Result<String, ProtocolError> {
    let raw_len = read_var_int(buf)?;
    let max_bytes = max_chars.saturating_mul(4);
    if raw_len < 0 || (raw_len as usize) > max_bytes {
        return Err(ProtocolError::InvalidLength {
            len: raw_len,
            max: max_bytes,
        });
    }
    let byte_len = raw_len as usize;
    ensure_rem(buf, byte_len)?;
    let mut raw = vec![0_u8; byte_len];
    buf.copy_to_slice(&mut raw);
    let s = String::from_utf8(raw).map_err(|_| ProtocolError::InvalidUtf8)?;
    let char_count = s.chars().count();
    if char_count > max_chars {
        return Err(ProtocolError::StringTooLong {
            len: char_count,
            max: max_chars,
        });
    }
    Ok(s)
}

/// Writes a `VarInt`-prefixed byte slice.
pub fn write_byte_array<B: BufMut>(
    buf: &mut B,
    bytes: &[u8],
    max_len: usize,
) -> Result<(), ProtocolError> {
    if bytes.len() > max_len {
        return Err(ProtocolError::InvalidLength {
            len: bytes.len() as i32,
            max: max_len,
        });
    }
    write_var_int(buf, bytes.len() as i32);
    buf.put_slice(bytes);
    Ok(())
}

/// Reads a `VarInt`-prefixed `Vec<u8>` bounded by `max_len`.
pub fn read_byte_array<B: Buf>(buf: &mut B, max_len: usize) -> Result<Vec<u8>, ProtocolError> {
    let raw_len = read_var_int(buf)?;
    if raw_len < 0 || (raw_len as usize) > max_len {
        return Err(ProtocolError::InvalidLength {
            len: raw_len,
            max: max_len,
        });
    }
    let len = raw_len as usize;
    ensure_rem(buf, len)?;
    let mut out = vec![0_u8; len];
    buf.copy_to_slice(&mut out);
    Ok(out)
}

/// Writes a `VarInt`-prefixed slice of `Encode` elements.
pub fn write_prefixed_vec<B: BufMut, T: Encode>(
    buf: &mut B,
    items: &[T],
    max_len: usize,
) -> Result<(), ProtocolError> {
    if items.len() > max_len {
        return Err(ProtocolError::InvalidLength {
            len: items.len() as i32,
            max: max_len,
        });
    }
    write_var_int(buf, items.len() as i32);
    for item in items {
        item.encode(buf)?;
    }
    Ok(())
}

/// Reads a `VarInt`-prefixed `Vec<T>` bounded by `max_len`, capping initial capacity to
/// `buf.remaining()` so forged length headers cannot pre-allocate gigabytes.
pub fn read_prefixed_vec<B: Buf, T: Decode>(
    buf: &mut B,
    max_len: usize,
) -> Result<Vec<T>, ProtocolError> {
    let raw_len = read_var_int(buf)?;
    if raw_len < 0 || (raw_len as usize) > max_len {
        return Err(ProtocolError::InvalidLength {
            len: raw_len,
            max: max_len,
        });
    }
    let len = raw_len as usize;
    let init_cap = len.min(buf.remaining()).min(65_536);
    let mut out = Vec::with_capacity(init_cap);
    for _ in 0..len {
        out.push(T::decode(buf)?);
    }
    Ok(out)
}

impl Encode for bool {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_u8(u8::from(*self));
        Ok(())
    }
}

impl Decode for bool {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 1)?;
        match buf.get_u8() {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "bool",
                value: i32::from(other),
            }),
        }
    }
}

impl Encode for i8 {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_i8(*self);
        Ok(())
    }
}

impl Decode for i8 {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 1)?;
        Ok(buf.get_i8())
    }
}

impl Encode for u8 {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_u8(*self);
        Ok(())
    }
}

impl Decode for u8 {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 1)?;
        Ok(buf.get_u8())
    }
}

impl Encode for i16 {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_i16(*self);
        Ok(())
    }
}

impl Decode for i16 {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 2)?;
        Ok(buf.get_i16())
    }
}

impl Encode for u16 {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_u16(*self);
        Ok(())
    }
}

impl Decode for u16 {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 2)?;
        Ok(buf.get_u16())
    }
}

impl Encode for i32 {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_i32(*self);
        Ok(())
    }
}

impl Decode for i32 {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 4)?;
        Ok(buf.get_i32())
    }
}

impl Encode for u32 {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_u32(*self);
        Ok(())
    }
}

impl Decode for u32 {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 4)?;
        Ok(buf.get_u32())
    }
}

impl Encode for i64 {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_i64(*self);
        Ok(())
    }
}

impl Decode for i64 {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 8)?;
        Ok(buf.get_i64())
    }
}

impl Encode for u64 {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_u64(*self);
        Ok(())
    }
}

impl Decode for u64 {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 8)?;
        Ok(buf.get_u64())
    }
}

impl Encode for f32 {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_f32(*self);
        Ok(())
    }
}

impl Decode for f32 {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 4)?;
        Ok(buf.get_f32())
    }
}

impl Encode for f64 {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_f64(*self);
        Ok(())
    }
}

impl Decode for f64 {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 8)?;
        Ok(buf.get_f64())
    }
}

impl Encode for String {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, self, DEFAULT_MAX_STRING_CHARS)
    }
}

impl Decode for String {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        read_string(buf, DEFAULT_MAX_STRING_CHARS)
    }
}

impl Encode for Uuid {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_u128(self.as_u128());
        Ok(())
    }
}

impl Decode for Uuid {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 16)?;
        Ok(Self::from_u128(buf.get_u128()))
    }
}

impl Encode for Key {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, self.as_str(), DEFAULT_MAX_STRING_CHARS)
    }
}

impl Decode for Key {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let s = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        Ok(Self::parse(&s)?)
    }
}

impl Encode for BlockVec {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        buf.put_i64(self.to_packed_i64());
        Ok(())
    }
}

impl Decode for BlockVec {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        ensure_rem(buf, 8)?;
        Ok(Self::from_packed_i64(buf.get_i64()))
    }
}

impl Encode for NbtTag {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        minestom_nbt::write_network(buf, self)?;
        Ok(())
    }
}

impl Decode for NbtTag {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(minestom_nbt::read_network(buf)?)
    }
}

impl Encode for NbtCompound {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        minestom_nbt::write_network_compound(buf, self)?;
        Ok(())
    }
}

impl Decode for NbtCompound {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(minestom_nbt::read_network_compound(buf)?)
    }
}

impl Encode for Component {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        minestom_nbt::write_network_compound(buf, &self.to_nbt_compound())?;
        Ok(())
    }
}

impl Decode for Component {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let tag = minestom_nbt::read_network(buf)?;
        Ok(Self::from_nbt(&tag)?)
    }
}

impl<T: Encode> Encode for Option<T> {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        match self {
            None => false.encode(buf),
            Some(v) => {
                true.encode(buf)?;
                v.encode(buf)
            }
        }
    }
}

impl<T: Decode> Decode for Option<T> {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        if bool::decode(buf)? {
            Ok(Some(T::decode(buf)?))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn varint_roundtrip(val in any::<i32>()) {
            let mut buf = BytesMut::new();
            VarInt(val).encode(&mut buf).unwrap();
            prop_assert_eq!(buf.len(), var_int_len(val));
            let mut reader = buf.freeze();
            let decoded = VarInt::decode(&mut reader).unwrap();
            prop_assert_eq!(decoded.0, val);
            prop_assert_eq!(reader.remaining(), 0);
        }

        #[test]
        fn varlong_roundtrip(val in any::<i64>()) {
            let mut buf = BytesMut::new();
            VarLong(val).encode(&mut buf).unwrap();
            let mut reader = buf.freeze();
            let decoded = VarLong::decode(&mut reader).unwrap();
            prop_assert_eq!(decoded.0, val);
            prop_assert_eq!(reader.remaining(), 0);
        }

        #[test]
        fn optional_varint_roundtrip(val in any::<Option<i32>>()) {
            let mut buf = BytesMut::new();
            OptionalVarInt(val).encode(&mut buf).unwrap();
            let mut reader = buf.freeze();
            let decoded = OptionalVarInt::decode(&mut reader).unwrap();
            prop_assert_eq!(decoded.0, val);
        }

        #[test]
        fn string_roundtrip(s in ".{0,256}") {
            let mut buf = BytesMut::new();
            write_string(&mut buf, &s, 256).unwrap();
            let mut reader = buf.freeze();
            let decoded = read_string(&mut reader, 256).unwrap();
            prop_assert_eq!(decoded, s);
            prop_assert_eq!(reader.remaining(), 0);
        }

        #[test]
        fn arbitrary_bytes_never_panic_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let mut s1 = raw.as_slice();
            let _ = VarInt::decode(&mut s1);
            let mut s2 = raw.as_slice();
            let _ = VarLong::decode(&mut s2);
            let mut s3 = raw.as_slice();
            let _ = read_string(&mut s3, 255);
            let mut s4 = raw.as_slice();
            let _ = BlockVec::decode(&mut s4);
            let mut s5 = raw.as_slice();
            let _ = Component::decode(&mut s5);
        }
    }

    #[test]
    fn six_byte_varint_rejected() {
        let bad = [0x80_u8, 0x80, 0x80, 0x80, 0x80, 0x01];
        let mut reader = bad.as_slice();
        assert!(matches!(
            VarInt::decode(&mut reader),
            Err(ProtocolError::VarIntTooLarge)
        ));
    }
}
