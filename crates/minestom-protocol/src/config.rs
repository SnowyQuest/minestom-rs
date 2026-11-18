//! Configuration state (`ConnectionState::Configuration`) packets for protocol 776 (MC 26.2).

use bytes::{Buf, BufMut, BytesMut};
use minestom_core::MainHand;
use minestom_data::VERSION_NAME;
use minestom_macros::PacketMeta;
use minestom_nbt::NbtTag;
use minestom_text::Component;

use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_prefixed_vec, read_string, read_var_int, write_prefixed_vec, write_string, write_var_int,
};

/// Client settings reported in `ClientSettingsPacket` (`0x00`).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientSettingsPacket {
    /// Client locale code (e.g. `"en_us"`).
    pub locale: String,
    /// Client render distance in chunks.
    pub view_distance: i8,
    /// Chat visibility (`0 = Full`, `1 = System`, `2 = Hidden`).
    pub chat_mode: i32,
    /// Whether chat colors are enabled.
    pub chat_colors: bool,
    /// Bitmask of visible skin layers (`0x7F`).
    pub displayed_skin_parts: u8,
    /// Dominant hand (`Left` or `Right`).
    pub main_hand: MainHand,
    /// Whether profanity text filtering is enabled.
    pub enable_text_filtering: bool,
    /// Whether the player consents to appear in server list samples.
    pub allow_server_listings: bool,
    /// Particle setting (`0 = All`, `1 = Decreased`, `2 = Minimal`).
    pub particle_setting: i32,
}

impl PacketPayload for ClientSettingsPacket {
    const PACKET_ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for ClientSettingsPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.locale, 16)?;
        self.view_distance.encode(buf)?;
        write_var_int(buf, self.chat_mode);
        self.chat_colors.encode(buf)?;
        self.displayed_skin_parts.encode(buf)?;
        write_var_int(buf, self.main_hand as i32);
        self.enable_text_filtering.encode(buf)?;
        self.allow_server_listings.encode(buf)?;
        write_var_int(buf, self.particle_setting);
        Ok(())
    }
}

impl Decode for ClientSettingsPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let locale = read_string(buf, 16)?;
        let view_distance = i8::decode(buf)?;
        let chat_mode = read_var_int(buf)?;
        let chat_colors = bool::decode(buf)?;
        let displayed_skin_parts = u8::decode(buf)?;
        let main_hand = MainHand::from_id(read_var_int(buf)?)?;
        let enable_text_filtering = bool::decode(buf)?;
        let allow_server_listings = bool::decode(buf)?;
        let particle_setting = read_var_int(buf)?;
        Ok(Self {
            locale,
            view_distance,
            chat_mode,
            chat_colors,
            displayed_skin_parts,
            main_hand,
            enable_text_filtering,
            allow_server_listings,
            particle_setting,
        })
    }
}

/// Serverbound `ClientPluginMessagePacket` (`0x02` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientConfigPluginMessagePacket {
    /// Channel identifier (e.g. `"minecraft:brand"`).
    pub channel: String,
    /// Remaining raw channel payload bytes.
    pub data: Vec<u8>,
}

impl PacketPayload for ClientConfigPluginMessagePacket {
    const PACKET_ID: i32 = 0x02;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for ClientConfigPluginMessagePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.channel, DEFAULT_MAX_STRING_CHARS)?;
        buf.put_slice(&self.data);
        Ok(())
    }
}

impl Decode for ClientConfigPluginMessagePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let channel = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        if buf.remaining() > 32_767 {
            return Err(ProtocolError::InvalidLength {
                len: buf.remaining() as i32,
                max: 32_767,
            });
        }
        let mut data = vec![0_u8; buf.remaining()];
        buf.copy_to_slice(&mut data);
        Ok(Self { channel, data })
    }
}

/// Serverbound `ClientFinishConfigurationPacket` (`0x03` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PacketMeta)]
pub struct ClientFinishConfigurationPacket;

impl PacketPayload for ClientFinishConfigurationPacket {
    const PACKET_ID: i32 = 0x03;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for ClientFinishConfigurationPacket {
    fn encode<B: BufMut>(&self, _buf: &mut B) -> Result<(), ProtocolError> {
        Ok(())
    }
}

impl Decode for ClientFinishConfigurationPacket {
    fn decode<B: Buf>(_buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self)
    }
}

/// Serverbound `ClientKeepAlivePacket` (`0x04` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientConfigKeepAlivePacket {
    /// Echoed keep-alive ID.
    pub id: i64,
}

impl PacketPayload for ClientConfigKeepAlivePacket {
    const PACKET_ID: i32 = 0x04;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for ClientConfigKeepAlivePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.id.encode(buf)
    }
}

impl Decode for ClientConfigKeepAlivePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: i64::decode(buf)?,
        })
    }
}

/// Built-in datapack descriptor exchanged via `SelectKnownPacks` (`("minecraft", "core", "26.2")`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownPack {
    /// Pack namespace (e.g. `"minecraft"`).
    pub namespace: String,
    /// Pack ID (e.g. `"core"`).
    pub id: String,
    /// Pack version string (e.g. `"26.2"`).
    pub version: String,
}

