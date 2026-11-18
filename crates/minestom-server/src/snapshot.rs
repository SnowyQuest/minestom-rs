//! Immutable point-in-time server, instance, chunk, entity, and player read snapshots
//! (`net.minestom.server.snapshot` equivalent).
//!
//! Updated atomically at the end of every 20 TPS tick via `ArcSwap` so async tasks,
//! HTTP/metrics endpoints, and background jobs can read a consistent snapshot in O(1)
//! without locking or racing the tick thread.

use std::sync::Arc;

use minestom_core::{ChunkPos, GameMode, InstanceId, PlayerId, Pos, Vec3};
use minestom_entity::EntityType;
use uuid::Uuid;

/// Point-in-time immutable snapshot of an online player (`PlayerSnapshot` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSnapshot {
    /// Generational [`PlayerId`].
    pub player_id: PlayerId,
    /// Protocol entity ID.
    pub entity_id: i32,
    /// Player UUID.
    pub uuid: Uuid,
    /// Player username.
    pub username: String,
    /// Active [`GameMode`].
    pub game_mode: GameMode,
    /// Current [`InstanceId`].
    pub instance_id: InstanceId,
    /// Current world [`Pos`].
    pub position: Pos,
    /// Current health points.
    pub health: f32,
    /// Maximum health points.
    pub max_health: f32,
    /// Whether resting on solid ground.
    pub on_ground: bool,
}

/// Point-in-time immutable snapshot of a non-player world entity (`EntitySnapshot` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySnapshot {
    /// Protocol entity ID.
    pub entity_id: i32,
    /// Entity UUID.
    pub uuid: Uuid,
    /// Vanilla [`EntityType`].
    pub entity_type: EntityType,
    /// Hosting [`InstanceId`].
    pub instance_id: InstanceId,
    /// Current world [`Pos`].
    pub position: Pos,
    /// Current velocity in blocks/tick.
    pub velocity: Vec3,
    /// Current health if living.
    pub health: Option<f32>,
}

/// Point-in-time summary of a loaded chunk (`ChunkSnapshot` in Minestom).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkSnapshot {
    /// Chunk coordinates.
    pub chunk_pos: ChunkPos,
    /// Number of viewing players with this chunk loaded.
    pub viewer_count: usize,
}

/// Point-in-time immutable snapshot of a registered world instance (`InstanceSnapshot` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceSnapshot {
    /// Instance handle ID.
    pub instance_id: InstanceId,
    /// Dimension key (e.g. `"minecraft:overworld"`).
    pub dimension_key: String,
    /// Whether this instance shares chunk storage (`SharedInstance`).
    pub is_shared: bool,
    /// World age in ticks.
    pub world_age: i64,
    /// Overworld day time in ticks.
    pub time_of_day: i64,
    /// Loaded chunk summaries.
    pub chunks: Vec<ChunkSnapshot>,
    /// Entity IDs currently inside this instance.
    pub entity_ids: Vec<i32>,
    /// Player IDs currently inside this instance.
    pub player_ids: Vec<PlayerId>,
}

/// Point-in-time immutable snapshot of the entire [`crate::MinecraftServer`] (`ServerSnapshot` in Minestom).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerSnapshot {
    /// Monotonic server tick number when this snapshot was captured.
    pub tick_number: u64,
    /// Snapshots of all online Play-state players.
    pub players: Vec<PlayerSnapshot>,
    /// Snapshots of all spawned non-player world entities.
    pub entities: Vec<EntitySnapshot>,
    /// Snapshots of all registered world instances.
    pub instances: Vec<InstanceSnapshot>,
}

impl ServerSnapshot {
    /// Wraps an empty initial snapshot in an [`Arc`].
    #[must_use]
    pub fn empty_arc() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Finds a [`PlayerSnapshot`] by [`PlayerId`].
    #[must_use]
    pub fn player(&self, player_id: PlayerId) -> Option<&PlayerSnapshot> {
        self.players.iter().find(|p| p.player_id == player_id)
    }

    /// Finds a [`PlayerSnapshot`] by username.
    #[must_use]
    pub fn player_by_name(&self, username: &str) -> Option<&PlayerSnapshot> {
        self.players.iter().find(|p| p.username == username)
    }

    /// Finds an [`EntitySnapshot`] by protocol `entity_id`.
    #[must_use]
    pub fn entity(&self, entity_id: i32) -> Option<&EntitySnapshot> {
        self.entities.iter().find(|e| e.entity_id == entity_id)
    }

    /// Finds an [`InstanceSnapshot`] by [`InstanceId`].
    #[must_use]
    pub fn instance(&self, instance_id: InstanceId) -> Option<&InstanceSnapshot> {
        self.instances.iter().find(|i| i.instance_id == instance_id)
    }
}
