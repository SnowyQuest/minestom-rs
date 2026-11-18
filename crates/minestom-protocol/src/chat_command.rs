//! Chat, Brigadier command graph (`DeclareCommandsPacket`), and tab-completion packets
//! for protocol 776 (MC 26.2).

use bytes::{Buf, BufMut, BytesMut};
use minestom_macros::PacketMeta;
use minestom_text::Component;

use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_prefixed_vec, read_string, read_var_int, write_prefixed_vec, write_string, write_var_int,
};

/// Serverbound `ClientCommandChatPacket` (`0x07` in [`ConnectionState::Play`], unsigned `/command`).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientCommandChatPacket {
    /// Command string without leading `/` (max `256` chars).
    pub message: String,
}

impl PacketPayload for ClientCommandChatPacket {
    const PACKET_ID: i32 = 0x07;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientCommandChatPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.message, 256)
    }
}

impl Decode for ClientCommandChatPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            message: read_string(buf, 256)?,
        })
    }
}

/// Argument signature pair inside [`ClientSignedCommandChatPacket`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentSignatureEntry {
    /// Argument name (max `16` chars).
    pub name: String,
    /// 256-byte RSA-SHA256 signature.
    pub signature: Vec<u8>,
}

impl Encode for ArgumentSignatureEntry {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        if self.signature.len() != 256 {
            return Err(ProtocolError::InvalidLength {
                len: self.signature.len() as i32,
                max: 256,
            });
        }
        write_string(buf, &self.name, 16)?;
        buf.put_slice(&self.signature);
        Ok(())
    }
}

impl Decode for ArgumentSignatureEntry {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let name = read_string(buf, 16)?;
        if buf.remaining() < 256 {
            return Err(ProtocolError::UnexpectedEof);
        }
        let mut signature = vec![0_u8; 256];
        buf.copy_to_slice(&mut signature);
        Ok(Self { name, signature })
    }
}

/// Serverbound `ClientSignedCommandChatPacket` (`0x08` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientSignedCommandChatPacket {
    /// Command string without leading `/` (max `256` chars).
    pub message: String,
    /// Epoch millis timestamp.
    pub timestamp: i64,
    /// Random salt.
    pub salt: i64,
    /// Per-argument signatures (max `8`).
    pub signatures: Vec<ArgumentSignatureEntry>,
    /// Last-seen message offset.
    pub ack_offset: i32,
    /// 20-bit `FixedBitSet(20)` (`3` bytes).
    pub ack_bitset: [u8; 3],
    /// 1-byte last-seen checksum.
    pub checksum: i8,
}

impl PacketPayload for ClientSignedCommandChatPacket {
    const PACKET_ID: i32 = 0x08;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientSignedCommandChatPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.message, 256)?;
        self.timestamp.encode(buf)?;
        self.salt.encode(buf)?;
        write_prefixed_vec(buf, &self.signatures, 8)?;
        write_var_int(buf, self.ack_offset);
        buf.put_slice(&self.ack_bitset);
        self.checksum.encode(buf)
    }
}

impl Decode for ClientSignedCommandChatPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let message = read_string(buf, 256)?;
        let timestamp = i64::decode(buf)?;
        let salt = i64::decode(buf)?;
        let signatures = read_prefixed_vec(buf, 8)?;
        let ack_offset = read_var_int(buf)?;
        if buf.remaining() < 3 {
            return Err(ProtocolError::UnexpectedEof);
        }
        let mut ack_bitset = [0_u8; 3];
        buf.copy_to_slice(&mut ack_bitset);
        let checksum = i8::decode(buf)?;
        Ok(Self {
            message,
            timestamp,
            salt,
            signatures,
            ack_offset,
            ack_bitset,
            checksum,
        })
    }
}

/// Serverbound `ClientChatMessagePacket` (`0x09` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientChatMessagePacket {
    /// Chat text (`1..=256` chars).
    pub message: String,
    /// Epoch millis timestamp.
    pub timestamp: i64,
    /// Random salt.
    pub salt: i64,
    /// Optional 256-byte message signature.
    pub signature: Option<Vec<u8>>,
    /// Last-seen message offset.
    pub ack_offset: i32,
    /// 20-bit `FixedBitSet(20)` (`3` bytes).
    pub ack_bitset: [u8; 3],
    /// 1-byte last-seen checksum.
    pub checksum: i8,
}

