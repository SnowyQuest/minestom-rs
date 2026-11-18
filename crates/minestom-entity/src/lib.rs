//! Compositional `Entity`, `Player`, metadata, physics, spatial tracking, and AI traits.

use minestom_core::{GameMode, Pos};
use uuid::Uuid;

/// Errors returned by entity operations.
#[derive(Debug, thiserror::Error)]
pub enum EntityError {
    /// Referenced entity handle is no longer alive.
    #[error("entity no longer exists")]
    Despawned,
}

/// Mutable player configuration state exposed during `AsyncPlayerConfigurationEvent`
/// and initial spawn setup.
#[derive(Debug, Clone)]
pub struct ConfiguringPlayer {
    uuid: Uuid,
    username: String,
    respawn_point: Pos,
    game_mode: GameMode,
    permission_level: u8,
}

impl ConfiguringPlayer {
    /// Creates a new `ConfiguringPlayer` defaulting to `Pos(0, 40, 0)` and `GameMode::Survival`.
    #[must_use]
    pub const fn new(uuid: Uuid, username: String) -> Self {
        Self {
            uuid,
            username,
            respawn_point: Pos::new(0.0, 40.0, 0.0),
            game_mode: GameMode::Survival,
            permission_level: 0,
        }
    }

    /// Returns the player's UUID.
    #[must_use]
    pub const fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Returns the player's username.
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Returns the configured initial spawn / respawn point.
    #[must_use]
    pub const fn respawn_point(&self) -> Pos {
        self.respawn_point
    }

    /// Sets the initial spawn and respawn position (`player.set_respawn_point(...)` in Minestom).
    pub const fn set_respawn_point(&mut self, pos: Pos) {
        self.respawn_point = pos;
    }

    /// Returns the player's initial [`GameMode`].
    #[must_use]
    pub const fn game_mode(&self) -> GameMode {
        self.game_mode
    }

    /// Sets the player's initial [`GameMode`].
    pub const fn set_game_mode(&mut self, game_mode: GameMode) {
        self.game_mode = game_mode;
    }

    /// Returns the player's command permission level (`0..=4`).
    #[must_use]
    pub const fn permission_level(&self) -> u8 {
        self.permission_level
    }

    /// Sets the player's command permission level (`0..=4`).
    pub const fn set_permission_level(&mut self, level: u8) {
        self.permission_level = level;
    }
}
