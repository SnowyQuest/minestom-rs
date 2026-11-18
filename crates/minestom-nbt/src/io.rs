//! Binary NBT reader and writer (named disk format, nameless network format, GZip, Zlib).

use std::io::{Read, Write};

use bytes::{Buf, BufMut, BytesMut};
use flate2::Compression;
use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::{GzEncoder, ZlibEncoder};

use crate::{NbtCompound, NbtError, NbtTag, TagType};

/// Default maximum recursion depth when decoding NBT (`512`, matching vanilla & Minestom).
pub const DEFAULT_MAX_NBT_DEPTH: usize = 512;

/// Default maximum accounted heap bytes when decoding untrusted network NBT (`2 MiB`).
pub const DEFAULT_MAX_NBT_BYTES: usize = 2 * 1024 * 1024;

struct ReadLimiter {
    remaining: usize,
    max: usize,
    max_depth: usize,
}

impl ReadLimiter {
    const fn new(max: usize, max_depth: usize) -> Self {
        Self {
            remaining: max,
            max,
            max_depth,
        }
    }

    fn charge(&mut self, bytes: usize) -> Result<(), NbtError> {
        if bytes > self.remaining {
            return Err(NbtError::AllocationLimitExceeded {
                size: self
                    .max
                    .saturating_sub(self.remaining)
                    .saturating_add(bytes),
                max: self.max,
            });
        }
        self.remaining -= bytes;
        Ok(())
    }

    const fn check_depth(&self, depth: usize) -> Result<(), NbtError> {
        if depth > self.max_depth {
            Err(NbtError::DepthLimitExceeded(self.max_depth))
        } else {
            Ok(())
        }
    }
}

fn ensure_rem<B: Buf>(buf: &B, n: usize) -> Result<(), NbtError> {
    if buf.remaining() < n {
        Err(NbtError::UnexpectedEof)
    } else {
        Ok(())
    }
}

fn decode_mutf8_fallback(bytes: &[u8]) -> Result<String, NbtError> {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    let mut high_surrogate: Option<u16> = None;
    while i < bytes.len() {
        let b0 = bytes[i];
        if b0 < 0x80 {
            if high_surrogate.is_some() {
                return Err(NbtError::InvalidUtf8);
            }
            out.push(char::from(b0));
            i += 1;
        } else if (b0 & 0xE0) == 0xC0 {
            if i + 1 >= bytes.len() || high_surrogate.is_some() {
                return Err(NbtError::InvalidUtf8);
            }
            let b1 = bytes[i + 1];
            if (b1 & 0xC0) != 0x80 {
                return Err(NbtError::InvalidUtf8);
            }
            let cp = (u32::from(b0 & 0x1F) << 6) | u32::from(b1 & 0x3F);
            if cp == 0 && b0 == 0xC0 && b1 == 0x80 {
                out.push('\0');
            } else if cp >= 0x80
                && let Some(ch) = char::from_u32(cp)
            {
                out.push(ch);
            } else {
                return Err(NbtError::InvalidUtf8);
            }
            i += 2;
        } else if (b0 & 0xF0) == 0xE0 {
            if i + 2 >= bytes.len() {
                return Err(NbtError::InvalidUtf8);
            }
            let b1 = bytes[i + 1];
            let b2 = bytes[i + 2];
            if (b1 & 0xC0) != 0x80 || (b2 & 0xC0) != 0x80 {
                return Err(NbtError::InvalidUtf8);
            }
            let cp =
                (u32::from(b0 & 0x0F) << 12) | (u32::from(b1 & 0x3F) << 6) | u32::from(b2 & 0x3F);
            if (0xD800..=0xDBFF).contains(&cp) {
                if high_surrogate.is_some() {
                    return Err(NbtError::InvalidUtf8);
                }
                high_surrogate = Some(cp as u16);
            } else if (0xDC00..=0xDFFF).contains(&cp) {
                let Some(hi) = high_surrogate.take() else {
                    return Err(NbtError::InvalidUtf8);
                };
                let full = 0x1_0000 + ((u32::from(hi - 0xD800)) << 10) + (cp - 0xDC00);
                let ch = char::from_u32(full).ok_or(NbtError::InvalidUtf8)?;
                out.push(ch);
            } else if cp >= 0x800
                && let Some(ch) = char::from_u32(cp)
            {
                if high_surrogate.is_some() {
                    return Err(NbtError::InvalidUtf8);
                }
                out.push(ch);
            } else {
                return Err(NbtError::InvalidUtf8);
            }
            i += 3;
        } else if (b0 & 0xF8) == 0xF0 {
            if i + 3 >= bytes.len() || high_surrogate.is_some() {
                return Err(NbtError::InvalidUtf8);
            }
            let s = std::str::from_utf8(&bytes[i..i + 4]).map_err(|_| NbtError::InvalidUtf8)?;
            out.push_str(s);
            i += 4;
        } else {
            return Err(NbtError::InvalidUtf8);
        }
    }
    if high_surrogate.is_some() {
        return Err(NbtError::InvalidUtf8);
    }
    Ok(out)
}

