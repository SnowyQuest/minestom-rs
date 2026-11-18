//! Particles (`125` types), Sounds (`1,968` `SoundEvent`s), and Resource Pack packets (protocol 776, MC 26.2).

use std::fmt;

use bytes::{Buf, BufMut};
use minestom_core::{BlockVec, Vec3};
use minestom_data::generated::{particles, sounds};
use minestom_item::ItemStack;
use minestom_macros::PacketMeta;
use minestom_text::Component;
use uuid::Uuid;

use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, PacketPayload, ProtocolError,
    read_string, read_var_int, write_string, write_var_int,
};

/// Extra data payload carried by parameterized MC 26.2 particle types.
#[derive(Debug, Clone, PartialEq)]
pub enum ParticleData {
    /// Simple particle with no extra payload.
    None,
    /// Block state ID (`block`, `block_marker`, `falling_dust`, `dust_pillar`, `block_crumble`).
    BlockState(u16),
    /// RGB + scale (`dust`, `effect`, `instant_effect`).
    RgbScale {
        /// 24-bit RGB integer (`0xRRGGBB`).
        rgb: i32,
        /// Scale or power scalar.
        scale: f32,
    },
    /// Dust color transition (`dust_color_transition`).
    DustColorTransition {
        /// Start 24-bit RGB.
        from_rgb: i32,
        /// End 24-bit RGB.
        to_rgb: i32,
        /// Scale.
        scale: f32,
    },
    /// 32-bit ARGB color (`entity_effect`, `tinted_leaves`, `flash`).
    ArgbColor(i32),
    /// Single float scalar (`dragon_breath`, `sculk_charge`).
    ScalarFloat(f32),
    /// Single `VarInt` (`shriek` delay).
    VarIntValue(i32),
    /// Single `i32` (`geyser`, `geyser_plume` water blocks).
    IntValue(i32),
    /// `i32` + `f32` (`geyser_base`, `geyser_poof`).
    IntAndFloat(i32, f32),
    /// Item stack (`item`).
    Item(ItemStack),
    /// Trail particle (`trail`: target `Vec3`, `i32` RGB, `VarInt` duration).
    Trail {
        /// Target coordinate.
        target: Vec3,
        /// 24-bit RGB color.
        rgb: i32,
        /// Duration ticks.
        duration: i32,
    },
    /// Vibration particle (`vibration`).
    VibrationBlock {
        /// Destination block position.
        block_pos: BlockVec,
        /// Travel ticks.
        travel_ticks: i32,
    },
    /// Entity-targeted vibration (`vibration`).
    VibrationEntity {
        /// Target entity ID.
        entity_id: i32,
        /// Eye height offset.
        eye_height: f32,
        /// Travel ticks.
        travel_ticks: i32,
    },
}

/// Self-contained `(particle_id, data)` pair (`Particle.NETWORK_TYPE` in Minestom, shared by
/// [`ParticlePacket`] and `MetadataValue::Particle` / `MetadataValue::ParticleList`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleEntry {
    /// Particle protocol ID (`0..125`).
    pub particle_id: u32,
    /// Extra particle payload matching `particle_id`.
    pub data: ParticleData,
}

impl ParticleEntry {
    /// Creates a simple data-free [`ParticleEntry`].
    #[must_use]
    pub const fn simple(particle_id: u32) -> Self {
        Self {
            particle_id,
            data: ParticleData::None,
        }
    }

    /// Creates a redstone `minecraft:dust` [`ParticleEntry`].
    #[must_use]
    pub const fn dust(rgb: i32, scale: f32) -> Self {
        Self {
            particle_id: particles::DUST,
            data: ParticleData::RgbScale { rgb, scale },
        }
    }
}

impl Default for ParticleEntry {
    fn default() -> Self {
        Self::dust(0xFF_00_00, 1.0)
    }
}

