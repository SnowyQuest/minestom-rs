//! Asynchronous Tokio networking layer, framed packet streams, and pre-play Handshake/Status actors.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use minestom_data::{PROTOCOL_VERSION, VERSION_NAME};
use minestom_protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom_protocol::login::LoginDisconnectPacket;
use minestom_protocol::status::{
    ServerStatus, StatusPingRequestPacket, StatusPongResponsePacket, StatusRequestPacket,
    StatusResponsePacket,
};
use minestom_protocol::{
    ConnectionState, FrameCodec, MAX_PACKET_SIZE_PRE_AUTH, PacketPayload, ProtocolError,
    RawPacketFrame, decode_packet_exact,
};
use minestom_text::{Component, NamedTextColor};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tracing::{Instrument, debug, info_span};

/// Default pre-authentication socket read timeout (`15s`, matching Minestom).
pub const PRE_AUTH_SOCKET_TIMEOUT: Duration = Duration::from_secs(15);

/// Errors returned by network transport, framing, and authentication.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    /// Socket I/O error.
    #[error("network I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Protocol framing/codec error.
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    /// Peer timed out waiting for packet data.
    #[error("connection timed out")]
    Timeout,
    /// Peer closed the TCP stream cleanly or unexpectedly.
    #[error("peer disconnected")]
    Closed,
}

/// Callback interface supplying the dynamic [`ServerStatus`] for server-list pings.
pub trait StatusHandler: Send + Sync + 'static {
    /// Builds the [`ServerStatus`] returned to `peer` after a handshake with `client_protocol`.
    fn build_status(&self, peer: SocketAddr, client_protocol: i32) -> ServerStatus;
}

/// Framed asynchronous Minecraft TCP stream supporting read timeouts, Zlib compression,
/// and zero-panic packet decoding.
pub struct PacketStream {
    stream: TcpStream,
    codec: FrameCodec,
    read_buf: BytesMut,
    write_buf: BytesMut,
}

impl PacketStream {
    /// Wraps an accepted or connected [`TcpStream`] with `TCP_NODELAY` enabled.
    pub fn new(stream: TcpStream, max_inbound_size: usize) -> Result<Self, NetworkError> {
        let _ = stream.set_nodelay(true);
        Ok(Self {
            stream,
            codec: FrameCodec::new(max_inbound_size),
            read_buf: BytesMut::with_capacity(4096),
            write_buf: BytesMut::with_capacity(4096),
        })
    }

    /// Updates the Zlib compression threshold on this stream.
    pub const fn set_compression_threshold(&mut self, threshold: Option<usize>) {
        self.codec.set_compression_threshold(threshold);
    }

    /// Updates the maximum permitted inbound frame byte size.
    pub const fn set_max_inbound_size(&mut self, max_inbound_size: usize) {
        self.codec.set_max_inbound_size(max_inbound_size);
    }

    /// Reads the next complete [`RawPacketFrame`], returning `Ok(None)` on clean EOF at frame boundary.
    pub async fn read_frame_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<RawPacketFrame>, NetworkError> {
        loop {
            if let Some(frame) = self.codec.try_decode_frame(&mut self.read_buf)? {
                return Ok(Some(frame));
            }

            let read_res = tokio::time::timeout(timeout, self.stream.read_buf(&mut self.read_buf))
                .await
                .map_err(|_| NetworkError::Timeout)?;
            let n = read_res?;
            if n == 0 {
                if self.read_buf.is_empty() {
                    return Ok(None);
                }
                return Err(NetworkError::Protocol(ProtocolError::UnexpectedEof));
            }
        }
    }