impl PacketPayload for ClientChatMessagePacket {
    const PACKET_ID: i32 = 0x09;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientChatMessagePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.message, 256)?;
        self.timestamp.encode(buf)?;
        self.salt.encode(buf)?;
        match &self.signature {
            None => false.encode(buf)?,
            Some(sig) => {
                if sig.len() != 256 {
                    return Err(ProtocolError::InvalidLength {
                        len: sig.len() as i32,
                        max: 256,
                    });
                }
                true.encode(buf)?;
                buf.put_slice(sig);
            }
        }
        write_var_int(buf, self.ack_offset);
        buf.put_slice(&self.ack_bitset);
        self.checksum.encode(buf)
    }
}

impl Decode for ClientChatMessagePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let message = read_string(buf, 256)?;
        let timestamp = i64::decode(buf)?;
        let salt = i64::decode(buf)?;
        let signature = if bool::decode(buf)? {
            if buf.remaining() < 256 {
                return Err(ProtocolError::UnexpectedEof);
            }
            let mut sig = vec![0_u8; 256];
            buf.copy_to_slice(&mut sig);
            Some(sig)
        } else {
            None
        };
        let ack_offset = read_var_int(buf)?;
        if buf.remaining() < 3 {
            return Err(ProtocolError::UnexpectedEof);
        }
        let mut ack_bitset = [0_u8; 3];
        buf.copy_to_slice(&mut ack_bitset);
        let checksum = i8::decode(buf)?;
        Ok(Self {
            message,
            timestamp,
            salt,
            signature,
            ack_offset,
            ack_bitset,
            checksum,
        })
    }
}

/// Serverbound `ClientTabCompletePacket` (`0x0F` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientTabCompletePacket {
    /// Correlation ID echoed in [`TabCompletePacket`].
    pub transaction_id: i32,
    /// Full chat box input up to cursor (`/` included, max `32_500` chars).
    pub text: String,
}

impl PacketPayload for ClientTabCompletePacket {
    const PACKET_ID: i32 = 0x0F;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientTabCompletePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.transaction_id);
        write_string(buf, &self.text, 32_500)
    }
}

impl Decode for ClientTabCompletePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            transaction_id: read_var_int(buf)?,
            text: read_string(buf, 32_500)?,
        })
    }
}

/// Single completion candidate inside [`TabCompletePacket`].
#[derive(Debug, Clone, PartialEq)]
pub struct TabCompleteMatch {
    /// Candidate token string.
    pub match_str: String,
    /// Optional hover tooltip component.
    pub tooltip: Option<Component>,
}

impl Encode for TabCompleteMatch {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.match_str, DEFAULT_MAX_STRING_CHARS)?;
        self.tooltip.encode(buf)
    }
}

impl Decode for TabCompleteMatch {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            match_str: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
            tooltip: Option::<Component>::decode(buf)?,
        })
    }
}

/// Clientbound `TabCompletePacket` (`0x0F` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct TabCompletePacket {
    /// Echoed client `transaction_id`.
    pub transaction_id: i32,
    /// Start character index in input string to replace.
    pub start: i32,
    /// Character length of substring to replace.
    pub length: i32,
    /// Suggested completions.
    pub matches: Vec<TabCompleteMatch>,
}

impl PacketPayload for TabCompletePacket {
    const PACKET_ID: i32 = 0x0F;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for TabCompletePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.transaction_id);
        write_var_int(buf, self.start);
        write_var_int(buf, self.length);
        write_prefixed_vec(buf, &self.matches, 32_767)
    }
}

impl Decode for TabCompletePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            transaction_id: read_var_int(buf)?,
            start: read_var_int(buf)?,
            length: read_var_int(buf)?,
            matches: read_prefixed_vec(buf, 32_767)?,
        })
    }
}

