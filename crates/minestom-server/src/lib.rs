//! `MinecraftServer` runtime, explicit cloneable handle, `ServerConfig`,
//! dedicated 20 TPS tick loop (`Ms-TickScheduler`), and TCP listener lifecycle.

pub mod config;
pub mod event;
pub mod features;
mod session;
pub mod snapshot;
mod tick;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

use arc_swap::ArcSwap;
use bytes::Bytes;
use minestom_block::{Block, BlockManager};
use minestom_command::CommandManager;
use minestom_core::{BlockVec, PlayerId, Pos, Vec3};
use minestom_data::{PROTOCOL_VERSION, VERSION_NAME};
use minestom_entity::{Attribute, AttributeModifier, Damage, Entity, Potion, PotionEffect};
use minestom_event::GlobalEventHandler;
use minestom_item::{Inventory, ItemStack};
use minestom_nbt::NbtCompound;
use minestom_network::{MojangSessionClient, ServerEncryptionKeys};
use minestom_protocol::encode_packet_payload;
use minestom_protocol::status::{ServerStatus, StatusPlayerSample, StatusPlayers, StatusVersion};
use minestom_protocol::ui_media::{
    ActionBarPacket, ClearDialogPacket, ClearTitlesPacket, ParticlePacket,
    PlayerListHeaderAndFooterPacket, ResourcePackPopPacket, SoundSource, StopSoundPacket,
    TeamSettings,
};
use minestom_registry::Registries;
use minestom_scheduler::SchedulerManager;
use minestom_text::Component;
use minestom_world::{
    AbsoluteBlockBatch, InstanceContainer, InstanceManager, Weather, WorldBorder, WorldTime,
};
use tokio::net::{TcpListener, ToSocketAddrs};
use tokio::sync::watch;
use tracing::{Instrument, debug, info, info_span};
use uuid::Uuid;

pub use config::{AuthMode, ServerConfig};
pub use event::{
    AsyncPlayerConfigurationEvent, ClientPingServerEvent, PlayerBlockBreakEvent,
    PlayerBlockPlaceEvent, PlayerChatEvent, PlayerCommandEvent, PlayerDisconnectEvent,
    PlayerMoveEvent, PlayerSpawnEvent, ServerListPingEvent,
};
pub use features::{
    AdvancementTab, BossBar, Dialog, Particle, PlayerResourcePackStatusEvent, RecipeManager,
    ResourcePackRequest, Sidebar, SidebarLine, Sound, TeamManager, Title, TitleTimes,
    ToastNotification,
};
pub use snapshot::{
    ChunkSnapshot, EntitySnapshot, InstanceSnapshot, PlayerSnapshot, ServerSnapshot,
};

use session::{build_cached_vanilla_config_packets, run_client_session};
use tick::{
    EntityCommand, PlayerInvCommand, TickMessage, UiCommand, WorldBlockCommand, WorldEnvCommand,
    spawn_tick_thread,
};

