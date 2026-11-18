//! Play state (`ConnectionState::Play`) packets for protocol 776 (MC 26.2).

use bytes::{Buf, BufMut};
use minestom_core::{BlockVec, Difficulty, GameMode, Pos, Vec3};
use minestom_macros::PacketMeta;
use minestom_nbt::NbtTag;
use minestom_text::Component;
use uuid::Uuid;

use crate::login::ProfileProperty;
use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_byte_array, read_prefixed_vec, read_string, read_var_int, write_byte_array,
    write_prefixed_vec, write_string, write_var_int,
};

fn write_vec3<B: BufMut>(buf: &mut B, v: Vec3) -> Result<(), ProtocolError> {
    v.x.encode(buf)?;
    v.y.encode(buf)?;
    v.z.encode(buf)
}

fn read_vec3<B: Buf>(buf: &mut B) -> Result<Vec3, ProtocolError> {
    Ok(Vec3::new(
        f64::decode(buf)?,
        f64::decode(buf)?,
        f64::decode(buf)?,
    ))
}

fn write_long_vec<B: BufMut>(buf: &mut B, items: &[u64]) {
    write_var_int(buf, items.len() as i32);
    for &w in items {
        buf.put_u64(w);
    }
}

fn read_long_vec<B: Buf>(buf: &mut B, max_len: usize) -> Result<Vec<u64>, ProtocolError> {
    let raw_len = read_var_int(buf)?;
    if raw_len < 0 || (raw_len as usize) > max_len {
        return Err(ProtocolError::InvalidLength {
            len: raw_len,
            max: max_len,
        });
    }
    let len = raw_len as usize;
    let byte_len = len.checked_mul(8).ok_or(ProtocolError::UnexpectedEof)?;
    if buf.remaining() < byte_len {
        return Err(ProtocolError::UnexpectedEof);
    }
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        out.push(buf.get_u64());
    }
    Ok(out)
}

/// Dimension + block coordinate pair (`WorldPos` in Minestom).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldPos {
    /// Dimension key (e.g. `"minecraft:overworld"`).
    pub dimension: String,
    /// Packed 64-bit block coordinate.
    pub position: BlockVec,
}

impl Encode for WorldPos {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_string(buf, &self.dimension, DEFAULT_MAX_STRING_CHARS)?;
        self.position.encode(buf)
    }
}

impl Decode for WorldPos {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            dimension: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
            position: BlockVec::decode(buf)?,
        })
    }
}

/// Embedded spawn state inside [`JoinGamePacket`] and `RespawnPacket`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerSpawnInfo {
    /// Numeric protocol ID in `minecraft:dimension_type`.
    pub dimension_type: i32,
    /// Dimension identifier (e.g. `"minecraft:overworld"`).
    pub world: String,
    /// First 8 bytes of SHA-256 world seed.
    pub hashed_seed: i64,
    /// Active [`GameMode`].
    pub game_mode: GameMode,
    /// Previous [`GameMode`], if any (`-1` on wire if `None`).
    pub previous_game_mode: Option<GameMode>,
    /// Whether this is a debug-all-block-states world.
    pub is_debug: bool,
    /// Whether this is a superflat world (affects void fog horizon).
    pub is_flat: bool,
    /// Last death location, if any.
    pub death_location: Option<WorldPos>,
    /// Portal cooldown ticks.
    pub portal_cooldown: i32,
    /// Sea level Y coordinate (default `63`).
    pub sea_level: i32,
}

impl Encode for PlayerSpawnInfo {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.dimension_type);
        write_string(buf, &self.world, DEFAULT_MAX_STRING_CHARS)?;
        self.hashed_seed.encode(buf)?;
        (self.game_mode as i8).encode(buf)?;
        let prev = self.previous_game_mode.map_or(-1_i8, |gm| gm as i8);
        prev.encode(buf)?;
        self.is_debug.encode(buf)?;
        self.is_flat.encode(buf)?;
        self.death_location.encode(buf)?;
        write_var_int(buf, self.portal_cooldown);
        write_var_int(buf, self.sea_level);
        Ok(())
    }
}

