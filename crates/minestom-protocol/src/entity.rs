//! Entity lifecycle, metadata, movement/rotation, velocity (`LpVec3`), animation,
//! and interaction packets for protocol 776 (MC 26.2).

use std::collections::BTreeMap;

use bytes::{Buf, BufMut};
use minestom_core::{BlockVec, Direction, Hand, MainHand, Pos, Vec3};
use minestom_item::ItemStack;
use minestom_macros::PacketMeta;
use minestom_text::Component;
use uuid::Uuid;

use crate::{
    ConnectionState, DEFAULT_MAX_STRING_CHARS, Decode, Encode, OptionalVarInt, PacketPayload,
    ProtocolError, read_prefixed_vec, read_string, read_var_int, read_var_long, write_prefixed_vec,
    write_string, write_var_int, write_var_long,
};

const LP_DATA_BITS_MASK: u64 = 0x7FFF;
const LP_MAX_QUANTIZED: f64 = 32_766.0;
const LP_SCALE_BITS_MASK: u64 = 0b11;
const LP_CONTINUATION_FLAG: u64 = 4;
const LP_ABS_MAX: f64 = 1.717_986_918_3e10;
const LP_ABS_MIN: f64 = 3.051_944_088_384_301e-5;

#[inline]
fn sanitize_lp(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-LP_ABS_MAX, LP_ABS_MAX)
    }
}

#[inline]
fn pack_lp_unit(v: f64) -> u64 {
    ((v * 0.5 + 0.5) * LP_MAX_QUANTIZED).round() as u64
}

#[inline]
fn unpack_lp_unit(raw: u64) -> f64 {
    ((raw & LP_DATA_BITS_MASK) as f64).min(LP_MAX_QUANTIZED) * 2.0 / LP_MAX_QUANTIZED - 1.0
}

/// Encodes a [`Vec3`] using Minecraft 26.2's `LpVec3` (`LP_VECTOR3`) variable-scale format.
pub fn write_lp_vec3<B: BufMut>(buf: &mut B, v: Vec3) {
    let x = sanitize_lp(v.x);
    let y = sanitize_lp(v.y);
    let z = sanitize_lp(v.z);
    let max = x.abs().max(y.abs().max(z.abs()));
    if max < LP_ABS_MIN {
        buf.put_u8(0);
    } else {
        let scale = max.ceil() as u64;
        let has_cont = (scale & LP_SCALE_BITS_MASK) != scale;
        let flags = if has_cont {
            (scale & LP_SCALE_BITS_MASK) | LP_CONTINUATION_FLAG
        } else {
            scale
        };
        let scale_f = scale as f64;
        let px = pack_lp_unit(x / scale_f) << 3;
        let py = pack_lp_unit(y / scale_f) << 18;
        let pz = pack_lp_unit(z / scale_f) << 33;
        let packed = flags | px | py | pz;
        buf.put_u8((packed & 0xFF) as u8);
        buf.put_u8(((packed >> 8) & 0xFF) as u8);
        buf.put_u32(((packed >> 16) & 0xFFFF_FFFF) as u32);
        if has_cont {
            write_var_int(buf, (scale >> 2) as i32);
        }
    }
}

/// Decodes a [`Vec3`] from Minecraft 26.2's `LpVec3` (`LP_VECTOR3`) wire format.
pub fn read_lp_vec3<B: Buf>(buf: &mut B) -> Result<Vec3, ProtocolError> {
    if buf.remaining() < 1 {
        return Err(ProtocolError::UnexpectedEof);
    }
    let flags = u64::from(buf.get_u8());
    if flags == 0 {
        return Ok(Vec3::ZERO);
    }
    if buf.remaining() < 5 {
        return Err(ProtocolError::UnexpectedEof);
    }
    let p2 = u64::from(buf.get_u8());
    let p3 = u64::from(buf.get_u32());
    let packed = (p3 << 16) | (p2 << 8) | flags;
    let mut scale = flags & LP_SCALE_BITS_MASK;
    if (flags & LP_CONTINUATION_FLAG) != 0 {
        let extra = read_var_int(buf)? as u32;
        scale |= u64::from(extra) << 2;
    }
    let s = scale as f64;
    Ok(Vec3::new(
        unpack_lp_unit(packed >> 3) * s,
        unpack_lp_unit(packed >> 18) * s,
        unpack_lp_unit(packed >> 33) * s,
    ))
}

#[inline]
fn angle_to_byte(deg: f32) -> i8 {
    (deg * 256.0 / 360.0) as i8
}

#[inline]
fn byte_to_angle(b: i8) -> f32 {
    f32::from(b) * 360.0 / 256.0
}