/// Brigadier parser IDs from `command_arguments.json` (`26.2-rv3`).
pub mod parser_ids {
    /// `brigadier:bool` (`0`).
    pub const BOOL: i32 = 0;
    /// `brigadier:float` (`1`).
    pub const FLOAT: i32 = 1;
    /// `brigadier:double` (`2`).
    pub const DOUBLE: i32 = 2;
    /// `brigadier:integer` (`3`).
    pub const INTEGER: i32 = 3;
    /// `brigadier:long` (`4`).
    pub const LONG: i32 = 4;
    /// `brigadier:string` (`5`).
    pub const STRING: i32 = 5;
    /// `minecraft:entity` (`6`).
    pub const ENTITY: i32 = 6;
    /// `minecraft:block_pos` (`8`).
    pub const BLOCK_POS: i32 = 8;
    /// `minecraft:vec3` (`10`).
    pub const VEC3: i32 = 10;
    /// `minecraft:score_holder` (`31`).
    pub const SCORE_HOLDER: i32 = 31;
    /// `minecraft:gamemode` (`42`).
    pub const GAMEMODE: i32 = 42;
    /// `minecraft:time` (`43`).
    pub const TIME: i32 = 43;
}

/// Node flags in [`CommandGraphNode`].
pub mod node_flags {
    /// Mask for node type (`0 = Root`, `1 = Literal`, `2 = Argument`).
    pub const TYPE_MASK: u8 = 0x03;
    /// Root node (`0`).
    pub const ROOT: u8 = 0x00;
    /// Literal keyword node (`1`).
    pub const LITERAL: u8 = 0x01;
    /// Typed argument node (`2`).
    pub const ARGUMENT: u8 = 0x02;
    /// Command is executable at this node (`0x04`).
    pub const EXECUTABLE: u8 = 0x04;
    /// Node redirects to `redirect_node` (`0x08`).
    pub const HAS_REDIRECT: u8 = 0x08;
    /// Node specifies custom `suggestions_type` (`0x10`, e.g. `"minecraft:ask_server"`).
    pub const HAS_SUGGESTIONS_TYPE: u8 = 0x10;
}

/// Single node inside [`DeclareCommandsPacket`] (`0x10`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandGraphNode {
    /// Bitmask of [`node_flags`].
    pub flags: u8,
    /// Indices of child nodes inside `DeclareCommandsPacket::nodes`.
    pub children: Vec<i32>,
    /// Target node index if `flags & HAS_REDIRECT != 0`.
    pub redirect_node: Option<i32>,
    /// Node name (present for `LITERAL` and `ARGUMENT` nodes).
    pub name: Option<String>,
    /// Brigadier/Minecraft argument parser ID (`0..=56`, present for `ARGUMENT` nodes).
    pub parser_id: Option<i32>,
    /// Parser-specific serialized property bytes (present for `ARGUMENT` nodes).
    pub properties: Vec<u8>,
    /// Suggestion provider key (e.g. `"minecraft:ask_server"`, present if `HAS_SUGGESTIONS_TYPE`).
    pub suggestions_type: Option<String>,
}

fn read_parser_properties<B: Buf>(buf: &mut B, parser_id: i32) -> Result<Vec<u8>, ProtocolError> {
    let mut out = BytesMut::new();
    match parser_id {
        parser_ids::FLOAT => {
            let flags = u8::decode(buf)?;
            out.put_u8(flags);
            if (flags & 0x01) != 0 {
                out.put_f32(f32::decode(buf)?);
            }
            if (flags & 0x02) != 0 {
                out.put_f32(f32::decode(buf)?);
            }
        }
        parser_ids::DOUBLE => {
            let flags = u8::decode(buf)?;
            out.put_u8(flags);
            if (flags & 0x01) != 0 {
                out.put_f64(f64::decode(buf)?);
            }
            if (flags & 0x02) != 0 {
                out.put_f64(f64::decode(buf)?);
            }
        }
        parser_ids::INTEGER => {
            let flags = u8::decode(buf)?;
            out.put_u8(flags);
            if (flags & 0x01) != 0 {
                out.put_i32(i32::decode(buf)?);
            }
            if (flags & 0x02) != 0 {
                out.put_i32(i32::decode(buf)?);
            }
        }
        parser_ids::LONG => {
            let flags = u8::decode(buf)?;
            out.put_u8(flags);
            if (flags & 0x01) != 0 {
                out.put_i64(i64::decode(buf)?);
            }
            if (flags & 0x02) != 0 {
                out.put_i64(i64::decode(buf)?);
            }
        }
        parser_ids::STRING => {
            let mode = read_var_int(buf)?;
            write_var_int(&mut out, mode);
        }
        parser_ids::ENTITY | parser_ids::SCORE_HOLDER => {
            out.put_u8(u8::decode(buf)?);
        }
        parser_ids::TIME => {
            out.put_i32(i32::decode(buf)?);
        }
        44..=48 => {
            let reg = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
            write_string(&mut out, &reg, DEFAULT_MAX_STRING_CHARS)?;
        }
        0..=56 => {}
        other => {
            return Err(ProtocolError::InvalidDiscriminant {
                type_name: "ArgumentParserType",
                value: other,
            });
        }
    }
    Ok(out.to_vec())
}

