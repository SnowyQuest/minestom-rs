//! `ChunkLoader` trait and vanilla `.mca` region `AnvilLoader` (MC 26.2 `dimensions/<ns>/<val>/region`
//! and classic `region/r.<rx>.<rz>.mca` layouts, Zlib/GZip/Uncompressed sector streams, paletted
//! `sections` + `block_entities` + `level.dat` read/write).

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use minestom_block::Block;
use minestom_core::ChunkPos;
use minestom_data::DATA_VERSION;
use minestom_nbt::{
    NbtCompound, NbtTag, read_gzip_named, read_named, read_zlib_named, write_gzip_named,
    write_zlib_named,
};
use minestom_registry::{DimensionType, Registries};
use parking_lot::RwLock;
use rustc_hash::FxHashMap;

use crate::palette::bits_to_represent;
use crate::{Chunk, WorldError};

const SECTOR_BYTES: usize = 4096;
const HEADER_BYTES: usize = SECTOR_BYTES * 2;

/// Pluggable chunk storage backend (`ChunkLoader` in Minestom).
///
/// Implementations are invoked on parallel `rayon` / blocking worker threads rather than Tokio
/// I/O reactor threads.
pub trait ChunkLoader: Send + Sync + 'static {
    /// Attempts to load `pos` from persistent storage (`Ok(None)` if not saved yet).
    fn load_chunk(
        &self,
        dimension: &DimensionType,
        pos: ChunkPos,
    ) -> Result<Option<Chunk>, WorldError>;

    /// Persists `chunk` to storage.
    fn save_chunk(&self, chunk: &Chunk, world_age: i64) -> Result<(), WorldError>;

    /// Loads world-level `level.dat` NBT if present.
    fn load_instance_nbt(&self) -> Result<Option<NbtCompound>, WorldError> {
        Ok(None)
    }

    /// Saves world-level `level.dat` NBT.
    fn save_instance_nbt(&self, _nbt: &NbtCompound) -> Result<(), WorldError> {
        Ok(())
    }
}

type RegionLockMap = FxHashMap<(i32, i32), Arc<RwLock<()>>>;

/// Vanilla Minecraft `.mca` region file loader & saver (`AnvilLoader` in Minestom).
///
/// Supports both:
/// - [`AnvilLoader::with_dimension`] (`<world>/dimensions/<namespace>/<value>/region/r.<rx>.<rz>.mca`, MC 26.1+ / 26.2 layout)
/// - [`AnvilLoader::new`] (`<world>/region/r.<rx>.<rz>.mca`, classic layout)
///
/// Includes automatic read fallback between the two!
pub struct AnvilLoader {
    world_dir: PathBuf,
    primary_region_dir: PathBuf,
    fallback_region_dir: PathBuf,
    region_locks: RwLock<RegionLockMap>,
}

impl std::fmt::Debug for AnvilLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnvilLoader")
            .field("world_dir", &self.world_dir)
            .field("region_dir", &self.primary_region_dir)
            .finish_non_exhaustive()
    }
}

impl AnvilLoader {
    /// Creates an [`AnvilLoader`] targeting `<world_dir>/region` (falling back to
    /// `<world_dir>/dimensions/minecraft/overworld/region` on reads if needed).
    #[must_use]
    pub fn new(world_dir: impl AsRef<Path>) -> Self {
        let root = world_dir.as_ref().to_path_buf();
        let primary = root.join("region");
        let fallback = root
            .join("dimensions")
            .join("minecraft")
            .join("overworld")
            .join("region");
        Self {
            world_dir: root,
            primary_region_dir: primary,
            fallback_region_dir: fallback,
            region_locks: RwLock::new(FxHashMap::default()),
        }
    }

    /// Creates an [`AnvilLoader`] using the MC 26.2 per-dimension path
    /// `<world_dir>/dimensions/<namespace>/<value>/region`.
    #[must_use]
    pub fn with_dimension(world_dir: impl AsRef<Path>, dimension_key: &str) -> Self {
        let root = world_dir.as_ref().to_path_buf();
        let (ns, val) = dimension_key
            .split_once(':')
            .unwrap_or(("minecraft", dimension_key));
        let primary = root.join("dimensions").join(ns).join(val).join("region");
        let fallback = root.join("region");
        Self {
            world_dir: root,
            primary_region_dir: primary,
            fallback_region_dir: fallback,
            region_locks: RwLock::new(FxHashMap::default()),
        }
    }

