//! Asynchronous Tokio networking layer, connection actors, zlib framing,
//! AES/CFB8 encryption, Mojang authentication, and Velocity/BungeeCord forwarding.

/// Errors returned by network transport, framing, and authentication.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    /// Socket I/O error.
    #[error("network I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Protocol framing/codec error.
    #[error("protocol error: {0}")]
    Protocol(#[from] minestom_protocol::ProtocolError),
    /// Authentication or proxy forwarding verification failure.
    #[error("authentication failed: {0}")]
    Auth(String),
    /// Peer exceeded packet rate limits or channel capacity.
    #[error("connection closed: {0}")]
    Disconnected(String),
}