impl Encode for CommandGraphNode {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.flags.encode(buf)?;
        write_var_int(buf, self.children.len() as i32);
        for &child_idx in &self.children {
            write_var_int(buf, child_idx);
        }
        if (self.flags & node_flags::HAS_REDIRECT) != 0 {
            write_var_int(buf, self.redirect_node.unwrap_or(0));
        }
        let kind = self.flags & node_flags::TYPE_MASK;
        if kind == node_flags::LITERAL || kind == node_flags::ARGUMENT {
            write_string(
                buf,
                self.name.as_deref().unwrap_or(""),
                DEFAULT_MAX_STRING_CHARS,
            )?;
        }
        if kind == node_flags::ARGUMENT {
            write_var_int(buf, self.parser_id.unwrap_or(parser_ids::STRING));
            buf.put_slice(&self.properties);
        }
        if (self.flags & node_flags::HAS_SUGGESTIONS_TYPE) != 0 {
            write_string(
                buf,
                self.suggestions_type
                    .as_deref()
                    .unwrap_or("minecraft:ask_server"),
                DEFAULT_MAX_STRING_CHARS,
            )?;
        }
        Ok(())
    }
}

impl Decode for CommandGraphNode {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let flags = u8::decode(buf)?;
        let raw_len = read_var_int(buf)?;
        if !(0..=32_767).contains(&raw_len) {
            return Err(ProtocolError::InvalidLength {
                len: raw_len,
                max: 32_767,
            });
        }
        let mut children = Vec::with_capacity((raw_len as usize).min(buf.remaining()));
        for _ in 0..raw_len {
            children.push(read_var_int(buf)?);
        }
        let redirect_node = if (flags & node_flags::HAS_REDIRECT) != 0 {
            Some(read_var_int(buf)?)
        } else {
            None
        };
        let kind = flags & node_flags::TYPE_MASK;
        let name = if kind == node_flags::LITERAL || kind == node_flags::ARGUMENT {
            Some(read_string(buf, DEFAULT_MAX_STRING_CHARS)?)
        } else {
            None
        };
        let (parser_id, properties) = if kind == node_flags::ARGUMENT {
            let pid = read_var_int(buf)?;
            let props = read_parser_properties(buf, pid)?;
            (Some(pid), props)
        } else {
            (None, Vec::new())
        };
        let suggestions_type = if (flags & node_flags::HAS_SUGGESTIONS_TYPE) != 0 {
            Some(read_string(buf, DEFAULT_MAX_STRING_CHARS)?)
        } else {
            None
        };
        Ok(Self {
            flags,
            children,
            redirect_node,
            name,
            parser_id,
            properties,
            suggestions_type,
        })
    }
}

/// Clientbound `DeclareCommandsPacket` (`0x10` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct DeclareCommandsPacket {
    /// Flattened Brigadier command graph nodes.
    pub nodes: Vec<CommandGraphNode>,
    /// Index of the `ROOT` node inside `nodes` (usually `0`).
    pub root_index: i32,
}

impl PacketPayload for DeclareCommandsPacket {
    const PACKET_ID: i32 = 0x10;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for DeclareCommandsPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_prefixed_vec(buf, &self.nodes, 32_767)?;
        write_var_int(buf, self.root_index);
        Ok(())
    }
}

impl Decode for DeclareCommandsPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            nodes: read_prefixed_vec(buf, 32_767)?,
            root_index: read_var_int(buf)?,
        })
    }
}