impl KnownPack {
    /// Returns the built-in `minecraft:core` pack for MC 26.2.
    #[must_use]
    pub fn minecraft_core() -> Self {
        Self {
            namespace: "minecraft".to_owned(),
            id: "core".to_owned(),
            version: VERSION_NAME.to_owned(),
        }
    }
}

impl Encode for KnownPack {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.namespace, 256)?;
        write_string(buf, &self.id, 256)?;
        write_string(buf, &self.version, 256)
    }
}

impl Decode for KnownPack {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            namespace: read_string(buf, 256)?,
            id: read_string(buf, 256)?,
            version: read_string(buf, 256)?,
        })
    }
}

/// Serverbound `ClientSelectKnownPacksPacket` (`0x07` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ClientSelectKnownPacksPacket {
    /// Known packs confirmed by the client (max 64).
    pub entries: Vec<KnownPack>,
}

impl PacketPayload for ClientSelectKnownPacksPacket {
    const PACKET_ID: i32 = 0x07;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for ClientSelectKnownPacksPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_prefixed_vec(buf, &self.entries, 64)
    }
}

impl Decode for ClientSelectKnownPacksPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            entries: read_prefixed_vec(buf, 64)?,
        })
    }
}

/// Clientbound `PluginMessagePacket` (`0x01` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ConfigPluginMessagePacket {
    /// Channel identifier (e.g. `"minecraft:brand"`).
    pub channel: String,
    /// Raw channel payload bytes.
    pub data: Vec<u8>,
}

impl ConfigPluginMessagePacket {
    /// Constructs a `minecraft:brand` plugin message packet.
    pub fn brand(brand_name: &str) -> Result<Self, ProtocolError> {
        let mut payload = BytesMut::with_capacity(brand_name.len() + 4);
        write_string(&mut payload, brand_name, 256)?;
        Ok(Self {
            channel: "minecraft:brand".to_owned(),
            data: payload.to_vec(),
        })
    }
}

impl PacketPayload for ConfigPluginMessagePacket {
    const PACKET_ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for ConfigPluginMessagePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.channel, DEFAULT_MAX_STRING_CHARS)?;
        buf.put_slice(&self.data);
        Ok(())
    }
}

impl Decode for ConfigPluginMessagePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let channel = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        let mut data = vec![0_u8; buf.remaining()];
        buf.copy_to_slice(&mut data);
        Ok(Self { channel, data })
    }
}

/// Clientbound `DisconnectPacket` (`0x02` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct ConfigDisconnectPacket {
    /// Kick message encoded as MC 26.2 NBT [`Component`].
    pub message: Component,
}

impl PacketPayload for ConfigDisconnectPacket {
    const PACKET_ID: i32 = 0x02;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for ConfigDisconnectPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.message.encode(buf)
    }
}

impl Decode for ConfigDisconnectPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            message: Component::decode(buf)?,
        })
    }
}

/// Clientbound `FinishConfigurationPacket` (`0x03` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PacketMeta)]
pub struct FinishConfigurationPacket;

impl PacketPayload for FinishConfigurationPacket {
    const PACKET_ID: i32 = 0x03;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for FinishConfigurationPacket {
    fn encode<B: BufMut>(&self, _buf: &mut B) -> Result<(), ProtocolError> {
        Ok(())
    }
}

impl Decode for FinishConfigurationPacket {
    fn decode<B: Buf>(_buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self)
    }
}

/// Clientbound `KeepAlivePacket` (`0x04` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ConfigKeepAlivePacket {
    /// Keep-alive nonce.
    pub id: i64,
}

impl PacketPayload for ConfigKeepAlivePacket {
    const PACKET_ID: i32 = 0x04;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for ConfigKeepAlivePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.id.encode(buf)
    }
}

impl Decode for ConfigKeepAlivePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: i64::decode(buf)?,
        })
    }
}

/// Single entry inside [`RegistryDataPacket`].
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryDataEntry {
    /// Canonical entry key (e.g. `"minecraft:overworld"`).
    pub id: String,
    /// `None` when the client already knows this entry via `minecraft:core:26.2`; `Some(nbt)` otherwise.
    pub data: Option<NbtTag>,
}

impl Encode for RegistryDataEntry {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.id, DEFAULT_MAX_STRING_CHARS)?;
        self.data.encode(buf)
    }
}

impl Decode for RegistryDataEntry {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
            data: Option::<NbtTag>::decode(buf)?,
        })
    }
}

/// Clientbound `RegistryDataPacket` (`0x07` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct RegistryDataPacket {
    /// Full registry key (e.g. `"minecraft:dimension_type"`).
    pub registry_id: String,
    /// Ordered entries defining numeric IDs `0..entries.len()`.
    pub entries: Vec<RegistryDataEntry>,
}

impl PacketPayload for RegistryDataPacket {
    const PACKET_ID: i32 = 0x07;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for RegistryDataPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.registry_id, DEFAULT_MAX_STRING_CHARS)?;
        write_prefixed_vec(buf, &self.entries, 65_536)
    }
}

