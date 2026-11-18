//! `MinecraftServer` runtime, explicit `Server` handle, `ServerConfig`,
//! partitioned 20 TPS tick loop (`ThreadDispatcher`), and protocol handlers.

/// Errors returned while configuring, starting, or running `MinecraftServer`.
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