    fn lock_for_region(&self, rx: i32, rz: i32) -> Arc<RwLock<()>> {
        if let Some(existing) = self.region_locks.read().get(&(rx, rz)) {
            return Arc::clone(existing);
        }
        let mut g = self.region_locks.write();
        Arc::clone(
            g.entry((rx, rz))
                .or_insert_with(|| Arc::new(RwLock::new(()))),
        )
    }

    /// Root world folder path.
    #[must_use]
    pub fn world_dir(&self) -> &Path {
        &self.world_dir
    }

    /// Primary `.mca` region directory path.
    #[must_use]
    pub fn region_dir(&self) -> &Path {
        &self.primary_region_dir
    }

    fn region_file_name(region_x: i32, region_z: i32) -> String {
        format!("r.{region_x}.{region_z}.mca")
    }

    fn resolve_existing_region_file(&self, region_x: i32, region_z: i32) -> Option<PathBuf> {
        let name = Self::region_file_name(region_x, region_z);
        let p1 = self.primary_region_dir.join(&name);
        if p1.exists() {
            return Some(p1);
        }
        let p2 = self.fallback_region_dir.join(&name);
        if p2.exists() {
            return Some(p2);
        }
        None
    }

    /// Reads the raw chunk [`NbtCompound`] for `pos` from its `.mca` region file, if present.
    pub fn read_chunk_nbt(&self, pos: ChunkPos) -> Result<Option<NbtCompound>, WorldError> {
        let rx = pos.x.div_euclid(32);
        let rz = pos.z.div_euclid(32);
        let reg_lock = self.lock_for_region(rx, rz);
        let _guard = reg_lock.read();
        let Some(mca_path) = self.resolve_existing_region_file(rx, rz) else {
            return Ok(None);
        };

        let mut file = OpenOptions::new().read(true).open(mca_path)?;
        let file_len = file.metadata()?.len() as usize;
        if file_len < HEADER_BYTES {
            return Ok(None);
        }

        let slot = (pos.x.rem_euclid(32) as usize) + (pos.z.rem_euclid(32) as usize) * 32;
        file.seek(SeekFrom::Start((slot * 4) as u64))?;
        let mut loc_bytes = [0_u8; 4];
        file.read_exact(&mut loc_bytes)?;
        let loc = u32::from_be_bytes(loc_bytes);
        let sector_offset = (loc >> 8) as usize;
        let sector_count = (loc & 0xFF) as usize;
        if sector_offset < 2 || sector_count == 0 {
            return Ok(None);
        }

        let byte_offset = sector_offset * SECTOR_BYTES;
        if byte_offset + 5 > file_len {
            return Err(WorldError::InvalidOperation(
                "corrupt .mca sector offset past EOF".to_owned(),
            ));
        }

        file.seek(SeekFrom::Start(byte_offset as u64))?;
        let mut hdr = [0_u8; 5];
        file.read_exact(&mut hdr)?;
        let exact_len = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
        let scheme = hdr[4];
        if exact_len == 0 || byte_offset + 4 + exact_len > file_len {
            return Err(WorldError::InvalidOperation(
                "corrupt .mca chunk stream length".to_owned(),
            ));
        }

        let mut payload = vec![0_u8; exact_len - 1];
        file.read_exact(&mut payload)?;
        let (_, compound) = match scheme {
            1 => read_gzip_named(&payload)?,
            2 => read_zlib_named(&payload)?,
            3 => {
                let mut slice = payload.as_slice();
                read_named(&mut slice)?
            }
            other => {
                return Err(WorldError::InvalidOperation(format!(
                    "unsupported .mca compression type {other}"
                )));
            }
        };
        Ok(Some(compound))
    }