impl Decode for PlayerSpawnInfo {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let dimension_type = read_var_int(buf)?;
        let world = read_string(buf, DEFAULT_MAX_STRING_CHARS)?;
        let hashed_seed = i64::decode(buf)?;
        let game_mode = GameMode::from_id(i32::from(i8::decode(buf)?))?;
        let prev_raw = i8::decode(buf)?;
        let previous_game_mode = if prev_raw == -1 {
            None
        } else {
            Some(GameMode::from_id(i32::from(prev_raw))?)
        };
        let is_debug = bool::decode(buf)?;
        let is_flat = bool::decode(buf)?;
        let death_location = Option::<WorldPos>::decode(buf)?;
        let portal_cooldown = read_var_int(buf)?;
        let sea_level = read_var_int(buf)?;
        Ok(Self {
            dimension_type,
            world,
            hashed_seed,
            game_mode,
            previous_game_mode,
            is_debug,
            is_flat,
            death_location,
            portal_cooldown,
            sea_level,
        })
    }
}

/// Clientbound `JoinGamePacket` (`0x31` in [`ConnectionState::Play`], MC 26.2).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct JoinGamePacket {
    /// Player protocol entity ID.
    pub entity_id: i32,
    /// Whether hardcore hearts are enabled.
    pub is_hardcore: bool,
    /// Dimension identifiers on the server.
    pub worlds: Vec<String>,
    /// Max player count (legacy field, `0` in Minestom).
    pub max_players: i32,
    /// Chunk view distance (`2..=32`).
    pub view_distance: i32,
    /// Simulation distance (`2..=32`).
    pub simulation_distance: i32,
    /// Reduced F3 debug screen flag.
    pub reduced_debug_info: bool,
    /// Show death screen (`true`) vs immediate respawn (`false`).
    pub enable_respawn_screen: bool,
    /// Restrict crafting to unlocked recipes.
    pub do_limited_crafting: bool,
    /// Dimension & game mode spawn info.
    pub spawn_info: PlayerSpawnInfo,
    /// MC 26.2 `onlineMode` flag (`true` in Minestom so tablist heads render).
    pub online_mode: bool,
    /// Whether secure chat is enforced.
    pub enforces_secure_chat: bool,
}

impl PacketPayload for JoinGamePacket {
    const PACKET_ID: i32 = 0x31;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for JoinGamePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.entity_id.encode(buf)?;
        self.is_hardcore.encode(buf)?;
        write_prefixed_vec(buf, &self.worlds, 32_767)?;
        write_var_int(buf, self.max_players);
        write_var_int(buf, self.view_distance);
        write_var_int(buf, self.simulation_distance);
        self.reduced_debug_info.encode(buf)?;
        self.enable_respawn_screen.encode(buf)?;
        self.do_limited_crafting.encode(buf)?;
        self.spawn_info.encode(buf)?;
        self.online_mode.encode(buf)?;
        self.enforces_secure_chat.encode(buf)
    }
}

impl Decode for JoinGamePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            entity_id: i32::decode(buf)?,
            is_hardcore: bool::decode(buf)?,
            worlds: read_prefixed_vec(buf, 32_767)?,
            max_players: read_var_int(buf)?,
            view_distance: read_var_int(buf)?,
            simulation_distance: read_var_int(buf)?,
            reduced_debug_info: bool::decode(buf)?,
            enable_respawn_screen: bool::decode(buf)?,
            do_limited_crafting: bool::decode(buf)?,
            spawn_info: PlayerSpawnInfo::decode(buf)?,
            online_mode: bool::decode(buf)?,
            enforces_secure_chat: bool::decode(buf)?,
        })
    }
}

/// Clientbound `ServerDifficultyPacket` (`0x0A` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ServerDifficultyPacket {
    /// World [`Difficulty`] (`VarInt` in MC 26.2).
    pub difficulty: Difficulty,
    /// Whether difficulty is locked.
    pub locked: bool,
}

impl PacketPayload for ServerDifficultyPacket {
    const PACKET_ID: i32 = 0x0A;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ServerDifficultyPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.difficulty as i32);
        self.locked.encode(buf)
    }
}

impl Decode for ServerDifficultyPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            difficulty: Difficulty::from_id(read_var_int(buf)?)?,
            locked: bool::decode(buf)?,
        })
    }
}

/// Clientbound `ChunkBatchFinishedPacket` (`0x0B` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ChunkBatchFinishedPacket {
    /// Number of chunks sent since `ChunkBatchStartPacket`.
    pub batch_size: i32,
}

impl PacketPayload for ChunkBatchFinishedPacket {
    const PACKET_ID: i32 = 0x0B;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ChunkBatchFinishedPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.batch_size);
        Ok(())
    }
}

