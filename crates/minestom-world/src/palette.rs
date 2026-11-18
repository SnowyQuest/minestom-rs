//! Vanilla-compatible 3D `Palette` container (single-value, indirect, and direct modes)
//! matching MC 26.2 (protocol 776) non-spanning 64-bit word layout.

use bytes::{Buf, BufMut};
use minestom_data::generated::dynamic_registries::{BIOME_COUNT, PLAINS_BIOME_ID};
use minestom_protocol::{ProtocolError, read_var_int, write_var_int};
use smallvec::SmallVec;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PaletteStorage {
    Single(u32),
    Indirect {
        bits_per_entry: u8,
        palette: SmallVec<[u32; 16]>,
        data: Box<[u64]>,
    },
    Direct {
        bits_per_entry: u8,
        data: Box<[u64]>,
    },
}

/// Minimum bits needed to represent `max_val` (`>= 1`).
#[must_use]
pub const fn bits_to_represent(max_val: usize) -> u8 {
    if max_val <= 1 {
        1
    } else {
        max_val.bit_width() as u8
    }
}

const fn word_count(total_entries: usize, bits_per_entry: u8) -> usize {
    let per_word = 64 / (bits_per_entry as usize);
    total_entries.div_ceil(per_word)
}

fn get_packed(data: &[u64], index: usize, bpe: u8) -> u32 {
    let per_word = 64 / usize::from(bpe);
    let word_idx = index / per_word;
    let bit_offset = (index % per_word) * usize::from(bpe);
    let mask = (1_u64 << bpe) - 1;
    ((data[word_idx] >> bit_offset) & mask) as u32
}

fn set_packed(data: &mut [u64], index: usize, bpe: u8, raw_val: u32) -> u32 {
    let per_word = 64 / usize::from(bpe);
    let word_idx = index / per_word;
    let bit_offset = (index % per_word) * usize::from(bpe);
    let mask = (1_u64 << bpe) - 1;
    let old = ((data[word_idx] >> bit_offset) & mask) as u32;
    data[word_idx] =
        (data[word_idx] & !(mask << bit_offset)) | ((u64::from(raw_val) & mask) << bit_offset);
    old
}

/// Compressed 3D paletted container for 16×16×16 blocks or 4×4×4 biomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    dimension: u8,
    min_indirect_bits: u8,
    max_indirect_bits: u8,
    direct_bits: u8,
    storage: PaletteStorage,
}

impl Palette {
    /// Creates an empty (`air = 0`) 16×16×16 block palette (`min=4, max=8, direct=15`).
    #[must_use]
    pub const fn blocks() -> Self {
        Self {
            dimension: 16,
            min_indirect_bits: 4,
            max_indirect_bits: 8,
            direct_bits: 15,
            storage: PaletteStorage::Single(0),
        }
    }

    /// Creates a 4×4×4 biome palette filled with `minecraft:plains`.
    #[must_use]
    pub const fn biomes() -> Self {
        let direct_bits = bits_to_represent(BIOME_COUNT.saturating_sub(1));
        Self {
            dimension: 4,
            min_indirect_bits: 1,
            max_indirect_bits: 3,
            direct_bits,
            storage: PaletteStorage::Single(PLAINS_BIOME_ID),
        }
    }

    /// Side length of this cubic container (`16` for blocks, `4` for biomes).
    #[must_use]
    pub const fn dimension(&self) -> usize {
        self.dimension as usize
    }

    /// Total number of cells (`4096` for blocks, `64` for biomes).
    #[must_use]
    pub const fn max_size(&self) -> usize {
        let d = self.dimension();
        d * d * d
    }

    const fn linear_index(&self, x: usize, y: usize, z: usize) -> usize {
        let d = self.dimension();
        (y * d + z) * d + x
    }

    /// Fills the entire container with `value` in `O(1)` using single-value mode (`0 bpe`).
    pub fn fill(&mut self, value: u32) {
        self.storage = PaletteStorage::Single(value);
    }

    /// Returns `Some(val)` if this palette is currently in `0 bpe` single-value mode.
    #[must_use]
    pub const fn single_value(&self) -> Option<u32> {
        if let PaletteStorage::Single(v) = self.storage {
            Some(v)
        } else {
            None
        }
    }

