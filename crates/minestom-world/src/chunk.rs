//! 16×16×16 `Section` (with MC 26.2 `liquid_count`) and 16×384×16 `Chunk` with cached packet bytes.

use std::sync::Arc;

use bytes::{BufMut, Bytes, BytesMut};
use minestom_block::{Block, BlockHandler};
use minestom_core::{ChunkPos, TagMap};
use minestom_nbt::{NbtCompound, NbtTag};
use minestom_protocol::encode_packet_payload;
use minestom_protocol::play::{ChunkBlockEntity, ChunkDataPacket, HeightmapType, LightData};
use minestom_registry::DimensionType;
use rustc_hash::FxHashMap;

use crate::WorldError;
use crate::light::compute_chunk_light;
use crate::palette::{Palette, bits_to_represent};

/// Single 16×16×16 vertical chunk section.
#[derive(Debug, Clone)]
pub struct Section {
    block_count: u16,
    liquid_count: u16,
    blocks: Palette,
    biomes: Palette,
}

impl Default for Section {
    fn default() -> Self {
        Self::new()
    }
}

impl Section {
    /// Creates an empty (`air`) section with `minecraft:plains` biomes.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            block_count: 0,
            liquid_count: 0,
            blocks: Palette::blocks(),
            biomes: Palette::biomes(),
        }
    }

    /// Non-air block count (`0..=4096`).
    #[must_use]
    pub const fn block_count(&self) -> u16 {
        self.block_count
    }

    /// Fluid-containing block count (`0..=4096`, required by MC 26.2 `ChunkData.Section`).
    #[must_use]
    pub const fn liquid_count(&self) -> u16 {
        self.liquid_count
    }

    /// Borrows the block state [`Palette`].
    #[must_use]
    pub const fn blocks(&self) -> &Palette {
        &self.blocks
    }

    /// Borrows the biome [`Palette`].
    #[must_use]
    pub const fn biomes(&self) -> &Palette {
        &self.biomes
    }

    /// Reads the [`Block`] at section-local `(x, y, z)` (`0..16`).
    #[must_use]
    pub fn get_block(&self, x: usize, y: usize, z: usize) -> Block {
        let state_id = self.blocks.get(x, y, z) as u16;
        Block::from_state_id(state_id).unwrap_or(Block::AIR)
    }

    /// Sets the [`Block`] at section-local `(x, y, z)` (`0..16`), updating `block_count` & `liquid_count`.
    pub fn set_block(&mut self, x: usize, y: usize, z: usize, block: Block) {
        let prev_id = self.blocks.set(x, y, z, u32::from(block.state_id())) as u16;
        if prev_id == block.state_id() {
            return;
        }
        let prev = Block::from_state_id(prev_id).unwrap_or(Block::AIR);
        match (prev.is_air(), block.is_air()) {
            (true, false) => self.block_count = self.block_count.saturating_add(1),
            (false, true) => self.block_count = self.block_count.saturating_sub(1),
            _ => {}
        }
        match (prev.is_liquid(), block.is_liquid()) {
            (false, true) => self.liquid_count = self.liquid_count.saturating_add(1),
            (true, false) => self.liquid_count = self.liquid_count.saturating_sub(1),
            _ => {}
        }
    }

    /// Fills all 4,096 blocks in this section with `block` in `O(1)`.
    pub fn fill(&mut self, block: Block) {
        self.blocks.fill(u32::from(block.state_id()));
        self.block_count = if block.is_air() { 0 } else { 4096 };
        self.liquid_count = if block.is_liquid() { 4096 } else { 0 };
    }

    /// Encodes this section in MC 26.2 `ChunkData.Section` format (`i16 blockCount`, `i16 liquidCount`,
    /// `Palette blockStates`, `Palette biomes`).
    pub fn write_network<B: BufMut>(&self, buf: &mut B) {
        buf.put_i16(self.block_count as i16);
        buf.put_i16(self.liquid_count as i16);
        self.blocks.write_network(buf);
        self.biomes.write_network(buf);
    }
}

/// Packs 256 column height offsets (`0..=dimension_height`) into non-spanning 64-bit words,
/// matching Minestom's `Heightmap.encode` verbatim.
fn pack_heightmap(heights: &[u16; 256], bits_per_entry: u8) -> Vec<u64> {
    let entries_per_long = 64 / usize::from(bits_per_entry);
    let len = 256_usize.div_ceil(entries_per_long);
    let mask = (1_u64 << bits_per_entry) - 1;
    let mut words = vec![0_u64; len];
    for (i, &h) in heights.iter().enumerate() {
        let word_idx = i / entries_per_long;
        let bit_idx = (i % entries_per_long) * usize::from(bits_per_entry);
        words[word_idx] |= (u64::from(h) & mask) << bit_idx;
    }
    words
}