    /// Writes `chunk_nbt` into the `.mca` region file for `pos` using Zlib compression (`scheme = 2`).
    pub fn write_chunk_nbt(
        &self,
        pos: ChunkPos,
        chunk_nbt: &NbtCompound,
    ) -> Result<(), WorldError> {
        let rx = pos.x.div_euclid(32);
        let rz = pos.z.div_euclid(32);
        let compressed = write_zlib_named("", chunk_nbt)?;
        let reg_lock = self.lock_for_region(rx, rz);
        let _guard = reg_lock.write();
        fs::create_dir_all(&self.primary_region_dir)?;
        let mca_path = self.primary_region_dir.join(Self::region_file_name(rx, rz));

        let exact_len = compressed.len() + 1;
        let total_stream_bytes = 4 + exact_len;
        let needed_sectors = total_stream_bytes.div_ceil(SECTOR_BYTES);
        if needed_sectors > 255 {
            return Err(WorldError::InvalidOperation(
                "chunk exceeds 255 .mca sectors (1 MiB)".to_owned(),
            ));
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(mca_path)?;

        let mut header = vec![0_u8; HEADER_BYTES];
        let cur_len = file.metadata()?.len() as usize;
        if cur_len >= HEADER_BYTES {
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut header)?;
        } else {
            file.set_len(HEADER_BYTES as u64)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&header)?;
        }

        let slot = (pos.x.rem_euclid(32) as usize) + (pos.z.rem_euclid(32) as usize) * 32;
        let total_existing_sectors = cur_len.max(HEADER_BYTES).div_ceil(SECTOR_BYTES);
        let mut occupied = vec![false; total_existing_sectors + needed_sectors + 2];
        occupied[0] = true;
        occupied[1] = true;

        for i in 0..1024 {
            if i == slot {
                continue;
            }
            let base = i * 4;
            let entry = u32::from_be_bytes([
                header[base],
                header[base + 1],
                header[base + 2],
                header[base + 3],
            ]);
            let off = (entry >> 8) as usize;
            let cnt = (entry & 0xFF) as usize;
            if off >= 2 && cnt > 0 {
                if off + cnt > occupied.len() {
                    occupied.resize(off + cnt + needed_sectors, false);
                }
                for flag in &mut occupied[off..off + cnt] {
                    *flag = true;
                }
            }
        }

        // Find the lowest contiguous free span of `needed_sectors` starting at sector >= 2
        let mut chosen_sector = 2_usize;
        'search: loop {
            if chosen_sector + needed_sectors > occupied.len() {
                break;
            }
            for s in 0..needed_sectors {
                if occupied[chosen_sector + s] {
                    chosen_sector += s + 1;
                    continue 'search;
                }
            }
            break;
        }

        let mut sector_buf = vec![0_u8; needed_sectors * SECTOR_BYTES];
        sector_buf[0..4].copy_from_slice(&(exact_len as u32).to_be_bytes());
        sector_buf[4] = 2; // Zlib
        sector_buf[5..5 + compressed.len()].copy_from_slice(&compressed);

        file.seek(SeekFrom::Start((chosen_sector * SECTOR_BYTES) as u64))?;
        file.write_all(&sector_buf)?;

        let new_loc = ((chosen_sector as u32) << 8) | (needed_sectors as u32);
        let loc_off = slot * 4;
        header[loc_off..loc_off + 4].copy_from_slice(&new_loc.to_be_bytes());

        let epoch_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        let ts_off = SECTOR_BYTES + slot * 4;
        header[ts_off..ts_off + 4].copy_from_slice(&epoch_secs.to_be_bytes());

        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header)?;
        file.flush()?;
        Ok(())
    }
}

impl ChunkLoader for AnvilLoader {
    fn load_chunk(
        &self,
        dimension: &DimensionType,
        pos: ChunkPos,
    ) -> Result<Option<Chunk>, WorldError> {
        let Some(nbt) = self.read_chunk_nbt(pos)? else {
            return Ok(None);
        };
        Ok(Some(decode_chunk_from_anvil_nbt(pos, dimension, &nbt)))
    }

    fn save_chunk(&self, chunk: &Chunk, world_age: i64) -> Result<(), WorldError> {
        let nbt = encode_chunk_to_anvil_nbt(chunk, world_age);
        self.write_chunk_nbt(chunk.pos(), &nbt)
    }

