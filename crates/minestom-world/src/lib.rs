//! World instances (`InstanceContainer`), paletted chunk sections, cached chunk packets,
//! and parallel `Generator` / `GenerationUnit` / `UnitModifier` API.

pub mod batch;
pub mod chunk;
pub mod generator;
pub mod instance;
pub mod palette;

pub use batch::{AbsoluteBlockBatch, Batch, RelativeBlockBatch};
pub use chunk::{BlockEntityState, Chunk, Section};
pub use generator::{GenerationUnit, Generator, UnitModifier};
pub use instance::{InstanceContainer, InstanceManager};
pub use palette::Palette;

/// Errors returned by world, chunk, or region operations.
#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    /// Underlying I/O error during chunk loading/saving.
    #[error("world I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Underlying NBT error in Anvil region data.
    #[error("world NBT error: {0}")]
    Nbt(#[from] minestom_nbt::NbtError),
    /// Protocol encoding error while building a chunk packet.
    #[error("chunk packet encoding error: {0}")]
    Protocol(#[from] minestom_protocol::ProtocolError),
    /// Requested coordinate or instance was invalid.
    #[error("invalid world operation: {0}")]
    InvalidOperation(String),
}