impl Decode for ChunkBatchFinishedPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            batch_size: read_var_int(buf)?,
        })
    }
}

/// Clientbound `ChunkBatchStartPacket` (`0x0C` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PacketMeta)]
pub struct ChunkBatchStartPacket;

impl PacketPayload for ChunkBatchStartPacket {
    const PACKET_ID: i32 = 0x0C;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ChunkBatchStartPacket {
    fn encode<B: BufMut>(&self, _buf: &mut B) -> Result<(), ProtocolError> {
        Ok(())
    }
}

impl Decode for ChunkBatchStartPacket {
    fn decode<B: Buf>(_buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self)
    }
}

/// Clientbound `DisconnectPacket` (`0x20` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct PlayDisconnectPacket {
    /// Disconnect reason component (MC 26.2 NBT).
    pub message: Component,
}

impl PacketPayload for PlayDisconnectPacket {
    const PACKET_ID: i32 = 0x20;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for PlayDisconnectPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.message.encode(buf)
    }
}

impl Decode for PlayDisconnectPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            message: Component::decode(buf)?,
        })
    }
}

/// Game state change reason ordinal in [`ChangeGameStatePacket`] (`0x26`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum GameStateReason {
    /// `0` — No respawn block available.
    NoRespawnBlock = 0,
    /// `1` — Stop raining.
    EndRaining = 1,
    /// `2` — Start raining.
    BeginRaining = 2,
    /// `3` — Change player game mode.
    ChangeGameMode = 3,
    /// `13` — Dismiss "Loading terrain..." screen and begin rendering level chunks!
    LevelChunksLoadStart = 13,
}

impl GameStateReason {
    const fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::NoRespawnBlock),
            1 => Ok(Self::EndRaining),
            2 => Ok(Self::BeginRaining),
            3 => Ok(Self::ChangeGameMode),
            13 => Ok(Self::LevelChunksLoadStart),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "GameStateReason",
                value: other,
            }),
        }
    }
}

/// Clientbound `ChangeGameStatePacket` (`0x26` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct ChangeGameStatePacket {
    /// State change reason (`VarInt` in MC 26.2).
    pub reason: GameStateReason,
    /// Associated parameter (`0.0` for `LevelChunksLoadStart`).
    pub value: f32,
}

impl PacketPayload for ChangeGameStatePacket {
    const PACKET_ID: i32 = 0x26;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ChangeGameStatePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.reason as i32);
        self.value.encode(buf)
    }
}

impl Decode for ChangeGameStatePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            reason: GameStateReason::from_id(read_var_int(buf)?)?,
            value: f32::decode(buf)?,
        })
    }
}

/// Clientbound `KeepAlivePacket` (`0x2C` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct PlayKeepAlivePacket {
    /// Keep-alive challenge ID.
    pub id: i64,
}

impl PacketPayload for PlayKeepAlivePacket {
    const PACKET_ID: i32 = 0x2C;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for PlayKeepAlivePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.id.encode(buf)
    }
}

impl Decode for PlayKeepAlivePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: i64::decode(buf)?,
        })
    }
}

/// Heightmap type discriminant inside [`ChunkDataPacket`] (`Heightmap.Type` ordinal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum HeightmapType {
    /// `1` — `WORLD_SURFACE`.
    WorldSurface = 1,
    /// `4` — `MOTION_BLOCKING`.
    MotionBlocking = 4,
    /// `5` — `MOTION_BLOCKING_NO_LEAVES`.
    MotionBlockingNoLeaves = 5,
}

impl HeightmapType {
    const fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            1 => Ok(Self::WorldSurface),
            4 => Ok(Self::MotionBlocking),
            5 => Ok(Self::MotionBlockingNoLeaves),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "HeightmapType",
                value: other,
            }),
        }
    }
}

/// Block entity record inside [`ChunkDataPacket`].
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkBlockEntity {
    /// Packed `((local_x & 15) << 4) | (local_z & 15)`.
    pub packed_xz: u8,
    /// World block Y coordinate.
    pub y: i16,
    /// Numeric `minecraft:block_entity_type` ID.
    pub block_entity_type: i32,
    /// Client-visible block entity NBT (`NbtTag::End` if none).
    pub nbt: NbtTag,
}

impl Encode for ChunkBlockEntity {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.packed_xz.encode(buf)?;
        self.y.encode(buf)?;
        write_var_int(buf, self.block_entity_type);
        self.nbt.encode(buf)
    }
}