    fn load_instance_nbt(&self) -> Result<Option<NbtCompound>, WorldError> {
        let level_dat = self.world_dir.join("level.dat");
        if !level_dat.exists() {
            return Ok(None);
        }
        let bytes = fs::read(level_dat)?;
        let (_, root) = read_gzip_named(&bytes)?;
        Ok(Some(root))
    }

    fn save_instance_nbt(&self, nbt: &NbtCompound) -> Result<(), WorldError> {
        fs::create_dir_all(&self.world_dir)?;
        let bytes = write_gzip_named("", nbt)?;
        fs::write(self.world_dir.join("level.dat"), bytes)?;
        Ok(())
    }
}

/// Serializes a [`Chunk`] into an MC 26.2 Anvil chunk [`NbtCompound`].
#[must_use]
pub fn encode_chunk_to_anvil_nbt(chunk: &Chunk, world_age: i64) -> NbtCompound {
    let mut root = NbtCompound::new();
    root.insert("DataVersion", NbtTag::Int(DATA_VERSION));
    root.insert("xPos", NbtTag::Int(chunk.pos().x));
    root.insert("yPos", NbtTag::Int(chunk.min_y().div_euclid(16)));
    root.insert("zPos", NbtTag::Int(chunk.pos().z));
    root.insert("status", NbtTag::String("minecraft:full".to_owned()));
    root.insert("LastUpdate", NbtTag::Long(world_age));

    let min_sec_y = chunk.min_y().div_euclid(16);
    let mut section_tags = Vec::with_capacity(chunk.sections().len());

    for (sec_idx, sec) in chunk.sections().iter().enumerate() {
        let sec_y = (min_sec_y + sec_idx as i32) as i8;
        let mut sec_nbt = NbtCompound::new();
        sec_nbt.insert("Y", NbtTag::Byte(sec_y));

        let mut block_states_nbt = NbtCompound::new();
        if let Some(single_id) = sec.blocks().single_value() {
            let blk = Block::from_state_id(single_id as u16).unwrap_or(Block::AIR);
            block_states_nbt.insert("palette", NbtTag::List(vec![block_to_palette_nbt(blk)]));
        } else {
            let mut palette_tags = Vec::new();
            let mut state_to_pidx: FxHashMap<u16, u16> = FxHashMap::default();
            let mut indices = [0_u16; 4096];

            for ly in 0..16 {
                for lz in 0..16 {
                    for lx in 0..16 {
                        let blk = sec.get_block(lx, ly, lz);
                        let sid = blk.state_id();
                        let pidx = *state_to_pidx.entry(sid).or_insert_with(|| {
                            let next = palette_tags.len() as u16;
                            palette_tags.push(block_to_palette_nbt(blk));
                            next
                        });
                        indices[(ly << 8) | (lz << 4) | lx] = pidx;
                    }
                }
            }

            let palette_len = palette_tags.len();
            block_states_nbt.insert("palette", NbtTag::List(palette_tags));
            if palette_len > 1 {
                let bpe = bits_to_represent(palette_len - 1).max(4);
                let packed = pack_anvil_section_indices(&indices, bpe);
                block_states_nbt.insert("data", NbtTag::LongArray(packed));
            }
        }
        sec_nbt.insert("block_states", NbtTag::Compound(block_states_nbt));

        let mut biomes_nbt = NbtCompound::new();
        let regs = Registries::vanilla();
        if let Some(single_biome) = sec.biomes().single_value() {
            let bkey = regs
                .lookup_dynamic_key("minecraft:worldgen/biome", single_biome)
                .unwrap_or("minecraft:plains");
            biomes_nbt.insert(
                "palette",
                NbtTag::List(vec![NbtTag::String(bkey.to_owned())]),
            );
        } else {
            let mut biome_palette: Vec<NbtTag> = Vec::new();
            let mut biome_to_pidx: FxHashMap<u32, u16> = FxHashMap::default();
            let mut biome_indices = [0_u16; 64];
            for by in 0..4 {
                for bz in 0..4 {
                    for bx in 0..4 {
                        let bid = sec.get_biome_cell(bx, by, bz);
                        let pidx = *biome_to_pidx.entry(bid).or_insert_with(|| {
                            let next = biome_palette.len() as u16;
                            let bkey = regs
                                .lookup_dynamic_key("minecraft:worldgen/biome", bid)
                                .unwrap_or("minecraft:plains");
                            biome_palette.push(NbtTag::String(bkey.to_owned()));
                            next
                        });
                        biome_indices[(by << 4) | (bz << 2) | bx] = pidx;
                    }
                }
            }
            let bpal_len = biome_palette.len();
            biomes_nbt.insert("palette", NbtTag::List(biome_palette));
            if bpal_len > 1 {
                let bpe = bits_to_represent(bpal_len - 1).max(1);
                let packed = pack_anvil_indices(&biome_indices, bpe);
                biomes_nbt.insert("data", NbtTag::LongArray(packed));
            }
        }
        sec_nbt.insert("biomes", NbtTag::Compound(biomes_nbt));
        section_tags.push(NbtTag::Compound(sec_nbt));
    }
    root.insert("sections", NbtTag::List(section_tags));

    let mut be_list = Vec::new();
    for (&(lx, wy, lz), state) in chunk.block_entities() {
        let blk = chunk.get_block(usize::from(lx), i32::from(wy), usize::from(lz));
        if blk.block_entity_id().is_some() || state.nbt.is_some() || state.handler.is_some() {
            let mut be_nbt = state.nbt.clone().unwrap_or_default();
            let id_str = state
                .handler
                .as_ref()
                .map_or_else(|| blk.key().to_owned(), |h| h.key().to_owned());
            be_nbt.insert("id", NbtTag::String(id_str));
            be_nbt.insert("x", NbtTag::Int(chunk.pos().x * 16 + i32::from(lx)));
            be_nbt.insert("y", NbtTag::Int(i32::from(wy)));
            be_nbt.insert("z", NbtTag::Int(chunk.pos().z * 16 + i32::from(lz)));
            be_nbt.insert("keepPacked", NbtTag::Byte(0));
            be_list.push(NbtTag::Compound(be_nbt));
        }
    }
    root.insert("block_entities", NbtTag::List(be_list));

    root
}

