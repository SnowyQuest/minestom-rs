//! Parallel-safe terrain `Generator`, `GenerationUnit`, and `UnitModifier` API.

use minestom_block::Block;
use minestom_core::{BlockVec, Point};

use crate::Chunk;

/// Fluent block modification view over a [`GenerationUnit`], matching Minestom's `UnitModifier`.
pub struct UnitModifier<'a> {
    chunk: &'a mut Chunk,
    base_x: i32,
    base_z: i32,
}

impl UnitModifier<'_> {
    /// Fills horizontal layers `min_y..max_y` (half-open `[min_y, max_y)`) across the entire unit with `block`.
    pub fn fill_height(&mut self, min_y: i32, max_y: i32, block: Block) {
        self.chunk.fill_height(min_y, max_y, block);
    }

    /// Fills the entire vertical range of this unit with `block`.
    pub fn fill(&mut self, block: Block) {
        let min_y = self.chunk.min_y();
        let max_y = self.chunk.max_y();
        self.chunk.fill_height(min_y, max_y, block);
    }

    /// Sets `block` at absolute world coordinates `pos` if `pos` falls inside this unit.
    pub fn set_block(&mut self, pos: impl Point, block: Block) {
        let bx = pos.block_x();
        let by = pos.block_y();
        let bz = pos.block_z();
        let lx = bx - self.base_x;
        let lz = bz - self.base_z;
        if (0..16).contains(&lx) && (0..16).contains(&lz) {
            self.chunk.set_block(lx as usize, by, lz as usize, block);
        }
    }

    /// Sets `block` at unit-relative `(x, y, z)` where `x, z` are in `0..16` and `y` is relative to `min_y`.
    pub fn set_relative(&mut self, x: usize, y: i32, z: usize, block: Block) {
        let world_y = self.chunk.min_y() + y;
        self.chunk.set_block(x, world_y, z, block);
    }
}

/// Chunk-scoped terrain generation context passed to [`Generator::generate`].
pub struct GenerationUnit<'a> {
    chunk: &'a mut Chunk,
}

impl<'a> GenerationUnit<'a> {
    /// Wraps `chunk` in a `GenerationUnit`.
    pub const fn new(chunk: &'a mut Chunk) -> Self {
        Self { chunk }
    }

    /// Minimum inclusive world block coordinate `(chunk_x * 16, min_y, chunk_z * 16)` of this unit.
    #[must_use]
    pub const fn absolute_start(&self) -> BlockVec {
        BlockVec::new(
            self.chunk.pos().x * 16,
            self.chunk.min_y(),
            self.chunk.pos().z * 16,
        )
    }

    /// Exclusive maximum world block coordinate `(chunk_x * 16 + 16, max_y, chunk_z * 16 + 16)`.
    #[must_use]
    pub const fn absolute_end(&self) -> BlockVec {
        BlockVec::new(
            self.chunk.pos().x * 16 + 16,
            self.chunk.max_y(),
            self.chunk.pos().z * 16 + 16,
        )
    }

    /// Returns a [`UnitModifier`] for mutating blocks inside this unit.
    pub const fn modifier(&mut self) -> UnitModifier<'_> {
        let base_x = self.chunk.pos().x * 16;
        let base_z = self.chunk.pos().z * 16;
        UnitModifier {
            chunk: self.chunk,
            base_x,
            base_z,
        }
    }
}

/// Thread-safe terrain generator invoked in parallel across chunks (`Generator` in Minestom).
pub trait Generator: Send + Sync + 'static {
    /// Populates blocks and biomes inside `unit`.
    fn generate(&self, unit: &mut GenerationUnit<'_>);
}

impl<F> Generator for F
where
    F: Fn(&mut GenerationUnit<'_>) + Send + Sync + 'static,
{
    fn generate(&self, unit: &mut GenerationUnit<'_>) {
        (self)(unit);
    }
}