    /// Encodes `packet` and flushes it onto the underlying TCP socket.
    pub async fn send_packet<P: PacketPayload>(&mut self, packet: &P) -> Result<(), NetworkError> {
        self.write_buf.clear();
        self.codec.encode_packet(packet, &mut self.write_buf)?;
        self.stream.write_all(&self.write_buf).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Shuts down the write half of the TCP socket gracefully.
    pub async fn shutdown(&mut self) -> Result<(), NetworkError> {
        self.stream.shutdown().await?;
        Ok(())
    }
}

/// Handles a single inbound client connection through `Handshake` and `Status` (or version-checked `Login`).
pub async fn handle_status_connection(
    tcp: TcpStream,
    peer_addr: SocketAddr,
    status_handler: Arc<dyn StatusHandler>,
) -> Result<(), NetworkError> {
    let mut conn = PacketStream::new(tcp, MAX_PACKET_SIZE_PRE_AUTH)?;

    let Some(first_frame) = conn.read_frame_timeout(PRE_AUTH_SOCKET_TIMEOUT).await? else {
        return Ok(());
    };
    if first_frame.packet_id != ClientHandshakePacket::PACKET_ID {
        return Err(NetworkError::Protocol(ProtocolError::UnknownPacketId {
            state: ConnectionState::Handshake,
            id: first_frame.packet_id,
        }));
    }
    let handshake = decode_packet_exact::<ClientHandshakePacket>(first_frame.payload)?;

    match handshake.intent {
        HandshakeIntent::Status => {
            let mut status_requested = false;
            while let Some(frame) = conn.read_frame_timeout(PRE_AUTH_SOCKET_TIMEOUT).await? {
                match frame.packet_id {
                    StatusRequestPacket::PACKET_ID => {
                        if status_requested {
                            // Duplicate StatusRequestPacket is rejected per vanilla & Minestom StatusListener.
                            return Err(NetworkError::Closed);
                        }
                        let _ = decode_packet_exact::<StatusRequestPacket>(frame.payload)?;
                        status_requested = true;
                        let status =
                            status_handler.build_status(peer_addr, handshake.protocol_version);
                        conn.send_packet(&StatusResponsePacket { status }).await?;
                    }
                    StatusPingRequestPacket::PACKET_ID => {
                        let ping = decode_packet_exact::<StatusPingRequestPacket>(frame.payload)?;
                        conn.send_packet(&StatusPongResponsePacket {
                            payload: ping.payload,
                        })
                        .await?;
                        let _ = conn.shutdown().await;
                        return Ok(());
                    }
                    other => {
                        return Err(NetworkError::Protocol(ProtocolError::UnknownPacketId {
                            state: ConnectionState::Status,
                            id: other,
                        }));
                    }
                }
            }
            Ok(())
        }
        HandshakeIntent::Login | HandshakeIntent::Transfer => {
            let reason = if handshake.protocol_version != PROTOCOL_VERSION {
                Component::text(format!("Invalid Version, please use {VERSION_NAME}"))
                    .color(NamedTextColor::Red)
            } else if handshake.intent == HandshakeIntent::Transfer {
                Component::translatable("multiplayer.disconnect.transfers_disabled")
            } else {
                Component::text("Login state will be enabled in M2").color(NamedTextColor::Yellow)
            };
            conn.send_packet(&LoginDisconnectPacket { reason }).await?;
            let _ = conn.shutdown().await;
            Ok(())
        }
    }
}

/// Runs the TCP accept loop on `listener` until `shutdown_rx` signals `true`.
pub async fn run_accept_loop(
    listener: TcpListener,
    status_handler: Arc<dyn StatusHandler>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), NetworkError> {
    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (tcp, peer_addr) = match accepted {
                    Ok(pair) => pair,
                    Err(err) => {
                        debug!(%err, "transient TCP accept error");
                        continue;
                    }
                };
                let handler = Arc::clone(&status_handler);
                let span = info_span!("connection", %peer_addr);
                tokio::spawn(
                    async move {
                        if let Err(err) = handle_status_connection(tcp, peer_addr, handler).await {
                            debug!(%err, "client connection closed");
                        }
                    }
                    .instrument(span),
                );
            }
        }
    }
    Ok(())
}