/// Deserializes an MC 1.18+ / 26.2 Anvil chunk [`NbtCompound`] into a [`Chunk`].
#[must_use]
pub fn decode_chunk_from_anvil_nbt(
    pos: ChunkPos,
    dimension: &DimensionType,
    chunk_nbt: &NbtCompound,
) -> Chunk {
    let mut chunk = Chunk::new(pos, dimension);
    let min_sec_y = chunk.min_y().div_euclid(16);
    let max_sec_y = chunk.max_y().div_euclid(16);

    if let Some(NbtTag::List(sections)) = chunk_nbt.get("sections") {
        for tag in sections {
            let NbtTag::Compound(sec_nbt) = tag else {
                continue;
            };
            let Some(sec_y) = sec_nbt
                .get_int("Y")
                .or_else(|| sec_nbt.get_byte("Y").map(i32::from))
            else {
                continue;
            };
            if sec_y < min_sec_y || sec_y >= max_sec_y {
                continue;
            }
            let base_wy = sec_y * 16;

            let Some(block_states) = sec_nbt.get_compound("block_states") else {
                continue;
            };
            let Some(NbtTag::List(palette_list)) = block_states.get("palette") else {
                continue;
            };
            if palette_list.is_empty() {
                continue;
            }

            let decoded_palette: Vec<Block> = palette_list
                .iter()
                .map(|entry| match entry {
                    NbtTag::Compound(c) => palette_nbt_to_block(c),
                    _ => Block::AIR,
                })
                .collect();

            if decoded_palette.len() == 1 {
                let single = decoded_palette[0];
                if !single.is_air() {
                    chunk.fill_height(base_wy, base_wy + 16, single);
                }
            } else if let Some(NbtTag::LongArray(longs)) = block_states.get("data")
                && !longs.is_empty()
            {
                let bpe = bits_to_represent(decoded_palette.len() - 1).max(4);
                let indices = unpack_anvil_section_indices(longs, bpe);
                for (flat, &pidx) in indices.iter().enumerate() {
                    let blk = decoded_palette
                        .get(usize::from(pidx))
                        .copied()
                        .unwrap_or(Block::AIR);
                    if !blk.is_air() {
                        let lx = flat & 15;
                        let lz = (flat >> 4) & 15;
                        let ly = (flat >> 8) & 15;
                        chunk.set_block(lx, base_wy + (ly as i32), lz, blk);
                    }
                }
            }

            // Decode 4×4×4 biome palette if present
            if let Some(biomes_nbt) = sec_nbt.get_compound("biomes")
                && let Some(NbtTag::List(bpal)) = biomes_nbt.get("palette")
                && !bpal.is_empty()
            {
                let regs = Registries::vanilla();
                let decoded_biomes: Vec<u32> = bpal
                    .iter()
                    .map(|t| {
                        t.as_str()
                            .and_then(|k| regs.lookup_dynamic_id("minecraft:worldgen/biome", k))
                            .unwrap_or_else(|| regs.plains_biome_id())
                    })
                    .collect();
                if decoded_biomes.len() == 1 {
                    let bid = decoded_biomes[0];
                    if bid != regs.plains_biome_id() {
                        for by in 0..4 {
                            for bz in 0..4 {
                                for bx in 0..4 {
                                    chunk.set_biome(bx * 4, base_wy + by * 4, bz * 4, bid);
                                }
                            }
                        }
                    }
                } else if let Some(NbtTag::LongArray(blongs)) = biomes_nbt.get("data")
                    && !blongs.is_empty()
                {
                    let bpe = bits_to_represent(decoded_biomes.len() - 1).max(1);
                    let mut bindices = [0_u16; 64];
                    unpack_anvil_indices(blongs, bpe, &mut bindices);
                    for (flat, &pidx) in bindices.iter().enumerate() {
                        let bid = decoded_biomes
                            .get(usize::from(pidx))
                            .copied()
                            .unwrap_or_else(|| regs.plains_biome_id());
                        let bx = flat & 3;
                        let bz = (flat >> 2) & 3;
                        let by = (flat >> 4) & 3;
                        chunk.set_biome(bx * 4, base_wy + (by as i32) * 4, bz * 4, bid);
                    }
                }
            }
        }
    }

    if let Some(NbtTag::List(be_list)) = chunk_nbt.get("block_entities") {
        for item in be_list {
            let NbtTag::Compound(be_nbt) = item else {
                continue;
            };
            let (Some(wx), Some(wy), Some(wz)) = (
                be_nbt.get_int("x"),
                be_nbt.get_int("y"),
                be_nbt.get_int("z"),
            ) else {
                continue;
            };
            if wy < chunk.min_y() || wy >= chunk.max_y() {
                continue;
            }
            let lx = wx.rem_euclid(16) as usize;
            let lz = wz.rem_euclid(16) as usize;
            let mut trimmed = be_nbt.clone();
            trimmed.remove("id");
            trimmed.remove("x");
            trimmed.remove("y");
            trimmed.remove("z");
            trimmed.remove("keepPacked");
            if !trimmed.is_empty() {
                chunk.block_entity_state_mut(lx, wy, lz).nbt = Some(trimmed);
            }
        }
    }

    chunk
}