fn read_nbt_string<B: Buf>(buf: &mut B, limiter: &mut ReadLimiter) -> Result<String, NbtError> {
    ensure_rem(buf, 2)?;
    let len = usize::from(buf.get_u16());
    ensure_rem(buf, len)?;
    limiter.charge(24 + len)?;
    let mut raw = vec![0_u8; len];
    buf.copy_to_slice(&mut raw);
    match String::from_utf8(raw) {
        Ok(s) => Ok(s),
        Err(err) => decode_mutf8_fallback(err.as_bytes()),
    }
}

fn write_nbt_string<B: BufMut>(buf: &mut B, s: &str) -> Result<(), NbtError> {
    let bytes = s.as_bytes();
    if bytes.len() > usize::from(u16::MAX) {
        return Err(NbtError::AllocationLimitExceeded {
            size: bytes.len(),
            max: usize::from(u16::MAX),
        });
    }
    buf.put_u16(bytes.len() as u16);
    buf.put_slice(bytes);
    Ok(())
}

fn read_payload<B: Buf>(
    buf: &mut B,
    tag_type: TagType,
    depth: usize,
    limiter: &mut ReadLimiter,
) -> Result<NbtTag, NbtError> {
    limiter.check_depth(depth)?;
    limiter.charge(8)?;
    match tag_type {
        TagType::End => Ok(NbtTag::End),
        TagType::Byte => {
            ensure_rem(buf, 1)?;
            Ok(NbtTag::Byte(buf.get_i8()))
        }
        TagType::Short => {
            ensure_rem(buf, 2)?;
            Ok(NbtTag::Short(buf.get_i16()))
        }
        TagType::Int => {
            ensure_rem(buf, 4)?;
            Ok(NbtTag::Int(buf.get_i32()))
        }
        TagType::Long => {
            ensure_rem(buf, 8)?;
            Ok(NbtTag::Long(buf.get_i64()))
        }
        TagType::Float => {
            ensure_rem(buf, 4)?;
            Ok(NbtTag::Float(buf.get_f32()))
        }
        TagType::Double => {
            ensure_rem(buf, 8)?;
            Ok(NbtTag::Double(buf.get_f64()))
        }
        TagType::ByteArray => {
            ensure_rem(buf, 4)?;
            let raw_len = buf.get_i32();
            if raw_len < 0 {
                return Err(NbtError::NegativeLength(raw_len));
            }
            let len = raw_len as usize;
            ensure_rem(buf, len)?;
            limiter.charge(24 + len)?;
            let mut out = Vec::with_capacity(len);
            for _ in 0..len {
                out.push(buf.get_i8());
            }
            Ok(NbtTag::ByteArray(out))
        }
        TagType::String => Ok(NbtTag::String(read_nbt_string(buf, limiter)?)),
        TagType::List => {
            ensure_rem(buf, 5)?;
            let elem_type = TagType::from_id(buf.get_u8())?;
            let raw_len = buf.get_i32();
            if raw_len < 0 {
                return Err(NbtError::NegativeLength(raw_len));
            }
            let len = raw_len as usize;
            if len == 0 {
                return Ok(NbtTag::List(Vec::new()));
            }
            if elem_type == TagType::End {
                return Err(NbtError::InvalidListElementType);
            }
            let min_bytes = len
                .checked_mul(elem_type.min_wire_bytes())
                .ok_or(NbtError::UnexpectedEof)?;
            ensure_rem(buf, min_bytes)?;
            limiter.charge(len.saturating_mul(8))?;
            let mut items = Vec::with_capacity(len.min(65_536));
            for _ in 0..len {
                let elem = read_payload(buf, elem_type, depth + 1, limiter)?;
                if elem_type == TagType::Compound
                    && let NbtTag::Compound(mut c) = elem
                {
                    if c.len() == 1 && c.contains_key("") {
                        items.push(c.remove("").unwrap_or(NbtTag::End));
                    } else {
                        items.push(NbtTag::Compound(c));
                    }
                } else {
                    items.push(elem);
                }
            }
            Ok(NbtTag::List(items))
        }
        TagType::Compound => {
            let mut compound = NbtCompound::new();
            loop {
                ensure_rem(buf, 1)?;
                let child_type = TagType::from_id(buf.get_u8())?;
                if child_type == TagType::End {
                    break;
                }
                let key = read_nbt_string(buf, limiter)?;
                limiter.charge(32)?;
                let val = read_payload(buf, child_type, depth + 1, limiter)?;
                compound.insert(key, val);
            }
            Ok(NbtTag::Compound(compound))
        }
        TagType::IntArray => {
            ensure_rem(buf, 4)?;
            let raw_len = buf.get_i32();
            if raw_len < 0 {
                return Err(NbtError::NegativeLength(raw_len));
            }
            let len = raw_len as usize;
            let byte_len = len.checked_mul(4).ok_or(NbtError::UnexpectedEof)?;
            ensure_rem(buf, byte_len)?;
            limiter.charge(24 + byte_len)?;
            let mut out = Vec::with_capacity(len);
            for _ in 0..len {
                out.push(buf.get_i32());
            }
            Ok(NbtTag::IntArray(out))
        }
        TagType::LongArray => {
            ensure_rem(buf, 4)?;
            let raw_len = buf.get_i32();
            if raw_len < 0 {
                return Err(NbtError::NegativeLength(raw_len));
            }
            let len = raw_len as usize;
            let byte_len = len.checked_mul(8).ok_or(NbtError::UnexpectedEof)?;
            ensure_rem(buf, byte_len)?;
            limiter.charge(24 + byte_len)?;
            let mut out = Vec::with_capacity(len);
            for _ in 0..len {
                out.push(buf.get_i64());
            }
            Ok(NbtTag::LongArray(out))
        }
    }
}