impl Decode for ChunkBlockEntity {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            packed_xz: u8::decode(buf)?,
            y: i16::decode(buf)?,
            block_entity_type: read_var_int(buf)?,
            nbt: NbtTag::decode(buf)?,
        })
    }
}

/// Sky and block lighting payload inside [`ChunkDataPacket`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightData {
    /// BitSet of sections (`min_section - 1 .. max_section + 1`) present in `sky_light`.
    pub sky_mask: Vec<u64>,
    /// BitSet of sections present in `block_light`.
    pub block_mask: Vec<u64>,
    /// BitSet of explicitly all-zero sky light sections.
    pub empty_sky_mask: Vec<u64>,
    /// BitSet of explicitly all-zero block light sections.
    pub empty_block_mask: Vec<u64>,
    /// 2048-byte nibble arrays for each set bit in `sky_mask`.
    pub sky_light: Vec<Vec<u8>>,
    /// 2048-byte nibble arrays for each set bit in `block_light`.
    pub block_light: Vec<Vec<u8>>,
}

impl LightData {
    /// Constructs full-daylight (`0xFF` = level 15) sky light for `section_count` vertical sections
    /// (plus the 2 boundary light sections below & above), so flat worlds render brightly lit.
    #[must_use]
    pub fn full_skylight(section_count: usize) -> Self {
        let total_light_sections = section_count + 2;
        let num_words = total_light_sections.div_ceil(64);
        let mut sky_mask = vec![0_u64; num_words];
        for i in 0..total_light_sections {
            sky_mask[i / 64] |= 1_u64 << (i % 64);
        }
        let sky_light = vec![vec![0xFF_u8; 2048]; total_light_sections];
        let mut empty_block_mask = vec![0_u64; num_words];
        for i in 0..total_light_sections {
            empty_block_mask[i / 64] |= 1_u64 << (i % 64);
        }
        Self {
            sky_mask,
            block_mask: Vec::new(),
            empty_sky_mask: Vec::new(),
            empty_block_mask,
            sky_light,
            block_light: Vec::new(),
        }
    }
}

impl Encode for LightData {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_long_vec(buf, &self.sky_mask);
        write_long_vec(buf, &self.block_mask);
        write_long_vec(buf, &self.empty_sky_mask);
        write_long_vec(buf, &self.empty_block_mask);
        write_var_int(buf, self.sky_light.len() as i32);
        for arr in &self.sky_light {
            write_byte_array(buf, arr, 2048)?;
        }
        write_var_int(buf, self.block_light.len() as i32);
        for arr in &self.block_light {
            write_byte_array(buf, arr, 2048)?;
        }
        Ok(())
    }
}

impl Decode for LightData {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let sky_mask = read_long_vec(buf, 8)?;
        let block_mask = read_long_vec(buf, 8)?;
        let empty_sky_mask = read_long_vec(buf, 8)?;
        let empty_block_mask = read_long_vec(buf, 8)?;

        let sky_count = read_var_int(buf)?;
        if !(0..=256).contains(&sky_count) {
            return Err(ProtocolError::InvalidLength {
                len: sky_count,
                max: 256,
            });
        }
        let mut sky_light = Vec::with_capacity(sky_count as usize);
        for _ in 0..sky_count {
            sky_light.push(read_byte_array(buf, 2048)?);
        }

        let block_count = read_var_int(buf)?;
        if !(0..=256).contains(&block_count) {
            return Err(ProtocolError::InvalidLength {
                len: block_count,
                max: 256,
            });
        }
        let mut block_light = Vec::with_capacity(block_count as usize);
        for _ in 0..block_count {
            block_light.push(read_byte_array(buf, 2048)?);
        }

        Ok(Self {
            sky_mask,
            block_mask,
            empty_sky_mask,
            empty_block_mask,
            sky_light,
            block_light,
        })
    }
}

/// Clientbound `ChunkDataPacket` (`0x2D` in [`ConnectionState::Play`], MC 26.2).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct ChunkDataPacket {
    /// Chunk X coordinate.
    pub chunk_x: i32,
    /// Chunk Z coordinate.
    pub chunk_z: i32,
    /// Binary heightmap entries `(HeightmapType, packed_u64_words)`.
    pub heightmaps: Vec<(HeightmapType, Vec<u64>)>,
    /// Concatenated serialized `Section` bytes (`min_section..max_section`).
    pub sections_data: Vec<u8>,
    /// Client-visible block entities in this chunk.
    pub block_entities: Vec<ChunkBlockEntity>,
    /// Sky and block lighting arrays.
    pub light_data: LightData,
}

