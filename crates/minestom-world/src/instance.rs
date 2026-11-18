//! `InstanceContainer` and `InstanceManager` world management.

use std::sync::Arc;

use arc_swap::ArcSwapOption;
use bytes::Bytes;
use minestom_block::{Block, BlockGetter, BlockHandler};
use minestom_core::{BlockVec, ChunkPos, InstanceId, Point, Tag, tag::TagSerializable};
use minestom_nbt::NbtCompound;
use minestom_protocol::block::MultiBlockChangePacket;
use minestom_registry::DimensionType;
use parking_lot::RwLock;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use slotmap::SlotMap;

use crate::{Chunk, GenerationUnit, Generator, WorldError};

struct InstanceInner {
    id: InstanceId,
    dimension: DimensionType,
    generator: ArcSwapOption<Box<dyn Generator>>,
    chunks: RwLock<FxHashMap<ChunkPos, Chunk>>,
}

/// Cheaply cloneable handle to a world instance owning its chunk storage (`InstanceContainer` in Minestom).
#[derive(Clone)]
pub struct InstanceContainer {
    inner: Arc<InstanceInner>,
}

impl std::fmt::Debug for InstanceContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceContainer")
            .field("id", &self.inner.id)
            .field("dimension", &self.inner.dimension.key)
            .field("loaded_chunks", &self.inner.chunks.read().len())
            .finish()
    }
}

impl InstanceContainer {
    fn new(id: InstanceId, dimension: DimensionType) -> Self {
        Self {
            inner: Arc::new(InstanceInner {
                id,
                dimension,
                generator: ArcSwapOption::empty(),
                chunks: RwLock::new(FxHashMap::default()),
            }),
        }
    }

    /// Returns the generational [`InstanceId`] of this instance.
    #[must_use]
    pub fn id(&self) -> InstanceId {
        self.inner.id
    }

    /// Returns the [`DimensionType`] of this instance.
    #[must_use]
    pub fn dimension_type(&self) -> &DimensionType {
        &self.inner.dimension
    }

    /// Installs or replaces the parallel chunk [`Generator`] for newly loaded chunks.
    pub fn set_generator(&self, generator: impl Generator) {
        self.inner
            .generator
            .store(Some(Arc::new(Box::new(generator))));
    }

    /// Sets a single [`Block`] at world coordinate `pos`, generating the target chunk first if needed.
    pub fn set_block(&self, pos: impl Point, block: Block) {
        let cpos = pos.chunk_pos();
        self.ensure_chunks_generated(&[cpos]);
        let lx = pos.block_x().rem_euclid(16) as usize;
        let lz = pos.block_z().rem_euclid(16) as usize;
        if let Some(chunk) = self.inner.chunks.write().get_mut(&cpos) {
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
        if let Some(chunk) = self.inner.chunks.write().get_mut(&cpos) {
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
        if let Some(chunk) = self.inner.chunks.write().get_mut(&cpos) {
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
        let mut guard = self.inner.chunks.write();

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

    /// Generates any missing chunks in `positions` in parallel on `rayon` worker threads.
    pub fn ensure_chunks_generated(&self, positions: &[ChunkPos]) {
        let missing: Vec<ChunkPos> = {
            let guard = self.inner.chunks.read();
            positions
                .iter()
                .copied()
                .filter(|p| !guard.contains_key(p))
                .collect()
        };
        if missing.is_empty() {
            return;
        }

        let generator = self.inner.generator.load_full();
        let dim = self.inner.dimension.clone();
        let generated: Vec<(ChunkPos, Chunk)> = missing
            .into_par_iter()
            .map(|cpos| {
                let mut chunk = Chunk::new(cpos, &dim);
                if let Some(gen_fn) = &generator {
                    let mut unit = GenerationUnit::new(&mut chunk);
                    gen_fn.generate(&mut unit);
                }
                // Pre-encode packet payload on the parallel worker thread so tick/network threads do zero work!
                let _ = chunk.encoded_packet_bytes();
                (cpos, chunk)
            })
            .collect();

        let mut guard = self.inner.chunks.write();
        for (cpos, chunk) in generated {
            guard.entry(cpos).or_insert(chunk);
        }
    }

    /// Returns cached `[VarInt 0x2D][ChunkDataPacket]` payloads for `positions`, generating any missing
    /// chunks in parallel first.
    pub fn chunk_packet_payloads(&self, positions: &[ChunkPos]) -> Result<Vec<Bytes>, WorldError> {
        self.ensure_chunks_generated(positions);
        let mut out = Vec::with_capacity(positions.len());
        let mut guard = self.inner.chunks.write();
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
        self.inner.chunks.read().len()
    }
}

impl BlockGetter for InstanceContainer {
    fn get_block(&self, pos: BlockVec) -> Block {
        let cpos = pos.chunk_pos();
        let guard = self.inner.chunks.read();
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
    fn generator_fills_flat_grass_world_and_caches_packets() {
        let mgr = InstanceManager::new();
        let instance = mgr.create_instance_container();
        instance.set_generator(|unit: &mut GenerationUnit| {
            unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
        });

        let payloads = instance
            .chunk_packet_payloads(&[ChunkPos::new(0, 0), ChunkPos::new(1, -1)])
            .expect("generate chunk payloads");
        assert_eq!(payloads.len(), 2);
        assert_eq!(
            instance.get_block(BlockVec::new(5, 0, 5)),
            Block::GRASS_BLOCK
        );
        assert_eq!(
            instance.get_block(BlockVec::new(5, 39, 5)),
            Block::GRASS_BLOCK
        );
        assert_eq!(instance.get_block(BlockVec::new(5, 40, 5)), Block::AIR);
        assert_eq!(instance.get_block(BlockVec::new(5, -1, 5)), Block::AIR);
    }
}
