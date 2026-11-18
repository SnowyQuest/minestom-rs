//! Login state (`ConnectionState::Login`) disconnect packet (expanded with full Login flow in M2/M8).

use bytes::{Buf, BufMut};
use minestom_macros::PacketMeta;
use minestom_text::Component;

use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_string, write_string,
};

/// Clientbound `LoginDisconnectPacket` (`0x00` in [`ConnectionState::Login`]).
///
/// Note: Unlike Configuration/Play `DisconnectPacket` (which encodes `Component` as NBT),
/// `LoginDisconnectPacket` in protocol 776 still encodes `reason` as a JSON string!
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
