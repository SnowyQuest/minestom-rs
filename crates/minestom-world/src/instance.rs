//! `InstanceContainer`, `SharedInstance`, `InstanceManager`, `WorldBorder`, `WorldTime`,
//! and `Weather` world management.

use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arc_swap::ArcSwapOption;
use bytes::Bytes;
use minestom_block::{Block, BlockGetter, BlockHandler};
use minestom_core::{BlockVec, ChunkPos, InstanceId, Point, Tag, TagMap, tag::TagSerializable};
use minestom_nbt::NbtCompound;
use minestom_protocol::block::MultiBlockChangePacket;
use minestom_protocol::play::{InitializeWorldBorderPacket, SetTimePacket, UpdateLightPacket};
use minestom_registry::DimensionType;
use parking_lot::RwLock;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use slotmap::SlotMap;

use crate::anvil::ChunkLoader;
use crate::light::build_update_light_packet;
use crate::{Chunk, GenerationUnit, Generator, WorldError};

/// World border configuration (`WorldBorder` in Minestom).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldBorder {
    /// Border center X coordinate.
    pub center_x: f64,
    /// Border center Z coordinate.
    pub center_z: f64,
    /// Current border diameter in blocks (`59_999_968.0` vanilla default).
    pub diameter: f64,
    /// Portal teleport boundary (`29_999_984` vanilla default).
    pub portal_teleport_boundary: i32,
    /// Red vignette warning time in seconds (`15` default).
    pub warning_time: i32,
    /// Red vignette warning reach in blocks (`5` default).
    pub warning_blocks: i32,
}

impl Default for WorldBorder {
    fn default() -> Self {
        Self {
            center_x: 0.0,
            center_z: 0.0,
            diameter: 59_999_968.0,
            portal_teleport_boundary: 29_999_984,
            warning_time: 15,
            warning_blocks: 5,
        }
    }
}

impl WorldBorder {
    /// Creates a centered square [`WorldBorder`] with `diameter`.
    #[must_use]
    pub fn with_diameter(center_x: f64, center_z: f64, diameter: f64) -> Self {
        Self {
            center_x,
            center_z,
            diameter: diameter.max(1.0),
            ..Self::default()
        }
    }

    /// Converts this border state into an [`InitializeWorldBorderPacket`].
    #[must_use]
    pub const fn to_initialize_packet(self) -> InitializeWorldBorderPacket {
        InitializeWorldBorderPacket {
            x: self.center_x,
            z: self.center_z,
            old_diameter: self.diameter,
            new_diameter: self.diameter,
            speed: 0,
            portal_teleport_boundary: self.portal_teleport_boundary,
            warning_time: self.warning_time,
            warning_blocks: self.warning_blocks,
        }
    }
}

/// World age and MC 26.2 clock state (`minecraft:overworld` day/night cycle).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldTime {
    /// Total elapsed world ticks (`gameTime`).
    pub world_age: i64,
    /// Current clock ticks (`0..24_000` day/night cycle; `6_000` = noon, `18_000` = midnight).
    pub day_time: i64,
    /// Clock progression rate (`1.0` = normal 20 TPS, `0.0` = frozen).
    pub clock_rate: f32,
}

impl Default for WorldTime {
    fn default() -> Self {
        Self {
            world_age: 0,
            day_time: 1000,
            clock_rate: 1.0,
        }
    }
}

impl WorldTime {
    /// Builds the MC 26.2 [`SetTimePacket`] for this world's clock state.
    #[must_use]
    pub fn to_set_time_packet(self) -> SetTimePacket {
        SetTimePacket::overworld(self.world_age, self.day_time, self.clock_rate)
    }
}

/// Rain and thunder weather state of an instance (`Weather` in Minestom).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Weather {
    /// Rain intensity (`0.0..=1.0`; `> 0.0` means raining).
    pub rain_level: f32,
    /// Thunder intensity (`0.0..=1.0`).
    pub thunder_level: f32,
}

impl Weather {
    /// Clear sunny weather (`rain = 0.0, thunder = 0.0`).
    pub const CLEAR: Self = Self {
        rain_level: 0.0,
        thunder_level: 0.0,
    };

