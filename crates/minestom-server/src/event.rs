//! Server-level and connection-lifecycle events (`AsyncPlayerConfigurationEvent`, `PlayerSpawnEvent`).

use minestom_core::{PlayerId, Pos};
use minestom_entity::ConfiguringPlayer;
use minestom_event::Event;
use minestom_world::InstanceContainer;

/// Fired during the Configuration phase after Login succeeds, allowing listeners to assign
/// the player's spawning [`InstanceContainer`], respawn point, and initial [`minestom_core::GameMode`].
#[derive(Debug)]
pub struct AsyncPlayerConfigurationEvent {
    player: ConfiguringPlayer,
    spawning_instance: Option<InstanceContainer>,
}

impl Event for AsyncPlayerConfigurationEvent {}

impl AsyncPlayerConfigurationEvent {
    /// Constructs a new `AsyncPlayerConfigurationEvent`.
    #[must_use]
    pub const fn new(player: ConfiguringPlayer) -> Self {
        Self {
            player,
            spawning_instance: None,
        }
    }

    /// Mutable access to the configuring player (`event.player().set_respawn_point(...)`).
    pub const fn player(&mut self) -> &mut ConfiguringPlayer {
        &mut self.player
    }

    /// Shared access to the configuring player.
    #[must_use]
    pub const fn player_ref(&self) -> &ConfiguringPlayer {
        &self.player
    }

    /// Assigns the [`InstanceContainer`] into which this player will spawn upon entering `Play`.
    pub fn set_spawning_instance(&mut self, instance: &InstanceContainer) {
        self.spawning_instance = Some(instance.clone());
    }

    /// Returns the assigned spawning [`InstanceContainer`], if set.
    #[must_use]
    pub const fn spawning_instance(&self) -> Option<&InstanceContainer> {
        self.spawning_instance.as_ref()
    }

    pub(crate) fn into_parts(self) -> (ConfiguringPlayer, Option<InstanceContainer>) {
        (self.player, self.spawning_instance)
    }
}

/// Fired on the 20 TPS tick thread once a player enters the `Play` state inside an instance.
#[derive(Debug, Clone)]
pub struct PlayerSpawnEvent {
    /// Generational handle of the spawned player.
    pub player_id: PlayerId,
    /// Player username.
    pub username: String,
    /// Spawn position.
    pub position: Pos,
    /// Target instance.
    pub instance: InstanceContainer,
    /// `true` on initial join, `false` on subsequent dimension switches.
    pub first_spawn: bool,
}

impl Event for PlayerSpawnEvent {}
