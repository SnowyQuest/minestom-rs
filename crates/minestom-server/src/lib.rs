//! `MinecraftServer` runtime, explicit cloneable handle, `ServerConfig`,
//! dedicated 20 TPS tick loop (`Ms-TickScheduler`), and TCP listener lifecycle.

pub mod config;
pub mod event;
mod session;
mod tick;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

use arc_swap::ArcSwap;
use bytes::Bytes;
use minestom_command::CommandManager;
use minestom_core::{Pos, Vec3};
use minestom_data::{PROTOCOL_VERSION, VERSION_NAME};
use minestom_entity::Entity;
use minestom_event::GlobalEventHandler;
use minestom_protocol::status::{ServerStatus, StatusPlayerSample, StatusPlayers, StatusVersion};
use minestom_registry::Registries;
use minestom_text::Component;
use minestom_world::{InstanceContainer, InstanceManager};
use tokio::net::{TcpListener, ToSocketAddrs};
use tokio::sync::watch;
use tracing::{Instrument, debug, info, info_span};

pub use config::ServerConfig;
pub use event::{
    AsyncPlayerConfigurationEvent, ClientPingServerEvent, PlayerChatEvent, PlayerCommandEvent,
    PlayerDisconnectEvent, PlayerMoveEvent, PlayerSpawnEvent, ServerListPingEvent,
};

use session::{build_cached_vanilla_config_packets, run_client_session};
use tick::{EntityCommand, TickMessage, spawn_tick_thread};

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
    pub(crate) registries: Registries,
    pub(crate) instance_manager: InstanceManager,
    pub(crate) command_manager: CommandManager,
    pub(crate) events: GlobalEventHandler,
    pub(crate) online_count: Arc<AtomicU32>,
    pub(crate) next_entity_id: AtomicI32,
    pub(crate) entity_cmd_tx: flume::Sender<EntityCommand>,
    pub(crate) entity_cmd_rx: flume::Receiver<EntityCommand>,
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

    /// Validates configuration, pre-caches vanilla Configuration packets, and builds a [`MinecraftServer`] handle.
    pub fn build(self) -> Result<MinecraftServer, ServerError> {
        self.config.validate()?;
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
        Ok(MinecraftServer {
            shared: Arc::new(ServerShared {
                config: self.config,
                registries,
                instance_manager: InstanceManager::new(),
                command_manager: CommandManager::new(),
                events: GlobalEventHandler::all("global"),
                online_count: Arc::new(AtomicU32::new(0)),
                next_entity_id: AtomicI32::new(1),
                entity_cmd_tx,
                entity_cmd_rx,
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

    /// Borrows the [`CommandManager`] (`server.command_manager()`).
    pub fn command_manager(&self) -> &CommandManager {
        &self.shared.command_manager
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
            self.server.shared.command_manager.clone(),
            Arc::clone(&self.server.shared.online_count),
            tick_rx,
            self.server.shared.entity_cmd_rx.clone(),
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
        Ok(())
    }
}