/// Sparse per-block metadata (`NbtCompound`, `TagMap`, and optional `BlockHandler`) stored only
/// for coordinates inside a [`Chunk`] that carry block-entity state or custom handlers.
#[derive(Clone, Default)]
pub struct BlockEntityState {
    /// Client/disk block-entity NBT compound.
    pub nbt: Option<NbtCompound>,
    /// Server-side typed [`TagMap`] attached to this block position.
    pub tags: TagMap,
    /// Optional attached [`BlockHandler`].
    pub handler: Option<Arc<dyn BlockHandler>>,
}

impl std::fmt::Debug for BlockEntityState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockEntityState")
            .field("has_nbt", &self.nbt.is_some())
            .field("handler", &self.handler.as_ref().map(|h| h.key()))
            .finish_non_exhaustive()
    }
}

/// A 16×H×16 column of [`Section`]s with lazy packet caching (`CachedPacket` equivalent).
#[derive(Debug, Clone)]
pub struct Chunk {
    pos: ChunkPos,
    min_y: i32,
    height: i32,
    sections: Vec<Section>,
    block_entities: FxHashMap<(u8, i16, u8), BlockEntityState>,
    lighting_enabled: bool,
    cached_payload: Option<Bytes>,
}

impl Chunk {
    /// Creates an empty `Chunk` at `pos` sized for `dimension`.
    #[must_use]
    pub fn new(pos: ChunkPos, dimension: &DimensionType) -> Self {
        let count = dimension.section_count();
        Self {
            pos,
            min_y: dimension.min_y,
            height: dimension.height,
            sections: vec![Section::new(); count],
            block_entities: FxHashMap::default(),
            lighting_enabled: false,
            cached_payload: None,
        }
    }

    /// Enables or disables dynamic 3D BFS sky & block lighting for this chunk (`LightingChunk` mode).
    pub fn set_lighting_enabled(&mut self, enabled: bool) {
        if self.lighting_enabled != enabled {
            self.lighting_enabled = enabled;
            self.cached_payload = None;
        }
    }

    /// Returns `true` if dynamic 3D BFS sky & block lighting is enabled on this chunk.
    #[must_use]
    pub const fn is_lighting_enabled(&self) -> bool {
        self.lighting_enabled
    }

    /// Borrows all sparse block-entity records `(local_x, world_y, local_z) -> BlockEntityState`.
    #[must_use]
    pub const fn block_entities(&self) -> &FxHashMap<(u8, i16, u8), BlockEntityState> {
        &self.block_entities
    }

    /// Returns this chunk's `(chunk_x, chunk_z)` position.
    #[must_use]
    pub const fn pos(&self) -> ChunkPos {
        self.pos
    }

    /// Minimum buildable Y coordinate (`-64` for Overworld).
    #[must_use]
    pub const fn min_y(&self) -> i32 {
        self.min_y
    }

    /// Exclusive maximum buildable Y coordinate (`320` for Overworld).
    #[must_use]
    pub const fn max_y(&self) -> i32 {
        self.min_y + self.height
    }

    /// Borrows all vertical sections (`min_section..max_section`).
    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Reads the [`Block`] at chunk-local `x, z` (`0..16`) and world `y`.
    #[must_use]
    pub fn get_block(&self, local_x: usize, y: i32, local_z: usize) -> Block {
        if y < self.min_y || y >= self.max_y() || local_x >= 16 || local_z >= 16 {
            return Block::AIR;
        }
        let rel_y = (y - self.min_y) as usize;
        let sec_idx = rel_y >> 4;
        let sec_y = rel_y & 15;
        self.sections[sec_idx].get_block(local_x, sec_y, local_z)
    }

    /// Sets the [`Block`] at chunk-local `x, z` (`0..16`) and world `y`, invalidating cached packet bytes.
    pub fn set_block(&mut self, local_x: usize, y: i32, local_z: usize, block: Block) {
        if y < self.min_y || y >= self.max_y() || local_x >= 16 || local_z >= 16 {
            return;
        }
        let rel_y = (y - self.min_y) as usize;
        let sec_idx = rel_y >> 4;
        let sec_y = rel_y & 15;
        self.sections[sec_idx].set_block(local_x, sec_y, local_z, block);
        if block.is_air() {
            self.block_entities
                .remove(&(local_x as u8, y as i16, local_z as u8));
        }
        self.cached_payload = None;
    }

    /// Borrows the sparse [`BlockEntityState`] at `(local_x, y, local_z)`, if present.
    #[must_use]
    pub fn block_entity_state(
        &self,
        local_x: usize,
        y: i32,
        local_z: usize,
    ) -> Option<&BlockEntityState> {
        self.block_entities
            .get(&(local_x as u8, y as i16, local_z as u8))
    }

    /// Mutably accesses or initializes the [`BlockEntityState`] at `(local_x, y, local_z)`,
    /// invalidating cached packet bytes.
    pub fn block_entity_state_mut(
        &mut self,
        local_x: usize,
        y: i32,
        local_z: usize,
    ) -> &mut BlockEntityState {
        self.cached_payload = None;
        self.block_entities
            .entry((local_x as u8, y as i16, local_z as u8))
            .or_default()
    }