/// Computes the 1/4096th block delta `i16` between `new_coord` and `old_coord`
/// (`CoordConversion.deltaShort4096` in Minestom).
#[must_use]
pub fn delta_short_4096(new_coord: f64, old_coord: f64) -> i16 {
    let n = (new_coord * 4096.0).round() as i64;
    let o = (old_coord * 4096.0).round() as i64;
    (n.saturating_sub(o)).clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

/// Entity pose (`EntityPose` in Minestom 26.2, ordinal `0..=17`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(i32)]
pub enum EntityPose {
    /// `0` — Standing upright.
    #[default]
    Standing = 0,
    /// `1` — Gliding with Elytra.
    FallFlying = 1,
    /// `2` — Sleeping in bed.
    Sleeping = 2,
    /// `3` — Swimming / crawling.
    Swimming = 3,
    /// `4` — Riptide trident spin attack.
    SpinAttack = 4,
    /// `5` — Crouching / sneaking.
    Sneaking = 5,
    /// `6` — Goat / frog long jump.
    LongJumping = 6,
    /// `7` — Death tilt animation.
    Dying = 7,
    /// `8` — Frog croaking.
    Croaking = 8,
    /// `9` — Frog tongue attack.
    UsingTongue = 9,
    /// `10` — Camel sitting.
    Sitting = 10,
    /// `11` — Warden roaring.
    Roaring = 11,
    /// `12` — Warden/Sniffer sniffing.
    Sniffing = 12,
    /// `13` — Warden emerging.
    Emerging = 13,
    /// `14` — Warden/Sniffer digging.
    Digging = 14,
    /// `15` — Breeze sliding.
    Sliding = 15,
    /// `16` — Breeze shooting.
    Shooting = 16,
    /// `17` — Breeze inhaling.
    Inhaling = 17,
}

impl EntityPose {
    const fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::Standing),
            1 => Ok(Self::FallFlying),
            2 => Ok(Self::Sleeping),
            3 => Ok(Self::Swimming),
            4 => Ok(Self::SpinAttack),
            5 => Ok(Self::Sneaking),
            6 => Ok(Self::LongJumping),
            7 => Ok(Self::Dying),
            8 => Ok(Self::Croaking),
            9 => Ok(Self::UsingTongue),
            10 => Ok(Self::Sitting),
            11 => Ok(Self::Roaring),
            12 => Ok(Self::Sniffing),
            13 => Ok(Self::Emerging),
            14 => Ok(Self::Digging),
            15 => Ok(Self::Sliding),
            16 => Ok(Self::Shooting),
            17 => Ok(Self::Inhaling),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "EntityPose",
                value: other,
            }),
        }
    }
}

/// Typed MC 26.2 entity metadata value (`Metadata.Entry<?>` in Minestom, serializer IDs `0..=42`).
#[derive(Debug, Clone, PartialEq)]
pub enum MetadataValue {
    /// `0` — `BYTE` (`i8`).
    Byte(i8),
    /// `1` — `VAR_INT` (`i32`).
    VarInt(i32),
    /// `2` — `VAR_LONG` (`i64`).
    VarLong(i64),
    /// `3` — `FLOAT` (`f32`).
    Float(f32),
    /// `4` — `STRING`.
    String(String),
    /// `5` — `CHAT` (`Component`).
    Component(Component),
    /// `6` — `OPT_CHAT` (`Option<Component>`).
    OptComponent(Option<Component>),
    /// `7` — `ITEM_STACK` (`ItemStack`, used by `ItemEntity`, `ItemDisplay`, `ItemFrame`!).
    ItemStack(ItemStack),
    /// `8` — `BOOLEAN`.
    Boolean(bool),
    /// `9` — `ROTATION` (`[f32; 3]` pitch/yaw/roll).
    Rotation([f32; 3]),
    /// `10` — `BLOCK_POSITION` (`BlockVec`).
    BlockPosition(BlockVec),
    /// `11` — `OPT_BLOCK_POSITION` (`Option<BlockVec>`).
    OptBlockPosition(Option<BlockVec>),
    /// `12` — `DIRECTION` (`Direction`).
    Direction(Direction),
    /// `13` — `OPT_UUID` (`Option<Uuid>`).
    OptUuid(Option<Uuid>),
    /// `14` — `BLOCK_STATE` (`u32` state ID).
    BlockState(u32),
    /// `15` — `OPT_BLOCK_STATE` (`0 = None`, else `Some(state_id)`).
    OptBlockState(Option<u32>),
    /// `19` — `OPT_VAR_INT` (`OptionalVarInt`).
    OptVarInt(Option<i32>),
    /// `20` — `POSE` (`EntityPose`).
    Pose(EntityPose),
    /// `39` — `VECTOR3` (`[f32; 3]`).
    Vector3([f32; 3]),
    /// `40` — `QUATERNION` (`[f32; 4]`).
    Quaternion([f32; 4]),
    /// `42` — `MAIN_HAND` (`MainHand`).
    MainHand(MainHand),
}