pub(crate) fn encode_particle_data<B: BufMut>(
    buf: &mut B,
    particle_id: u32,
    data: &ParticleData,
) -> Result<(), ProtocolError> {
    write_var_int(buf, particle_id as i32);
    match data {
        ParticleData::None => Ok(()),
        ParticleData::BlockState(state) => {
            write_var_int(buf, i32::from(*state));
            Ok(())
        }
        ParticleData::RgbScale { rgb, scale } => {
            rgb.encode(buf)?;
            scale.encode(buf)
        }
        ParticleData::DustColorTransition {
            from_rgb,
            to_rgb,
            scale,
        } => {
            from_rgb.encode(buf)?;
            to_rgb.encode(buf)?;
            scale.encode(buf)
        }
        ParticleData::ArgbColor(argb) => argb.encode(buf),
        ParticleData::ScalarFloat(v) => v.encode(buf),
        ParticleData::VarIntValue(v) => {
            write_var_int(buf, *v);
            Ok(())
        }
        ParticleData::IntValue(v) => v.encode(buf),
        ParticleData::IntAndFloat(i, f) => {
            i.encode(buf)?;
            f.encode(buf)
        }
        ParticleData::Item(stack) => stack.encode(buf),
        ParticleData::Trail {
            target,
            rgb,
            duration,
        } => {
            target.x.encode(buf)?;
            target.y.encode(buf)?;
            target.z.encode(buf)?;
            rgb.encode(buf)?;
            write_var_int(buf, *duration);
            Ok(())
        }
        ParticleData::VibrationBlock {
            block_pos,
            travel_ticks,
        } => {
            write_var_int(buf, 0);
            block_pos.encode(buf)?;
            write_var_int(buf, *travel_ticks);
            Ok(())
        }
        ParticleData::VibrationEntity {
            entity_id,
            eye_height,
            travel_ticks,
        } => {
            write_var_int(buf, 1);
            write_var_int(buf, *entity_id);
            eye_height.encode(buf)?;
            write_var_int(buf, *travel_ticks);
            Ok(())
        }
    }
}

pub(crate) fn decode_particle_data<B: Buf>(
    buf: &mut B,
) -> Result<(u32, ParticleData), ProtocolError> {
    let raw_id = read_var_int(buf)?;
    if !(0..125).contains(&raw_id) {
        return Err(ProtocolError::InvalidDiscriminant {
            type_name: "Particle",
            value: raw_id,
        });
    }
    let particle_id = raw_id as u32;
    let data = match particle_id {
        1 | 2 | 36 | 118 | 122 => {
            let st = read_var_int(buf)?;
            let st_u16 = u16::try_from(st).map_err(|_| ProtocolError::InvalidDiscriminant {
                type_name: "BlockState",
                value: st,
            })?;
            ParticleData::BlockState(st_u16)
        }
        7 | 10 => ParticleData::IntValue(i32::decode(buf)?),
        8 | 9 => ParticleData::IntAndFloat(i32::decode(buf)?, f32::decode(buf)?),
        15 | 45 => ParticleData::ScalarFloat(f32::decode(buf)?),
        21 | 23 | 53 => ParticleData::RgbScale {
            rgb: i32::decode(buf)?,
            scale: f32::decode(buf)?,
        },
        22 => ParticleData::DustColorTransition {
            from_rgb: i32::decode(buf)?,
            to_rgb: i32::decode(buf)?,
            scale: f32::decode(buf)?,
        },
        28 | 43 | 49 => ParticleData::ArgbColor(i32::decode(buf)?),
        54 => ParticleData::Item(ItemStack::decode(buf)?),
        55 => match read_var_int(buf)? {
            0 => ParticleData::VibrationBlock {
                block_pos: BlockVec::decode(buf)?,
                travel_ticks: read_var_int(buf)?,
            },
            1 => ParticleData::VibrationEntity {
                entity_id: read_var_int(buf)?,
                eye_height: f32::decode(buf)?,
                travel_ticks: read_var_int(buf)?,
            },
            other => {
                return Err(ProtocolError::InvalidDiscriminant {
                    type_name: "VibrationSource",
                    value: other,
                });
            }
        },
        56 => ParticleData::Trail {
            target: Vec3::new(f64::decode(buf)?, f64::decode(buf)?, f64::decode(buf)?),
            rgb: i32::decode(buf)?,
            duration: read_var_int(buf)?,
        },
        112 => ParticleData::VarIntValue(read_var_int(buf)?),
        _ => ParticleData::None,
    };
    Ok((particle_id, data))
}

impl Encode for ParticleEntry {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        encode_particle_data(buf, self.particle_id, &self.data)
    }
}

impl Decode for ParticleEntry {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let (particle_id, data) = decode_particle_data(buf)?;
        Ok(Self { particle_id, data })
    }
}