fn write_compound_payload<B: BufMut>(
    buf: &mut B,
    map: &NbtCompound,
    depth: usize,
) -> Result<(), NbtError> {
    if depth > DEFAULT_MAX_NBT_DEPTH {
        return Err(NbtError::DepthLimitExceeded(DEFAULT_MAX_NBT_DEPTH));
    }
    for (k, v) in map {
        if *v == NbtTag::End {
            continue;
        }
        buf.put_u8(v.tag_type().id());
        write_nbt_string(buf, k)?;
        write_payload(buf, v, depth + 1)?;
    }
    buf.put_u8(TagType::End.id());
    Ok(())
}

fn write_payload<B: BufMut>(buf: &mut B, tag: &NbtTag, depth: usize) -> Result<(), NbtError> {
    if depth > DEFAULT_MAX_NBT_DEPTH {
        return Err(NbtError::DepthLimitExceeded(DEFAULT_MAX_NBT_DEPTH));
    }
    match tag {
        NbtTag::End => {}
        NbtTag::Byte(v) => buf.put_i8(*v),
        NbtTag::Short(v) => buf.put_i16(*v),
        NbtTag::Int(v) => buf.put_i32(*v),
        NbtTag::Long(v) => buf.put_i64(*v),
        NbtTag::Float(v) => buf.put_f32(*v),
        NbtTag::Double(v) => buf.put_f64(*v),
        NbtTag::ByteArray(items) => {
            buf.put_i32(items.len() as i32);
            for &b in items {
                buf.put_i8(b);
            }
        }
        NbtTag::String(s) => write_nbt_string(buf, s)?,
        NbtTag::List(items) => {
            if items.is_empty() {
                buf.put_u8(TagType::End.id());
                buf.put_i32(0);
            } else {
                let first_type = items[0].tag_type();
                let homogeneous = first_type != TagType::End
                    && items.iter().all(|item| item.tag_type() == first_type);
                if homogeneous {
                    buf.put_u8(first_type.id());
                    buf.put_i32(items.len() as i32);
                    for item in items {
                        write_payload(buf, item, depth + 1)?;
                    }
                } else {
                    // Wrap heterogeneous list entries into `{ "": item }` compounds per MC 1.21.5+ / 26.2.
                    buf.put_u8(TagType::Compound.id());
                    buf.put_i32(items.len() as i32);
                    for item in items {
                        if let NbtTag::Compound(c) = item
                            && !c.contains_key("")
                        {
                            write_payload(buf, item, depth + 1)?;
                        } else if *item != NbtTag::End {
                            buf.put_u8(item.tag_type().id());
                            write_nbt_string(buf, "")?;
                            write_payload(buf, item, depth + 1)?;
                            buf.put_u8(TagType::End.id());
                        } else {
                            buf.put_u8(TagType::End.id());
                        }
                    }
                }
            }
        }
        NbtTag::Compound(map) => write_compound_payload(buf, map, depth)?,
        NbtTag::IntArray(items) => {
            buf.put_i32(items.len() as i32);
            for &v in items {
                buf.put_i32(v);
            }
        }
        NbtTag::LongArray(items) => {
            buf.put_i32(items.len() as i32);
            for &v in items {
                buf.put_i64(v);
            }
        }
    }
    Ok(())
}

