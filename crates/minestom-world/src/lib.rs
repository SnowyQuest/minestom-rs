//! World instances (`InstanceContainer`, `SharedInstance`), paletted chunk sections, cached
//! chunk packets, parallel `Generator`s, 3D BFS sky/block lighting (`LightingChunk`), Anvil `.mca`
//! region persistence (`AnvilLoader`), and `WorldBorder` / `WorldTime` / `Weather` state.

pub mod anvil;
pub mod batch;
pub mod chunk;
pub mod generator;
pub mod instance;
pub mod light;
pub mod palette;

pub use anvil::{AnvilLoader, ChunkLoader, decode_chunk_from_anvil_nbt, encode_chunk_to_anvil_nbt};
pub use batch::{AbsoluteBlockBatch, Batch, RelativeBlockBatch};
pub use chunk::{BlockEntityState, Chunk, Section};
pub use generator::{GenerationUnit, Generator, UnitModifier};
pub use instance::{
    InstanceContainer, InstanceManager, SharedInstance, Weather, WorldBorder, WorldTime,
};
pub use light::{
    build_update_light_packet, compute_chunk_light, pack_light_nibbles, unpack_light_nibbles,
};
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
