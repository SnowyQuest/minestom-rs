//! Core value types, coordinates (`Pos`, `Vec3`, `BlockVec`, `ChunkPos`),
//! `Key` (`NamespaceId`), generational handles, and typed `Tag<T>` storage for `minestom-rs`.

pub mod coord;
pub mod key;
pub mod tag;
pub mod types;

pub use coord::{BlockVec, ChunkPos, Point, Pos, Vec3};
pub use key::{Key, NamespaceId};
pub use tag::{Tag, TagMap, TagSerializable};
pub use types::{
    AlphaColor, BlockFace, Color, Difficulty, Direction, EntityId, GameMode, Hand, InstanceId,
    MainHand, PlayerId, offline_player_uuid,
};

/// Errors produced by `minestom-core` types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    /// Invalid resource location / namespace key (`namespace:value`).
    #[error("invalid resource key `{0}`")]
    InvalidKey(String),
    /// Out-of-range enum value.
    #[error("invalid {kind} ID: {value}")]
    InvalidEnumId {
        /// Enum type name.
        kind: &'static str,
        /// Invalid numeric ID.
        value: i32,
    },
}