impl PacketPayload for ChunkDataPacket {
    const PACKET_ID: i32 = 0x2D;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ChunkDataPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.chunk_x.encode(buf)?;
        self.chunk_z.encode(buf)?;
        write_var_int(buf, self.heightmaps.len() as i32);
        for (hm_type, words) in &self.heightmaps {
            write_var_int(buf, *hm_type as i32);
            write_long_vec(buf, words);
        }
        write_byte_array(buf, &self.sections_data, 2_097_151)?;
        write_prefixed_vec(buf, &self.block_entities, 65_536)?;
        self.light_data.encode(buf)
    }
}

impl Decode for ChunkDataPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let chunk_x = i32::decode(buf)?;
        let chunk_z = i32::decode(buf)?;
        let hm_count = read_var_int(buf)?;
        if !(0..=6).contains(&hm_count) {
            return Err(ProtocolError::InvalidLength {
                len: hm_count,
                max: 6,
            });
        }
        let mut heightmaps = Vec::with_capacity(hm_count as usize);
        for _ in 0..hm_count {
            let hm_type = HeightmapType::from_id(read_var_int(buf)?)?;
            let words = read_long_vec(buf, 64)?;
            heightmaps.push((hm_type, words));
        }
        let sections_data = read_byte_array(buf, 2_097_151)?;
        let block_entities = read_prefixed_vec(buf, 65_536)?;
        let light_data = LightData::decode(buf)?;
        Ok(Self {
            chunk_x,
            chunk_z,
            heightmaps,
            sections_data,
            block_entities,
            light_data,
        })
    }
}

/// Clientbound `PlayerAbilitiesPacket` (`0x40` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct PlayerAbilitiesPacket {
    /// Bitmask (`0x01` invulnerable, `0x02` flying, `0x04` allowFlying, `0x08` instantBreak).
    pub flags: u8,
    /// Flying speed (default `0.05`).
    pub flying_speed: f32,
    /// Field-of-view / walking speed modifier (default `0.1`).
    pub walking_speed: f32,
}

impl Default for PlayerAbilitiesPacket {
    fn default() -> Self {
        Self {
            flags: 0,
            flying_speed: 0.05,
            walking_speed: 0.1,
        }
    }
}

impl PacketPayload for PlayerAbilitiesPacket {
    const PACKET_ID: i32 = 0x40;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for PlayerAbilitiesPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.flags.encode(buf)?;
        self.flying_speed.encode(buf)?;
        self.walking_speed.encode(buf)
    }
}

impl Decode for PlayerAbilitiesPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            flags: u8::decode(buf)?,
            flying_speed: f32::decode(buf)?,
            walking_speed: f32::decode(buf)?,
        })
    }
}

/// Clientbound `PlayerInfoUpdatePacket` (`0x46` in [`ConnectionState::Play`]) with common
/// `ADD_PLAYER | UPDATE_GAME_MODE | UPDATE_LISTED` (`0x0D`) action set.
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct PlayerInfoAddEntry {
    /// Player UUID.
    pub uuid: Uuid,
    /// Player username (`1..=16`).
    pub username: String,
    /// Skin/profile properties.
    pub properties: Vec<ProfileProperty>,
    /// Active [`GameMode`].
    pub game_mode: GameMode,
    /// Whether shown in the tablist.
    pub listed: bool,
}

/// Clientbound `PlayerInfoUpdatePacket` (`0x46` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct PlayerInfoUpdatePacket {
    /// Players to add/update in the tablist (`actions = 0x0D`: `ADD_PLAYER | UPDATE_GAME_MODE | UPDATE_LISTED`).
    pub entries: Vec<PlayerInfoAddEntry>,
}

impl PacketPayload for PlayerInfoUpdatePacket {
    const PACKET_ID: i32 = 0x46;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for PlayerInfoUpdatePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        // Bit 0 (ADD_PLAYER = 0x01) | Bit 2 (UPDATE_GAME_MODE = 0x04) | Bit 3 (UPDATE_LISTED = 0x08) = 0x0D
        buf.put_u8(0x0D);
        write_var_int(buf, self.entries.len() as i32);
        for e in &self.entries {
            e.uuid.encode(buf)?;
            write_string(buf, &e.username, 16)?;
            write_prefixed_vec(buf, &e.properties, 16)?;
            write_var_int(buf, e.game_mode as i32);
            e.listed.encode(buf)?;
        }
        Ok(())
    }
}