    /// Heavy rain without thunder (`rain = 1.0, thunder = 0.0`).
    pub const RAIN: Self = Self {
        rain_level: 1.0,
        thunder_level: 0.0,
    };

    /// Full thunderstorm (`rain = 1.0, thunder = 1.0`).
    pub const THUNDER: Self = Self {
        rain_level: 1.0,
        thunder_level: 1.0,
    };

    /// Returns `true` if `rain_level > 0.0`.
    #[must_use]
    pub fn is_raining(self) -> bool {
        self.rain_level > 0.0
    }
}

struct ChunkStorage {
    dimension: DimensionType,
    generator: ArcSwapOption<Box<dyn Generator>>,
    chunk_loader: ArcSwapOption<Box<dyn ChunkLoader>>,
    lighting_enabled: AtomicBool,
    chunks: RwLock<FxHashMap<ChunkPos, Chunk>>,
}

struct InstanceInner {
    id: InstanceId,
    shared: bool,
    storage: Arc<ChunkStorage>,
    world_border: RwLock<WorldBorder>,
    world_time: RwLock<WorldTime>,
    weather: RwLock<Weather>,
    tags: RwLock<TagMap>,
}

/// Cheaply cloneable handle to a world instance (`InstanceContainer` in Minestom).
#[derive(Clone)]
pub struct InstanceContainer {
    inner: Arc<InstanceInner>,
}

impl std::fmt::Debug for InstanceContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceContainer")
            .field("id", &self.inner.id)
            .field("shared", &self.inner.shared)
            .field("dimension", &self.inner.storage.dimension.key)
            .field("loaded_chunks", &self.inner.storage.chunks.read().len())
            .finish()
    }
}

/// Lightweight instance sharing its underlying [`Chunk`] storage with a parent [`InstanceContainer`]
/// while maintaining its own distinct [`InstanceId`], entity/player visibility, [`WorldBorder`],
/// [`WorldTime`], and [`Weather`] (`SharedInstance` in Minestom).
#[derive(Debug, Clone)]
pub struct SharedInstance {
    instance: InstanceContainer,
}

impl SharedInstance {
    /// Borrows this shared instance as an [`InstanceContainer`] handle (compatible with all server APIs).
    #[must_use]
    pub const fn as_instance(&self) -> &InstanceContainer {
        &self.instance
    }

    /// Consumes this wrapper and returns the [`InstanceContainer`] handle.
    #[must_use]
    pub fn into_instance(self) -> InstanceContainer {
        self.instance
    }
}

impl Deref for SharedInstance {
    type Target = InstanceContainer;

    fn deref(&self) -> &Self::Target {
        &self.instance
    }
}

impl From<SharedInstance> for InstanceContainer {
    fn from(s: SharedInstance) -> Self {
        s.instance
    }
}

impl InstanceContainer {
    fn new(id: InstanceId, dimension: DimensionType) -> Self {
        let storage = Arc::new(ChunkStorage {
            dimension,
            generator: ArcSwapOption::empty(),
            chunk_loader: ArcSwapOption::empty(),
            lighting_enabled: AtomicBool::new(false),
            chunks: RwLock::new(FxHashMap::default()),
        });
        Self {
            inner: Arc::new(InstanceInner {
                id,
                shared: false,
                storage,
                world_border: RwLock::new(WorldBorder::default()),
                world_time: RwLock::new(WorldTime::default()),
                weather: RwLock::new(Weather::CLEAR),
                tags: RwLock::new(TagMap::new()),
            }),
        }
    }

    fn new_shared(id: InstanceId, parent: &Self) -> Self {
        Self {
            inner: Arc::new(InstanceInner {
                id,
                shared: true,
                storage: Arc::clone(&parent.inner.storage),
                world_border: RwLock::new(parent.world_border()),
                world_time: RwLock::new(parent.world_time()),
                weather: RwLock::new(parent.weather()),
                tags: RwLock::new(TagMap::new()),
            }),
        }
    }

