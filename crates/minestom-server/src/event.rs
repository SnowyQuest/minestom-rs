//! Server-level and connection-lifecycle events (`ServerListPingEvent`, `ClientPingServerEvent`,
//! `AsyncPlayerConfigurationEvent`, `PlayerSpawnEvent`, `PlayerMoveEvent`, `PlayerChatEvent`,
//! `PlayerCommandEvent`, and `PlayerDisconnectEvent`).

use minestom_core::{InstanceId, PlayerId, Pos};
use minestom_entity::ConfiguringPlayer;
use minestom_event::{CancellableEvent, Event, InstanceEvent, PlayerEvent};
use minestom_protocol::status::ServerStatus;
use minestom_text::Component;
use minestom_world::InstanceContainer;

/// Fired during the Status phase when a client sends `StatusRequestPacket` (`0x00`),
/// allowing listeners to dynamically customize the [`ServerStatus`] or cancel the response.
#[derive(Debug, Clone)]
pub struct ServerListPingEvent {
    /// Outbound [`ServerStatus`] payload (MOTD, player count/sample, favicon).
    pub status: ServerStatus,
    cancelled: bool,
}

impl Event for ServerListPingEvent {}

impl CancellableEvent for ServerListPingEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

impl ServerListPingEvent {
    /// Creates a new `ServerListPingEvent` wrapping `status`.
    #[must_use]
    pub const fn new(status: ServerStatus) -> Self {
        Self {
            status,
            cancelled: false,
        }
    }
}

/// Fired during the Status phase when a client sends `StatusPingRequestPacket` (`0x01`),
/// allowing listeners to override the echoed `payload` or cancel the pong response.
#[derive(Debug, Clone, Copy)]
pub struct ClientPingServerEvent {
    /// Echoed 64-bit ping payload.
    pub payload: i64,
    cancelled: bool,
}

impl Event for ClientPingServerEvent {}

impl CancellableEvent for ClientPingServerEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

impl ClientPingServerEvent {
    /// Creates a new `ClientPingServerEvent`.
    #[must_use]
    pub const fn new(payload: i64) -> Self {
        Self {
            payload,
            cancelled: false,
        }
    }
}

/// Fired during the Configuration phase after Login succeeds, allowing listeners to assign
/// the player's spawning [`InstanceContainer`], respawn point, initial [`minestom_core::GameMode`],
/// and permission level.
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

impl PlayerEvent for PlayerSpawnEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

impl InstanceEvent for PlayerSpawnEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance.id()
    }
}

/// Fired on the 20 TPS tick thread whenever a player sends a position or rotation movement packet.
///
/// Cancelling this event rejects the move and snaps the client back to `old_position` via
/// `PlayerPositionAndLookPacket`. Listeners may also rewrite `new_position` directly!
#[derive(Debug, Clone)]
pub struct PlayerMoveEvent {
    /// Generational [`PlayerId`].
    pub player_id: PlayerId,
    /// Player username.
    pub username: String,
    /// Current [`InstanceId`].
    pub instance_id: InstanceId,
    /// Position prior to this movement packet.
    pub old_position: Pos,
    /// Proposed target position (mutable by listeners).
    pub new_position: Pos,
    /// Whether the client reported standing on ground.
    pub on_ground: bool,
    cancelled: bool,
}

impl Event for PlayerMoveEvent {}

impl CancellableEvent for PlayerMoveEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

impl PlayerEvent for PlayerMoveEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

impl InstanceEvent for PlayerMoveEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
}

impl PlayerMoveEvent {
    /// Creates a new `PlayerMoveEvent`.
    #[must_use]
    pub const fn new(
        player_id: PlayerId,
        username: String,
        instance_id: InstanceId,
        old_position: Pos,
        new_position: Pos,
        on_ground: bool,
    ) -> Self {
        Self {
            player_id,
            username,
            instance_id,
            old_position,
            new_position,
            on_ground,
            cancelled: false,
        }
    }
}

/// Fired when a player sends a chat message (`ClientChatMessagePacket`).
///
/// Listeners can customize `formatted_message`, restrict `recipients`, or cancel broadcasting.
#[derive(Debug, Clone)]
pub struct PlayerChatEvent {
    /// Sender [`PlayerId`].
    pub player_id: PlayerId,
    /// Sender username.
    pub username: String,
    /// Raw chat string sent by the client.
    pub raw_message: String,
    /// Optional custom formatted [`Component`] (defaults to `<username> raw_message` when `None`).
    pub formatted_message: Option<Component>,
    /// Optional recipient override (`None` broadcasts to all online players).
    pub recipients: Option<Vec<PlayerId>>,
    cancelled: bool,
}

impl Event for PlayerChatEvent {}

impl CancellableEvent for PlayerChatEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

impl PlayerEvent for PlayerChatEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

impl PlayerChatEvent {
    /// Creates a new `PlayerChatEvent`.
    #[must_use]
    pub const fn new(player_id: PlayerId, username: String, raw_message: String) -> Self {
        Self {
            player_id,
            username,
            raw_message,
            formatted_message: None,
            recipients: None,
            cancelled: false,
        }
    }

    /// Resolves the final [`Component`] to broadcast.
    #[must_use]
    pub fn build_component(&self) -> Component {
        self.formatted_message
            .clone()
            .unwrap_or_else(|| Component::text(format!("<{}> {}", self.username, self.raw_message)))
    }
}

/// Fired when a player invokes a `/` command (`ClientCommandChatPacket` or `ClientSignedCommandChatPacket`)
/// before [`minestom_command::CommandManager`] executes it.
#[derive(Debug, Clone)]
pub struct PlayerCommandEvent {
    /// Invoking [`PlayerId`].
    pub player_id: PlayerId,
    /// Invoking username.
    pub username: String,
    /// Command line without leading `/` (mutable by listeners to rewrite aliases/macros).
    pub command: String,
    cancelled: bool,
}

impl Event for PlayerCommandEvent {}

impl CancellableEvent for PlayerCommandEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

impl PlayerEvent for PlayerCommandEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

impl PlayerCommandEvent {
    /// Creates a new `PlayerCommandEvent`.
    #[must_use]
    pub const fn new(player_id: PlayerId, username: String, command: String) -> Self {
        Self {
            player_id,
            username,
            command,
            cancelled: false,
        }
    }
}

/// Fired on the 20 TPS tick thread when an active player disconnects or times out.
#[derive(Debug, Clone)]
pub struct PlayerDisconnectEvent {
    /// Disconnected [`PlayerId`].
    pub player_id: PlayerId,
    /// Disconnected player's username.
    pub username: String,
    /// Last [`InstanceId`] the player occupied.
    pub instance_id: InstanceId,
}

impl Event for PlayerDisconnectEvent {}

impl PlayerEvent for PlayerDisconnectEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

impl InstanceEvent for PlayerDisconnectEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
}