/// Clientbound `DisguisedChatPacket` (`0x21` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct DisguisedChatPacket {
    /// Chat content component.
    pub message: Component,
    /// `minecraft:chat_type` registry ID.
    pub chat_type: i32,
    /// Formatted sender display name.
    pub sender_name: Component,
    /// Optional target display name (e.g. for `/msg`).
    pub target_name: Option<Component>,
}

impl PacketPayload for DisguisedChatPacket {
    const PACKET_ID: i32 = 0x21;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for DisguisedChatPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.message.encode(buf)?;
        write_var_int(buf, self.chat_type);
        self.sender_name.encode(buf)?;
        self.target_name.encode(buf)
    }
}

impl Decode for DisguisedChatPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            message: Component::decode(buf)?,
            chat_type: read_var_int(buf)?,
            sender_name: Component::decode(buf)?,
            target_name: Option::<Component>::decode(buf)?,
        })
    }
}

/// Clientbound `SystemChatPacket` (`0x79` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct SystemChatPacket {
    /// Chat or action-bar message (MC 26.2 NBT [`Component`]).
    pub message: Component,
    /// `false` = standard chat box, `true` = above-hotbar action bar!
    pub overlay: bool,
}

impl PacketPayload for SystemChatPacket {
    const PACKET_ID: i32 = 0x79;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for SystemChatPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.message.encode(buf)?;
        self.overlay.encode(buf)
    }
}

impl Decode for SystemChatPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            message: Component::decode(buf)?,
            overlay: bool::decode(buf)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::{decode_packet_exact, encode_packet_payload};

    #[test]
    fn declare_commands_and_chat_roundtrip() {
        let decl = DeclareCommandsPacket {
            nodes: vec![
                CommandGraphNode {
                    flags: node_flags::ROOT,
                    children: vec![1],
                    redirect_node: None,
                    name: None,
                    parser_id: None,
                    properties: Vec::new(),
                    suggestions_type: None,
                },
                CommandGraphNode {
                    flags: node_flags::LITERAL | node_flags::EXECUTABLE,
                    children: vec![2],
                    redirect_node: None,
                    name: Some("gamemode".to_owned()),
                    parser_id: None,
                    properties: Vec::new(),
                    suggestions_type: None,
                },
                CommandGraphNode {
                    flags: node_flags::ARGUMENT
                        | node_flags::EXECUTABLE
                        | node_flags::HAS_SUGGESTIONS_TYPE,
                    children: Vec::new(),
                    redirect_node: None,
                    name: Some("mode".to_owned()),
                    parser_id: Some(parser_ids::STRING),
                    properties: vec![0],
                    suggestions_type: Some("minecraft:ask_server".to_owned()),
                },
            ],
            root_index: 0,
        };
        let mut b = encode_packet_payload(&decl).unwrap();
        assert_eq!(read_var_int(&mut b).unwrap(), 0x10);
        assert_eq!(
            decode_packet_exact::<DeclareCommandsPacket>(b).unwrap(),
            decl
        );

        let chat = ClientChatMessagePacket {
            message: "Hello world!".to_owned(),
            timestamp: 123_456,
            salt: 789,
            signature: None,
            ack_offset: 0,
            ack_bitset: [0, 0, 0],
            checksum: 0,
        };
        let mut b2 = encode_packet_payload(&chat).unwrap();
        assert_eq!(read_var_int(&mut b2).unwrap(), 0x09);
        assert_eq!(
            decode_packet_exact::<ClientChatMessagePacket>(b2).unwrap(),
            chat
        );
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic_chat_command_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let b = bytes::Bytes::from(raw);
            let _ = decode_packet_exact::<ClientCommandChatPacket>(b.clone());
            let _ = decode_packet_exact::<ClientSignedCommandChatPacket>(b.clone());
            let _ = decode_packet_exact::<ClientChatMessagePacket>(b.clone());
            let _ = decode_packet_exact::<ClientTabCompletePacket>(b.clone());
            let _ = decode_packet_exact::<DeclareCommandsPacket>(b.clone());
            let _ = decode_packet_exact::<TabCompletePacket>(b.clone());
            let _ = decode_packet_exact::<DisguisedChatPacket>(b.clone());
            let _ = decode_packet_exact::<SystemChatPacket>(b);
        }
    }
}