    /// Returns the generational [`InstanceId`] of this instance.
    #[must_use]
    pub fn id(&self) -> InstanceId {
        self.inner.id
    }

    /// Returns `true` if this handle was created as a [`SharedInstance`].
    #[must_use]
    pub fn is_shared(&self) -> bool {
        self.inner.shared
    }

    /// Returns `true` if `self` and `other` share the exact same underlying [`Chunk`] storage.
    #[must_use]
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner.storage, &other.inner.storage)
    }

    /// Returns the [`DimensionType`] of this instance.
    #[must_use]
    pub fn dimension_type(&self) -> &DimensionType {
        &self.inner.storage.dimension
    }

    /// Installs or replaces the parallel chunk [`Generator`] for newly generated chunks.
    pub fn set_generator(&self, generator: impl Generator) {
        self.inner
            .storage
            .generator
            .store(Some(Arc::new(Box::new(generator))));
    }

    /// Installs a persistent [`ChunkLoader`] (such as [`crate::AnvilLoader`]) consulted before
    /// running the procedural [`Generator`].
    pub fn set_chunk_loader(&self, loader: impl ChunkLoader) {
        self.inner
            .storage
            .chunk_loader
            .store(Some(Arc::new(Box::new(loader))));
    }

    /// Enables or disables 3D BFS sky & block lighting (`LightingChunk` mode) for all chunks in
    /// this instance storage!
    pub fn set_lighting_enabled(&self, enabled: bool) {
        self.inner
            .storage
            .lighting_enabled
            .store(enabled, Ordering::Release);
        let mut g = self.inner.storage.chunks.write();
        for chunk in g.values_mut() {
            chunk.set_lighting_enabled(enabled);
        }
    }

    /// Returns `true` if dynamic 3D BFS lighting (`LightingChunk`) is enabled.
    #[must_use]
    pub fn is_lighting_enabled(&self) -> bool {
        self.inner.storage.lighting_enabled.load(Ordering::Acquire)
    }

    /// Recomputes and returns an [`UpdateLightPacket`] (`0x30`) for `cpos` if dynamic lighting is
    /// enabled and `cpos` is loaded.
    #[must_use]
    pub fn build_light_update_packet(&self, cpos: ChunkPos) -> Option<UpdateLightPacket> {
        if !self.is_lighting_enabled() {
            return None;
        }
        let g = self.inner.storage.chunks.read();
        g.get(&cpos).map(build_update_light_packet)
    }

    /// Reads the current [`WorldBorder`].
    #[must_use]
    pub fn world_border(&self) -> WorldBorder {
        *self.inner.world_border.read()
    }

    /// Updates the instance [`WorldBorder`].
    pub fn set_world_border(&self, border: WorldBorder) {
        *self.inner.world_border.write() = border;
    }

    /// Reads the current [`WorldTime`].
    #[must_use]
    pub fn world_time(&self) -> WorldTime {
        *self.inner.world_time.read()
    }

    /// Updates the instance [`WorldTime`].
    pub fn set_world_time(&self, time: WorldTime) {
        *self.inner.world_time.write() = time;
    }

    /// Advances `world_age` by 1 tick and `day_time` by `clock_rate` (`mod 24_000`).
    pub fn tick_time(&self) -> WorldTime {
        let mut g = self.inner.world_time.write();
        g.world_age = g.world_age.wrapping_add(1);
        if g.clock_rate > 0.0 {
            let delta = (g.clock_rate.round() as i64).max(1);
            g.day_time = (g.day_time + delta).rem_euclid(24_000);
        }
        *g
    }

    /// Reads the current [`Weather`].
    #[must_use]
    pub fn weather(&self) -> Weather {
        *self.inner.weather.read()
    }

    /// Updates the instance [`Weather`].
    pub fn set_weather(&self, weather: Weather) {
        *self.inner.weather.write() = weather;
    }

    /// Writes an instance-level [`Tag<T>`].
    pub fn set_tag<T: TagSerializable>(&self, tag: &Tag<T>, value: T) {
        self.inner.tags.write().set_tag(tag, value);
    }

    /// Reads an instance-level [`Tag<T>`].
    #[must_use]
    pub fn get_tag<T: TagSerializable>(&self, tag: &Tag<T>) -> Option<T> {
        self.inner.tags.read().get_tag(tag)
    }

    /// Saves all currently loaded chunks and instance metadata through the configured [`ChunkLoader`].
    ///
    /// Safe to invoke from background worker threads or `tokio::task::spawn_blocking`.
    pub fn save_chunks_to_loader(&self) -> Result<usize, WorldError> {
        let Some(loader) = self.inner.storage.chunk_loader.load_full() else {
            return Ok(0);
        };
        let snapshots: Vec<Chunk> = self.inner.storage.chunks.read().values().cloned().collect();
        let age = self.world_time().world_age;
        for chunk in &snapshots {
            loader.save_chunk(chunk, age)?;
        }
        let nbt = self.inner.tags.read().as_compound().clone();
        if !nbt.is_empty() {
            loader.save_instance_nbt(&nbt)?;
        }
        Ok(snapshots.len())
    }

    /// Sets a single [`Block`] at world coordinate `pos`, generating/loading the target chunk first if needed.
    pub fn set_block(&self, pos: impl Point, block: Block) {
        let cpos = pos.chunk_pos();
        self.ensure_chunks_generated(&[cpos]);
        let lx = pos.block_x().rem_euclid(16) as usize;
        let lz = pos.block_z().rem_euclid(16) as usize;
        if let Some(chunk) = self.inner.storage.chunks.write().get_mut(&cpos) {
            chunk.set_block(lx, pos.block_y(), lz, block);
        }
    }

    /// Sets `block` along with block-entity `nbt` and optional `handler` at `pos`.
    pub fn set_block_with_data(
        &self,
        pos: impl Point,
        block: Block,
        nbt: Option<NbtCompound>,
        handler: Option<Arc<dyn BlockHandler>>,
    ) {
        let cpos = pos.chunk_pos();
        self.ensure_chunks_generated(&[cpos]);
        let lx = pos.block_x().rem_euclid(16) as usize;
        let lz = pos.block_z().rem_euclid(16) as usize;
        let y = pos.block_y();
        if let Some(chunk) = self.inner.storage.chunks.write().get_mut(&cpos) {
            chunk.set_block(lx, y, lz, block);
            if nbt.is_some() || handler.is_some() {
                let st = chunk.block_entity_state_mut(lx, y, lz);
                st.nbt = nbt;
                st.handler = handler;
            }
        }
    }

    /// Reads the client/disk block-entity [`NbtCompound`] at `pos`, if set.
    #[must_use]
    pub fn get_block_nbt(&self, pos: impl Point) -> Option<NbtCompound> {
        let cpos = pos.chunk_pos();
        let lx = pos.block_x().rem_euclid(16) as usize;
        let lz = pos.block_z().rem_euclid(16) as usize;
        self.inner
            .storage
            .chunks
            .read()
            .get(&cpos)
            .and_then(|c| c.block_entity_state(lx, pos.block_y(), lz))
            .and_then(|s| s.nbt.clone())
    }

    /// Writes a typed [`Tag<T>`] onto the block at `pos`.
    pub fn set_block_tag<T: TagSerializable>(&self, pos: impl Point, tag: &Tag<T>, value: T) {
        let cpos = pos.chunk_pos();
        self.ensure_chunks_generated(&[cpos]);
        let lx = pos.block_x().rem_euclid(16) as usize;
        let lz = pos.block_z().rem_euclid(16) as usize;
        if let Some(chunk) = self.inner.storage.chunks.write().get_mut(&cpos) {
            chunk
                .block_entity_state_mut(lx, pos.block_y(), lz)
                .tags
                .set_tag(tag, value);
        }
    }

    /// Reads a typed [`Tag<T>`] from the block at `pos`.
    #[must_use]
    pub fn get_block_tag<T: TagSerializable>(&self, pos: impl Point, tag: &Tag<T>) -> Option<T> {
        let cpos = pos.chunk_pos();
        let lx = pos.block_x().rem_euclid(16) as usize;
        let lz = pos.block_z().rem_euclid(16) as usize;
        self.inner
            .storage
            .chunks
            .read()
            .get(&cpos)
            .and_then(|c| c.block_entity_state(lx, pos.block_y(), lz))
            .and_then(|s| s.tags.get_tag(tag))
    }

    /// Reads the attached [`BlockHandler`] at `pos`, if any.
    #[must_use]
    pub fn get_block_handler(&self, pos: impl Point) -> Option<Arc<dyn BlockHandler>> {
        let cpos = pos.chunk_pos();
        let lx = pos.block_x().rem_euclid(16) as usize;
        let lz = pos.block_z().rem_euclid(16) as usize;
        self.inner
            .storage
            .chunks
            .read()
            .get(&cpos)
            .and_then(|c| c.block_entity_state(lx, pos.block_y(), lz))
            .and_then(|s| s.handler.clone())
    }

    pub(crate) fn apply_absolute_batch(
        &self,
        by_chunk: &FxHashMap<ChunkPos, Vec<(BlockVec, Block)>>,
    ) -> Vec<(ChunkPos, MultiBlockChangePacket)> {
        let chunk_keys: Vec<ChunkPos> = by_chunk.keys().copied().collect();
        self.ensure_chunks_generated(&chunk_keys);

        let mut packets = Vec::new();
        let mut guard = self.inner.storage.chunks.write();

        for (&cpos, changes) in by_chunk {
            let Some(chunk) = guard.get_mut(&cpos) else {
                continue;
            };
            let mut per_section: FxHashMap<i32, Vec<(u8, u8, u8, u16)>> = FxHashMap::default();
            for &(bv, block) in changes {
                if bv.y < chunk.min_y() || bv.y >= chunk.max_y() {
                    continue;
                }
                let lx = bv.x.rem_euclid(16) as usize;
                let lz = bv.z.rem_euclid(16) as usize;
                chunk.set_block(lx, bv.y, lz, block);
                let section_y = bv.y.div_euclid(16);
                let ly = bv.y.rem_euclid(16) as u8;
                per_section.entry(section_y).or_default().push((
                    lx as u8,
                    ly,
                    lz as u8,
                    block.state_id(),
                ));
            }
            for (sec_y, sec_entries) in per_section {
                packets.push((
                    cpos,
                    MultiBlockChangePacket::from_section_entries(
                        cpos.x,
                        sec_y,
                        cpos.z,
                        &sec_entries,
                    ),
                ));
            }
        }
        packets
    }

    /// Loads or generates any missing chunks in `positions` in parallel on `rayon` worker threads.
    pub fn ensure_chunks_generated(&self, positions: &[ChunkPos]) {
        let missing: Vec<ChunkPos> = {
            let guard = self.inner.storage.chunks.read();
            positions
                .iter()
                .copied()
                .filter(|p| !guard.contains_key(p))
                .collect()
        };
        if missing.is_empty() {
            return;
        }

        let loader = self.inner.storage.chunk_loader.load_full();
        let generator = self.inner.storage.generator.load_full();
        let lighting = self.inner.storage.lighting_enabled.load(Ordering::Acquire);
        let dim = self.inner.storage.dimension.clone();

        let generated: Vec<(ChunkPos, Chunk)> = missing
            .into_par_iter()
            .map(|cpos| {
                let mut chunk = loader
                    .as_ref()
                    .and_then(|l| l.load_chunk(&dim, cpos).ok().flatten())
                    .unwrap_or_else(|| {
                        let mut c = Chunk::new(cpos, &dim);
                        if let Some(gen_fn) = &generator {
                            let mut unit = GenerationUnit::new(&mut c);
                            gen_fn.generate(&mut unit);
                        }
                        c
                    });
                chunk.set_lighting_enabled(lighting);
                let _ = chunk.encoded_packet_bytes();
                (cpos, chunk)
            })
            .collect();

        let mut guard = self.inner.storage.chunks.write();
        for (cpos, chunk) in generated {
            guard.entry(cpos).or_insert(chunk);
        }
    }

    /// Returns cached `[VarInt 0x2D][ChunkDataPacket]` payloads for `positions`, loading/generating
    /// any missing chunks in parallel first.
    pub fn chunk_packet_payloads(&self, positions: &[ChunkPos]) -> Result<Vec<Bytes>, WorldError> {
        self.ensure_chunks_generated(positions);
        let mut out = Vec::with_capacity(positions.len());
        let mut guard = self.inner.storage.chunks.write();
        for cpos in positions {
            if let Some(chunk) = guard.get_mut(cpos) {
                out.push(chunk.encoded_packet_bytes()?);
            }
        }
        Ok(out)
    }

    /// Returns the number of currently loaded chunks in memory.
    #[must_use]
    pub fn loaded_chunk_count(&self) -> usize {
        self.inner.storage.chunks.read().len()
    }
}