fn block_to_palette_nbt(block: Block) -> NbtTag {
    let mut c = NbtCompound::new();
    c.insert("Name", NbtTag::String(block.key().to_owned()));
    let props = block.properties();
    if !props.is_empty() {
        let mut p_nbt = NbtCompound::new();
        for (k, v) in props {
            p_nbt.insert(k, NbtTag::String(v.to_owned()));
        }
        c.insert("Properties", NbtTag::Compound(p_nbt));
    }
    NbtTag::Compound(c)
}

fn palette_nbt_to_block(entry: &NbtCompound) -> Block {
    let Some(name) = entry.get_str("Name") else {
        return Block::AIR;
    };
    let Some(mut blk) = Block::from_key(name) else {
        return Block::AIR;
    };
    if let Some(props) = entry.get_compound("Properties") {
        for (k, v) in props {
            if let NbtTag::String(val_str) = v
                && let Ok(updated) = blk.with_property(k, val_str)
            {
                blk = updated;
            }
        }
    }
    blk
}

fn pack_anvil_indices(indices: &[u16], bpe: u8) -> Vec<i64> {
    let entries_per_long = (64 / usize::from(bpe)).max(1);
    let len = indices.len().div_ceil(entries_per_long);
    let mask = (1_u64 << bpe) - 1;
    let mut words = vec![0_i64; len];
    for (i, &idx) in indices.iter().enumerate() {
        let word_idx = i / entries_per_long;
        let bit_idx = (i % entries_per_long) * usize::from(bpe);
        let cur = words[word_idx] as u64;
        words[word_idx] = (cur | ((u64::from(idx) & mask) << bit_idx)) as i64;
    }
    words
}

