//! Minecraft: Java Edition **26.2** (protocol **776**) wire primitives, packet framing,
//! and clientbound/serverbound packet structs.

pub mod block;
pub mod chat_command;
pub mod codec;
pub mod config;
pub mod entity;
pub mod frame;
pub mod handshake;
pub mod inventory;
pub mod login;
pub mod play;
pub mod status;

pub use codec::{
    DEFAULT_MAX_STRING_CHARS, Decode, Encode, OptionalVarInt, PacketPayload, VarInt, VarLong,
    read_byte_array, read_prefixed_vec, read_string, read_var_int, read_var_long, var_int_len,
    write_byte_array, write_prefixed_vec, write_string, write_var_int, write_var_long,
};
pub use frame::{
    FrameCodec, MAX_PACKET_SIZE, MAX_PACKET_SIZE_PRE_AUTH, RawPacketFrame, decode_packet_exact,
    encode_packet_payload,
};

/// Protocol states in the Minecraft connection lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    /// Initial state before handshake intent is read.
    Handshake,
    /// Server list ping and MOTD status query state.
    Status,
    /// Authentication, encryption, and compression negotiation state.
    Login,
    /// Registry, known-pack, tag, and feature synchronization state.
    Configuration,
    /// Active gameplay state.
    Play,
}

/// Errors produced while decoding or encoding protocol packets and frames.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// Buffer ended before a complete value could be read.
    #[error("unexpected end of packet buffer")]
    UnexpectedEof,
    /// VarInt exceeded 5 bytes.
    #[error("VarInt is too large (exceeds 5 bytes)")]
    VarIntTooLarge,
    /// VarLong exceeded 10 bytes.
    #[error("VarLong is too large (exceeds 10 bytes)")]
    VarLongTooLarge,
    /// String exceeded its declared maximum character or UTF-8 byte length.
    #[error("string length {len} exceeds maximum allowed ({max})")]
    StringTooLong {
        /// Actual length encountered.
        len: usize,
        /// Maximum permitted length.
        max: usize,
    },
    /// Invalid UTF-8 bytes in protocol string.
    #[error("invalid UTF-8 string in packet")]
    InvalidUtf8,
    /// Collection/slice length prefix was negative or exceeded bounds.
    #[error("invalid length prefix {len} (max {max})")]
    InvalidLength {
        /// Decoded length value.
        len: i32,
        /// Maximum permitted length.
        max: usize,
    },
    /// Packet frame exceeded maximum permitted byte size.
    #[error("packet frame length {len} exceeds maximum ({max})")]
    PacketTooLarge {
        /// Declared frame size.
        len: usize,
        /// Maximum allowed size.
        max: usize,
    },
    /// Unknown packet ID for a given connection state.
    #[error("unknown packet ID 0x{id:02x} in state {state:?}")]
    UnknownPacketId {
        /// State in which the packet arrived.
        state: ConnectionState,
        /// Numeric packet ID.
        id: i32,
    },
    /// Invalid enum discriminant value on the wire.
    #[error("invalid discriminant {value} for {type_name}")]
    InvalidDiscriminant {
        /// Target type name.
        type_name: &'static str,
        /// Wire value.
        value: i32,
    },
    /// Packet had trailing unread bytes after decoding.
    #[error("packet had {0} trailing unread bytes")]
    TrailingBytes(usize),
    /// Frame compression or decompression error.
    #[error("packet compression error: {0}")]
    Compression(String),
    /// Invalid payload format (e.g. invalid base64 PNG favicon).
    #[error("invalid packet field: {0}")]
    InvalidField(String),
    /// Underlying NBT error.
    #[error("protocol NBT error: {0}")]
    Nbt(#[from] minestom_nbt::NbtError),
    /// Underlying JSON error.
    #[error("protocol JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// Underlying text error.
    #[error("protocol text error: {0}")]
    Text(#[from] minestom_text::TextError),
    /// Underlying core error.
    #[error("protocol core error: {0}")]
    Core(#[from] minestom_core::CoreError),
}