impl Encode for MetadataValue {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        match self {
            Self::Byte(v) => {
                write_var_int(buf, 0);
                v.encode(buf)
            }
            Self::VarInt(v) => {
                write_var_int(buf, 1);
                write_var_int(buf, *v);
                Ok(())
            }
            Self::VarLong(v) => {
                write_var_int(buf, 2);
                write_var_long(buf, *v);
                Ok(())
            }
            Self::Float(v) => {
                write_var_int(buf, 3);
                v.encode(buf)
            }
            Self::String(s) => {
                write_var_int(buf, 4);
                write_string(buf, s, DEFAULT_MAX_STRING_CHARS)
            }
            Self::Component(c) => {
                write_var_int(buf, 5);
                c.encode(buf)
            }
            Self::OptComponent(opt) => {
                write_var_int(buf, 6);
                opt.encode(buf)
            }
            Self::ItemStack(item) => {
                write_var_int(buf, 7);
                item.encode(buf)
            }
            Self::Boolean(b) => {
                write_var_int(buf, 8);
                b.encode(buf)
            }
            Self::Rotation(r) => {
                write_var_int(buf, 9);
                r[0].encode(buf)?;
                r[1].encode(buf)?;
                r[2].encode(buf)
            }
            Self::BlockPosition(bp) => {
                write_var_int(buf, 10);
                bp.encode(buf)
            }
            Self::OptBlockPosition(opt) => {
                write_var_int(buf, 11);
                opt.encode(buf)
            }
            Self::Direction(d) => {
                write_var_int(buf, 12);
                write_var_int(buf, *d as i32);
                Ok(())
            }
            Self::OptUuid(opt) => {
                write_var_int(buf, 13);
                opt.encode(buf)
            }
            Self::BlockState(id) => {
                write_var_int(buf, 14);
                write_var_int(buf, *id as i32);
                Ok(())
            }
            Self::OptBlockState(opt) => {
                write_var_int(buf, 15);
                write_var_int(buf, opt.unwrap_or(0) as i32);
                Ok(())
            }
            Self::OptVarInt(opt) => {
                write_var_int(buf, 19);
                OptionalVarInt(*opt).encode(buf)
            }
            Self::Pose(pose) => {
                write_var_int(buf, 20);
                write_var_int(buf, *pose as i32);
                Ok(())
            }
            Self::Vector3(v) => {
                write_var_int(buf, 39);
                v[0].encode(buf)?;
                v[1].encode(buf)?;
                v[2].encode(buf)
            }
            Self::Quaternion(q) => {
                write_var_int(buf, 40);
                q[0].encode(buf)?;
                q[1].encode(buf)?;
                q[2].encode(buf)?;
                q[3].encode(buf)
            }
            Self::MainHand(mh) => {
                write_var_int(buf, 42);
                write_var_int(buf, *mh as i32);
                Ok(())
            }
        }
    }
}

impl Decode for MetadataValue {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let type_id = read_var_int(buf)?;
        match type_id {
            0 => Ok(Self::Byte(i8::decode(buf)?)),
            1 => Ok(Self::VarInt(read_var_int(buf)?)),
            2 => Ok(Self::VarLong(read_var_long(buf)?)),
            3 => Ok(Self::Float(f32::decode(buf)?)),
            4 => Ok(Self::String(read_string(buf, DEFAULT_MAX_STRING_CHARS)?)),
            5 => Ok(Self::Component(Component::decode(buf)?)),
            6 => Ok(Self::OptComponent(Option::<Component>::decode(buf)?)),
            7 => Ok(Self::ItemStack(ItemStack::decode(buf)?)),
            8 => Ok(Self::Boolean(bool::decode(buf)?)),
            9 => Ok(Self::Rotation([
                f32::decode(buf)?,
                f32::decode(buf)?,
                f32::decode(buf)?,
            ])),
            10 => Ok(Self::BlockPosition(BlockVec::decode(buf)?)),
            11 => Ok(Self::OptBlockPosition(Option::<BlockVec>::decode(buf)?)),
            12 => {
                let raw = read_var_int(buf)?;
                let dir = match raw {
                    0 => Direction::Down,
                    1 => Direction::Up,
                    2 => Direction::North,
                    3 => Direction::South,
                    4 => Direction::West,
                    5 => Direction::East,
                    other => {
                        return Err(ProtocolError::InvalidDiscriminant {
                            type_name: "Direction",
                            value: other,
                        });
                    }
                };
                Ok(Self::Direction(dir))
            }
            13 => Ok(Self::OptUuid(Option::<Uuid>::decode(buf)?)),
            14 => Ok(Self::BlockState(read_var_int(buf)? as u32)),
            15 => {
                let raw = read_var_int(buf)?;
                Ok(Self::OptBlockState(if raw <= 0 {
                    None
                } else {
                    Some(raw as u32)
                }))
            }
            19 => Ok(Self::OptVarInt(OptionalVarInt::decode(buf)?.0)),
            20 => Ok(Self::Pose(EntityPose::from_id(read_var_int(buf)?)?)),
            39 => Ok(Self::Vector3([
                f32::decode(buf)?,
                f32::decode(buf)?,
                f32::decode(buf)?,
            ])),
            40 => Ok(Self::Quaternion([
                f32::decode(buf)?,
                f32::decode(buf)?,
                f32::decode(buf)?,
                f32::decode(buf)?,
            ])),
            42 => {
                let mh = match read_var_int(buf)? {
                    0 => MainHand::Left,
                    1 => MainHand::Right,
                    other => {
                        return Err(ProtocolError::InvalidDiscriminant {
                            type_name: "MainHand",
                            value: other,
                        });
                    }
                };
                Ok(Self::MainHand(mh))
            }
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "MetadataType",
                value: other,
            }),
        }
    }
}

