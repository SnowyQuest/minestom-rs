//! Core value types, coordinates (`Pos`, `Vec3`, `BlockVec`, `ChunkPos`),
//! `Key` (`NamespaceId`), generational handles, and typed `Tag<T>` storage for `minestom-rs`.

/// Errors produced by `minestom-core` types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    /// Invalid resource location / namespace key (`namespace:value`).
    #[error("invalid resource key `{0}`")]
    InvalidKey(String),
}