impl Decode for PlayerInfoUpdatePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let mask = u8::decode(buf)?;
        if mask != 0x0D {
            return Err(ProtocolError::InvalidDiscriminant {
                type_name: "PlayerInfoUpdateActions",
                value: i32::from(mask),
            });
        }
        let count = read_var_int(buf)?;
        if !(0..=1024).contains(&count) {
            return Err(ProtocolError::InvalidLength {
                len: count,
                max: 1024,
            });
        }
        let mut entries = Vec::with_capacity((count as usize).min(buf.remaining()));
        for _ in 0..count {
            let uuid = Uuid::decode(buf)?;
            let username = read_string(buf, 16)?;
            let properties = read_prefixed_vec(buf, 16)?;
            let game_mode = GameMode::from_id(read_var_int(buf)?)?;
            let listed = bool::decode(buf)?;
            entries.push(PlayerInfoAddEntry {
                uuid,
                username,
                properties,
                game_mode,
                listed,
            });
        }
        Ok(Self { entries })
    }
}

/// Clientbound `PlayerPositionAndLookPacket` (`0x48` in [`ConnectionState::Play`], Synchronize Player Position).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct PlayerPositionAndLookPacket {
    /// Teleport correlation ID echoed in [`ClientTeleportConfirmPacket`].
    pub teleport_id: i32,
    /// Target position.
    pub position: Vec3,
    /// Target velocity delta (`Vec3::ZERO` for absolute teleports).
    pub delta: Vec3,
    /// Target yaw in degrees.
    pub yaw: f32,
    /// Target pitch in degrees.
    pub pitch: f32,
    /// Relative teleport flags (`0` for all-absolute coordinates).
    pub flags: i32,
}

impl PacketPayload for PlayerPositionAndLookPacket {
    const PACKET_ID: i32 = 0x48;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for PlayerPositionAndLookPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.teleport_id);
        write_vec3(buf, self.position)?;
        write_vec3(buf, self.delta)?;
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)?;
        self.flags.encode(buf)
    }
}

impl Decode for PlayerPositionAndLookPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            teleport_id: read_var_int(buf)?,
            position: read_vec3(buf)?,
            delta: read_vec3(buf)?,
            yaw: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
            flags: i32::decode(buf)?,
        })
    }
}

/// Clientbound `UpdateViewPositionPacket` (`0x5E` in [`ConnectionState::Play`], Set Center Chunk).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct UpdateViewPositionPacket {
    /// Center chunk X coordinate.
    pub chunk_x: i32,
    /// Center chunk Z coordinate.
    pub chunk_z: i32,
}

impl PacketPayload for UpdateViewPositionPacket {
    const PACKET_ID: i32 = 0x5E;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for UpdateViewPositionPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.chunk_x);
        write_var_int(buf, self.chunk_z);
        Ok(())
    }
}

impl Decode for UpdateViewPositionPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            chunk_x: read_var_int(buf)?,
            chunk_z: read_var_int(buf)?,
        })
    }
}

/// Clientbound `SpawnPositionPacket` (`0x61` in [`ConnectionState::Play`], MC 26.2).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct SpawnPositionPacket {
    /// World spawn dimension and block coordinate.
    pub world_pos: WorldPos,
    /// Compass/spawn yaw angle.
    pub yaw: f32,
    /// Spawn pitch angle.
    pub pitch: f32,
}

impl PacketPayload for SpawnPositionPacket {
    const PACKET_ID: i32 = 0x61;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for SpawnPositionPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.world_pos.encode(buf)?;
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)
    }
}

impl Decode for SpawnPositionPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            world_pos: WorldPos::decode(buf)?,
            yaw: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
        })
    }
}

/// Clientbound `UpdateHealthPacket` (`0x68` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct UpdateHealthPacket {
    /// Health points (`0.0..=20.0`).
    pub health: f32,
    /// Hunger bar (`0..=20`).
    pub food: i32,
    /// Food saturation (`0.0..=5.0`).
    pub food_saturation: f32,
}