/// Errors returned while configuring, starting, or running [`MinecraftServer`].
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// Underlying I/O or socket binding error.
    #[error("server I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Network subsystem error.
    #[error("network error: {0}")]
    Network(#[from] minestom_network::NetworkError),
    /// Protocol subsystem error.
    #[error("protocol error: {0}")]
    Protocol(#[from] minestom_protocol::ProtocolError),
    /// World subsystem error.
    #[error("world error: {0}")]
    World(#[from] minestom_world::WorldError),
    /// Configuration validation or TOML parsing error.
    #[error("invalid server configuration: {0}")]
    Config(String),
}

struct LiveStatusState {
    motd: Component,
    favicon: Option<Vec<u8>>,
    max_players: u32,
    sample: Vec<StatusPlayerSample>,
    enforces_secure_chat: bool,
}

pub(crate) struct ServerShared {
    pub(crate) config: ServerConfig,
    pub(crate) encryption_keys: Option<ServerEncryptionKeys>,
    pub(crate) session_client: Option<MojangSessionClient>,
    pub(crate) registries: Registries,
    pub(crate) instance_manager: InstanceManager,
    pub(crate) block_manager: BlockManager,
    pub(crate) command_manager: CommandManager,
    pub(crate) scheduler_manager: SchedulerManager,
    pub(crate) team_manager: TeamManager,
    pub(crate) recipe_manager: RecipeManager,
    pub(crate) latest_snapshot: Arc<ArcSwap<ServerSnapshot>>,
    pub(crate) events: GlobalEventHandler,
    pub(crate) online_count: Arc<AtomicU32>,
    pub(crate) next_entity_id: Arc<AtomicI32>,
    pub(crate) entity_cmd_tx: flume::Sender<EntityCommand>,
    pub(crate) entity_cmd_rx: flume::Receiver<EntityCommand>,
    pub(crate) block_cmd_tx: flume::Sender<WorldBlockCommand>,
    pub(crate) block_cmd_rx: flume::Receiver<WorldBlockCommand>,
    pub(crate) inv_cmd_tx: flume::Sender<PlayerInvCommand>,
    pub(crate) inv_cmd_rx: flume::Receiver<PlayerInvCommand>,
    pub(crate) env_cmd_tx: flume::Sender<WorldEnvCommand>,
    pub(crate) env_cmd_rx: flume::Receiver<WorldEnvCommand>,
    pub(crate) ui_cmd_tx: flume::Sender<UiCommand>,
    pub(crate) ui_cmd_rx: flume::Receiver<UiCommand>,
    live_status: ArcSwap<LiveStatusState>,
    pub(crate) cached_registry_packets: Vec<Bytes>,
    pub(crate) cached_tags_packet: Bytes,
    shutdown_tx: watch::Sender<bool>,
}

impl ServerShared {
    pub(crate) fn build_status(&self) -> ServerStatus {
        let snap = self.live_status.load();
        let online = self.online_count.load(Ordering::Relaxed);
        ServerStatus {
            version: StatusVersion {
                name: VERSION_NAME.to_owned(),
                protocol: PROTOCOL_VERSION,
            },
            players: Some(StatusPlayers {
                max: snap.max_players,
                online,
                sample: snap.sample.clone(),
            }),
            description: snap.motd.clone(),
            favicon: snap.favicon.clone(),
            enforces_secure_chat: snap.enforces_secure_chat,
        }
    }
}

/// Fluent builder for constructing a [`MinecraftServer`].
#[derive(Debug, Clone, Default)]
pub struct MinecraftServerBuilder {
    config: ServerConfig,
}

impl MinecraftServerBuilder {
    /// Replaces the entire [`ServerConfig`].
    #[must_use]
    pub fn config(mut self, config: ServerConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the server list MOTD [`Component`].
    #[must_use]
    pub fn motd(mut self, motd: impl Into<Component>) -> Self {
        self.config.motd = motd.into();
        self
    }

    /// Sets the maximum player slot count advertised in server status.
    #[must_use]
    pub const fn max_players(mut self, max_players: u32) -> Self {
        self.config.max_players = max_players;
        self
    }

    /// Sets the raw 64×64 PNG bytes used for the server list favicon.
    #[must_use]
    pub fn favicon(mut self, png_bytes: Vec<u8>) -> Self {
        self.config.favicon = Some(png_bytes);
        self
    }

    /// Sets the hover player sample shown in the server list.
    #[must_use]
    pub fn player_sample(mut self, sample: Vec<StatusPlayerSample>) -> Self {
        self.config.player_sample = sample;
        self
    }

    /// Sets the simulation tick rate (ticks per second, default `20`).
    #[must_use]
    pub const fn ticks_per_second(mut self, tps: u32) -> Self {
        self.config.ticks_per_second = tps;
        self
    }

    /// Sets the chunk view distance (in chunks, default `8`).
    #[must_use]
    pub const fn chunk_view_distance(mut self, chunks: u8) -> Self {
        self.config.chunk_view_distance = chunks;
        self
    }

    /// Sets the Zlib compression threshold (`None` disables compression; default `Some(256)`).
    #[must_use]
    pub const fn compression_threshold(mut self, threshold: Option<usize>) -> Self {
        self.config.compression_threshold = threshold;
        self
    }

    /// Sets the player authentication & proxy forwarding mode ([`AuthMode`]).
    #[must_use]
    pub fn auth(mut self, auth: AuthMode) -> Self {
        self.config.auth = auth;
        self
    }

    /// Sets the per-connection sustained inbound packet rate limit (`None` disables rate limiting).
    #[must_use]
    pub const fn max_packets_per_second(mut self, max_pps: Option<u32>) -> Self {
        self.config.max_packets_per_second = max_pps;
        self
    }

    /// Validates configuration, pre-caches vanilla Configuration packets, and builds a [`MinecraftServer`] handle.
    pub fn build(self) -> Result<MinecraftServer, ServerError> {
        self.config.validate()?;
        let (encryption_keys, session_client) = match &self.config.auth {
            AuthMode::Online {
                session_server_url,
                prevent_proxy_connections,
            } => (
                Some(ServerEncryptionKeys::generate()?),
                Some(MojangSessionClient::new(
                    session_server_url.clone(),
                    *prevent_proxy_connections,
                )),
            ),
            _ => (None, None),
        };
        let registries = Registries::vanilla();
        let (cached_registry_packets, cached_tags_packet) =
            build_cached_vanilla_config_packets(&registries)?;

        let live = LiveStatusState {
            motd: self.config.motd.clone(),
            favicon: self.config.favicon.clone(),
            max_players: self.config.max_players,
            sample: self.config.player_sample.clone(),
            enforces_secure_chat: self.config.enforces_secure_chat,
        };
        let (shutdown_tx, _) = watch::channel(false);
        let (entity_cmd_tx, entity_cmd_rx) = flume::unbounded();
        let (block_cmd_tx, block_cmd_rx) = flume::unbounded();
        let (inv_cmd_tx, inv_cmd_rx) = flume::unbounded();
        let (env_cmd_tx, env_cmd_rx) = flume::unbounded();
        let (ui_cmd_tx, ui_cmd_rx) = flume::unbounded();
        let tps = self.config.ticks_per_second;
        Ok(MinecraftServer {
            shared: Arc::new(ServerShared {
                config: self.config,
                encryption_keys,
                session_client,
                registries,
                instance_manager: InstanceManager::new(),
                block_manager: BlockManager::new(),
                command_manager: CommandManager::new(),
                scheduler_manager: SchedulerManager::with_tps(tps),
                team_manager: TeamManager::new(),
                recipe_manager: RecipeManager::new(),
                latest_snapshot: Arc::new(ArcSwap::from(ServerSnapshot::empty_arc())),
                events: GlobalEventHandler::all("global"),
                online_count: Arc::new(AtomicU32::new(0)),
                next_entity_id: Arc::new(AtomicI32::new(1)),
                entity_cmd_tx,
                entity_cmd_rx,
                block_cmd_tx,
                block_cmd_rx,
                inv_cmd_tx,
                inv_cmd_rx,
                env_cmd_tx,
                env_cmd_rx,
                ui_cmd_tx,
                ui_cmd_rx,
                live_status: ArcSwap::from_pointee(live),
                cached_registry_packets,
                cached_tags_packet,
                shutdown_tx,
            }),
        })
    }
}

/// Cheaply cloneable handle to a configured [`MinecraftServer`] instance (no hidden global state).
#[derive(Clone)]
pub struct MinecraftServer {
    shared: Arc<ServerShared>,
}

impl std::fmt::Debug for MinecraftServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MinecraftServer")
            .field("brand", &self.shared.config.brand_name)
            .field("tps", &self.shared.config.ticks_per_second)
            .finish_non_exhaustive()
    }
}

impl MinecraftServer {
    /// Creates a new [`MinecraftServerBuilder`] with default settings.
    #[must_use]
    pub fn builder() -> MinecraftServerBuilder {
        MinecraftServerBuilder::default()
    }

    /// Borrows the [`ServerConfig`].
    pub fn config(&self) -> &ServerConfig {
        &self.shared.config
    }

    /// Borrows the [`InstanceManager`] (`server.instance_manager()`).
    pub fn instance_manager(&self) -> &InstanceManager {
        &self.shared.instance_manager
    }

    /// Borrows the [`BlockManager`] (`server.block_manager()`).
    pub fn block_manager(&self) -> &BlockManager {
        &self.shared.block_manager
    }

    /// Borrows the [`CommandManager`] (`server.command_manager()`).
    pub fn command_manager(&self) -> &CommandManager {
        &self.shared.command_manager
    }

    /// Borrows the deterministic 20 TPS [`SchedulerManager`] (`server.scheduler_manager()`).
    pub fn scheduler_manager(&self) -> &SchedulerManager {
        &self.shared.scheduler_manager
    }

    /// Borrows the scoreboard [`TeamManager`] (`server.team_manager()`).
    pub fn team_manager(&self) -> &TeamManager {
        &self.shared.team_manager
    }

    /// Borrows the [`RecipeManager`] (`server.recipe_manager()`).
    pub fn recipe_manager(&self) -> &RecipeManager {
        &self.shared.recipe_manager
    }

    /// Returns a lock-free, consistent point-in-time [`ServerSnapshot`] captured at the end of
    /// the most recent 20 TPS tick (`ServerSnapshot` in Minestom).
    #[must_use]
    pub fn snapshot(&self) -> Arc<ServerSnapshot> {
        self.shared.latest_snapshot.load_full()
    }

    /// Borrows the root [`GlobalEventHandler`] (`server.global_event_handler()`).
    pub fn global_event_handler(&self) -> &GlobalEventHandler {
        &self.shared.events
    }

    /// Borrows the server [`Registries`].
    pub fn registries(&self) -> &Registries {
        &self.shared.registries
    }

    /// Returns the number of players currently connected in the `Play` state.
    pub fn online_count(&self) -> u32 {
        self.shared.online_count.load(Ordering::Relaxed)
    }

    /// Dynamically updates the server list MOTD without restarting.
    pub fn set_motd(&self, motd: impl Into<Component>) {
        let motd = motd.into();
        self.shared.live_status.rcu(|old| LiveStatusState {
            motd: motd.clone(),
            favicon: old.favicon.clone(),
            max_players: old.max_players,
            sample: old.sample.clone(),
            enforces_secure_chat: old.enforces_secure_chat,
        });
    }

    /// Dynamically updates the server list PNG favicon without restarting.
    pub fn set_favicon(&self, favicon: Option<&[u8]>) {
        self.shared.live_status.rcu(|old| LiveStatusState {
            motd: old.motd.clone(),
            favicon: favicon.map(<[u8]>::to_vec),
            max_players: old.max_players,
            sample: old.sample.clone(),
            enforces_secure_chat: old.enforces_secure_chat,
        });
    }

    /// Overrides the advertised online player count (useful for status-only frontends).
    pub fn set_online_count(&self, count: u32) {
        self.shared.online_count.store(count, Ordering::Relaxed);
    }

    /// Spawns a non-player [`Entity`] into `instance` at `position`, returning its assigned
    /// runtime network `entity_id`. Can be called before or after [`Self::start`]!
    pub fn spawn_entity(&self, instance: &InstanceContainer, entity: Entity, position: Pos) -> i32 {
        let network_id = self.shared.next_entity_id.fetch_add(1, Ordering::Relaxed);
        let _ = self.shared.entity_cmd_tx.send(EntityCommand::Spawn {
            network_id,
            instance: instance.clone(),
            entity: Box::new(entity),
            position,
        });
        network_id
    }

    /// Schedules removal/despawn of a spawned world entity by its `network_id`.
    pub fn remove_entity(&self, network_id: i32) {
        let _ = self
            .shared
            .entity_cmd_tx
            .send(EntityCommand::Remove { network_id });
    }

    /// Teleports a spawned world entity to `position` and syncs all viewers.
    pub fn teleport_entity(&self, network_id: i32, position: Pos) {
        let _ = self.shared.entity_cmd_tx.send(EntityCommand::Teleport {
            network_id,
            position,
        });
    }

    /// Sets the velocity of a spawned world entity and broadcasts `EntityVelocityPacket`.
    pub fn set_entity_velocity(&self, network_id: i32, velocity: Vec3) {
        let _ = self.shared.entity_cmd_tx.send(EntityCommand::SetVelocity {
            network_id,
            velocity,
        });
    }

    /// Applies [`Damage`] to any living world entity or online player identified by `target_network_id`
    /// (firing [`minestom_entity::EntityDamageEvent`], knockback, hit animation, and death handling).
    pub fn damage_entity(&self, target_network_id: i32, damage: Damage) {
        let _ = self
            .shared
            .entity_cmd_tx
            .send(EntityCommand::DamageByNetworkId {
                target_id: target_network_id,
                damage,
            });
    }

    /// Applies [`Damage`] to `player_id`.
    pub fn damage_player(&self, player_id: PlayerId, damage: Damage) {
        let _ = self
            .shared
            .entity_cmd_tx
            .send(EntityCommand::DamagePlayer { player_id, damage });
    }

    /// Restores `player_id` to full maximum health and standing pose.
    pub fn heal_player(&self, player_id: PlayerId) {
        let _ = self
            .shared
            .entity_cmd_tx
            .send(EntityCommand::HealPlayer { player_id });
    }

    /// Sets the base value of `attribute` on `target_network_id` and syncs `EntityAttributesPacket`.
    pub fn set_entity_attribute(
        &self,
        target_network_id: i32,
        attribute: Attribute,
        base_value: f64,
    ) {
        let _ = self
            .shared
            .entity_cmd_tx
            .send(EntityCommand::SetAttributeBase {
                target_id: target_network_id,
                attribute,
                base_value,
            });
    }

    /// Adds or replaces an [`AttributeModifier`] on `target_network_id` and syncs `EntityAttributesPacket`.
    pub fn add_entity_attribute_modifier(
        &self,
        target_network_id: i32,
        attribute: Attribute,
        modifier: AttributeModifier,
    ) {
        let _ = self
            .shared
            .entity_cmd_tx
            .send(EntityCommand::AddAttributeModifier {
                target_id: target_network_id,
                attribute,
                modifier,
            });
    }

    /// Sets the base value of `attribute` on `player_id` and syncs `EntityAttributesPacket`.
    pub fn set_player_attribute(&self, player_id: PlayerId, attribute: Attribute, base_value: f64) {
        let _ = self
            .shared
            .entity_cmd_tx
            .send(EntityCommand::SetPlayerAttributeBase {
                player_id,
                attribute,
                base_value,
            });
    }

    /// Adds or replaces an [`AttributeModifier`] on `player_id` and syncs `EntityAttributesPacket`.
    pub fn add_player_attribute_modifier(
        &self,
        player_id: PlayerId,
        attribute: Attribute,
        modifier: AttributeModifier,
    ) {
        let _ = self
            .shared
            .entity_cmd_tx
            .send(EntityCommand::AddPlayerAttributeModifier {
                player_id,
                attribute,
                modifier,
            });
    }

    /// Applies `potion` to `target_network_id` and broadcasts `EntityEffectPacket`.
    pub fn add_entity_effect(&self, target_network_id: i32, potion: Potion) {
        let _ = self.shared.entity_cmd_tx.send(EntityCommand::AddEffect {
            target_id: target_network_id,
            potion,
        });
    }

    /// Removes `effect` from `target_network_id` and broadcasts `RemoveEntityEffectPacket`.
    pub fn remove_entity_effect(&self, target_network_id: i32, effect: PotionEffect) {
        let _ = self.shared.entity_cmd_tx.send(EntityCommand::RemoveEffect {
            target_id: target_network_id,
            effect,
        });
    }

    /// Applies `potion` to `player_id` and broadcasts `EntityEffectPacket` to self & viewers.
    pub fn add_player_effect(&self, player_id: PlayerId, potion: Potion) {
        let _ = self
            .shared
            .entity_cmd_tx
            .send(EntityCommand::AddPlayerEffect { player_id, potion });
    }

    /// Removes `effect` from `player_id` and broadcasts `RemoveEntityEffectPacket`.
    pub fn remove_player_effect(&self, player_id: PlayerId, effect: PotionEffect) {
        let _ = self
            .shared
            .entity_cmd_tx
            .send(EntityCommand::RemovePlayerEffect { player_id, effect });
    }

    /// Spawns a dropped world `ItemEntity` into `instance` at `position` with `velocity` and
    /// `pickup_delay_ticks`, returning its assigned runtime network `entity_id`.
    pub fn spawn_item(
        &self,
        instance: &InstanceContainer,
        item_stack: ItemStack,
        position: Pos,
        velocity: Vec3,
        pickup_delay_ticks: u32,
    ) -> i32 {
        let entity = Entity::item(item_stack, pickup_delay_ticks).with_velocity(velocity);
        self.spawn_entity(instance, entity, position)
    }

    /// Adds `item` into an online player's [`minestom_item::PlayerInventory`] and synchronizes
    /// window `0` (and visible equipment if main/off-hand changed).
    pub fn give_player_item(&self, player_id: PlayerId, item: ItemStack) {
        let _ = self
            .shared
            .inv_cmd_tx
            .send(PlayerInvCommand::GiveItem { player_id, item });
    }

    /// Writes `item` directly into protocol window-0 `slot` (`0..46`) of `player_id` and syncs viewers.
    pub fn set_player_slot(&self, player_id: PlayerId, slot: usize, item: ItemStack) {
        let _ = self.shared.inv_cmd_tx.send(PlayerInvCommand::SetSlot {
            player_id,
            slot,
            item,
        });
    }

    /// Opens a custom container [`Inventory`] window for `player_id` (`OpenWindowPacket` + `WindowItemsPacket`).
    pub fn open_inventory(&self, player_id: PlayerId, inventory: &Inventory) {
        let _ = self.shared.inv_cmd_tx.send(PlayerInvCommand::OpenWindow {
            player_id,
            inventory: inventory.clone(),
        });
    }

    /// Closes any currently open container [`Inventory`] window for `player_id`.
    pub fn close_inventory(&self, player_id: PlayerId) {
        let _ = self
            .shared
            .inv_cmd_tx
            .send(PlayerInvCommand::CloseWindow { player_id });
    }

    /// Writes `block` (and optional block-entity `nbt`) into `instance` at `position` and
    /// broadcasts `BlockChangePacket` (+ `BlockEntityDataPacket` when applicable) to all chunk viewers!
    pub fn set_block_synced(
        &self,
        instance: &InstanceContainer,
        position: BlockVec,
        block: Block,
        nbt: Option<NbtCompound>,
    ) {
        let handler = self.shared.block_manager.get_handler(block.key());
        instance.set_block_with_data(position, block, nbt.clone(), handler);
        let _ = self
            .shared
            .block_cmd_tx
            .send(WorldBlockCommand::BroadcastSingle {
                instance: instance.clone(),
                position,
                block,
                nbt,
            });
    }

    /// Applies an [`AbsoluteBlockBatch`] to `instance` and broadcasts compact per-section
    /// `MultiBlockChangePacket`s to all chunk viewers!
    pub fn apply_batch_synced(&self, instance: &InstanceContainer, batch: &AbsoluteBlockBatch) {
        let packets = batch.apply(instance);
        if !packets.is_empty() {
            let _ = self
                .shared
                .block_cmd_tx
                .send(WorldBlockCommand::BroadcastBatch {
                    instance: instance.clone(),
                    packets,
                });
        }
    }

    /// Updates `instance`'s [`WorldBorder`] and broadcasts border packets (`lerp_ms = 0` for instant).
    pub fn set_world_border(
        &self,
        instance: &InstanceContainer,
        border: WorldBorder,
        lerp_ms: i64,
    ) {
        let old_diameter = instance.world_border().diameter;
        instance.set_world_border(border);
        let _ = self.shared.env_cmd_tx.send(WorldEnvCommand::Border {
            instance_id: instance.id(),
            old_diameter,
            border,
            lerp_ms,
        });
    }

    /// Updates `instance`'s [`WorldTime`] and immediately broadcasts MC 26.2 `SetTimePacket`.
    pub fn set_world_time(&self, instance: &InstanceContainer, time: WorldTime) {
        instance.set_world_time(time);
        let _ = self.shared.env_cmd_tx.send(WorldEnvCommand::Time {
            instance_id: instance.id(),
            time,
        });
    }

    /// Updates `instance`'s [`Weather`] and broadcasts rain/thunder `ChangeGameStatePacket`s.
    pub fn set_weather(&self, instance: &InstanceContainer, weather: Weather) {
        instance.set_weather(weather);
        let _ = self.shared.env_cmd_tx.send(WorldEnvCommand::Weather {
            instance_id: instance.id(),
            weather,
        });
    }

    fn send_play_packet_to_player<P: minestom_protocol::PacketPayload>(
        &self,
        player_id: PlayerId,
        pkt: &P,
    ) {
        if let Ok(payload) = encode_packet_payload(pkt) {
            let _ = self
                .shared
                .ui_cmd_tx
                .send(UiCommand::ToPlayer { player_id, payload });
        }
    }

    fn send_play_packet_to_players<P: minestom_protocol::PacketPayload>(
        &self,
        player_ids: Vec<PlayerId>,
        pkt: &P,
    ) {
        if player_ids.is_empty() {
            return;
        }
        if let Ok(payload) = encode_packet_payload(pkt) {
            let _ = self.shared.ui_cmd_tx.send(UiCommand::ToPlayers {
                player_ids,
                payload,
            });
        }
    }

    fn send_play_packet_to_all<P: minestom_protocol::PacketPayload>(&self, pkt: &P) {
        if let Ok(payload) = encode_packet_payload(pkt) {
            let _ = self.shared.ui_cmd_tx.send(UiCommand::ToAll { payload });
        }
    }

    /// Shows `sidebar` to `player_id` and subscribes them to live line/title updates.
    pub fn add_sidebar_viewer(&self, sidebar: &Sidebar, player_id: PlayerId) {
        let (create, display, scores) = sidebar.register_viewer(player_id);
        self.send_play_packet_to_player(player_id, &create);
        self.send_play_packet_to_player(player_id, &display);
        for s in scores {
            self.send_play_packet_to_player(player_id, &s);
        }
    }

    /// Removes `sidebar` from `player_id`.
    pub fn remove_sidebar_viewer(&self, sidebar: &Sidebar, player_id: PlayerId) {
        if let Some(pkt) = sidebar.unregister_viewer(player_id) {
            self.send_play_packet_to_player(player_id, &pkt);
        }
    }

    /// Inserts or updates `line` in `sidebar` and syncs all active viewers.
    pub fn set_sidebar_line(&self, sidebar: &Sidebar, line: SidebarLine) {
        if let Some((viewers, pkt)) = sidebar.set_line(line) {
            self.send_play_packet_to_players(viewers, &pkt);
        }
    }

    /// Updates `sidebar`'s title and syncs all active viewers.
    pub fn set_sidebar_title(&self, sidebar: &Sidebar, title: impl Into<Component>) {
        let (viewers, pkt) = sidebar.set_title(title);
        self.send_play_packet_to_players(viewers, &pkt);
    }

    /// Creates a scoreboard team and broadcasts `TeamsPacket` to all online players.
    pub fn create_team(&self, name: impl Into<String>, settings: TeamSettings) {
        let pkt = self.shared.team_manager.create_team(name, settings);
        self.send_play_packet_to_all(&pkt);
    }

    /// Adds a member (username or entity UUID string) to `team_name` and broadcasts `TeamsPacket`.
    pub fn add_team_member(&self, team_name: &str, entity: impl Into<String>) {
        if let Some(pkt) = self.shared.team_manager.add_member(team_name, entity) {
            self.send_play_packet_to_all(&pkt);
        }
    }

    /// Removes a member from `team_name` and broadcasts `TeamsPacket`.
    pub fn remove_team_member(&self, team_name: &str, entity: &str) {
        if let Some(pkt) = self.shared.team_manager.remove_member(team_name, entity) {
            self.send_play_packet_to_all(&pkt);
        }
    }

    /// Shows `boss_bar` to `player_id`.
    pub fn add_boss_bar_viewer(&self, boss_bar: &BossBar, player_id: PlayerId) {
        let pkt = boss_bar.register_viewer(player_id);
        self.send_play_packet_to_player(player_id, &pkt);
    }

    /// Hides `boss_bar` from `player_id`.
    pub fn remove_boss_bar_viewer(&self, boss_bar: &BossBar, player_id: PlayerId) {
        if let Some(pkt) = boss_bar.unregister_viewer(player_id) {
            self.send_play_packet_to_player(player_id, &pkt);
        }
    }

    /// Updates `boss_bar` progress (`0.0..=1.0`) and syncs all active viewers.
    pub fn set_boss_bar_progress(&self, boss_bar: &BossBar, progress: f32) {
        let (viewers, pkt) = boss_bar.set_progress(progress);
        self.send_play_packet_to_players(viewers, &pkt);
    }

    /// Updates `boss_bar` title and syncs all active viewers.
    pub fn set_boss_bar_title(&self, boss_bar: &BossBar, title: impl Into<Component>) {
        let (viewers, pkt) = boss_bar.set_title(title);
        self.send_play_packet_to_players(viewers, &pkt);
    }

    /// Sends a full-screen [`Title`] (`SetTitleTimePacket` + `SetTitleSubTitlePacket` + `SetTitleTextPacket`) to `player_id`.
    pub fn send_title(&self, player_id: PlayerId, title: &Title) {
        let (times, sub, main) = title.to_packets();
        if let Some(t) = times {
            self.send_play_packet_to_player(player_id, &t);
        }
        self.send_play_packet_to_player(player_id, &sub);
        self.send_play_packet_to_player(player_id, &main);
    }

    /// Clears active titles on `player_id`.
    pub fn clear_titles(&self, player_id: PlayerId, reset_times: bool) {
        self.send_play_packet_to_player(player_id, &ClearTitlesPacket { reset: reset_times });
    }

    /// Sends an above-hotbar action bar message (`ActionBarPacket` `0x57`) to `player_id`.
    pub fn send_action_bar(&self, player_id: PlayerId, text: impl Into<Component>) {
        self.send_play_packet_to_player(player_id, &ActionBarPacket { text: text.into() });
    }

    /// Sets the Tab-list header and footer (`PlayerListHeaderAndFooterPacket` `0x7A`) for `player_id`.
    pub fn set_player_list_header_footer(
        &self,
        player_id: PlayerId,
        header: impl Into<Component>,
        footer: impl Into<Component>,
    ) {
        self.send_play_packet_to_player(
            player_id,
            &PlayerListHeaderAndFooterPacket {
                header: header.into(),
                footer: footer.into(),
            },
        );
    }

    /// Broadcasts a [`ParticlePacket`] (`0x2F`) to all players in `instance`.
    pub fn spawn_particle(&self, instance: &InstanceContainer, packet: &ParticlePacket) {
        if let Ok(payload) = encode_packet_payload(packet) {
            let _ = self.shared.ui_cmd_tx.send(UiCommand::ToInstance {
                instance_id: instance.id(),
                payload,
            });
        }
    }

    /// Plays `sound` at `origin` to all players in `instance` (`SoundEffectPacket` `0x75`).
    pub fn play_sound_in_instance(
        &self,
        instance: &InstanceContainer,
        sound: &Sound,
        origin: Vec3,
    ) {
        let pkt = sound.to_packet_at(origin);
        if let Ok(payload) = encode_packet_payload(&pkt) {
            let _ = self.shared.ui_cmd_tx.send(UiCommand::ToInstance {
                instance_id: instance.id(),
                payload,
            });
        }
    }

    /// Plays `sound` at `origin` exclusively to `player_id`.
    pub fn play_sound_to_player(&self, player_id: PlayerId, sound: &Sound, origin: Vec3) {
        self.send_play_packet_to_player(player_id, &sound.to_packet_at(origin));
    }

    /// Stops matching sounds on `player_id` (`StopSoundPacket` `0x77`).
    pub fn stop_sound(
        &self,
        player_id: PlayerId,
        source: Option<SoundSource>,
        sound_key: Option<String>,
    ) {
        self.send_play_packet_to_player(
            player_id,
            &StopSoundPacket {
                source,
                sound: sound_key,
            },
        );
    }

    /// Sends a transient pop-up [`ToastNotification`] (`AdvancementsPacket` add + immediate remove) to `player_id`.
    pub fn send_toast(&self, player_id: PlayerId, toast: &ToastNotification) {
        let (add, remove) = toast.to_add_and_remove_packets();
        self.send_play_packet_to_player(player_id, &add);
        self.send_play_packet_to_player(player_id, &remove);
    }

    /// Synchronizes an [`AdvancementTab`] to `player_id`.
    pub fn send_advancement_tab(&self, player_id: PlayerId, tab: &AdvancementTab, reset: bool) {
        self.send_play_packet_to_player(player_id, &tab.to_packet(reset));
    }

    /// Broadcasts the current [`RecipeManager`] state (`DeclareRecipesPacket` `0x85`) to all online players.
    pub fn sync_recipes(&self) {
        self.send_play_packet_to_all(&self.shared.recipe_manager.to_declare_packet());
    }

    /// Pushes a [`ResourcePackRequest`] (`ResourcePackPushPacket` `0x51`) to `player_id` and tracks
    /// `forced` enforcement against `ClientResourcePackStatusPacket`.
    pub fn push_resource_pack(&self, player_id: PlayerId, request: &ResourcePackRequest) {
        if let Ok(push_payload) = encode_packet_payload(&request.to_packet()) {
            let _ = self.shared.ui_cmd_tx.send(UiCommand::TrackResourcePack {
                player_id,
                pack_id: request.id,
                forced: request.forced,
                push_payload,
            });
        }
    }

    /// Pops one (`Some(id)`) or all (`None`) server resource packs from `player_id` (`ResourcePackPopPacket` `0x50`).
    pub fn pop_resource_pack(&self, player_id: PlayerId, id: Option<Uuid>) {
        self.send_play_packet_to_player(player_id, &ResourcePackPopPacket { id });
    }

    /// Opens an MC 26.2 modal [`Dialog`] (`ShowDialogPacket` `0x8C`) on `player_id`.
    pub fn show_dialog(&self, player_id: PlayerId, dialog: &Dialog) {
        self.send_play_packet_to_player(player_id, &dialog.to_packet());
    }

    /// Closes any active modal dialog (`ClearDialogPacket` `0x8B`) on `player_id`.
    pub fn clear_dialog(&self, player_id: PlayerId) {
        self.send_play_packet_to_player(player_id, &ClearDialogPacket);
    }

    /// Saves all loaded chunks of `instance` via its [`minestom_world::ChunkLoader`] on Tokio's
    /// blocking thread pool (never blocking async I/O workers!).
    pub async fn save_instance_async(
        &self,
        instance: &InstanceContainer,
    ) -> Result<usize, ServerError> {
        let inst = instance.clone();
        let saved = tokio::task::spawn_blocking(move || inst.save_chunks_to_loader())
            .await
            .map_err(|e| ServerError::Io(std::io::Error::other(e.to_string())))??;
        Ok(saved)
    }

    /// Signals a running server loop and its 20 TPS tick thread to stop cleanly.
    pub fn stop_cleanly(&self) {
        let _ = self.shared.shutdown_tx.send(true);
    }

    /// Binds the TCP socket immediately without blocking on the accept loop yet,
    /// returning a [`BoundServer`] whose ephemeral port can be inspected via [`BoundServer::local_addr`].
    pub async fn bind<A: ToSocketAddrs>(&self, addr: A) -> Result<BoundServer, ServerError> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        Ok(BoundServer {
            server: self.clone(),
            listener,
            local_addr,
        })
    }

    /// Binds `addr`, starts the dedicated 20 TPS tick thread, and runs the network accept loop
    /// until [`Self::stop_cleanly`] is called.
    pub async fn start<A: ToSocketAddrs>(&self, addr: A) -> Result<(), ServerError> {
        let bound = self.bind(addr).await?;
        bound.run().await
    }
}

