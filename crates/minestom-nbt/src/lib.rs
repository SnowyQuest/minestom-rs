//! Safe Named Binary Tag (NBT) and Stringified NBT (SNBT) support for `minestom-rs`.
//!
//! Supports both standard named-root NBT (used by Anvil region files and schematics)
//! and Minecraft 1.20.2+ nameless-root network NBT (used in protocol 776 packets),
//! including heterogeneous list wrapping (`{ "": tag }`) introduced in modern Minecraft.

mod io;
mod json;
mod snbt;
mod tag;

pub use io::{
    DEFAULT_MAX_NBT_BYTES, DEFAULT_MAX_NBT_DEPTH, read_gzip_named, read_named, read_network,
    read_network_compound, read_network_limited, read_zlib_named, write_gzip_named, write_named,
    write_network, write_network_compound, write_zlib_named,
};
pub use json::{json_to_nbt, nbt_to_json};
pub use snbt::{from_snbt, to_snbt};
pub use tag::{NbtCompound, NbtTag, TagType};

/// Errors returned while reading, writing, or transcoding NBT payloads.
#[derive(Debug, thiserror::Error)]
pub enum NbtError {
    /// Underlying I/O failure.
    #[error("NBT I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Encountered an unknown NBT tag type ID byte.
    #[error("invalid NBT tag ID: 0x{0:02x}")]
    InvalidTagId(u8),
    /// Expected a compound root tag (`0x0A`), found another tag type.
    #[error("expected NBT compound tag, got {0:?}")]
    ExpectedCompound(TagType),
    /// Non-empty NBT list declared `TAG_End` as its element type.
    #[error("non-empty NBT list cannot have TAG_End element type")]
    InvalidListElementType,
    /// Negative array or list length on the wire.
    #[error("negative NBT sequence length: {0}")]
    NegativeLength(i32),
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
    /// Invalid UTF-8 string payload.
    #[error("invalid UTF-8 in NBT string")]
    InvalidUtf8,
    /// Malformed SNBT string literal.
    #[error("SNBT syntax error: {0}")]
    SnbtSyntax(String),
}
