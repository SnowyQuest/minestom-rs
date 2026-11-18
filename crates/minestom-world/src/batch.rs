//! Bulk block mutation batches (`Batch`, `AbsoluteBlockBatch`, `RelativeBlockBatch`)
//! applying updates in a single per-chunk lock pass and emitting compact
//! [`MultiBlockChangePacket`]s (`net.minestom.server.instance.batch` in Minestom).

use minestom_block::{Block, BlockSetter};
use minestom_core::{BlockVec, ChunkPos, Point};
use minestom_protocol::block::MultiBlockChangePacket;
use rustc_hash::FxHashMap;

use crate::InstanceContainer;

/// Common interface implemented by bulk block batches (`Batch` in Minestom).
pub trait Batch: BlockSetter {
    /// Clears all buffered block entries.
    fn clear(&mut self);

    /// Number of buffered block placements.
    fn len(&self) -> usize;

    /// Returns `true` if the batch contains zero block placements.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Batch storing world-space `(BlockVec, Block)` mutations grouped by [`ChunkPos`]
/// (`AbsoluteBlockBatch` in Minestom).
#[derive(Debug, Clone, Default)]
pub struct AbsoluteBlockBatch {
    by_chunk: FxHashMap<ChunkPos, Vec<(BlockVec, Block)>>,
    total: usize,
}

impl AbsoluteBlockBatch {
    /// Creates an empty [`AbsoluteBlockBatch`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Buffers `block` at world coordinate `pos`.
    pub fn set_block_at(&mut self, pos: impl Point, block: Block) {
        let bv = pos.to_block_vec();
        self.by_chunk
            .entry(bv.chunk_pos())
            .or_default()
            .push((bv, block));
        self.total += 1;
    }

    /// Applies all buffered block changes to `instance` and returns grouped
    /// `(ChunkPos, MultiBlockChangePacket)` pairs ready to broadcast to chunk viewers!
    pub fn apply(&self, instance: &InstanceContainer) -> Vec<(ChunkPos, MultiBlockChangePacket)> {
        instance.apply_absolute_batch(&self.by_chunk)
    }
}

impl BlockSetter for AbsoluteBlockBatch {
    fn set_block(&mut self, pos: BlockVec, block: Block) {
        self.set_block_at(pos, block);
    }
}

impl Batch for AbsoluteBlockBatch {
    fn clear(&mut self) {
        self.by_chunk.clear();
        self.total = 0;
    }

    fn len(&self) -> usize {
        self.total
    }
}

/// Reusable relative structure/schematic batch anchored at `(0, 0, 0)` that can be stamped
/// at any world origin (`RelativeBlockBatch` in Minestom).
#[derive(Debug, Clone, Default)]
pub struct RelativeBlockBatch {
    entries: Vec<(BlockVec, Block)>,
}

impl RelativeBlockBatch {
    /// Creates an empty [`RelativeBlockBatch`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Buffers `block` at relative offset `(dx, dy, dz)`.
    pub fn set_relative(&mut self, dx: i32, dy: i32, dz: i32, block: Block) {
        self.entries.push((BlockVec::new(dx, dy, dz), block));
    }

    /// Translates this relative batch to `origin`, producing an [`AbsoluteBlockBatch`].
    #[must_use]
    pub fn to_absolute(&self, origin: impl Point) -> AbsoluteBlockBatch {
        let base = origin.to_block_vec();
        let mut abs = AbsoluteBlockBatch::new();
        for &(rel, block) in &self.entries {
            abs.set_block(
                BlockVec::new(base.x + rel.x, base.y + rel.y, base.z + rel.z),
                block,
            );
        }
        abs
    }

    /// Stamps this relative batch into `instance` at `origin` and returns per-section
    /// `(ChunkPos, MultiBlockChangePacket)` updates.
    pub fn apply_at(
        &self,
        instance: &InstanceContainer,
        origin: impl Point,
    ) -> Vec<(ChunkPos, MultiBlockChangePacket)> {
        self.to_absolute(origin).apply(instance)
    }
}

impl BlockSetter for RelativeBlockBatch {
    fn set_block(&mut self, pos: BlockVec, block: Block) {
        self.entries.push((pos, block));
    }
}

impl Batch for RelativeBlockBatch {
    fn clear(&mut self) {
        self.entries.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use minestom_block::BlockGetter;

    use crate::InstanceManager;

    use super::*;

    #[test]
    fn relative_and_absolute_batches_mutate_chunks_and_emit_multi_block_packets() {
        let mgr = InstanceManager::new();
        let inst = mgr.create_instance_container();

        let mut pillar = RelativeBlockBatch::new();
        for dy in 0..5 {
            pillar.set_relative(0, dy, 0, Block::DIAMOND_BLOCK);
        }

        let updates = pillar.apply_at(&inst, BlockVec::new(2, 40, 3));
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].1.blocks.len(), 5);
        assert_eq!(
            inst.get_block(BlockVec::new(2, 44, 3)),
            Block::DIAMOND_BLOCK
        );
    }
}