/// Clientbound `ParticlePacket` (`0x2F` in [`ConnectionState::Play`], MC 26.2).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct ParticlePacket {
    /// Particle protocol ID (`0..125`, `minestom_data::generated::particles`).
    pub particle_id: u32,
    /// Optional particle-specific extra data.
    pub data: ParticleData,
    /// Bypass client minimal-particle limiter.
    pub override_limiter: bool,
    /// Render up to 512 blocks away instead of 32.
    pub long_distance: bool,
    /// World spawn origin.
    pub position: Vec3,
    /// Gaussian spread offset `(dx, dy, dz)`.
    pub offset: (f32, f32, f32),
    /// Maximum particle speed scalar.
    pub max_speed: f32,
    /// Number of particles to spawn.
    pub particle_count: i32,
}

impl ParticlePacket {
    /// Constructs a simple (data-free) [`ParticlePacket`].
    #[must_use]
    pub const fn simple(
        particle_id: u32,
        position: Vec3,
        offset: (f32, f32, f32),
        max_speed: f32,
        particle_count: i32,
    ) -> Self {
        Self {
            particle_id,
            data: ParticleData::None,
            override_limiter: false,
            long_distance: false,
            position,
            offset,
            max_speed,
            particle_count,
        }
    }
}

impl PacketPayload for ParticlePacket {
    const PACKET_ID: i32 = 0x2F;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ParticlePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.override_limiter.encode(buf)?;
        self.long_distance.encode(buf)?;
        self.position.x.encode(buf)?;
        self.position.y.encode(buf)?;
        self.position.z.encode(buf)?;
        self.offset.0.encode(buf)?;
        self.offset.1.encode(buf)?;
        self.offset.2.encode(buf)?;
        self.max_speed.encode(buf)?;
        self.particle_count.encode(buf)?;
        encode_particle_data(buf, self.particle_id, &self.data)
    }
}

impl Decode for ParticlePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let override_limiter = bool::decode(buf)?;
        let long_distance = bool::decode(buf)?;
        let position = Vec3::new(f64::decode(buf)?, f64::decode(buf)?, f64::decode(buf)?);
        let offset = (f32::decode(buf)?, f32::decode(buf)?, f32::decode(buf)?);
        let max_speed = f32::decode(buf)?;
        let particle_count = i32::decode(buf)?;
        let (particle_id, data) = decode_particle_data(buf)?;
        Ok(Self {
            particle_id,
            data,
            override_limiter,
            long_distance,
            position,
            offset,
            max_speed,
            particle_count,
        })
    }
}

/// Strongly typed `Copy` handle for all **1,968** vanilla MC 26.2 `SoundEvent`s (`minecraft:sound_event`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SoundEvent(pub u32);

minestom_data::impl_sound_event_constants!(SoundEvent);

impl SoundEvent {
    /// Zero-allocation `O(log N)` lookup by `"minecraft:..."` or bare `"..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        sounds::lookup_key(key).map(Self)
    }

    /// Numeric protocol ID (`0..1968`).
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Canonical `"minecraft:..."` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        sounds::ENTRIES[self.0 as usize].1
    }
}

impl From<SoundEvent> for u32 {
    fn from(value: SoundEvent) -> Self {
        value.0
    }
}

impl From<u32> for SoundEvent {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl fmt::Display for SoundEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// Audio mixer category (`Sound.Source` in Adventure / `sound_sources.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SoundSource {
    /// Master (`0`).
    Master = 0,
    /// Music (`1`).
    Music = 1,
    /// Jukebox/records (`2`).
    Record = 2,
    /// Weather (`3`).
    Weather = 3,
    /// Blocks (`4`).
    Block = 4,
    /// Hostile mobs (`5`).
    Hostile = 5,
    /// Neutral mobs (`6`).
    Neutral = 6,
    /// Players (`7`).
    Player = 7,
    /// Ambient (`8`).
    Ambient = 8,
    /// Voice (`9`).
    Voice = 9,
    /// UI (`10`).
    Ui = 10,
}