/// Clientbound `SpawnEntityPacket` (`0x01` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct SpawnEntityPacket {
    /// Runtime network entity ID.
    pub entity_id: i32,
    /// Entity UUID.
    pub uuid: Uuid,
    /// Protocol `entity_type` ID (`minecraft:entity_type`).
    pub entity_type: i32,
    /// Initial position and yaw/pitch.
    pub position: Pos,
    /// Initial head yaw in degrees.
    pub head_rot: f32,
    /// Type-specific object data (`0` for mobs/players, block state ID for falling blocks, etc.).
    pub data: i32,
    /// Initial velocity encoded via MC 26.2 `LpVec3`.
    pub velocity: Vec3,
}

impl PacketPayload for SpawnEntityPacket {
    const PACKET_ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for SpawnEntityPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.entity_id);
        self.uuid.encode(buf)?;
        write_var_int(buf, self.entity_type);
        self.position.x.encode(buf)?;
        self.position.y.encode(buf)?;
        self.position.z.encode(buf)?;
        write_lp_vec3(buf, self.velocity);
        angle_to_byte(self.position.pitch).encode(buf)?;
        angle_to_byte(self.position.yaw).encode(buf)?;
        angle_to_byte(self.head_rot).encode(buf)?;
        write_var_int(buf, self.data);
        Ok(())
    }
}

impl Decode for SpawnEntityPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let entity_id = read_var_int(buf)?;
        let uuid = Uuid::decode(buf)?;
        let entity_type = read_var_int(buf)?;
        let x = f64::decode(buf)?;
        let y = f64::decode(buf)?;
        let z = f64::decode(buf)?;
        let velocity = read_lp_vec3(buf)?;
        let pitch = byte_to_angle(i8::decode(buf)?);
        let yaw = byte_to_angle(i8::decode(buf)?);
        let head_rot = byte_to_angle(i8::decode(buf)?);
        let data = read_var_int(buf)?;
        Ok(Self {
            entity_id,
            uuid,
            entity_type,
            position: Pos::with_look(x, y, z, yaw, pitch),
            head_rot,
            data,
            velocity,
        })
    }
}

/// Animation kind inside [`EntityAnimationPacket`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum EntityAnimation {
    /// `0` — Swing main hand.
    SwingMainArm = 0,
    /// `1` — Damage tilt.
    TakeDamage = 1,
    /// `2` — Leave bed.
    LeaveBed = 2,
    /// `3` — Swing off-hand.
    SwingOffHand = 3,
    /// `4` — Critical hit sparks.
    CriticalEffect = 4,
    /// `5` — Enchanted weapon hit sparks.
    MagicalCriticalEffect = 5,
}

impl EntityAnimation {
    const fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::SwingMainArm),
            1 => Ok(Self::TakeDamage),
            2 => Ok(Self::LeaveBed),
            3 => Ok(Self::SwingOffHand),
            4 => Ok(Self::CriticalEffect),
            5 => Ok(Self::MagicalCriticalEffect),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "EntityAnimation",
                value: other,
            }),
        }
    }
}

/// Clientbound `EntityAnimationPacket` (`0x02` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct EntityAnimationPacket {
    /// Animated entity ID.
    pub entity_id: i32,
    /// Animation variant.
    pub animation: EntityAnimation,
}

impl PacketPayload for EntityAnimationPacket {
    const PACKET_ID: i32 = 0x02;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for EntityAnimationPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.entity_id);
        write_var_int(buf, self.animation as i32);
        Ok(())
    }
}

impl Decode for EntityAnimationPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            entity_id: read_var_int(buf)?,
            animation: EntityAnimation::from_id(read_var_int(buf)?)?,
        })
    }
}