    /// Reads the entry at local coordinates `(x, y, z)`.
    #[must_use]
    pub fn get(&self, x: usize, y: usize, z: usize) -> u32 {
        let idx = self.linear_index(x, y, z);
        match &self.storage {
            PaletteStorage::Single(v) => *v,
            PaletteStorage::Indirect {
                bits_per_entry,
                palette,
                data,
            } => {
                let p_idx = get_packed(data, idx, *bits_per_entry) as usize;
                palette.get(p_idx).copied().unwrap_or(0)
            }
            PaletteStorage::Direct {
                bits_per_entry,
                data,
            } => get_packed(data, idx, *bits_per_entry),
        }
    }

    /// Sets the entry at local coordinates `(x, y, z)` to `value`, returning the previous value.
    pub fn set(&mut self, x: usize, y: usize, z: usize, value: u32) -> u32 {
        let idx = self.linear_index(x, y, z);
        let total = self.max_size();

        if let PaletteStorage::Single(single) = self.storage {
            if single == value {
                return single;
            }
            let bpe = self.min_indirect_bits;
            let mut palette = SmallVec::new();
            palette.push(single);
            palette.push(value);
            let mut data = vec![0_u64; word_count(total, bpe)].into_boxed_slice();
            set_packed(&mut data, idx, bpe, 1);
            self.storage = PaletteStorage::Indirect {
                bits_per_entry: bpe,
                palette,
                data,
            };
            return single;
        }

        // Check if indirect needs upsizing
        if let PaletteStorage::Indirect {
            bits_per_entry,
            palette,
            data,
        } = &mut self.storage
        {
            if let Some(p_idx) = palette.iter().position(|&v| v == value) {
                let old_p = set_packed(data, idx, *bits_per_entry, p_idx as u32) as usize;
                return palette.get(old_p).copied().unwrap_or(0);
            }

            let max_len = 1_usize << (*bits_per_entry as usize);
            if palette.len() < max_len {
                let new_p_idx = palette.len() as u32;
                palette.push(value);
                let old_p = set_packed(data, idx, *bits_per_entry, new_p_idx) as usize;
                return palette.get(old_p).copied().unwrap_or(0);
            }

            // Upsize indirect -> wider indirect or direct
            let old_bpe = *bits_per_entry;
            let next_bpe = old_bpe + 1;
            if next_bpe <= self.max_indirect_bits {
                let mut new_data = vec![0_u64; word_count(total, next_bpe)].into_boxed_slice();
                for i in 0..total {
                    let p = get_packed(data, i, old_bpe);
                    set_packed(&mut new_data, i, next_bpe, p);
                }
                let new_p_idx = palette.len() as u32;
                palette.push(value);
                let old_p = set_packed(&mut new_data, idx, next_bpe, new_p_idx) as usize;
                let old_val = palette.get(old_p).copied().unwrap_or(0);
                *bits_per_entry = next_bpe;
                *data = new_data;
                return old_val;
            }

            // Promote to Direct
            let dir_bpe = self.direct_bits;
            let mut dir_data = vec![0_u64; word_count(total, dir_bpe)].into_boxed_slice();
            for i in 0..total {
                let p = get_packed(data, i, old_bpe) as usize;
                let global_val = palette.get(p).copied().unwrap_or(0);
                set_packed(&mut dir_data, i, dir_bpe, global_val);
            }
            let old_val = set_packed(&mut dir_data, idx, dir_bpe, value);
            self.storage = PaletteStorage::Direct {
                bits_per_entry: dir_bpe,
                data: dir_data,
            };
            return old_val;
        }

        if let PaletteStorage::Direct {
            bits_per_entry,
            data,
        } = &mut self.storage
        {
            return set_packed(data, idx, *bits_per_entry, value);
        }

        0
    }