/// A [`MinecraftServer`] bound to a local TCP socket and ready to run.
pub struct BoundServer {
    server: MinecraftServer,
    listener: TcpListener,
    local_addr: SocketAddr,
}

impl BoundServer {
    /// Returns the bound local [`SocketAddr`] (useful when binding `"127.0.0.1:0"` in tests).
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Returns a cloneable [`MinecraftServer`] handle for controlling or stopping this server.
    #[must_use]
    pub fn handle(&self) -> MinecraftServer {
        self.server.clone()
    }

    /// Starts the dedicated 20 TPS tick thread (`Ms-TickScheduler`) and runs the Tokio accept loop
    /// until [`MinecraftServer::stop_cleanly`] is invoked.
    pub async fn run(self) -> Result<(), ServerError> {
        info!(
            address = %self.local_addr,
            version = VERSION_NAME,
            protocol = PROTOCOL_VERSION,
            tps = self.server.shared.config.ticks_per_second,
            "minestom-rs listening"
        );

        let (tick_tx, tick_rx) = flume::bounded::<TickMessage>(4096);
        let stop_flag = Arc::new(AtomicBool::new(false));
        let tick_handle = spawn_tick_thread(
            self.server.shared.config.clone(),
            self.server.shared.events.clone(),
            self.server.shared.instance_manager.clone(),
            self.server.shared.block_manager.clone(),
            self.server.shared.command_manager.clone(),
            self.server.shared.scheduler_manager.clone(),
            self.server.shared.team_manager.clone(),
            self.server.shared.recipe_manager.clone(),
            Arc::clone(&self.server.shared.latest_snapshot),
            Arc::clone(&self.server.shared.online_count),
            Arc::clone(&self.server.shared.next_entity_id),
            tick_rx,
            self.server.shared.entity_cmd_rx.clone(),
            self.server.shared.block_cmd_rx.clone(),
            self.server.shared.inv_cmd_rx.clone(),
            self.server.shared.env_cmd_rx.clone(),
            self.server.shared.ui_cmd_rx.clone(),
            Arc::clone(&stop_flag),
        )?;

        let mut shutdown_rx = self.server.shared.shutdown_tx.subscribe();
        loop {
            tokio::select! {
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }
                accepted = self.listener.accept() => {
                    let (tcp, peer_addr) = match accepted {
                        Ok(pair) => pair,
                        Err(err) => {
                            debug!(%err, "transient TCP accept error");
                            continue;
                        }
                    };
                    let shared = Arc::clone(&self.server.shared);
                    let tx = tick_tx.clone();
                    let span = info_span!("connection", %peer_addr);
                    tokio::spawn(
                        async move {
                            if let Err(err) = run_client_session(tcp, shared, tx).await {
                                debug!(%err, "client connection closed");
                            }
                        }
                        .instrument(span),
                    );
                }
            }
        }

        let _ = tick_tx.send(TickMessage::Shutdown);
        stop_flag.store(true, Ordering::Release);
        let _ = tokio::task::spawn_blocking(move || tick_handle.join()).await;
        // Allow async writer tasks to flush PlayDisconnectPacket frames cleanly
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Persist all non-shared world instances that have a configured ChunkLoader!
        let instances = self.server.shared.instance_manager.instances();
        let _ = tokio::task::spawn_blocking(move || {
            for inst in instances {
                if !inst.is_shared() {
                    let _ = inst.save_chunks_to_loader();
                }
            }
        })
        .await;
        Ok(())
    }
}