impl PacketPayload for UpdateHealthPacket {
    const PACKET_ID: i32 = 0x68;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for UpdateHealthPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.health.encode(buf)?;
        write_var_int(buf, self.food);
        self.food_saturation.encode(buf)
    }
}

impl Decode for UpdateHealthPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            health: f32::decode(buf)?,
            food: read_var_int(buf)?,
            food_saturation: f32::decode(buf)?,
        })
    }
}

/// Serverbound `ClientTeleportConfirmPacket` (`0x00` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientTeleportConfirmPacket {
    /// Confirmed `teleport_id`.
    pub teleport_id: i32,
}

impl PacketPayload for ClientTeleportConfirmPacket {
    const PACKET_ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientTeleportConfirmPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.teleport_id);
        Ok(())
    }
}

impl Decode for ClientTeleportConfirmPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            teleport_id: read_var_int(buf)?,
        })
    }
}

/// Serverbound `ClientChunkBatchReceivedPacket` (`0x0B` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct ClientChunkBatchReceivedPacket {
    /// Desired chunks per tick reported by client bandwidth estimator.
    pub chunks_per_tick: f32,
}

impl PacketPayload for ClientChunkBatchReceivedPacket {
    const PACKET_ID: i32 = 0x0B;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientChunkBatchReceivedPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.chunks_per_tick.encode(buf)
    }
}

impl Decode for ClientChunkBatchReceivedPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            chunks_per_tick: f32::decode(buf)?,
        })
    }
}

/// Serverbound `ClientTickEndPacket` (`0x0D` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PacketMeta)]
pub struct ClientTickEndPacket;

impl PacketPayload for ClientTickEndPacket {
    const PACKET_ID: i32 = 0x0D;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientTickEndPacket {
    fn encode<B: BufMut>(&self, _buf: &mut B) -> Result<(), ProtocolError> {
        Ok(())
    }
}

impl Decode for ClientTickEndPacket {
    fn decode<B: Buf>(_buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self)
    }
}

/// Serverbound `ClientKeepAlivePacket` (`0x1C` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientPlayKeepAlivePacket {
    /// Echoed keep-alive ID.
    pub id: i64,
}

impl PacketPayload for ClientPlayKeepAlivePacket {
    const PACKET_ID: i32 = 0x1C;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientPlayKeepAlivePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.id.encode(buf)
    }
}

impl Decode for ClientPlayKeepAlivePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: i64::decode(buf)?,
        })
    }
}

/// Serverbound `ClientPlayerPositionPacket` (`0x1E` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct ClientPlayerPositionPacket {
    /// Reported `(x, y, z)` feet position.
    pub position: Vec3,
    /// Bit 0 = `on_ground`, Bit 1 = `horizontal_collision`.
    pub flags: u8,
}

impl PacketPayload for ClientPlayerPositionPacket {
    const PACKET_ID: i32 = 0x1E;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientPlayerPositionPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_vec3(buf, self.position)?;
        self.flags.encode(buf)
    }
}

impl Decode for ClientPlayerPositionPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let position = read_vec3(buf)?;
        if !position.x.is_finite() || !position.y.is_finite() || !position.z.is_finite() {
            return Err(ProtocolError::InvalidField(
                "non-finite player position".to_owned(),
            ));
        }
        Ok(Self {
            position,
            flags: u8::decode(buf)?,
        })
    }
}

/// Serverbound `ClientPlayerPositionAndRotationPacket` (`0x1F` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct ClientPlayerPositionAndRotationPacket {
    /// Reported `(x, y, z, yaw, pitch)`.
    pub position: Pos,
    /// Bit 0 = `on_ground`, Bit 1 = `horizontal_collision`.
    pub flags: u8,
}

impl PacketPayload for ClientPlayerPositionAndRotationPacket {
    const PACKET_ID: i32 = 0x1F;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientPlayerPositionAndRotationPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.position.x.encode(buf)?;
        self.position.y.encode(buf)?;
        self.position.z.encode(buf)?;
        self.position.yaw.encode(buf)?;
        self.position.pitch.encode(buf)?;
        self.flags.encode(buf)
    }
}