/// Clientbound `EntityPositionSyncPacket` (`0x23` in [`ConnectionState::Play`], exact double sync).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct EntityPositionSyncPacket {
    /// Target entity ID.
    pub entity_id: i32,
    /// Exact world position (`3 x f64`).
    pub position: Vec3,
    /// Velocity/delta (`3 x f64`).
    pub delta: Vec3,
    /// Yaw in degrees (`f32`).
    pub yaw: f32,
    /// Pitch in degrees (`f32`).
    pub pitch: f32,
    /// Grounded state.
    pub on_ground: bool,
}

impl PacketPayload for EntityPositionSyncPacket {
    const PACKET_ID: i32 = 0x23;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for EntityPositionSyncPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.entity_id);
        self.position.x.encode(buf)?;
        self.position.y.encode(buf)?;
        self.position.z.encode(buf)?;
        self.delta.x.encode(buf)?;
        self.delta.y.encode(buf)?;
        self.delta.z.encode(buf)?;
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)?;
        self.on_ground.encode(buf)
    }
}

impl Decode for EntityPositionSyncPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            entity_id: read_var_int(buf)?,
            position: Vec3::new(f64::decode(buf)?, f64::decode(buf)?, f64::decode(buf)?),
            delta: Vec3::new(f64::decode(buf)?, f64::decode(buf)?, f64::decode(buf)?),
            yaw: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
            on_ground: bool::decode(buf)?,
        })
    }
}

/// Clientbound `EntityPositionPacket` (`0x35` in [`ConnectionState::Play`], `< 8` blocks delta).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct EntityPositionPacket {
    /// Entity ID.
    pub entity_id: i32,
    /// `1/4096` block X delta.
    pub delta_x: i16,
    /// `1/4096` block Y delta.
    pub delta_y: i16,
    /// `1/4096` block Z delta.
    pub delta_z: i16,
    /// Grounded state.
    pub on_ground: bool,
}

impl PacketPayload for EntityPositionPacket {
    const PACKET_ID: i32 = 0x35;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for EntityPositionPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.entity_id);
        self.delta_x.encode(buf)?;
        self.delta_y.encode(buf)?;
        self.delta_z.encode(buf)?;
        self.on_ground.encode(buf)
    }
}

impl Decode for EntityPositionPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            entity_id: read_var_int(buf)?,
            delta_x: i16::decode(buf)?,
            delta_y: i16::decode(buf)?,
            delta_z: i16::decode(buf)?,
            on_ground: bool::decode(buf)?,
        })
    }
}

/// Clientbound `EntityPositionAndRotationPacket` (`0x36` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct EntityPositionAndRotationPacket {
    /// Entity ID.
    pub entity_id: i32,
    /// `1/4096` block X delta.
    pub delta_x: i16,
    /// `1/4096` block Y delta.
    pub delta_y: i16,
    /// `1/4096` block Z delta.
    pub delta_z: i16,
    /// Yaw in degrees (quantized to `1/256` turn on wire).
    pub yaw: f32,
    /// Pitch in degrees (quantized to `1/256` turn on wire).
    pub pitch: f32,
    /// Grounded state.
    pub on_ground: bool,
}

impl PacketPayload for EntityPositionAndRotationPacket {
    const PACKET_ID: i32 = 0x36;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for EntityPositionAndRotationPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.entity_id);
        self.delta_x.encode(buf)?;
        self.delta_y.encode(buf)?;
        self.delta_z.encode(buf)?;
        angle_to_byte(self.yaw).encode(buf)?;
        angle_to_byte(self.pitch).encode(buf)?;
        self.on_ground.encode(buf)
    }
}

impl Decode for EntityPositionAndRotationPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            entity_id: read_var_int(buf)?,
            delta_x: i16::decode(buf)?,
            delta_y: i16::decode(buf)?,
            delta_z: i16::decode(buf)?,
            yaw: byte_to_angle(i8::decode(buf)?),
            pitch: byte_to_angle(i8::decode(buf)?),
            on_ground: bool::decode(buf)?,
        })
    }
}

/// Clientbound `EntityRotationPacket` (`0x38` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct EntityRotationPacket {
    /// Entity ID.
    pub entity_id: i32,
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// Grounded state.
    pub on_ground: bool,
}

impl PacketPayload for EntityRotationPacket {
    const PACKET_ID: i32 = 0x38;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for EntityRotationPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.entity_id);
        angle_to_byte(self.yaw).encode(buf)?;
        angle_to_byte(self.pitch).encode(buf)?;
        self.on_ground.encode(buf)
    }
}

impl Decode for EntityRotationPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            entity_id: read_var_int(buf)?,
            yaw: byte_to_angle(i8::decode(buf)?),
            pitch: byte_to_angle(i8::decode(buf)?),
            on_ground: bool::decode(buf)?,
        })
    }
}