/// Reads a nameless-root network [`NbtTag`] using custom byte and depth limits.
pub fn read_network_limited<B: Buf>(
    buf: &mut B,
    max_bytes: usize,
    max_depth: usize,
) -> Result<NbtTag, NbtError> {
    ensure_rem(buf, 1)?;
    let root_type = TagType::from_id(buf.get_u8())?;
    if root_type == TagType::End {
        return Ok(NbtTag::End);
    }
    let mut limiter = ReadLimiter::new(max_bytes, max_depth);
    read_payload(buf, root_type, 1, &mut limiter)
}

/// Reads a nameless-root network [`NbtTag`] with default untrusted safety limits (`2 MiB`, depth `512`).
pub fn read_network<B: Buf>(buf: &mut B) -> Result<NbtTag, NbtError> {
    read_network_limited(buf, DEFAULT_MAX_NBT_BYTES, DEFAULT_MAX_NBT_DEPTH)
}

/// Reads a nameless-root network [`NbtCompound`], returning [`NbtError::ExpectedCompound`] if not `0x0A`.
pub fn read_network_compound<B: Buf>(buf: &mut B) -> Result<NbtCompound, NbtError> {
    match read_network(buf)? {
        NbtTag::Compound(c) => Ok(c),
        other => Err(NbtError::ExpectedCompound(other.tag_type())),
    }
}

/// Writes a nameless-root network [`NbtTag`] (`[type_id][payload]`).
pub fn write_network<B: BufMut>(buf: &mut B, tag: &NbtTag) -> Result<(), NbtError> {
    buf.put_u8(tag.tag_type().id());
    write_payload(buf, tag, 1)
}

/// Writes a nameless-root network [`NbtCompound`] (`0x0A` + entries + `0x00`).
pub fn write_network_compound<B: BufMut>(
    buf: &mut B,
    compound: &NbtCompound,
) -> Result<(), NbtError> {
    buf.put_u8(TagType::Compound.id());
    write_compound_payload(buf, compound, 1)
}

/// Reads a standard disk-format named-root NBT compound (`[0x0A][u16 name_len][name][payload]`).
pub fn read_named<B: Buf>(buf: &mut B) -> Result<(String, NbtCompound), NbtError> {
    ensure_rem(buf, 1)?;
    let root_type = TagType::from_id(buf.get_u8())?;
    if root_type != TagType::Compound {
        return Err(NbtError::ExpectedCompound(root_type));
    }
    let mut limiter = ReadLimiter::new(64 * 1024 * 1024, DEFAULT_MAX_NBT_DEPTH);
    let root_name = read_nbt_string(buf, &mut limiter)?;
    match read_payload(buf, TagType::Compound, 1, &mut limiter)? {
        NbtTag::Compound(c) => Ok((root_name, c)),
        other => Err(NbtError::ExpectedCompound(other.tag_type())),
    }
}

/// Writes a standard disk-format named-root NBT compound without cloning `compound`.
pub fn write_named<B: BufMut>(
    buf: &mut B,
    root_name: &str,
    compound: &NbtCompound,
) -> Result<(), NbtError> {
    buf.put_u8(TagType::Compound.id());
    write_nbt_string(buf, root_name)?;
    write_compound_payload(buf, compound, 1)
}

/// Reads a GZip-compressed named-root NBT compound.
pub fn read_gzip_named(bytes: &[u8]) -> Result<(String, NbtCompound), NbtError> {
    let mut decoder = GzDecoder::new(bytes).take(64 * 1024 * 1024);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw)?;
    let mut slice = raw.as_slice();
    read_named(&mut slice)
}

/// Writes a GZip-compressed named-root NBT compound.
pub fn write_gzip_named(root_name: &str, compound: &NbtCompound) -> Result<Vec<u8>, NbtError> {
    let mut uncompressed = BytesMut::new();
    write_named(&mut uncompressed, root_name, compound)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&uncompressed)?;
    Ok(encoder.finish()?)
}