fn unpack_anvil_indices(words: &[i64], bpe: u8, out: &mut [u16]) {
    let entries_per_long = (64 / usize::from(bpe)).max(1);
    let mask = (1_u64 << bpe) - 1;
    for (i, slot) in out.iter_mut().enumerate() {
        let word_idx = i / entries_per_long;
        if let Some(&raw) = words.get(word_idx) {
            let bit_idx = (i % entries_per_long) * usize::from(bpe);
            *slot = (((raw as u64) >> bit_idx) & mask) as u16;
        }
    }
}

fn pack_anvil_section_indices(indices: &[u16; 4096], bpe: u8) -> Vec<i64> {
    pack_anvil_indices(indices, bpe)
}

fn unpack_anvil_section_indices(words: &[i64], bpe: u8) -> [u16; 4096] {
    let mut out = [0_u16; 4096];
    unpack_anvil_indices(words, bpe, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn anvil_mca_region_roundtrips_sections_properties_and_block_entities() {
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "minestom_rs_anvil_test_{}_{seq}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);

        let dim = DimensionType::overworld();
        let loader = AnvilLoader::with_dimension(&dir, &dim.key);

        let pos_a = ChunkPos::new(0, 0);
        let mut chunk_a = Chunk::new(pos_a, &dim);
        chunk_a.fill_height(0, 16, Block::STONE);
        let oak_x = Block::OAK_LOG.with_property("axis", "x").unwrap();
        chunk_a.set_block(3, 20, 7, oak_x);
        let chest_south = Block::CHEST.with_property("facing", "south").unwrap();
        chunk_a.set_block(5, 20, 5, chest_south);
        chunk_a.block_entity_state_mut(5, 20, 5).nbt =
            Some(NbtCompound::new().with("CustomName", "SavedVault"));

        let pos_b = ChunkPos::new(-3, 2);
        let mut chunk_b = Chunk::new(pos_b, &dim);
        chunk_b.set_block(1, 64, 2, Block::GLOWSTONE);

        loader.save_chunk(&chunk_a, 999).unwrap();
        loader.save_chunk(&chunk_b, 999).unwrap();

        // Overwrite chunk_a once to test sector reallocation!
        chunk_a.set_block(4, 21, 8, Block::DIAMOND_BLOCK);
        loader.save_chunk(&chunk_a, 1000).unwrap();

        let loaded_a = loader
            .load_chunk(&dim, pos_a)
            .unwrap()
            .expect("chunk_a present");
        assert_eq!(loaded_a.get_block(0, 10, 0), Block::STONE);
        assert_eq!(loaded_a.get_block(3, 20, 7), oak_x);
        assert_eq!(loaded_a.get_block(4, 21, 8), Block::DIAMOND_BLOCK);
        assert_eq!(loaded_a.get_block(5, 20, 5), chest_south);
        assert_eq!(
            loaded_a
                .block_entity_state(5, 20, 5)
                .and_then(|s| s.nbt.as_ref())
                .and_then(|n| n.get_str("CustomName")),
            Some("SavedVault")
        );

        let loaded_b = loader
            .load_chunk(&dim, pos_b)
            .unwrap()
            .expect("chunk_b present");
        assert_eq!(loaded_b.get_block(1, 64, 2), Block::GLOWSTONE);
        assert!(
            loader
                .load_chunk(&dim, ChunkPos::new(12, 12))
                .unwrap()
                .is_none()
        );

        let level_nbt = NbtCompound::new().with("LevelName", "MinestomRustWorld");
        loader.save_instance_nbt(&level_nbt).unwrap();
        assert_eq!(loader.load_instance_nbt().unwrap(), Some(level_nbt));

        let _ = fs::remove_dir_all(&dir);
    }
}