    /// Serializes this palette in exact MC 26.2 wire format (`Palette.java` lines 246–255:
    /// `RAW_LONGS` without a `VarInt` array-length prefix!).
    pub fn write_network<B: BufMut>(&self, buf: &mut B) {
        match &self.storage {
            PaletteStorage::Single(val) => {
                buf.put_u8(0);
                write_var_int(buf, *val as i32);
            }
            PaletteStorage::Indirect {
                bits_per_entry,
                palette,
                data,
            } => {
                buf.put_u8(*bits_per_entry);
                write_var_int(buf, palette.len() as i32);
                for &entry in palette {
                    write_var_int(buf, entry as i32);
                }
                for &word in data {
                    buf.put_u64(word);
                }
            }
            PaletteStorage::Direct {
                bits_per_entry,
                data,
            } => {
                buf.put_u8(*bits_per_entry);
                for &word in data {
                    buf.put_u64(word);
                }
            }
        }
    }

    /// Deserializes a `Palette` with identical dimension/bit bounds from MC 26.2 wire bytes.
    pub fn read_network<B: Buf>(
        buf: &mut B,
        dimension: u8,
        min_indirect_bits: u8,
        max_indirect_bits: u8,
        direct_bits: u8,
    ) -> Result<Self, ProtocolError> {
        if buf.remaining() < 1 {
            return Err(ProtocolError::UnexpectedEof);
        }
        let bpe = buf.get_u8();
        let total = usize::from(dimension).pow(3);
        if bpe == 0 {
            let val = read_var_int(buf)? as u32;
            return Ok(Self {
                dimension,
                min_indirect_bits,
                max_indirect_bits,
                direct_bits,
                storage: PaletteStorage::Single(val),
            });
        }
        if bpe < min_indirect_bits || (bpe > max_indirect_bits && bpe != direct_bits) {
            return Err(ProtocolError::InvalidDiscriminant {
                type_name: "Palette.bitsPerEntry",
                value: i32::from(bpe),
            });
        }

        let palette_opt = if bpe <= max_indirect_bits {
            let p_len = read_var_int(buf)?;
            let max_p = 1_usize << usize::from(bpe);
            if p_len <= 0 || (p_len as usize) > max_p {
                return Err(ProtocolError::InvalidLength {
                    len: p_len,
                    max: max_p,
                });
            }
            let mut p = SmallVec::with_capacity(p_len as usize);
            for _ in 0..p_len {
                p.push(read_var_int(buf)? as u32);
            }
            Some(p)
        } else {
            None
        };

        let words_len = word_count(total, bpe);
        if buf.remaining() < words_len * 8 {
            return Err(ProtocolError::UnexpectedEof);
        }
        let mut words = Vec::with_capacity(words_len);
        for _ in 0..words_len {
            words.push(buf.get_u64());
        }

        let storage = if let Some(palette) = palette_opt {
            PaletteStorage::Indirect {
                bits_per_entry: bpe,
                palette,
                data: words.into_boxed_slice(),
            }
        } else {
            PaletteStorage::Direct {
                bits_per_entry: bpe,
                data: words.into_boxed_slice(),
            }
        };

        Ok(Self {
            dimension,
            min_indirect_bits,
            max_indirect_bits,
            direct_bits,
            storage,
        })
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;

    use super::*;

    #[test]
    fn palette_single_indirect_and_direct_roundtrip() {
        let mut p = Palette::blocks();
        assert_eq!(p.get(0, 0, 0), 0);

        // Transition Single -> Indirect -> Direct (> 256 unique states)
        for i in 0..300_u32 {
            let x = (i as usize) % 16;
            let z = ((i as usize) / 16) % 16;
            let y = (i as usize) / 256;
            p.set(x, y, z, i + 1);
        }

        for i in 0..300_u32 {
            let x = (i as usize) % 16;
            let z = ((i as usize) / 16) % 16;
            let y = (i as usize) / 256;
            assert_eq!(p.get(x, y, z), i + 1);
        }

        let mut buf = BytesMut::new();
        p.write_network(&mut buf);
        let mut reader = buf.freeze();
        let decoded = Palette::read_network(&mut reader, 16, 4, 8, 15).unwrap();
        assert_eq!(reader.remaining(), 0);
        for y in 0..2 {
            for z in 0..16 {
                for x in 0..16 {
                    assert_eq!(decoded.get(x, y, z), p.get(x, y, z));
                }
            }
        }
    }
}