/// Clientbound `PlayerInfoRemovePacket` (`0x45` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct PlayerInfoRemovePacket {
    /// Player UUIDs to remove from the tab list (max `1024`).
    pub uuids: Vec<Uuid>,
}

impl PacketPayload for PlayerInfoRemovePacket {
    const PACKET_ID: i32 = 0x45;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for PlayerInfoRemovePacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_prefixed_vec(buf, &self.uuids, 1024)
    }
}

impl Decode for PlayerInfoRemovePacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            uuids: read_prefixed_vec(buf, 1024)?,
        })
    }
}

/// Clientbound `DestroyEntitiesPacket` (`0x4D` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, Eq, PacketMeta)]
pub struct DestroyEntitiesPacket {
    /// Network entity IDs to despawn on the client (max `32_767`).
    pub entity_ids: Vec<i32>,
}

impl PacketPayload for DestroyEntitiesPacket {
    const PACKET_ID: i32 = 0x4D;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for DestroyEntitiesPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        if self.entity_ids.len() > 32_767 {
            return Err(ProtocolError::InvalidLength {
                len: self.entity_ids.len() as i32,
                max: 32_767,
            });
        }
        write_var_int(buf, self.entity_ids.len() as i32);
        for &id in &self.entity_ids {
            write_var_int(buf, id);
        }
        Ok(())
    }
}

impl Decode for DestroyEntitiesPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let raw_len = read_var_int(buf)?;
        if !(0..=32_767).contains(&raw_len) {
            return Err(ProtocolError::InvalidLength {
                len: raw_len,
                max: 32_767,
            });
        }
        let mut entity_ids = Vec::with_capacity((raw_len as usize).min(buf.remaining()));
        for _ in 0..raw_len {
            entity_ids.push(read_var_int(buf)?);
        }
        Ok(Self { entity_ids })
    }
}

/// Clientbound `EntityHeadLookPacket` (`0x53` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct EntityHeadLookPacket {
    /// Entity ID.
    pub entity_id: i32,
    /// Head yaw in degrees.
    pub yaw: f32,
}

impl PacketPayload for EntityHeadLookPacket {
    const PACKET_ID: i32 = 0x53;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for EntityHeadLookPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.entity_id);
        angle_to_byte(self.yaw).encode(buf)
    }
}

impl Decode for EntityHeadLookPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            entity_id: read_var_int(buf)?,
            yaw: byte_to_angle(i8::decode(buf)?),
        })
    }
}

/// Clientbound `EntityMetaDataPacket` (`0x63` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, PartialEq, PacketMeta)]
pub struct EntityMetaDataPacket {
    /// Target entity ID.
    pub entity_id: i32,
    /// Metadata index (`0..255`) -> [`MetadataValue`], terminated by `0xFF` on the wire.
    pub entries: BTreeMap<u8, MetadataValue>,
}

impl PacketPayload for EntityMetaDataPacket {
    const PACKET_ID: i32 = 0x63;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for EntityMetaDataPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.entity_id);
        for (&idx, val) in &self.entries {
            if idx == 0xFF {
                return Err(ProtocolError::InvalidField(
                    "metadata index 0xFF is reserved as terminator".to_owned(),
                ));
            }
            buf.put_u8(idx);
            val.encode(buf)?;
        }
        buf.put_u8(0xFF);
        Ok(())
    }
}

impl Decode for EntityMetaDataPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let entity_id = read_var_int(buf)?;
        let mut entries = BTreeMap::new();
        loop {
            let idx = u8::decode(buf)?;
            if idx == 0xFF {
                break;
            }
            if entries.len() >= 254 {
                return Err(ProtocolError::InvalidLength { len: 255, max: 254 });
            }
            let val = MetadataValue::decode(buf)?;
            entries.insert(idx, val);
        }
        Ok(Self { entity_id, entries })
    }
}

/// Clientbound `EntityVelocityPacket` (`0x65` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct EntityVelocityPacket {
    /// Entity ID.
    pub entity_id: i32,
    /// Velocity in blocks/tick encoded via `LpVec3`.
    pub velocity: Vec3,
}

impl PacketPayload for EntityVelocityPacket {
    const PACKET_ID: i32 = 0x65;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for EntityVelocityPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.entity_id);
        write_lp_vec3(buf, self.velocity);
        Ok(())
    }
}

impl Decode for EntityVelocityPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            entity_id: read_var_int(buf)?,
            velocity: read_lp_vec3(buf)?,
        })
    }
}

/// Serverbound `ClientAttackPacket` (`0x01` in [`ConnectionState::Play`], MC 26.2 standalone attack).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientAttackPacket {
    /// Attacked target entity ID.
    pub target_id: i32,
}

impl PacketPayload for ClientAttackPacket {
    const PACKET_ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientAttackPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.target_id);
        Ok(())
    }
}