    /// Fills horizontal layers `start_y..end_y` (clamped to chunk height bounds) with `block`,
    /// using `O(1)` whole-section fills wherever an entire 16-block section is covered!
    pub fn fill_height(&mut self, start_y: i32, end_y: i32, block: Block) {
        let lo = start_y.max(self.min_y);
        let hi = end_y.min(self.max_y());
        if lo >= hi {
            return;
        }
        let mut y = lo;
        while y < hi {
            let rel_y = (y - self.min_y) as usize;
            let sec_idx = rel_y >> 4;
            let sec_y = rel_y & 15;
            if sec_y == 0 && y + 16 <= hi {
                self.sections[sec_idx].fill(block);
                y += 16;
            } else {
                let sec_end = ((hi - y) as usize).min(16 - sec_y);
                for ly in sec_y..(sec_y + sec_end) {
                    for lz in 0..16 {
                        for lx in 0..16 {
                            self.sections[sec_idx].set_block(lx, ly, lz, block);
                        }
                    }
                }
                y += sec_end as i32;
            }
        }
        self.cached_payload = None;
    }

    fn compute_heightmaps(&self) -> (Vec<u64>, Vec<u64>) {
        let mut world_surface = [0_u16; 256];
        let mut motion_blocking = [0_u16; 256];

        for lz in 0..16_usize {
            for lx in 0..16_usize {
                let col = (lz << 4) | lx;
                let mut found_ws = false;
                let mut found_mb = false;

                for (sec_idx, sec) in self.sections.iter().enumerate().rev() {
                    if sec.block_count() == 0 {
                        continue;
                    }
                    if let Some(single_state) = sec.blocks().single_value() {
                        let b = Block::from_state_id(single_state as u16).unwrap_or(Block::AIR);
                        if !b.is_air() {
                            let top_offset = ((sec_idx + 1) * 16) as u16;
                            if !found_ws {
                                world_surface[col] = top_offset;
                                found_ws = true;
                            }
                            if !found_mb && (b.is_solid() || b.is_liquid()) {
                                motion_blocking[col] = top_offset;
                                found_mb = true;
                            }
                        }
                    } else {
                        for ly in (0..16_usize).rev() {
                            let b = sec.get_block(lx, ly, lz);
                            if !b.is_air() {
                                let offset = (sec_idx * 16 + ly + 1) as u16;
                                if !found_ws {
                                    world_surface[col] = offset;
                                    found_ws = true;
                                }
                                if !found_mb && (b.is_solid() || b.is_liquid()) {
                                    motion_blocking[col] = offset;
                                    found_mb = true;
                                }
                            }
                            if found_ws && found_mb {
                                break;
                            }
                        }
                    }
                    if found_ws && found_mb {
                        break;
                    }
                }
            }
        }

        let bpe = bits_to_represent(self.height as usize);
        (
            pack_heightmap(&world_surface, bpe),
            pack_heightmap(&motion_blocking, bpe),
        )
    }

    /// Builds a [`ChunkDataPacket`] representing the current state of this chunk.
    #[must_use]
    pub fn to_packet(&self) -> ChunkDataPacket {
        let mut sections_buf = BytesMut::with_capacity(self.sections.len() * 32);
        for sec in &self.sections {
            sec.write_network(&mut sections_buf);
        }
        let (ws, mb) = self.compute_heightmaps();
        let mut be_records = Vec::new();
        for (&(lx, y, lz), state) in &self.block_entities {
            let block = self.get_block(usize::from(lx), i32::from(y), usize::from(lz));
            if let Some(be_id) = block.block_entity_id() {
                be_records.push(ChunkBlockEntity {
                    packed_xz: ((lx & 15) << 4) | (lz & 15),
                    y,
                    block_entity_type: i32::from(be_id),
                    nbt: state.nbt.clone().map_or(NbtTag::End, NbtTag::Compound),
                });
            }
        }
        let light_data = if self.lighting_enabled {
            compute_chunk_light(self)
        } else {
            LightData::full_skylight(self.sections.len())
        };
        ChunkDataPacket {
            chunk_x: self.pos.x,
            chunk_z: self.pos.z,
            heightmaps: vec![
                (HeightmapType::WorldSurface, ws),
                (HeightmapType::MotionBlocking, mb),
            ],
            sections_data: sections_buf.to_vec(),
            block_entities: be_records,
            light_data,
        }
    }

    /// Returns the cached `[VarInt 0x2D][ChunkDataPacket]` uncompressed payload (`O(1)` `Bytes` clone
    /// on cache hits), encoding and caching it if the chunk was modified.
    pub fn encoded_packet_bytes(&mut self) -> Result<Bytes, WorldError> {
        if let Some(cached) = &self.cached_payload {
            return Ok(cached.clone());
        }
        let pkt = self.to_packet();
        let bytes = encode_packet_payload(&pkt)?;
        self.cached_payload = Some(bytes.clone());
        Ok(bytes)
    }
}
