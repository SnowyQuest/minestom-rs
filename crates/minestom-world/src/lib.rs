//! World instances (`InstanceContainer`, `SharedInstance`), paletted chunk sections,
//! cached chunk packets, parallel `Generator` API, lighting, and Anvil region loading.

/// Errors returned by world, chunk, or region operations.
#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    /// Underlying I/O error during chunk loading/saving.
    #[error("world I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Underlying NBT error in Anvil region data.
    #[error("world NBT error: {0}")]
    Nbt(#[from] minestom_nbt::NbtError),
    /// Requested coordinate or instance was invalid.
    #[error("invalid world operation: {0}")]
    InvalidOperation(String),
}