impl Decode for ClientAttackPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            target_id: read_var_int(buf)?,
        })
    }
}

/// Serverbound `ClientInteractEntityPacket` (`0x1A` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, PacketMeta)]
pub struct ClientInteractEntityPacket {
    /// Interacted entity ID.
    pub target_id: i32,
    /// Hand used (`Main` or `Off`).
    pub hand: Hand,
    /// Hit location relative to entity (`LpVec3`).
    pub location: Vec3,
    /// Whether player held sneak during interaction.
    pub using_secondary_action: bool,
}

impl PacketPayload for ClientInteractEntityPacket {
    const PACKET_ID: i32 = 0x1A;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientInteractEntityPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.target_id);
        write_var_int(buf, self.hand as i32);
        write_lp_vec3(buf, self.location);
        self.using_secondary_action.encode(buf)
    }
}

impl Decode for ClientInteractEntityPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let target_id = read_var_int(buf)?;
        let hand = match read_var_int(buf)? {
            0 => Hand::Main,
            1 => Hand::Off,
            other => {
                return Err(ProtocolError::InvalidDiscriminant {
                    type_name: "Hand",
                    value: other,
                });
            }
        };
        let location = read_lp_vec3(buf)?;
        let using_secondary_action = bool::decode(buf)?;
        Ok(Self {
            target_id,
            hand,
            location,
            using_secondary_action,
        })
    }
}

/// Action kind inside [`ClientEntityActionPacket`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ClientEntityAction {
    /// `0` — Leave bed.
    LeaveBed = 0,
    /// `1` — Start sprinting.
    StartSprinting = 1,
    /// `2` — Stop sprinting.
    StopSprinting = 2,
    /// `3` — Start horse jump.
    StartJumpHorse = 3,
    /// `4` — Stop horse jump.
    StopJumpHorse = 4,
    /// `5` — Open horse inventory.
    OpenHorseInventory = 5,
    /// `6` — Start Elytra flight.
    StartFlyingElytra = 6,
}

impl ClientEntityAction {
    const fn from_id(id: i32) -> Result<Self, ProtocolError> {
        match id {
            0 => Ok(Self::LeaveBed),
            1 => Ok(Self::StartSprinting),
            2 => Ok(Self::StopSprinting),
            3 => Ok(Self::StartJumpHorse),
            4 => Ok(Self::StopJumpHorse),
            5 => Ok(Self::OpenHorseInventory),
            6 => Ok(Self::StartFlyingElytra),
            other => Err(ProtocolError::InvalidDiscriminant {
                type_name: "ClientEntityAction",
                value: other,
            }),
        }
    }
}

/// Serverbound `ClientEntityActionPacket` (`0x2A` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientEntityActionPacket {
    /// Player entity ID.
    pub player_id: i32,
    /// Action discriminant.
    pub action: ClientEntityAction,
    /// Horse jump boost (`0..=100`).
    pub horse_jump_boost: i32,
}

impl PacketPayload for ClientEntityActionPacket {
    const PACKET_ID: i32 = 0x2A;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientEntityActionPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.player_id);
        write_var_int(buf, self.action as i32);
        write_var_int(buf, self.horse_jump_boost);
        Ok(())
    }
}

impl Decode for ClientEntityActionPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            player_id: read_var_int(buf)?,
            action: ClientEntityAction::from_id(read_var_int(buf)?)?,
            horse_jump_boost: read_var_int(buf)?,
        })
    }
}

/// Serverbound `ClientInputPacket` (`0x2B` in [`ConnectionState::Play`], carries sneak/sprint/movement flags!).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientInputPacket {
    /// Bitmask (`0x01` forward, `0x02` backward, `0x04` left, `0x08` right, `0x10` jump, `0x20` shift/sneak, `0x40` sprint).
    pub flags: u8,
}

impl ClientInputPacket {
    /// Returns `true` if shift/sneak (`0x20`) is held.
    #[must_use]
    pub const fn shift(self) -> bool {
        (self.flags & 0x20) != 0
    }

    /// Returns `true` if sprint (`0x40`) is held.
    #[must_use]
    pub const fn sprint(self) -> bool {
        (self.flags & 0x40) != 0
    }

    /// Returns `true` if jump (`0x10`) is held.
    #[must_use]
    pub const fn jump(self) -> bool {
        (self.flags & 0x10) != 0
    }
}

impl PacketPayload for ClientInputPacket {
    const PACKET_ID: i32 = 0x2B;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientInputPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        self.flags.encode(buf)
    }
}

impl Decode for ClientInputPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        Ok(Self {
            flags: u8::decode(buf)?,
        })
    }
}

/// Serverbound `ClientAnimationPacket` (`0x3F` in [`ConnectionState::Play`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PacketMeta)]
pub struct ClientAnimationPacket {
    /// Swung hand.
    pub hand: Hand,
}