impl BlockGetter for InstanceContainer {
    fn get_block(&self, pos: BlockVec) -> Block {
        let cpos = pos.chunk_pos();
        let guard = self.inner.storage.chunks.read();
        let Some(chunk) = guard.get(&cpos) else {
            return Block::AIR;
        };
        let lx = pos.x.rem_euclid(16) as usize;
        let lz = pos.z.rem_euclid(16) as usize;
        chunk.get_block(lx, pos.y, lz)
    }
}

/// Registry of active world instances (`InstanceManager` in Minestom).
#[derive(Default, Clone)]
pub struct InstanceManager {
    instances: Arc<RwLock<SlotMap<InstanceId, InstanceContainer>>>,
}

impl std::fmt::Debug for InstanceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceManager")
            .field("count", &self.instances.read().len())
            .finish()
    }
}

impl InstanceManager {
    /// Creates an empty `InstanceManager`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates and registers a new Overworld [`InstanceContainer`].
    #[must_use]
    pub fn create_instance_container(&self) -> InstanceContainer {
        self.create_instance_with_dimension(DimensionType::overworld())
    }

    /// Creates and registers an [`InstanceContainer`] with a custom [`DimensionType`].
    #[must_use]
    pub fn create_instance_with_dimension(&self, dimension: DimensionType) -> InstanceContainer {
        let mut guard = self.instances.write();
        let id = guard.insert_with_key(|id| InstanceContainer::new(id, dimension));
        guard[id].clone()
    }