impl Decode for ClientPlayerPositionAndRotationPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let x = f64::decode(buf)?;
        let y = f64::decode(buf)?;
        let z = f64::decode(buf)?;
        let yaw = f32::decode(buf)?;
        let pitch = f32::decode(buf)?;
        if !x.is_finite()
            || !y.is_finite()
            || !z.is_finite()
            || !yaw.is_finite()
            || !pitch.is_finite()
        {
            return Err(ProtocolError::InvalidField(
                "non-finite player pos/rot".to_owned(),
            ));
        }
        Ok(Self {
            position: Pos::with_look(x, y, z, yaw, pitch),
            flags: u8::decode(buf)?,
        })
    }
}

/// Serverbound `ClientPlayerRotationPacket` (`0x20` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct ClientPlayerRotationPacket {
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// Bit 0 = `on_ground`, Bit 1 = `horizontal_collision`.
    pub flags: u8,
}

impl PacketPayload for ClientPlayerRotationPacket {
    const PACKET_ID: i32 = 0x20;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientPlayerRotationPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)?;
        self.flags.encode(buf)
    }
}

impl Decode for ClientPlayerRotationPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let yaw = f32::decode(buf)?;
        let pitch = f32::decode(buf)?;
        if !yaw.is_finite() || !pitch.is_finite() {
            return Err(ProtocolError::InvalidField(
                "non-finite player rotation".to_owned(),
            ));
        }
        Ok(Self {
            yaw,
            pitch,
            flags: u8::decode(buf)?,
        })
    }
}

/// Serverbound `ClientPlayerPositionStatusPacket` (`0x21` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientPlayerPositionStatusPacket {
    /// Bit 0 = `on_ground`, Bit 1 = `horizontal_collision`.
    pub flags: u8,
}

impl PacketPayload for ClientPlayerPositionStatusPacket {
    const PACKET_ID: i32 = 0x21;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientPlayerPositionStatusPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.flags.encode(buf)
    }
}

impl Decode for ClientPlayerPositionStatusPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            flags: u8::decode(buf)?,
        })
    }
}

/// Serverbound `ClientPlayerLoadedPacket` (`0x2C` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PacketMeta)]
pub struct ClientPlayerLoadedPacket;

impl PacketPayload for ClientPlayerLoadedPacket {
    const PACKET_ID: i32 = 0x2C;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientPlayerLoadedPacket {
    fn encode<B: BufMut>(&self, _buf: &mut B) -> Result<(), ProtocolError> {
        Ok(())
    }
}

impl Decode for ClientPlayerLoadedPacket {
    fn decode<B: Buf>(_buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::{decode_packet_exact, encode_packet_payload};

    #[test]
    fn join_game_and_chunk_data_roundtrip() {
        let join = JoinGamePacket {
            entity_id: 1,
            is_hardcore: false,
            worlds: vec!["minecraft:overworld".to_owned()],
            max_players: 0,
            view_distance: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            enable_respawn_screen: true,
            do_limited_crafting: false,
            spawn_info: PlayerSpawnInfo {
                dimension_type: 0,
                world: "minecraft:overworld".to_owned(),
                hashed_seed: 0,
                game_mode: GameMode::Creative,
                previous_game_mode: None,
                is_debug: false,
                is_flat: true,
                death_location: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: true,
            enforces_secure_chat: false,
        };
        let mut b = encode_packet_payload(&join).unwrap();
        assert_eq!(read_var_int(&mut b).unwrap(), 0x31);
        assert_eq!(decode_packet_exact::<JoinGamePacket>(b).unwrap(), join);

        let chunk = ChunkDataPacket {
            chunk_x: -2,
            chunk_z: 3,
            heightmaps: vec![(HeightmapType::MotionBlocking, vec![0_u64; 37])],
            sections_data: vec![0, 0, 0, 0, 0, 0, 0, 0],
            block_entities: Vec::new(),
            light_data: LightData::full_skylight(24),
        };
        let mut b2 = encode_packet_payload(&chunk).unwrap();
        assert_eq!(read_var_int(&mut b2).unwrap(), 0x2D);
        assert_eq!(decode_packet_exact::<ChunkDataPacket>(b2).unwrap(), chunk);
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic_play_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let b = bytes::Bytes::from(raw);
            let _ = decode_packet_exact::<JoinGamePacket>(b.clone());
            let _ = decode_packet_exact::<ChunkDataPacket>(b.clone());
            let _ = decode_packet_exact::<PlayerPositionAndLookPacket>(b.clone());
            let _ = decode_packet_exact::<ClientPlayerPositionAndRotationPacket>(b.clone());
            let _ = decode_packet_exact::<ClientPlayerPositionPacket>(b);
        }
    }
}
