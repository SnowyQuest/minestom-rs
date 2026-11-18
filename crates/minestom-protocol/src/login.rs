//! Login state (`ConnectionState::Login`) packets for protocol 776 (MC 26.2).

use bytes::{Buf, BufMut};
use minestom_macros::PacketMeta;
use minestom_text::Component;
use uuid::Uuid;

use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_byte_array, read_prefixed_vec, read_string, read_var_int, write_byte_array,
    write_prefixed_vec, write_string, write_var_int,
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

/// Clientbound `EncryptionRequestPacket` (`0x01` in [`ConnectionState::Login`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct EncryptionRequestPacket {
    /// Server ID string (`""` on modern servers, max 20 chars).
    pub server_id: String,
    /// X.509 `SubjectPublicKeyInfo` DER-encoded RSA-1024 public key.
    pub public_key: Vec<u8>,
    /// Random 4-byte challenge nonce.
    pub verify_token: Vec<u8>,
    /// Whether the client should authenticate with Mojang's Yggdrasil session server (`true` in online mode).
    pub should_authenticate: bool,
}

impl PacketPayload for EncryptionRequestPacket {
    const PACKET_ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Login;
}

impl Encode for EncryptionRequestPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.server_id, 20)?;
        write_byte_array(buf, &self.public_key, 4096)?;
        write_byte_array(buf, &self.verify_token, 4096)?;
        self.should_authenticate.encode(buf)
    }
}

impl Decode for EncryptionRequestPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            server_id: read_string(buf, 20)?,
            public_key: read_byte_array(buf, 4096)?,
            verify_token: read_byte_array(buf, 4096)?,
            should_authenticate: bool::decode(buf)?,
        })
    }
}

/// Serverbound `ClientEncryptionResponsePacket` (`0x01` in [`ConnectionState::Login`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientEncryptionResponsePacket {
    /// RSA-1024 PKCS#1 v1.5 encrypted 16-byte AES shared secret.
    pub shared_secret: Vec<u8>,
    /// RSA-1024 PKCS#1 v1.5 encrypted challenge verify token.
    pub encrypted_verify_token: Vec<u8>,
}

impl PacketPayload for ClientEncryptionResponsePacket {
    const PACKET_ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Login;
}

impl Encode for ClientEncryptionResponsePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_byte_array(buf, &self.shared_secret, 4096)?;
        write_byte_array(buf, &self.encrypted_verify_token, 4096)
    }
}

impl Decode for ClientEncryptionResponsePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            shared_secret: read_byte_array(buf, 4096)?,
            encrypted_verify_token: read_byte_array(buf, 4096)?,
        })
    }
}

/// Clientbound `LoginPluginRequestPacket` (`0x04` in [`ConnectionState::Login`], e.g. Velocity `velocity:player_info`).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct LoginPluginRequestPacket {
    /// Unique login plugin query ID.
    pub message_id: i32,
    /// Plugin channel identifier (e.g. `"velocity:player_info"`).
    pub channel: String,
    /// Unprefixed raw request payload (`RAW_BYTES` to end of packet).
    pub data: Vec<u8>,
}

impl PacketPayload for LoginPluginRequestPacket {
    const PACKET_ID: i32 = 0x04;
    const STATE: ConnectionState = ConnectionState::Login;
}

impl Encode for LoginPluginRequestPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        if self.data.len() > 1_048_576 {
            return Err(ProtocolError::InvalidLength {
                len: self.data.len() as i32,
                max: 1_048_576,
            });
        }
        write_var_int(buf, self.message_id);
        write_string(buf, &self.channel, DEFAULT_MAX_STRING_CHARS)?;
        buf.put_slice(&self.data);
        Ok(())
    }
}

impl Decode for LoginPluginRequestPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let message_id = read_var_int(buf)?;
        let channel = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        let rem = buf.remaining();
        if rem > 1_048_576 {
            return Err(ProtocolError::InvalidLength {
                len: rem as i32,
                max: 1_048_576,
            });
        }
        let mut data = vec![0_u8; rem];
        buf.copy_to_slice(&mut data);
        Ok(Self {
            message_id,
            channel,
            data,
        })
    }
}

/// Serverbound `ClientLoginPluginResponsePacket` (`0x02` in [`ConnectionState::Login`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientLoginPluginResponsePacket {
    /// Echoed login plugin query ID.
    pub message_id: i32,
    /// `Some(bytes)` if the proxy/client understood the plugin channel, `None` otherwise.
    pub data: Option<Vec<u8>>,
}

impl PacketPayload for ClientLoginPluginResponsePacket {
    const PACKET_ID: i32 = 0x02;
    const STATE: ConnectionState = ConnectionState::Login;
}

impl Encode for ClientLoginPluginResponsePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.message_id);
        match &self.data {
            None => false.encode(buf),
            Some(bytes) => {
                if bytes.len() > 1_048_576 {
                    return Err(ProtocolError::InvalidLength {
                        len: bytes.len() as i32,
                        max: 1_048_576,
                    });
                }
                true.encode(buf)?;
                buf.put_slice(bytes);
                Ok(())
            }
        }
    }
}

impl Decode for ClientLoginPluginResponsePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let message_id = read_var_int(buf)?;
        let understood = bool::decode(buf)?;
        let data = if understood {
            let rem = buf.remaining();
            if rem > 1_048_576 {
                return Err(ProtocolError::InvalidLength {
                    len: rem as i32,
                    max: 1_048_576,
                });
            }
            let mut raw = vec![0_u8; rem];
            buf.copy_to_slice(&mut raw);
            Some(raw)
        } else {
            None
        };
        Ok(Self { message_id, data })
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

            let enc_req = EncryptionRequestPacket {
                server_id: String::new(),
                public_key: vec![1, 2, 3, 4],
                verify_token: vec![9, 8, 7, 6],
                should_authenticate: true,
            };
            let mut b4 = encode_packet_payload(&enc_req).unwrap();
            prop_assert_eq!(read_var_int(&mut b4).unwrap(), 0x01);
            prop_assert_eq!(decode_packet_exact::<EncryptionRequestPacket>(b4).unwrap(), enc_req);

            let plug_resp = ClientLoginPluginResponsePacket {
                message_id: threshold,
                data: Some(vec![0xAA, 0xBB, 0xCC]),
            };
            let mut b5 = encode_packet_payload(&plug_resp).unwrap();
            prop_assert_eq!(read_var_int(&mut b5).unwrap(), 0x02);
            prop_assert_eq!(decode_packet_exact::<ClientLoginPluginResponsePacket>(b5).unwrap(), plug_resp);
        }

        #[test]
        fn arbitrary_bytes_never_panic_login_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let b = bytes::Bytes::from(raw);
            let _ = decode_packet_exact::<ClientLoginStartPacket>(b.clone());
            let _ = decode_packet_exact::<LoginSuccessPacket>(b.clone());
            let _ = decode_packet_exact::<SetCompressionPacket>(b.clone());
            let _ = decode_packet_exact::<EncryptionRequestPacket>(b.clone());
            let _ = decode_packet_exact::<ClientEncryptionResponsePacket>(b.clone());
            let _ = decode_packet_exact::<LoginPluginRequestPacket>(b.clone());
            let _ = decode_packet_exact::<ClientLoginPluginResponsePacket>(b);
        }
    }
}