/// Reads a Zlib-compressed named-root NBT compound (used in Anvil `.mca` sectors).
pub fn read_zlib_named(bytes: &[u8]) -> Result<(String, NbtCompound), NbtError> {
    let mut decoder = ZlibDecoder::new(bytes).take(64 * 1024 * 1024);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw)?;
    let mut slice = raw.as_slice();
    read_named(&mut slice)
}

/// Writes a Zlib-compressed named-root NBT compound (used in Anvil `.mca` sectors).
pub fn write_zlib_named(root_name: &str, compound: &NbtCompound) -> Result<Vec<u8>, NbtError> {
    let mut uncompressed = BytesMut::new();
    write_named(&mut uncompressed, root_name, compound)?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&uncompressed)?;
    Ok(encoder.finish()?)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn arb_nbt_tag() -> impl Strategy<Value = NbtTag> {
        let leaf = prop_oneof![
            any::<i8>().prop_map(NbtTag::Byte),
            any::<i16>().prop_map(NbtTag::Short),
            any::<i32>().prop_map(NbtTag::Int),
            any::<i64>().prop_map(NbtTag::Long),
            any::<f32>()
                .prop_filter("non-NaN float", |f| !f.is_nan())
                .prop_map(NbtTag::Float),
            any::<f64>()
                .prop_filter("non-NaN double", |d| !d.is_nan())
                .prop_map(NbtTag::Double),
            prop::collection::vec(any::<i8>(), 0..32).prop_map(NbtTag::ByteArray),
            "[a-zA-Z0-9_:/.-]{0,32}".prop_map(NbtTag::String),
            prop::collection::vec(any::<i32>(), 0..16).prop_map(NbtTag::IntArray),
            prop::collection::vec(any::<i64>(), 0..16).prop_map(NbtTag::LongArray),
        ];

        leaf.prop_recursive(4, 64, 8, |inner| {
            prop_oneof![
                // Homogeneous int list
                prop::collection::vec(any::<i32>().prop_map(NbtTag::Int), 0..8)
                    .prop_map(NbtTag::List),
                // Compound
                prop::collection::vec(("[a-z0-9_]{1,12}", inner), 0..8).prop_map(|pairs| {
                    let mut c = NbtCompound::new();
                    for (k, v) in pairs {
                        c.insert(k, v);
                    }
                    NbtTag::Compound(c)
                }),
            ]
        })
    }

    proptest! {
        #[test]
        fn network_nbt_roundtrip(tag in arb_nbt_tag()) {
            let mut buf = BytesMut::new();
            write_network(&mut buf, &tag).expect("encode network NBT");
            let mut reader = buf.freeze();
            let decoded = read_network(&mut reader).expect("decode network NBT");
            prop_assert_eq!(reader.remaining(), 0);
            prop_assert_eq!(decoded, tag);
        }

        #[test]
        fn named_gzip_and_zlib_roundtrip(
            name in "[a-z_]{0,16}",
            pairs in prop::collection::vec(("[a-z0-9_]{1,12}", arb_nbt_tag()), 0..8)
        ) {
            let mut compound = NbtCompound::new();
            for (k, v) in pairs {
                compound.insert(k, v);
            }

            let gz = write_gzip_named(&name, &compound).expect("write gzip");
            let (gz_name, gz_dec) = read_gzip_named(&gz).expect("read gzip");
            prop_assert_eq!(&gz_name, &name);
            prop_assert_eq!(&gz_dec, &compound);

            let zl = write_zlib_named(&name, &compound).expect("write zlib");
            let (zl_name, zl_dec) = read_zlib_named(&zl).expect("read zlib");
            prop_assert_eq!(&zl_name, &name);
            prop_assert_eq!(&zl_dec, &compound);
        }

        #[test]
        fn arbitrary_bytes_never_panic_network_nbt(raw in prop::collection::vec(any::<u8>(), 0..512)) {
            let mut slice = raw.as_slice();
            let _ = read_network(&mut slice);
        }
    }

    #[test]
    fn oversized_claimed_list_rejected_without_oom() {
        // TAG_List (0x09), element = TAG_Long (0x04), length = i32::MAX, no body bytes
        let mut crafted = BytesMut::new();
        crafted.put_u8(TagType::List.id());
        crafted.put_u8(TagType::Long.id());
        crafted.put_i32(i32::MAX);
        let mut reader = crafted.freeze();
        assert!(matches!(
            read_network(&mut reader),
            Err(NbtError::UnexpectedEof)
        ));
    }
}