    /// Creates and registers a [`SharedInstance`] backed by `parent`'s chunk storage (`0` extra
    /// chunk memory, independent [`InstanceId`], [`WorldBorder`], [`WorldTime`], and [`Weather`])!
    #[must_use]
    pub fn create_shared_instance(&self, parent: &InstanceContainer) -> SharedInstance {
        let mut guard = self.instances.write();
        let id = guard.insert_with_key(|id| InstanceContainer::new_shared(id, parent));
        SharedInstance {
            instance: guard[id].clone(),
        }
    }

    /// Looks up a registered [`InstanceContainer`] by [`InstanceId`].
    #[must_use]
    pub fn get_instance(&self, id: InstanceId) -> Option<InstanceContainer> {
        self.instances.read().get(id).cloned()
    }

    /// Returns a snapshot of all registered instances.
    #[must_use]
    pub fn instances(&self) -> Vec<InstanceContainer> {
        self.instances.read().values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_instance_shares_chunks_with_distinct_id_and_world_border() {
        let mgr = InstanceManager::new();
        let parent = mgr.create_instance_container();
        parent.set_generator(|unit: &mut GenerationUnit| {
            unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
        });
        parent.set_block(BlockVec::new(2, 40, 2), Block::DIAMOND_BLOCK);

        let shared = mgr.create_shared_instance(&parent);
        assert_ne!(parent.id(), shared.id());
        assert!(shared.is_shared());
        assert!(shared.shares_storage_with(&parent));
        assert_eq!(
            shared.get_block(BlockVec::new(2, 40, 2)),
            Block::DIAMOND_BLOCK
        );

        shared.set_world_border(WorldBorder::with_diameter(0.0, 0.0, 128.0));
        assert!((shared.world_border().diameter - 128.0).abs() < 1.0e-6);
        assert!(parent.world_border().diameter > 1_000_000.0);
    }
}
