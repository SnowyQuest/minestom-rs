//! Asynchronous Tokio networking layer, framed packet streams, and split reader/writer channels.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use minestom_protocol::{
    FrameCodec, MAX_PACKET_SIZE, PacketPayload, ProtocolError, RawPacketFrame,
    encode_packet_payload,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

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

/// Framed asynchronous Minecraft TCP stream used during Handshake, Status, Login, and Configuration,
/// before splitting into dedicated Play reader/writer halves.
pub struct PacketStream {
    stream: TcpStream,
    peer_addr: SocketAddr,
    codec: FrameCodec,
    read_buf: BytesMut,
    write_buf: BytesMut,
}

impl PacketStream {
    /// Wraps an accepted or connected [`TcpStream`] with `TCP_NODELAY` enabled.
    pub fn new(stream: TcpStream, max_inbound_size: usize) -> Result<Self, NetworkError> {
        let _ = stream.set_nodelay(true);
        let peer_addr = stream
            .peer_addr()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        Ok(Self {
            stream,
            peer_addr,
            codec: FrameCodec::new(max_inbound_size),
            read_buf: BytesMut::with_capacity(4096),
            write_buf: BytesMut::with_capacity(4096),
        })
    }

    /// Remote socket address of this peer.
    #[must_use]
    pub const fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
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

    /// Frames and sends an already-encoded `[VarInt packet_id][payload]` byte slice (e.g. cached registries or chunks).
    pub async fn send_raw_body(&mut self, uncompressed_body: &[u8]) -> Result<(), NetworkError> {
        self.write_buf.clear();
        self.codec
            .encode_raw_body(uncompressed_body, &mut self.write_buf)?;
        self.stream.write_all(&self.write_buf).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Shuts down the write half of the TCP socket gracefully.
    pub async fn shutdown(&mut self) -> Result<(), NetworkError> {
        self.stream.shutdown().await?;
        Ok(())
    }

    /// Splits this stream into independent [`PacketReader`] and [`PacketWriter`] halves for Play state.
    #[must_use]
    pub fn into_split(self) -> (PacketReader, PacketWriter) {
        let mut reader_codec = self.codec.clone();
        reader_codec.set_max_inbound_size(MAX_PACKET_SIZE);
        let writer_codec = self.codec;
        let (read_half, write_half) = self.stream.into_split();
        (
            PacketReader {
                stream: read_half,
                codec: reader_codec,
                read_buf: self.read_buf,
            },
            PacketWriter {
                stream: write_half,
                codec: writer_codec,
                write_buf: self.write_buf,
            },
        )
    }
}

/// Owned read half of a Play-state [`PacketStream`].
pub struct PacketReader {
    stream: OwnedReadHalf,
    codec: FrameCodec,
    read_buf: BytesMut,
}

impl PacketReader {
    /// Reads the next complete [`RawPacketFrame`] with `timeout`.
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
}

/// Owned write half of a Play-state [`PacketStream`].
pub struct PacketWriter {
    stream: OwnedWriteHalf,
    codec: FrameCodec,
    write_buf: BytesMut,
}

impl PacketWriter {
    /// Frames and writes a pre-encoded `[VarInt packet_id][payload]` body, flushing immediately.
    pub async fn send_raw_body(&mut self, uncompressed_body: &[u8]) -> Result<(), NetworkError> {
        self.write_buf.clear();
        self.codec
            .encode_raw_body(uncompressed_body, &mut self.write_buf)?;
        self.stream.write_all(&self.write_buf).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Encodes a typed [`PacketPayload`] to `[VarInt id][payload]` `Bytes`.
    pub fn encode_payload<P: PacketPayload>(packet: &P) -> Result<Bytes, NetworkError> {
        Ok(encode_packet_payload(packet)?)
    }

    /// Gracefully shuts down the TCP write half.
    pub async fn shutdown(&mut self) -> Result<(), NetworkError> {
        self.stream.shutdown().await?;
        Ok(())
    }
}
