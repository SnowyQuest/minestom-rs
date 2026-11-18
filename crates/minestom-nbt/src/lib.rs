//! Safe Named Binary Tag (NBT) and Stringified NBT (SNBT) support for `minestom-rs`.
//!
//! Supports both standard named-root NBT (used by Anvil region files and schematics)
//! and Minecraft 1.20.2+ nameless-root network NBT (used in protocol 776 packets).

/// Errors returned while reading, writing, or transcoding NBT payloads.
#[derive(Debug, thiserror::Error)]
pub enum NbtError {
    /// Underlying I/O failure.
    #[error("NBT I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Encountered an unknown NBT tag type ID byte.
    #[error("invalid NBT tag ID: 0x{0:02x}")]
    InvalidTagId(u8),
    /// Input buffer ended prematurely.
    #[error("unexpected end of NBT buffer")]
    UnexpectedEof,
    /// Recursion depth exceeded safety threshold.
    #[error("NBT nesting depth exceeded maximum ({0})")]
    DepthLimitExceeded(usize),
    /// Allocation size exceeded safety threshold.
    #[error("NBT allocation size exceeded limit ({size} > {max})")]
    AllocationLimitExceeded {
        /// Requested byte/element count.
        size: usize,
        /// Configured maximum count.
        max: usize,
    },
    /// Invalid UTF-8 / CESU-8 string payload.
    #[error("invalid UTF-8 in NBT string")]
    InvalidUtf8,
    /// Malformed SNBT string literal.
    #[error("SNBT syntax error: {0}")]
    SnbtSyntax(String),
}