impl SoundSource {
    fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::Master),
            1 => Ok(Self::Music),
            2 => Ok(Self::Record),
            3 => Ok(Self::Weather),
            4 => Ok(Self::Block),
            5 => Ok(Self::Hostile),
            6 => Ok(Self::Neutral),
            7 => Ok(Self::Player),
            8 => Ok(Self::Ambient),
            9 => Ok(Self::Voice),
            10 => Ok(Self::Ui),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "SoundSource",
                value: other,
            }),
        }
    }
}

/// Builtin or custom resource-pack `SoundEvent` reference (`SoundEvent.NETWORK_TYPE` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub enum SoundHolder {
    /// Builtin `minecraft:sound_event` protocol ID (`0..1968`, encoded on wire as `id + 1`).
    Builtin(u32),
    /// Custom resource-pack sound key (`VarInt 0` + `key` + optional fixed attenuation `range`).
    Custom {
        /// Sound resource key.
        key: String,
        /// Optional fixed attenuation distance.
        range: Option<f32>,
    },
}

impl Encode for SoundHolder {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        match self {
            Self::Builtin(id) => {
                write_var_int(buf, (*id as i32).saturating_add(1));
                Ok(())
            }
            Self::Custom { key, range } => {
                write_var_int(buf, 0);
                write_string(buf, key, DEFAULT_MAX_STRING_CHARS)?;
                range.encode(buf)
            }
        }
    }
}

impl Decode for SoundHolder {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let raw = read_var_int(buf)?;
        if raw < 0 {
            return Err(ProtocolError::InvalidDiscriminant {
                type_name: "SoundHolder",
                value: raw,
            });
        }
        if raw == 0 {
            Ok(Self::Custom {
                key: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
                range: Option::<f32>::decode(buf)?,
            })
        } else {
            Ok(Self::Builtin((raw - 1) as u32))
        }
    }
}

/// Clientbound `SoundEffectPacket` (`0x75` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct SoundEffectPacket {
    /// Sound event holder.
    pub sound: SoundHolder,
    /// Mixer category.
    pub source: SoundSource,
    /// Fixed-point `x * 8` coordinate.
    pub x_fixed8: i32,
    /// Fixed-point `y * 8` coordinate.
    pub y_fixed8: i32,
    /// Fixed-point `z * 8` coordinate.
    pub z_fixed8: i32,
    /// Volume (`1.0` = normal).
    pub volume: f32,
    /// Pitch (`1.0` = normal).
    pub pitch: f32,
    /// Variation seed.
    pub seed: i64,
}

impl SoundEffectPacket {
    /// Constructs a [`SoundEffectPacket`] at world `origin`.
    #[must_use]
    pub fn at(
        sound: SoundHolder,
        source: SoundSource,
        origin: Vec3,
        volume: f32,
        pitch: f32,
        seed: i64,
    ) -> Self {
        Self {
            sound,
            source,
            x_fixed8: (origin.x * 8.0) as i32,
            y_fixed8: (origin.y * 8.0) as i32,
            z_fixed8: (origin.z * 8.0) as i32,
            volume,
            pitch,
            seed,
        }
    }
}

impl PacketPayload for SoundEffectPacket {
    const PACKET_ID: i32 = 0x75;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for SoundEffectPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.sound.encode(buf)?;
        write_var_int(buf, self.source as i32);
        self.x_fixed8.encode(buf)?;
        self.y_fixed8.encode(buf)?;
        self.z_fixed8.encode(buf)?;
        self.volume.encode(buf)?;
        self.pitch.encode(buf)?;
        self.seed.encode(buf)
    }
}

impl Decode for SoundEffectPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            sound: SoundHolder::decode(buf)?,
            source: SoundSource::from_id(read_var_int(buf)?)?,
            x_fixed8: i32::decode(buf)?,
            y_fixed8: i32::decode(buf)?,
            z_fixed8: i32::decode(buf)?,
            volume: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
            seed: i64::decode(buf)?,
        })
    }
}

/// Clientbound `StopSoundPacket` (`0x77` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct StopSoundPacket {
    /// Optional category filter.
    pub source: Option<SoundSource>,
    /// Optional sound key filter.
    pub sound: Option<String>,
}

impl PacketPayload for StopSoundPacket {
    const PACKET_ID: i32 = 0x77;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for StopSoundPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        let mut flags = 0_u8;
        if self.source.is_some() {
            flags |= 0x01;
        }
        if self.sound.is_some() {
            flags |= 0x02;
        }
        flags.encode(buf)?;
        if let Some(src) = self.source {
            write_var_int(buf, src as i32);
        }
        if let Some(snd) = &self.sound {
            write_string(buf, snd, DEFAULT_MAX_STRING_CHARS)?;
        }
        Ok(())
    }
}