impl Decode for RegistryDataPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            registry_id: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
            entries: read_prefixed_vec(buf, 65_536)?,
        })
    }
}

/// Clientbound `UpdateEnabledFeaturesPacket` (`0x0C` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct UpdateEnabledFeaturesPacket {
    /// Enabled feature flag keys (default `["minecraft:vanilla"]`).
    pub features: Vec<String>,
}

impl Default for UpdateEnabledFeaturesPacket {
    fn default() -> Self {
        Self {
            features: vec!["minecraft:vanilla".to_owned()],
        }
    }
}

impl PacketPayload for UpdateEnabledFeaturesPacket {
    const PACKET_ID: i32 = 0x0C;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for UpdateEnabledFeaturesPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_prefixed_vec(buf, &self.features, 256)
    }
}

impl Decode for UpdateEnabledFeaturesPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            features: read_prefixed_vec(buf, 256)?,
        })
    }
}

/// Single tag binding `(identifier, entry_ids)` inside [`TagRegistryGroup`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagBinding {
    /// Tag identifier (e.g. `"minecraft:mineable/pickaxe"`).
    pub identifier: String,
    /// Protocol entry IDs belonging to this tag.
    pub entries: Vec<i32>,
}

impl Encode for TagBinding {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.identifier, DEFAULT_MAX_STRING_CHARS)?;
        write_var_int(buf, self.entries.len() as i32);
        for &id in &self.entries {
            write_var_int(buf, id);
        }
        Ok(())
    }
}

impl Decode for TagBinding {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let identifier = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        let raw_len = read_var_int(buf)?;
        if !(0..=65_536).contains(&raw_len) {
            return Err(ProtocolError::InvalidLength {
                len: raw_len,
                max: 65_536,
            });
        }
        let len = raw_len as usize;
        let mut entries = Vec::with_capacity(len.min(buf.remaining()));
        for _ in 0..len {
            entries.push(read_var_int(buf)?);
        }
        Ok(Self {
            identifier,
            entries,
        })
    }
}

/// Per-registry tag group inside [`ConfigTagsPacket`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRegistryGroup {
    /// Registry identifier (e.g. `"minecraft:block"`).
    pub registry: String,
    /// Tags belonging to this registry.
    pub tags: Vec<TagBinding>,
}

impl Encode for TagRegistryGroup {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.registry, DEFAULT_MAX_STRING_CHARS)?;
        write_prefixed_vec(buf, &self.tags, 16_384)
    }
}

impl Decode for TagRegistryGroup {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            registry: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
            tags: read_prefixed_vec(buf, 16_384)?,
        })
    }
}

/// Clientbound `TagsPacket` (`0x0D` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct ConfigTagsPacket {
    /// Tag groups for all client-relevant registries.
    pub registries: Vec<TagRegistryGroup>,
}

impl PacketPayload for ConfigTagsPacket {
    const PACKET_ID: i32 = 0x0D;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for ConfigTagsPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_prefixed_vec(buf, &self.registries, 256)
    }
}

impl Decode for ConfigTagsPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            registries: read_prefixed_vec(buf, 256)?,
        })
    }
}

/// Clientbound `SelectKnownPacksPacket` (`0x0E` in [`ConnectionState::Configuration`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct SelectKnownPacksPacket {
    /// Known packs offered by the server.
    pub entries: Vec<KnownPack>,
}

impl PacketPayload for SelectKnownPacksPacket {
    const PACKET_ID: i32 = 0x0E;
    const STATE: ConnectionState = ConnectionState::Configuration;
}

impl Encode for SelectKnownPacksPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_prefixed_vec(buf, &self.entries, 64)
    }
}

impl Decode for SelectKnownPacksPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            entries: read_prefixed_vec(buf, 64)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::{decode_packet_exact, encode_packet_payload};

    #[test]
    fn registry_data_and_known_packs_roundtrip() {
        let kp = SelectKnownPacksPacket {
            entries: vec![KnownPack::minecraft_core()],
        };
        let mut b = encode_packet_payload(&kp).unwrap();
        let _ = read_var_int(&mut b).unwrap();
        assert_eq!(
            decode_packet_exact::<SelectKnownPacksPacket>(b).unwrap(),
            kp
        );

        let rd = RegistryDataPacket {
            registry_id: "minecraft:dimension_type".to_owned(),
            entries: vec![RegistryDataEntry {
                id: "minecraft:overworld".to_owned(),
                data: None,
            }],
        };
        let mut b2 = encode_packet_payload(&rd).unwrap();
        let _ = read_var_int(&mut b2).unwrap();
        assert_eq!(decode_packet_exact::<RegistryDataPacket>(b2).unwrap(), rd);
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic_config_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let b = bytes::Bytes::from(raw);
            let _ = decode_packet_exact::<ClientSettingsPacket>(b.clone());
            let _ = decode_packet_exact::<ClientSelectKnownPacksPacket>(b.clone());
            let _ = decode_packet_exact::<RegistryDataPacket>(b.clone());
            let _ = decode_packet_exact::<ConfigTagsPacket>(b);
        }
    }
}
