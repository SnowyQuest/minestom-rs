//! `MinecraftServer` runtime, explicit cloneable handle, `ServerConfig`,
//! and TCP listener lifecycle.

pub mod config;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use arc_swap::ArcSwap;
use minestom_data::{PROTOCOL_VERSION, VERSION_NAME};
use minestom_network::{StatusHandler, run_accept_loop};
use minestom_protocol::status::{ServerStatus, StatusPlayerSample, StatusPlayers, StatusVersion};
use minestom_text::Component;
use tokio::net::{TcpListener, ToSocketAddrs};
use tokio::sync::watch;
use tracing::info;

pub use config::ServerConfig;

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

struct ServerShared {
    config: ServerConfig,
    online_count: AtomicU32,
    live_status: ArcSwap<LiveStatusState>,
    shutdown_tx: watch::Sender<bool>,
}

impl StatusHandler for ServerShared {
    fn build_status(&self, _peer: SocketAddr, _client_protocol: i32) -> ServerStatus {
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

    /// Validates configuration and builds a [`MinecraftServer`] handle.
    pub fn build(self) -> Result<MinecraftServer, ServerError> {
        self.config.validate()?;
        let live = LiveStatusState {
            motd: self.config.motd.clone(),
            favicon: self.config.favicon.clone(),
            max_players: self.config.max_players,
            sample: self.config.player_sample.clone(),
            enforces_secure_chat: self.config.enforces_secure_chat,
        };
        let (shutdown_tx, _) = watch::channel(false);
        Ok(MinecraftServer {
            shared: Arc::new(ServerShared {
                config: self.config,
                online_count: AtomicU32::new(0),
                live_status: ArcSwap::from_pointee(live),
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

    /// Borrows the initial [`ServerConfig`].
    pub fn config(&self) -> &ServerConfig {
        &self.shared.config
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

    /// Updates the advertised online player count (used before/alongside live Play sessions).
    pub fn set_online_count(&self, count: u32) {
        self.shared.online_count.store(count, Ordering::Relaxed);
    }

    /// Signals a running server loop to stop accepting new connections and shut down cleanly.
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

    /// Binds `addr` and runs the server until [`Self::stop_cleanly`] is called.
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

    /// Runs the network accept loop until [`MinecraftServer::stop_cleanly`] is invoked.
    pub async fn run(self) -> Result<(), ServerError> {
        info!(
            address = %self.local_addr,
            version = VERSION_NAME,
            protocol = PROTOCOL_VERSION,
            "minestom-rs listening"
        );
        let shutdown_rx = self.server.shared.shutdown_tx.subscribe();
        let handler: Arc<dyn StatusHandler> = Arc::clone(&self.server.shared) as _;
        run_accept_loop(self.listener, handler, shutdown_rx).await?;
        Ok(())
    }
}