impl Decode for StopSoundPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let flags = u8::decode(buf)?;
        if flags > 0x03 {
            return Err(ProtocolError::InvalidDiscriminant {
                type_name: "StopSoundFlags",
                value: i32::from(flags),
            });
        }
        let source = if (flags & 0x01) != 0 {
            Some(SoundSource::from_id(read_var_int(buf)?)?)
        } else {
            None
        };
        let sound = if (flags & 0x02) != 0 {
            Some(read_string(buf, DEFAULT_MAX_STRING_CHARS)?)
        } else {
            None
        };
        Ok(Self { source, sound })
    }
}

/// Client resource pack download/load status (`ResourcePackStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ResourcePackStatus {
    /// Pack loaded and applied (`0`).
    SuccessfullyLoaded = 0,
    /// User declined prompt (`1`).
    Declined = 1,
    /// Download HTTP/hash failed (`2`).
    FailedDownload = 2,
    /// User accepted prompt (`3`).
    Accepted = 3,
    /// Download finished (`4`).
    Downloaded = 4,
    /// Malformed URL (`5`).
    InvalidUrl = 5,
    /// Client reload failed (`6`).
    FailedReload = 6,
    /// Discarded by server/client (`7`).
    Discarded = 7,
}

impl ResourcePackStatus {
    fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::SuccessfullyLoaded),
            1 => Ok(Self::Declined),
            2 => Ok(Self::FailedDownload),
            3 => Ok(Self::Accepted),
            4 => Ok(Self::Downloaded),
            5 => Ok(Self::InvalidUrl),
            6 => Ok(Self::FailedReload),
            7 => Ok(Self::Discarded),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "ResourcePackStatus",
                value: other,
            }),
        }
    }
}

/// Clientbound `ResourcePackPushPacket` (`0x51` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct ResourcePackPushPacket {
    /// Pack UUID.
    pub id: Uuid,
    /// HTTPS download URL.
    pub url: String,
    /// 40-char lowercase SHA-1 hex string.
    pub hash: String,
    /// Disconnect player if declined.
    pub forced: bool,
    /// Optional custom prompt message.
    pub prompt: Option<Component>,
}

impl PacketPayload for ResourcePackPushPacket {
    const PACKET_ID: i32 = 0x51;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ResourcePackPushPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.id.encode(buf)?;
        write_string(buf, &self.url, DEFAULT_MAX_STRING_CHARS)?;
        write_string(buf, &self.hash, 40)?;
        self.forced.encode(buf)?;
        self.prompt.encode(buf)
    }
}

impl Decode for ResourcePackPushPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: Uuid::decode(buf)?,
            url: read_string(buf, DEFAULT_MAX_STRING_CHARS)?,
            hash: read_string(buf, 40)?,
            forced: bool::decode(buf)?,
            prompt: Option::<Component>::decode(buf)?,
        })
    }
}

/// Clientbound `ResourcePackPopPacket` (`0x50` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ResourcePackPopPacket {
    /// Specific pack UUID to unload, or `None` to pop all server packs.
    pub id: Option<Uuid>,
}

impl PacketPayload for ResourcePackPopPacket {
    const PACKET_ID: i32 = 0x50;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ResourcePackPopPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.id.encode(buf)
    }
}

impl Decode for ResourcePackPopPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: Option::<Uuid>::decode(buf)?,
        })
    }
}

/// Serverbound `ClientResourcePackStatusPacket` (`0x31` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientResourcePackStatusPacket {
    /// Pack UUID.
    pub id: Uuid,
    /// Reported status.
    pub status: ResourcePackStatus,
}

impl PacketPayload for ClientResourcePackStatusPacket {
    const PACKET_ID: i32 = 0x31;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientResourcePackStatusPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.id.encode(buf)?;
        write_var_int(buf, self.status as i32);
        Ok(())
    }
}

impl Decode for ClientResourcePackStatusPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            id: Uuid::decode(buf)?,
            status: ResourcePackStatus::from_id(read_var_int(buf)?)?,
        })
    }
}
