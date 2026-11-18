//! Login state (`ConnectionState::Login`) packets for protocol 776 (MC 26.2).

use bytes::{Buf, BufMut};
use minestom_macros::PacketMeta;
use minestom_text::Component;
use uuid::Uuid;

use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_prefixed_vec, read_string, read_var_int, write_prefixed_vec, write_string, write_var_int,
};

/// Signed Mojang profile property (e.g. `"textures"` skin blob).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileProperty {
    /// Property name (max 64 chars).
    pub name: String,
    /// Base64 property value (max 32767 chars).
    pub value: String,
    /// Optional Yggdrasil signature (max 1024 chars).
    pub signature: Option<String>,
}

impl Encode for ProfileProperty {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.name, 64)?;
        write_string(buf, &self.value, DEFAULT_MAX_STRING_CHARS)?;
        match &self.signature {
            None => false.encode(buf),
            Some(sig) => {
                true.encode(buf)?;
                write_string(buf, sig, 1024)
            }
        }
    }
}

impl Decode for ProfileProperty {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let name = read_string(buf, 64)?;
        let value = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        let signature = if bool::decode(buf)? {
            Some(read_string(buf, 1024)?)
        } else {
            None
        };
        Ok(Self {
            name,
            value,
            signature,
        })
    }
}

/// Authenticated or offline player `GameProfile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameProfile {
    /// Player UUID.
    pub uuid: Uuid,
    /// Player username (`1..=16` chars).
    pub username: String,
    /// Profile properties (skin textures, etc.).
    pub properties: Vec<ProfileProperty>,
}

impl Encode for GameProfile {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.uuid.encode(buf)?;
        write_string(buf, &self.username, 16)?;
        write_prefixed_vec(buf, &self.properties, 16)
    }
}

impl Decode for GameProfile {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let uuid = Uuid::decode(buf)?;
        let username = read_string(buf, 16)?;
        let properties = read_prefixed_vec(buf, 16)?;
        Ok(Self {
            uuid,
            username,
            properties,
        })
    }
}

/// Serverbound `ClientLoginStartPacket` (`0x00` in [`ConnectionState::Login`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientLoginStartPacket {
    /// Requested player username (`1..=16` ASCII chars).
    pub username: String,
    /// Client-advertised profile UUID.
    pub profile_id: Uuid,
}

impl PacketPayload for ClientLoginStartPacket {
    const PACKET_ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Login;
}

impl Encode for ClientLoginStartPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.username, 16)?;
        self.profile_id.encode(buf)
    }
}

impl Decode for ClientLoginStartPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            username: read_string(buf, 16)?,
            profile_id: Uuid::decode(buf)?,
        })
    }
}

/// Serverbound `ClientLoginAcknowledgedPacket` (`0x03` in [`ConnectionState::Login`]).
///
/// Switches client state from `Login` to `Configuration`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PacketMeta)]
pub struct ClientLoginAcknowledgedPacket;

impl PacketPayload for ClientLoginAcknowledgedPacket {
    const PACKET_ID: i32 = 0x03;
    const STATE: ConnectionState = ConnectionState::Login;
}

impl Encode for ClientLoginAcknowledgedPacket {
    fn encode<B: BufMut>(&self, _buf: &mut B) -> Result<(), ProtocolError> {
        Ok(())
    }
}

impl Decode for ClientLoginAcknowledgedPacket {
    fn decode<B: Buf>(_buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self)
    }
}

/// Clientbound `LoginDisconnectPacket` (`0x00` in [`ConnectionState::Login`]).
///
/// Note: Encodes `reason` as a JSON string in `Login` state.
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct LoginDisconnectPacket {
    /// Disconnect reason displayed on the client's screen.
    pub reason: Component,
}

impl PacketPayload for LoginDisconnectPacket {
    const PACKET_ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Login;
}

impl Encode for LoginDisconnectPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        let json = self.reason.to_json_string()?;
        write_string(buf, &json, DEFAULT_MAX_STRING_CHARS)
    }
}

impl Decode for LoginDisconnectPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let json = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        Ok(Self {
            reason: Component::from_json_str(&json)?,
        })
    }
}

/// Clientbound `LoginSuccessPacket` (`0x02` in [`ConnectionState::Login`]).
///
/// Includes the MC 26.2 `session_id: Uuid` field (`Uuid::nil()` by default) and switches
/// server state to `Configuration`.
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct LoginSuccessPacket {
    /// Resolved player `GameProfile`.
    pub profile: GameProfile,
    /// MC 26.2 session identifier (`Uuid::nil()` in Minestom).
    pub session_id: Uuid,
}

impl PacketPayload for LoginSuccessPacket {
    const PACKET_ID: i32 = 0x02;
    const STATE: ConnectionState = ConnectionState::Login;
}

impl Encode for LoginSuccessPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.profile.encode(buf)?;
        self.session_id.encode(buf)
    }
}

impl Decode for LoginSuccessPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            profile: GameProfile::decode(buf)?,
            session_id: Uuid::decode(buf)?,
        })
    }
}

/// Clientbound `SetCompressionPacket` (`0x03` in [`ConnectionState::Login`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct SetCompressionPacket {
    /// Zlib compression threshold in bytes (`<= 0` disables compression).
    pub threshold: i32,
}

impl PacketPayload for SetCompressionPacket {
    const PACKET_ID: i32 = 0x03;
    const STATE: ConnectionState = ConnectionState::Login;
}

impl Encode for SetCompressionPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.threshold);
        Ok(())
    }
}

impl Decode for SetCompressionPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            threshold: read_var_int(buf)?,
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
        fn login_packets_roundtrip(
            username in "[a-zA-Z0-9_]{1,16}",
            uuid_bits in any::<u128>(),
            session_bits in any::<u128>(),
            threshold in 0_i32..4096_i32,
        ) {
            let start = ClientLoginStartPacket {
                username: username.clone(),
                profile_id: Uuid::from_u128(uuid_bits),
            };
            let mut b1 = encode_packet_payload(&start).unwrap();
            prop_assert_eq!(read_var_int(&mut b1).unwrap(), 0x00);
            prop_assert_eq!(decode_packet_exact::<ClientLoginStartPacket>(b1).unwrap(), start);

            let success = LoginSuccessPacket {
                profile: GameProfile {
                    uuid: Uuid::from_u128(uuid_bits),
                    username,
                    properties: vec![ProfileProperty {
                        name: "textures".to_owned(),
                        value: "abc".to_owned(),
                        signature: Some("sig".to_owned()),
                    }],
                },
                session_id: Uuid::from_u128(session_bits),
            };
            let mut b2 = encode_packet_payload(&success).unwrap();
            prop_assert_eq!(read_var_int(&mut b2).unwrap(), 0x02);
            prop_assert_eq!(decode_packet_exact::<LoginSuccessPacket>(b2).unwrap(), success);

            let comp = SetCompressionPacket { threshold };
            let mut b3 = encode_packet_payload(&comp).unwrap();
            prop_assert_eq!(read_var_int(&mut b3).unwrap(), 0x03);
            prop_assert_eq!(decode_packet_exact::<SetCompressionPacket>(b3).unwrap(), comp);
        }

        #[test]
        fn arbitrary_bytes_never_panic_login_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let b = bytes::Bytes::from(raw);
            let _ = decode_packet_exact::<ClientLoginStartPacket>(b.clone());
            let _ = decode_packet_exact::<LoginSuccessPacket>(b.clone());
            let _ = decode_packet_exact::<SetCompressionPacket>(b);
        }
    }
}