impl PacketPayload for ClientAnimationPacket {
    const PACKET_ID: i32 = 0x3F;
    const STATE: ConnectionState = ConnectionState::Play;
}

impl Encode for ClientAnimationPacket {
    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), ProtocolError> {
        write_var_int(buf, self.hand as i32);
        Ok(())
    }
}

impl Decode for ClientAnimationPacket {
    fn decode<B: Buf>(buf: &mut B) -> Result<Self, ProtocolError> {
        let hand = match read_var_int(buf)? {
            0 => Hand::Main,
            1 => Hand::Off,
            other => {
                return Err(ProtocolError::InvalidDiscriminant {
                    type_name: "Hand",
                    value: other,
                });
            }
        };
        Ok(Self { hand })
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use proptest::prelude::*;

    use super::*;
    use crate::{decode_packet_exact, encode_packet_payload};

    #[test]
    fn spawn_entity_metadata_and_lp_vec3_roundtrip() {
        let spawn = SpawnEntityPacket {
            entity_id: 77,
            uuid: Uuid::from_u128(42),
            entity_type: 155,
            position: Pos::with_look(12.5, 42.0, -8.25, 90.0, 0.0),
            head_rot: 90.0,
            data: 0,
            velocity: Vec3::new(0.25, -0.5, 1.25),
        };
        let mut b = encode_packet_payload(&spawn).unwrap();
        assert_eq!(read_var_int(&mut b).unwrap(), 0x01);
        let decoded = decode_packet_exact::<SpawnEntityPacket>(b).unwrap();
        assert_eq!(decoded.entity_id, 77);
        assert!((decoded.velocity.x - 0.25).abs() < 1e-3);
        assert!((decoded.velocity.y - -0.5).abs() < 1e-3);
        assert!((decoded.velocity.z - 1.25).abs() < 1e-3);

        let mut entries = BTreeMap::new();
        entries.insert(0, MetadataValue::Byte(0x02));
        entries.insert(
            2,
            MetadataValue::OptComponent(Some(Component::text("Guard"))),
        );
        entries.insert(3, MetadataValue::Boolean(true));
        entries.insert(6, MetadataValue::Pose(EntityPose::Sneaking));
        entries.insert(15, MetadataValue::MainHand(MainHand::Right));
        let meta = EntityMetaDataPacket {
            entity_id: 77,
            entries,
        };
        let mut mb = encode_packet_payload(&meta).unwrap();
        assert_eq!(read_var_int(&mut mb).unwrap(), 0x63);
        assert_eq!(
            decode_packet_exact::<EntityMetaDataPacket>(mb).unwrap(),
            meta
        );
    }

    proptest! {
        #[test]
        fn lp_vec3_roundtrips_within_quantization_tolerance(
            x in -100.0_f64..100.0,
            y in -100.0_f64..100.0,
            z in -100.0_f64..100.0,
        ) {
            let mut buf = BytesMut::new();
            let v = Vec3::new(x, y, z);
            write_lp_vec3(&mut buf, v);
            let mut reader = buf.freeze();
            let out = read_lp_vec3(&mut reader).unwrap();
            prop_assert!((out.x - x).abs() < 0.02);
            prop_assert!((out.y - y).abs() < 0.02);
            prop_assert!((out.z - z).abs() < 0.02);
        }

        #[test]
        fn arbitrary_bytes_never_panic_entity_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let b = bytes::Bytes::from(raw);
            let _ = decode_packet_exact::<SpawnEntityPacket>(b.clone());
            let _ = decode_packet_exact::<EntityAnimationPacket>(b.clone());
            let _ = decode_packet_exact::<EntityPositionSyncPacket>(b.clone());
            let _ = decode_packet_exact::<EntityPositionPacket>(b.clone());
            let _ = decode_packet_exact::<EntityPositionAndRotationPacket>(b.clone());
            let _ = decode_packet_exact::<EntityRotationPacket>(b.clone());
            let _ = decode_packet_exact::<PlayerInfoRemovePacket>(b.clone());
            let _ = decode_packet_exact::<DestroyEntitiesPacket>(b.clone());
            let _ = decode_packet_exact::<EntityHeadLookPacket>(b.clone());
            let _ = decode_packet_exact::<EntityMetaDataPacket>(b.clone());
            let _ = decode_packet_exact::<EntityVelocityPacket>(b.clone());
            let _ = decode_packet_exact::<ClientAttackPacket>(b.clone());
            let _ = decode_packet_exact::<ClientInteractEntityPacket>(b.clone());
            let _ = decode_packet_exact::<ClientEntityActionPacket>(b.clone());
            let _ = decode_packet_exact::<ClientInputPacket>(b.clone());
            let _ = decode_packet_exact::<ClientAnimationPacket>(b);
        }
    }
}
