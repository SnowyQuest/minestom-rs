//! Handshake state (`ConnectionState::Handshake`) packets.

use bytes::{Buf, BufMut};
use minestom_macros::PacketMeta;

use crate::{
    ConnectionState, Decode, Encode, PacketPayload, ProtocolError, read_string, read_var_int,
    write_string, write_var_int,
};

/// Maximum server address length permitted for BungeeCord/BungeeGuard forwarding (`2500`),
/// while standard vanilla handshakes use `255`.
pub const MAX_HANDSHAKE_ADDRESS_CHARS: usize = 2500;

/// Next-state intent requested by [`ClientHandshakePacket`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum HandshakeIntent {
    /// Transition to [`ConnectionState::Status`] (`1`).
    Status = 1,
    /// Transition to [`ConnectionState::Login`] (`2`).
    Login = 2,
    /// Transition to [`ConnectionState::Login`] via server transfer (`3`).
    Transfer = 3,
}

impl HandshakeIntent {
    /// Converts a wire `VarInt` intent (`1..=3`) into [`HandshakeIntent`].
    pub const fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            1 => Ok(Self::Status),
            2 => Ok(Self::Login),
            3 => Ok(Self::Transfer),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "HandshakeIntent",
                value: other,
            }),
        }
    }

    /// Returns the target [`ConnectionState`] entered immediately after this handshake.
    #[must_use]
    pub const fn next_state(self) -> ConnectionState {
        match self {
            Self::Status => ConnectionState::Status,
            Self::Login | Self::Transfer => ConnectionState::Login,
        }
    }
}

/// Serverbound `ClientHandshakePacket` (`0x00` in [`ConnectionState::Handshake`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientHandshakePacket {
    /// Client protocol version (e.g. `776` for Minecraft 26.2).
    pub protocol_version: i32,
    /// Hostname or proxy-forwarded payload used to connect.
    pub server_address: String,
    /// TCP port used to connect.
    pub server_port: u16,
    /// Requested next connection state (`Status`, `Login`, or `Transfer`).
    pub intent: HandshakeIntent,
}

impl PacketPayload for ClientHandshakePacket {
    const PACKET_ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Handshake;
}

impl Encode for ClientHandshakePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.protocol_version);
        write_string(buf, &self.server_address, MAX_HANDSHAKE_ADDRESS_CHARS)?;
        self.server_port.encode(buf)?;
        write_var_int(buf, self.intent as i32);
        Ok(())
    }
}

impl Decode for ClientHandshakePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let protocol_version = read_var_int(buf)?;
        let server_address = read_string(buf, MAX_HANDSHAKE_ADDRESS_CHARS)?;
        let server_port = u16::decode(buf)?;
        let intent = HandshakeIntent::from_id(read_var_int(buf)?)?;
        Ok(Self {
            protocol_version,
            server_address,
            server_port,
            intent,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::{decode_packet_exact, encode_packet_payload, read_var_int};

    proptest! {
        #[test]
        fn handshake_roundtrip(
            proto in any::<i32>(),
            addr in "[a-zA-Z0-9._-]{0,64}",
            port in any::<u16>(),
            intent_idx in 1_i32..=3_i32,
        ) {
            let pkt = ClientHandshakePacket {
                protocol_version: proto,
                server_address: addr,
                server_port: port,
                intent: HandshakeIntent::from_id(intent_idx).unwrap(),
            };
            let mut body = encode_packet_payload(&pkt).unwrap();
            prop_assert_eq!(read_var_int(&mut body).unwrap(), ClientHandshakePacket::PACKET_ID);
            let decoded = decode_packet_exact::<ClientHandshakePacket>(body).unwrap();
            prop_assert_eq!(decoded, pkt);
        }

        #[test]
        fn arbitrary_bytes_never_panic_handshake(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let _ = decode_packet_exact::<ClientHandshakePacket>(bytes::Bytes::from(raw));
        }
    }
}
