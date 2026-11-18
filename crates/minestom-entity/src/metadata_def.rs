//! Complete MC 26.2 `MetadataDef` hierarchy (`110` entity classes, `298` typed keys & bitmasks)
//! matching upstream Minestom `MetadataDef.java` index-for-index.
//!
//! Use [`EntityMetadata::set_key`](crate::EntityMetadata::set_key),
//! [`EntityMetadata::get_key`](crate::EntityMetadata::get_key),
//! [`EntityMetadata::set_flag`](crate::EntityMetadata::set_flag), and
//! [`EntityMetadata::get_flag`](crate::EntityMetadata::get_flag) with the constants in these
//! modules for zero-cost compile-time type-checked metadata access across all 158 entity types.

#![allow(clippy::wildcard_imports, clippy::option_option, clippy::doc_markdown)]

use minestom_block::Block;
use minestom_core::{BlockVec, Direction, MainHand};
use minestom_item::ItemStack;
use minestom_protocol::entity::{
    ArmadilloState, CopperGolemState, EntityPose, MetadataValue, ResolvableProfile, SnifferState,
    VillagerData, WeatherState,
};
use minestom_protocol::ui_media::ParticleEntry;
use minestom_text::Component;
use uuid::Uuid;

/// Strongly typed entity metadata entry descriptor (`MetadataDef.Entry<T>` in Minestom).
#[derive(Debug)]
pub struct MetadataKey<T> {
    index: u8,
    to_meta: fn(T) -> MetadataValue,
    from_meta: fn(&MetadataValue) -> Option<T>,
}

impl<T> Clone for MetadataKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for MetadataKey<T> {}

impl<T> MetadataKey<T> {
    /// Constructs a typed [`MetadataKey`] at wire `index`.
    #[must_use]
    pub const fn new(
        index: u8,
        to_meta: fn(T) -> MetadataValue,
        from_meta: fn(&MetadataValue) -> Option<T>,
    ) -> Self {
        Self {
            index,
            to_meta,
            from_meta,
        }
    }

    /// Wire metadata index (`0..255`).
    #[must_use]
    pub const fn index(self) -> u8 {
        self.index
    }

    /// Converts `value` into its wire [`MetadataValue`].
    #[must_use]
    pub fn encode_value(self, value: T) -> MetadataValue {
        (self.to_meta)(value)
    }

    /// Extracts `T` from a wire [`MetadataValue`], returning `None` on variant mismatch.
    #[must_use]
    pub fn decode_value(self, raw: &MetadataValue) -> Option<T> {
        (self.from_meta)(raw)
    }
}

/// Boolean bitflag inside a `MetadataValue::Byte` entry (`MetadataDef.Entry.BitMask` in Minestom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BitMaskKey {
    index: u8,
    mask: u8,
    default: bool,
}

impl BitMaskKey {
    /// Constructs a [`BitMaskKey`] at `index` with single-bit `mask` and `default` state.
    #[must_use]
    pub const fn new(index: u8, mask: u8, default: bool) -> Self {
        Self {
            index,
            mask,
            default,
        }
    }

    /// Wire metadata byte index.
    #[must_use]
    pub const fn index(self) -> u8 {
        self.index
    }

    /// Single-bit mask inside the byte.
    #[must_use]
    pub const fn mask(self) -> u8 {
        self.mask
    }

    /// Default boolean state when unset.
    #[must_use]
    pub const fn default_value(self) -> bool {
        self.default
    }
}

/// Multi-bit sub-byte field inside a `MetadataValue::Byte` entry (`MetadataDef.Entry.ByteMask` in Minestom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ByteMaskKey {
    index: u8,
    mask: u8,
    shift: u8,
    default: u8,
}

impl ByteMaskKey {
    /// Constructs a [`ByteMaskKey`].
    #[must_use]
    pub const fn new(index: u8, mask: u8, shift: u8, default: u8) -> Self {
        Self {
            index,
            mask,
            shift,
            default,
        }
    }

    /// Wire metadata byte index.
    #[must_use]
    pub const fn index(self) -> u8 {
        self.index
    }

    /// Bitmask before shifting.
    #[must_use]
    pub const fn mask(self) -> u8 {
        self.mask
    }

    /// Bit shift offset.
    #[must_use]
    pub const fn shift(self) -> u8 {
        self.shift
    }

    /// Default value when unset.
    #[must_use]
    pub const fn default_value(self) -> u8 {
        self.default
    }
}

#[allow(clippy::needless_pass_by_value)]
mod conv {
    use super::*;

    pub fn to_byte(v: i8) -> MetadataValue {
        MetadataValue::Byte(v)
    }
    pub fn from_byte(m: &MetadataValue) -> Option<i8> {
        if let MetadataValue::Byte(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_var_int(v: i32) -> MetadataValue {
        MetadataValue::VarInt(v)
    }
    pub fn from_var_int(m: &MetadataValue) -> Option<i32> {
        if let MetadataValue::VarInt(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_var_long(v: i64) -> MetadataValue {
        MetadataValue::VarLong(v)
    }
    pub fn from_var_long(m: &MetadataValue) -> Option<i64> {
        if let MetadataValue::VarLong(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_float(v: f32) -> MetadataValue {
        MetadataValue::Float(v)
    }
    pub fn from_float(m: &MetadataValue) -> Option<f32> {
        if let MetadataValue::Float(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_string(v: String) -> MetadataValue {
        MetadataValue::String(v)
    }
    pub fn from_string(m: &MetadataValue) -> Option<String> {
        if let MetadataValue::String(v) = m {
            Some(v.clone())
        } else {
            None
        }
    }

    pub fn to_component(v: Component) -> MetadataValue {
        MetadataValue::Component(v)
    }
    pub fn from_component(m: &MetadataValue) -> Option<Component> {
        if let MetadataValue::Component(v) = m {
            Some(v.clone())
        } else {
            None
        }
    }

    pub fn to_opt_component(v: Option<Component>) -> MetadataValue {
        MetadataValue::OptComponent(v)
    }
    pub fn from_opt_component(m: &MetadataValue) -> Option<Option<Component>> {
        if let MetadataValue::OptComponent(v) = m {
            Some(v.clone())
        } else {
            None
        }
    }

    pub fn to_item_stack(v: ItemStack) -> MetadataValue {
        MetadataValue::ItemStack(v)
    }
    pub fn from_item_stack(m: &MetadataValue) -> Option<ItemStack> {
        if let MetadataValue::ItemStack(v) = m {
            Some(v.clone())
        } else {
            None
        }
    }

    pub fn to_bool(v: bool) -> MetadataValue {
        MetadataValue::Boolean(v)
    }
    pub fn from_bool(m: &MetadataValue) -> Option<bool> {
        if let MetadataValue::Boolean(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_rotation(v: [f32; 3]) -> MetadataValue {
        MetadataValue::Rotation(v)
    }
    pub fn from_rotation(m: &MetadataValue) -> Option<[f32; 3]> {
        if let MetadataValue::Rotation(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_block_pos(v: BlockVec) -> MetadataValue {
        MetadataValue::BlockPosition(v)
    }
    pub fn from_block_pos(m: &MetadataValue) -> Option<BlockVec> {
        if let MetadataValue::BlockPosition(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_opt_block_pos(v: Option<BlockVec>) -> MetadataValue {
        MetadataValue::OptBlockPosition(v)
    }
    pub fn from_opt_block_pos(m: &MetadataValue) -> Option<Option<BlockVec>> {
        if let MetadataValue::OptBlockPosition(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_direction(v: Direction) -> MetadataValue {
        MetadataValue::Direction(v)
    }
    pub fn from_direction(m: &MetadataValue) -> Option<Direction> {
        if let MetadataValue::Direction(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_opt_uuid(v: Option<Uuid>) -> MetadataValue {
        MetadataValue::OptUuid(v)
    }
    pub fn from_opt_uuid(m: &MetadataValue) -> Option<Option<Uuid>> {
        if let MetadataValue::OptUuid(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_block_state(v: Block) -> MetadataValue {
        MetadataValue::BlockState(u32::from(v.state_id()))
    }
    pub fn from_block_state(m: &MetadataValue) -> Option<Block> {
        if let MetadataValue::BlockState(id) = m {
            u16::try_from(*id).ok().and_then(Block::from_state_id_opt)
        } else {
            None
        }
    }

    pub fn to_opt_block_state(v: Option<Block>) -> MetadataValue {
        MetadataValue::OptBlockState(v.map(|b| u32::from(b.state_id())))
    }
    pub fn from_opt_block_state(m: &MetadataValue) -> Option<Option<Block>> {
        if let MetadataValue::OptBlockState(opt) = m {
            Some(
                opt.and_then(|id| u16::try_from(id).ok())
                    .and_then(Block::from_state_id_opt),
            )
        } else {
            None
        }
    }

    pub fn to_particle(v: ParticleEntry) -> MetadataValue {
        MetadataValue::Particle(v)
    }
    pub fn from_particle(m: &MetadataValue) -> Option<ParticleEntry> {
        if let MetadataValue::Particle(v) = m {
            Some(v.clone())
        } else {
            None
        }
    }

    pub fn to_particle_list(v: Vec<ParticleEntry>) -> MetadataValue {
        MetadataValue::ParticleList(v)
    }
    pub fn from_particle_list(m: &MetadataValue) -> Option<Vec<ParticleEntry>> {
        if let MetadataValue::ParticleList(v) = m {
            Some(v.clone())
        } else {
            None
        }
    }

    pub fn to_villager_data(v: VillagerData) -> MetadataValue {
        MetadataValue::VillagerData(v)
    }
    pub fn from_villager_data(m: &MetadataValue) -> Option<VillagerData> {
        if let MetadataValue::VillagerData(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_opt_var_int(v: Option<i32>) -> MetadataValue {
        MetadataValue::OptVarInt(v)
    }
    pub fn from_opt_var_int(m: &MetadataValue) -> Option<Option<i32>> {
        if let MetadataValue::OptVarInt(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_pose(v: EntityPose) -> MetadataValue {
        MetadataValue::Pose(v)
    }
    pub fn from_pose(m: &MetadataValue) -> Option<EntityPose> {
        if let MetadataValue::Pose(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_cat_variant(v: u32) -> MetadataValue {
        MetadataValue::CatVariant(v)
    }
    pub fn from_cat_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::CatVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_cat_sound_variant(v: u32) -> MetadataValue {
        MetadataValue::CatSoundVariant(v)
    }
    pub fn from_cat_sound_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::CatSoundVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_cow_variant(v: u32) -> MetadataValue {
        MetadataValue::CowVariant(v)
    }
    pub fn from_cow_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::CowVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_cow_sound_variant(v: u32) -> MetadataValue {
        MetadataValue::CowSoundVariant(v)
    }
    pub fn from_cow_sound_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::CowSoundVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_wolf_variant(v: u32) -> MetadataValue {
        MetadataValue::WolfVariant(v)
    }
    pub fn from_wolf_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::WolfVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_wolf_sound_variant(v: u32) -> MetadataValue {
        MetadataValue::WolfSoundVariant(v)
    }
    pub fn from_wolf_sound_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::WolfSoundVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_frog_variant(v: u32) -> MetadataValue {
        MetadataValue::FrogVariant(v)
    }
    pub fn from_frog_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::FrogVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_pig_variant(v: u32) -> MetadataValue {
        MetadataValue::PigVariant(v)
    }
    pub fn from_pig_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::PigVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_pig_sound_variant(v: u32) -> MetadataValue {
        MetadataValue::PigSoundVariant(v)
    }
    pub fn from_pig_sound_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::PigSoundVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_chicken_variant(v: u32) -> MetadataValue {
        MetadataValue::ChickenVariant(v)
    }
    pub fn from_chicken_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::ChickenVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_chicken_sound_variant(v: u32) -> MetadataValue {
        MetadataValue::ChickenSoundVariant(v)
    }
    pub fn from_chicken_sound_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::ChickenSoundVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_zombie_nautilus_variant(v: u32) -> MetadataValue {
        MetadataValue::ZombieNautilusVariant(v)
    }
    pub fn from_zombie_nautilus_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::ZombieNautilusVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_painting_variant(v: u32) -> MetadataValue {
        MetadataValue::PaintingVariant(v)
    }
    pub fn from_painting_variant(m: &MetadataValue) -> Option<u32> {
        if let MetadataValue::PaintingVariant(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_sniffer_state(v: SnifferState) -> MetadataValue {
        MetadataValue::SnifferState(v)
    }
    pub fn from_sniffer_state(m: &MetadataValue) -> Option<SnifferState> {
        if let MetadataValue::SnifferState(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_armadillo_state(v: ArmadilloState) -> MetadataValue {
        MetadataValue::ArmadilloState(v)
    }
    pub fn from_armadillo_state(m: &MetadataValue) -> Option<ArmadilloState> {
        if let MetadataValue::ArmadilloState(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_copper_golem_state(v: CopperGolemState) -> MetadataValue {
        MetadataValue::CopperGolemState(v)
    }
    pub fn from_copper_golem_state(m: &MetadataValue) -> Option<CopperGolemState> {
        if let MetadataValue::CopperGolemState(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_weather_state(v: WeatherState) -> MetadataValue {
        MetadataValue::WeatherState(v)
    }
    pub fn from_weather_state(m: &MetadataValue) -> Option<WeatherState> {
        if let MetadataValue::WeatherState(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_vector3(v: [f32; 3]) -> MetadataValue {
        MetadataValue::Vector3(v)
    }
    pub fn from_vector3(m: &MetadataValue) -> Option<[f32; 3]> {
        if let MetadataValue::Vector3(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_quaternion(v: [f32; 4]) -> MetadataValue {
        MetadataValue::Quaternion(v)
    }
    pub fn from_quaternion(m: &MetadataValue) -> Option<[f32; 4]> {
        if let MetadataValue::Quaternion(v) = m {
            Some(*v)
        } else {
            None
        }
    }

    pub fn to_profile(v: ResolvableProfile) -> MetadataValue {
        MetadataValue::ResolvableProfile(v)
    }
    pub fn from_profile(m: &MetadataValue) -> Option<ResolvableProfile> {
        if let MetadataValue::ResolvableProfile(v) = m {
            Some(v.clone())
        } else {
            None
        }
    }

    pub fn to_main_hand(v: MainHand) -> MetadataValue {
        MetadataValue::MainHand(v)
    }
    pub fn from_main_hand(m: &MetadataValue) -> Option<MainHand> {
        if let MetadataValue::MainHand(v) = m {
            Some(*v)
        } else {
            None
        }
    }
}

/// Metadata keys for `MetadataDef` (base entity indices `0..8`).
pub mod entity {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`0`).
    pub const START_INDEX: u8 = 0;
    /// Exclusive end metadata index of this class (`8`).
    pub const END_INDEX: u8 = 8;
    /// `ENTITY_FLAGS` at metadata index `0` (`Metadata::Byte`).
    pub const ENTITY_FLAGS: MetadataKey<i8> = MetadataKey::new(0, conv::to_byte, conv::from_byte);
    /// `IS_ON_FIRE` bitmask `0x01` at metadata index `0`.
    pub const IS_ON_FIRE: BitMaskKey = BitMaskKey::new(0, 0x01, false);
    /// `IS_CROUCHING` bitmask `0x02` at metadata index `0`.
    pub const IS_CROUCHING: BitMaskKey = BitMaskKey::new(0, 0x02, false);
    /// `IS_SPRINTING` bitmask `0x08` at metadata index `0`.
    pub const IS_SPRINTING: BitMaskKey = BitMaskKey::new(0, 0x08, false);
    /// `IS_SWIMMING` bitmask `0x10` at metadata index `0`.
    pub const IS_SWIMMING: BitMaskKey = BitMaskKey::new(0, 0x10, false);
    /// `IS_INVISIBLE` bitmask `0x20` at metadata index `0`.
    pub const IS_INVISIBLE: BitMaskKey = BitMaskKey::new(0, 0x20, false);
    /// `HAS_GLOWING_EFFECT` bitmask `0x40` at metadata index `0`.
    pub const HAS_GLOWING_EFFECT: BitMaskKey = BitMaskKey::new(0, 0x40, false);
    /// `IS_FLYING_WITH_ELYTRA` bitmask `0x80` at metadata index `0`.
    pub const IS_FLYING_WITH_ELYTRA: BitMaskKey = BitMaskKey::new(0, 0x80, false);
    /// `AIR_TICKS` at metadata index `1` (`Metadata::VarInt`).
    pub const AIR_TICKS: MetadataKey<i32> =
        MetadataKey::new(1, conv::to_var_int, conv::from_var_int);
    /// `CUSTOM_NAME` at metadata index `2` (`Metadata::OptComponent`).
    pub const CUSTOM_NAME: MetadataKey<Option<Component>> =
        MetadataKey::new(2, conv::to_opt_component, conv::from_opt_component);
    /// `CUSTOM_NAME_VISIBLE` at metadata index `3` (`Metadata::Boolean`).
    pub const CUSTOM_NAME_VISIBLE: MetadataKey<bool> =
        MetadataKey::new(3, conv::to_bool, conv::from_bool);
    /// `IS_SILENT` at metadata index `4` (`Metadata::Boolean`).
    pub const IS_SILENT: MetadataKey<bool> = MetadataKey::new(4, conv::to_bool, conv::from_bool);
    /// `HAS_NO_GRAVITY` at metadata index `5` (`Metadata::Boolean`).
    pub const HAS_NO_GRAVITY: MetadataKey<bool> =
        MetadataKey::new(5, conv::to_bool, conv::from_bool);
    /// `POSE` at metadata index `6` (`Metadata::Pose`).
    pub const POSE: MetadataKey<EntityPose> = MetadataKey::new(6, conv::to_pose, conv::from_pose);
    /// `TICKS_FROZEN` at metadata index `7` (`Metadata::VarInt`).
    pub const TICKS_FROZEN: MetadataKey<i32> =
        MetadataKey::new(7, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Interaction` (extends `entity`, indices `8..11`).
pub mod interaction {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`11`).
    pub const END_INDEX: u8 = 11;
    /// `WIDTH` at metadata index `8` (`Metadata::Float`).
    pub const WIDTH: MetadataKey<f32> = MetadataKey::new(8, conv::to_float, conv::from_float);
    /// `HEIGHT` at metadata index `9` (`Metadata::Float`).
    pub const HEIGHT: MetadataKey<f32> = MetadataKey::new(9, conv::to_float, conv::from_float);
    /// `RESPONSIVE` at metadata index `10` (`Metadata::Boolean`).
    pub const RESPONSIVE: MetadataKey<bool> = MetadataKey::new(10, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Display` (extends `entity`, indices `8..23`).
pub mod display {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`23`).
    pub const END_INDEX: u8 = 23;
    /// `INTERPOLATION_DELAY` at metadata index `8` (`Metadata::VarInt`).
    pub const INTERPOLATION_DELAY: MetadataKey<i32> =
        MetadataKey::new(8, conv::to_var_int, conv::from_var_int);
    /// `TRANSFORMATION_INTERPOLATION_DURATION` at metadata index `9` (`Metadata::VarInt`).
    pub const TRANSFORMATION_INTERPOLATION_DURATION: MetadataKey<i32> =
        MetadataKey::new(9, conv::to_var_int, conv::from_var_int);
    /// `POSITION_ROTATION_INTERPOLATION_DURATION` at metadata index `10` (`Metadata::VarInt`).
    pub const POSITION_ROTATION_INTERPOLATION_DURATION: MetadataKey<i32> =
        MetadataKey::new(10, conv::to_var_int, conv::from_var_int);
    /// `TRANSLATION` at metadata index `11` (`Metadata::Vector3`).
    pub const TRANSLATION: MetadataKey<[f32; 3]> =
        MetadataKey::new(11, conv::to_vector3, conv::from_vector3);
    /// `SCALE` at metadata index `12` (`Metadata::Vector3`).
    pub const SCALE: MetadataKey<[f32; 3]> =
        MetadataKey::new(12, conv::to_vector3, conv::from_vector3);
    /// `ROTATION_LEFT` at metadata index `13` (`Metadata::Quaternion`).
    pub const ROTATION_LEFT: MetadataKey<[f32; 4]> =
        MetadataKey::new(13, conv::to_quaternion, conv::from_quaternion);
    /// `ROTATION_RIGHT` at metadata index `14` (`Metadata::Quaternion`).
    pub const ROTATION_RIGHT: MetadataKey<[f32; 4]> =
        MetadataKey::new(14, conv::to_quaternion, conv::from_quaternion);
    /// `BILLBOARD_CONSTRAINTS` at metadata index `15` (`Metadata::Byte`).
    pub const BILLBOARD_CONSTRAINTS: MetadataKey<i8> =
        MetadataKey::new(15, conv::to_byte, conv::from_byte);
    /// `BRIGHTNESS_OVERRIDE` at metadata index `16` (`Metadata::VarInt`).
    pub const BRIGHTNESS_OVERRIDE: MetadataKey<i32> =
        MetadataKey::new(16, conv::to_var_int, conv::from_var_int);
    /// `VIEW_RANGE` at metadata index `17` (`Metadata::Float`).
    pub const VIEW_RANGE: MetadataKey<f32> = MetadataKey::new(17, conv::to_float, conv::from_float);
    /// `SHADOW_RADIUS` at metadata index `18` (`Metadata::Float`).
    pub const SHADOW_RADIUS: MetadataKey<f32> =
        MetadataKey::new(18, conv::to_float, conv::from_float);
    /// `SHADOW_STRENGTH` at metadata index `19` (`Metadata::Float`).
    pub const SHADOW_STRENGTH: MetadataKey<f32> =
        MetadataKey::new(19, conv::to_float, conv::from_float);
    /// `WIDTH` at metadata index `20` (`Metadata::Float`).
    pub const WIDTH: MetadataKey<f32> = MetadataKey::new(20, conv::to_float, conv::from_float);
    /// `HEIGHT` at metadata index `21` (`Metadata::Float`).
    pub const HEIGHT: MetadataKey<f32> = MetadataKey::new(21, conv::to_float, conv::from_float);
    /// `GLOW_COLOR_OVERRIDE` at metadata index `22` (`Metadata::VarInt`).
    pub const GLOW_COLOR_OVERRIDE: MetadataKey<i32> =
        MetadataKey::new(22, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `BlockDisplay` (extends `display`, indices `23..24`).
pub mod block_display {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`23`).
    pub const START_INDEX: u8 = 23;
    /// Exclusive end metadata index of this class (`24`).
    pub const END_INDEX: u8 = 24;
    /// `DISPLAYED_BLOCK_STATE` at metadata index `23` (`Metadata::BlockState`).
    pub const DISPLAYED_BLOCK_STATE: MetadataKey<Block> =
        MetadataKey::new(23, conv::to_block_state, conv::from_block_state);
}

/// Metadata keys for `ItemDisplay` (extends `display`, indices `23..25`).
pub mod item_display {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`23`).
    pub const START_INDEX: u8 = 23;
    /// Exclusive end metadata index of this class (`25`).
    pub const END_INDEX: u8 = 25;
    /// `DISPLAYED_ITEM` at metadata index `23` (`Metadata::ItemStack`).
    pub const DISPLAYED_ITEM: MetadataKey<ItemStack> =
        MetadataKey::new(23, conv::to_item_stack, conv::from_item_stack);
    /// `DISPLAY_TYPE` at metadata index `24` (`Metadata::Byte`).
    pub const DISPLAY_TYPE: MetadataKey<i8> = MetadataKey::new(24, conv::to_byte, conv::from_byte);
}

/// Metadata keys for `TextDisplay` (extends `display`, indices `23..28`).
pub mod text_display {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`23`).
    pub const START_INDEX: u8 = 23;
    /// Exclusive end metadata index of this class (`28`).
    pub const END_INDEX: u8 = 28;
    /// `TEXT` at metadata index `23` (`Metadata::Component`).
    pub const TEXT: MetadataKey<Component> =
        MetadataKey::new(23, conv::to_component, conv::from_component);
    /// `LINE_WIDTH` at metadata index `24` (`Metadata::VarInt`).
    pub const LINE_WIDTH: MetadataKey<i32> =
        MetadataKey::new(24, conv::to_var_int, conv::from_var_int);
    /// `BACKGROUND_COLOR` at metadata index `25` (`Metadata::VarInt`).
    pub const BACKGROUND_COLOR: MetadataKey<i32> =
        MetadataKey::new(25, conv::to_var_int, conv::from_var_int);
    /// `TEXT_OPACITY` at metadata index `26` (`Metadata::Byte`).
    pub const TEXT_OPACITY: MetadataKey<i8> = MetadataKey::new(26, conv::to_byte, conv::from_byte);
    /// `TEXT_DISPLAY_FLAGS` at metadata index `27` (`Metadata::Byte`).
    pub const TEXT_DISPLAY_FLAGS: MetadataKey<i8> =
        MetadataKey::new(27, conv::to_byte, conv::from_byte);
    /// `HAS_SHADOW` bitmask `0x01` at metadata index `27`.
    pub const HAS_SHADOW: BitMaskKey = BitMaskKey::new(27, 0x01, false);
    /// `IS_SEE_THROUGH` bitmask `0x02` at metadata index `27`.
    pub const IS_SEE_THROUGH: BitMaskKey = BitMaskKey::new(27, 0x02, false);
    /// `USE_DEFAULT_BACKGROUND_COLOR` bitmask `0x04` at metadata index `27`.
    pub const USE_DEFAULT_BACKGROUND_COLOR: BitMaskKey = BitMaskKey::new(27, 0x04, false);
    /// `ALIGNMENT` byte-mask `0x18 >> 3` at metadata index `27`.
    pub const ALIGNMENT: ByteMaskKey = ByteMaskKey::new(27, 0x18, 3, 0);
    /// `ALIGN_LEFT` bitmask `0x08` at metadata index `27`.
    pub const ALIGN_LEFT: BitMaskKey = BitMaskKey::new(27, 0x08, false);
    /// `ALIGN_RIGHT` bitmask `0x10` at metadata index `27`.
    pub const ALIGN_RIGHT: BitMaskKey = BitMaskKey::new(27, 0x10, false);
}

/// Metadata keys for `ExperienceOrb` (extends `entity`, indices `8..9`).
pub mod experience_orb {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`9`).
    pub const END_INDEX: u8 = 9;
    /// `VALUE` at metadata index `8` (`Metadata::VarInt`).
    pub const VALUE: MetadataKey<i32> = MetadataKey::new(8, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `ThrownItemProjectile` (extends `entity`, indices `8..9`).
pub mod thrown_item_projectile {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`9`).
    pub const END_INDEX: u8 = 9;
    /// `ITEM` at metadata index `8` (`Metadata::ItemStack`).
    pub const ITEM: MetadataKey<ItemStack> =
        MetadataKey::new(8, conv::to_item_stack, conv::from_item_stack);
}

/// Metadata keys for `EyeOfEnder` (extends `entity`, indices `8..9`).
pub mod eye_of_ender {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`9`).
    pub const END_INDEX: u8 = 9;
    /// `ITEM` at metadata index `8` (`Metadata::ItemStack`).
    pub const ITEM: MetadataKey<ItemStack> =
        MetadataKey::new(8, conv::to_item_stack, conv::from_item_stack);
}

/// Metadata keys for `FallingBlock` (extends `entity`, indices `8..9`).
pub mod falling_block {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`9`).
    pub const END_INDEX: u8 = 9;
    /// `SPAWN_POSITION` at metadata index `8` (`Metadata::BlockPosition`).
    pub const SPAWN_POSITION: MetadataKey<BlockVec> =
        MetadataKey::new(8, conv::to_block_pos, conv::from_block_pos);
}

/// Metadata keys for `AreaEffectCloud` (extends `entity`, indices `8..11`).
pub mod area_effect_cloud {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`11`).
    pub const END_INDEX: u8 = 11;
    /// `RADIUS` at metadata index `8` (`Metadata::Float`).
    pub const RADIUS: MetadataKey<f32> = MetadataKey::new(8, conv::to_float, conv::from_float);
    /// `WAITING` at metadata index `9` (`Metadata::Boolean`).
    pub const WAITING: MetadataKey<bool> = MetadataKey::new(9, conv::to_bool, conv::from_bool);
    /// `PARTICLE` at metadata index `10` (`Metadata::Particle`).
    pub const PARTICLE: MetadataKey<ParticleEntry> =
        MetadataKey::new(10, conv::to_particle, conv::from_particle);
}

/// Metadata keys for `FishingHook` (extends `entity`, indices `8..10`).
pub mod fishing_hook {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`10`).
    pub const END_INDEX: u8 = 10;
    /// `HOOKED` at metadata index `8` (`Metadata::VarInt`).
    pub const HOOKED: MetadataKey<i32> = MetadataKey::new(8, conv::to_var_int, conv::from_var_int);
    /// `IS_CATCHABLE` at metadata index `9` (`Metadata::Boolean`).
    pub const IS_CATCHABLE: MetadataKey<bool> = MetadataKey::new(9, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `AbstractArrow` (extends `entity`, indices `8..11`).
pub mod abstract_arrow {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`11`).
    pub const END_INDEX: u8 = 11;
    /// `ARROW_FLAGS` at metadata index `8` (`Metadata::Byte`).
    pub const ARROW_FLAGS: MetadataKey<i8> = MetadataKey::new(8, conv::to_byte, conv::from_byte);
    /// `IS_CRITICAL` bitmask `0x01` at metadata index `8`.
    pub const IS_CRITICAL: BitMaskKey = BitMaskKey::new(8, 0x01, false);
    /// `IS_NO_CLIP` bitmask `0x02` at metadata index `8`.
    pub const IS_NO_CLIP: BitMaskKey = BitMaskKey::new(8, 0x02, false);
    /// `PIERCING_LEVEL` at metadata index `9` (`Metadata::Byte`).
    pub const PIERCING_LEVEL: MetadataKey<i8> = MetadataKey::new(9, conv::to_byte, conv::from_byte);
    /// `IN_GROUND` at metadata index `10` (`Metadata::Boolean`).
    pub const IN_GROUND: MetadataKey<bool> = MetadataKey::new(10, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Arrow` (extends `abstract_arrow`, indices `11..12`).
pub mod arrow {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`11`).
    pub const START_INDEX: u8 = 11;
    /// Exclusive end metadata index of this class (`12`).
    pub const END_INDEX: u8 = 12;
    /// `COLOR` at metadata index `11` (`Metadata::VarInt`).
    pub const COLOR: MetadataKey<i32> = MetadataKey::new(11, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `ThrownTrident` (extends `abstract_arrow`, indices `11..13`).
pub mod thrown_trident {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`11`).
    pub const START_INDEX: u8 = 11;
    /// Exclusive end metadata index of this class (`13`).
    pub const END_INDEX: u8 = 13;
    /// `LOYALTY_LEVEL` at metadata index `11` (`Metadata::Byte`).
    pub const LOYALTY_LEVEL: MetadataKey<i8> = MetadataKey::new(11, conv::to_byte, conv::from_byte);
    /// `HAS_ENCHANTMENT_GLINT` at metadata index `12` (`Metadata::Boolean`).
    pub const HAS_ENCHANTMENT_GLINT: MetadataKey<bool> =
        MetadataKey::new(12, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `AbstractVehicle` (extends `entity`, indices `8..11`).
pub mod abstract_vehicle {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`11`).
    pub const END_INDEX: u8 = 11;
    /// `SHAKING_POWER` at metadata index `8` (`Metadata::VarInt`).
    pub const SHAKING_POWER: MetadataKey<i32> =
        MetadataKey::new(8, conv::to_var_int, conv::from_var_int);
    /// `SHAKING_DIRECTION` at metadata index `9` (`Metadata::VarInt`).
    pub const SHAKING_DIRECTION: MetadataKey<i32> =
        MetadataKey::new(9, conv::to_var_int, conv::from_var_int);
    /// `SHAKING_MULTIPLIER` at metadata index `10` (`Metadata::Float`).
    pub const SHAKING_MULTIPLIER: MetadataKey<f32> =
        MetadataKey::new(10, conv::to_float, conv::from_float);
}

/// Metadata keys for `Boat` (extends `abstract_vehicle`, indices `11..14`).
pub mod boat {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`11`).
    pub const START_INDEX: u8 = 11;
    /// Exclusive end metadata index of this class (`14`).
    pub const END_INDEX: u8 = 14;
    /// `IS_LEFT_PADDLE_TURNING` at metadata index `11` (`Metadata::Boolean`).
    pub const IS_LEFT_PADDLE_TURNING: MetadataKey<bool> =
        MetadataKey::new(11, conv::to_bool, conv::from_bool);
    /// `IS_RIGHT_PADDLE_TURNING` at metadata index `12` (`Metadata::Boolean`).
    pub const IS_RIGHT_PADDLE_TURNING: MetadataKey<bool> =
        MetadataKey::new(12, conv::to_bool, conv::from_bool);
    /// `SPLASH_TIMER` at metadata index `13` (`Metadata::VarInt`).
    pub const SPLASH_TIMER: MetadataKey<i32> =
        MetadataKey::new(13, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `AbstractMinecart` (extends `abstract_vehicle`, indices `11..13`).
pub mod abstract_minecart {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`11`).
    pub const START_INDEX: u8 = 11;
    /// Exclusive end metadata index of this class (`13`).
    pub const END_INDEX: u8 = 13;
    /// `CUSTOM_BLOCK_STATE` at metadata index `11` (`Metadata::OptBlockState`).
    pub const CUSTOM_BLOCK_STATE: MetadataKey<Option<Block>> =
        MetadataKey::new(11, conv::to_opt_block_state, conv::from_opt_block_state);
    /// `CUSTOM_BLOCK_Y_POSITION` at metadata index `12` (`Metadata::VarInt`).
    pub const CUSTOM_BLOCK_Y_POSITION: MetadataKey<i32> =
        MetadataKey::new(12, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `MinecartFurnace` (extends `abstract_minecart`, indices `13..14`).
pub mod minecart_furnace {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`13`).
    pub const START_INDEX: u8 = 13;
    /// Exclusive end metadata index of this class (`14`).
    pub const END_INDEX: u8 = 14;
    /// `HAS_FUEL` at metadata index `13` (`Metadata::Boolean`).
    pub const HAS_FUEL: MetadataKey<bool> = MetadataKey::new(13, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `MinecartCommandBlock` (extends `abstract_minecart`, indices `13..15`).
pub mod minecart_command_block {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`13`).
    pub const START_INDEX: u8 = 13;
    /// Exclusive end metadata index of this class (`15`).
    pub const END_INDEX: u8 = 15;
    /// `COMMAND` at metadata index `13` (`Metadata::String`).
    pub const COMMAND: MetadataKey<String> =
        MetadataKey::new(13, conv::to_string, conv::from_string);
    /// `LAST_OUTPUT` at metadata index `14` (`Metadata::Component`).
    pub const LAST_OUTPUT: MetadataKey<Component> =
        MetadataKey::new(14, conv::to_component, conv::from_component);
}

/// Metadata keys for `EndCrystal` (extends `entity`, indices `8..10`).
pub mod end_crystal {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`10`).
    pub const END_INDEX: u8 = 10;
    /// `BEAM_TARGET` at metadata index `8` (`Metadata::OptBlockPosition`).
    pub const BEAM_TARGET: MetadataKey<Option<BlockVec>> =
        MetadataKey::new(8, conv::to_opt_block_pos, conv::from_opt_block_pos);
    /// `SHOW_BOTTOM` at metadata index `9` (`Metadata::Boolean`).
    pub const SHOW_BOTTOM: MetadataKey<bool> = MetadataKey::new(9, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `SmartFireball` (extends `entity`, indices `8..9`).
pub mod smart_fireball {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`9`).
    pub const END_INDEX: u8 = 9;
    /// `ITEM` at metadata index `8` (`Metadata::ItemStack`).
    pub const ITEM: MetadataKey<ItemStack> =
        MetadataKey::new(8, conv::to_item_stack, conv::from_item_stack);
}

/// Metadata keys for `Fireball` (extends `entity`, indices `8..9`).
pub mod fireball {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`9`).
    pub const END_INDEX: u8 = 9;
    /// `ITEM` at metadata index `8` (`Metadata::ItemStack`).
    pub const ITEM: MetadataKey<ItemStack> =
        MetadataKey::new(8, conv::to_item_stack, conv::from_item_stack);
}

/// Metadata keys for `WitherSkull` (extends `entity`, indices `8..9`).
pub mod wither_skull {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`9`).
    pub const END_INDEX: u8 = 9;
    /// `IS_INVULNERABLE` at metadata index `8` (`Metadata::Boolean`).
    pub const IS_INVULNERABLE: MetadataKey<bool> =
        MetadataKey::new(8, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `FireworkRocketEntity` (extends `entity`, indices `8..11`).
pub mod firework_rocket_entity {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`11`).
    pub const END_INDEX: u8 = 11;
    /// `ITEM` at metadata index `8` (`Metadata::ItemStack`).
    pub const ITEM: MetadataKey<ItemStack> =
        MetadataKey::new(8, conv::to_item_stack, conv::from_item_stack);
    /// `SHOOTER_ENTITY_ID` at metadata index `9` (`Metadata::OptVarInt`).
    pub const SHOOTER_ENTITY_ID: MetadataKey<Option<i32>> =
        MetadataKey::new(9, conv::to_opt_var_int, conv::from_opt_var_int);
    /// `IS_SHOT_AT_ANGLE` at metadata index `10` (`Metadata::Boolean`).
    pub const IS_SHOT_AT_ANGLE: MetadataKey<bool> =
        MetadataKey::new(10, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Hanging` (extends `entity`, indices `8..9`).
pub mod hanging {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`9`).
    pub const END_INDEX: u8 = 9;
    /// `DIRECTION` at metadata index `8` (`Metadata::Direction`).
    pub const DIRECTION: MetadataKey<Direction> =
        MetadataKey::new(8, conv::to_direction, conv::from_direction);
}

/// Metadata keys for `ItemFrame` (extends `hanging`, indices `9..11`).
pub mod item_frame {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`9`).
    pub const START_INDEX: u8 = 9;
    /// Exclusive end metadata index of this class (`11`).
    pub const END_INDEX: u8 = 11;
    /// `ITEM` at metadata index `9` (`Metadata::ItemStack`).
    pub const ITEM: MetadataKey<ItemStack> =
        MetadataKey::new(9, conv::to_item_stack, conv::from_item_stack);
    /// `ROTATION` at metadata index `10` (`Metadata::VarInt`).
    pub const ROTATION: MetadataKey<i32> =
        MetadataKey::new(10, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Painting` (extends `hanging`, indices `9..10`).
pub mod painting {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`9`).
    pub const START_INDEX: u8 = 9;
    /// Exclusive end metadata index of this class (`10`).
    pub const END_INDEX: u8 = 10;
    /// `VARIANT` at metadata index `9` (`Metadata::PaintingVariant`).
    pub const VARIANT: MetadataKey<u32> =
        MetadataKey::new(9, conv::to_painting_variant, conv::from_painting_variant);
}

/// Metadata keys for `ItemEntity` (extends `entity`, indices `8..9`).
pub mod item_entity {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`9`).
    pub const END_INDEX: u8 = 9;
    /// `ITEM` at metadata index `8` (`Metadata::ItemStack`).
    pub const ITEM: MetadataKey<ItemStack> =
        MetadataKey::new(8, conv::to_item_stack, conv::from_item_stack);
}

/// Metadata keys for `LivingEntity` (extends `entity`, indices `8..15`).
pub mod living_entity {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`15`).
    pub const END_INDEX: u8 = 15;
    /// `LIVING_ENTITY_FLAGS` at metadata index `8` (`Metadata::Byte`).
    pub const LIVING_ENTITY_FLAGS: MetadataKey<i8> =
        MetadataKey::new(8, conv::to_byte, conv::from_byte);
    /// `IS_HAND_ACTIVE` bitmask `0x01` at metadata index `8`.
    pub const IS_HAND_ACTIVE: BitMaskKey = BitMaskKey::new(8, 0x01, false);
    /// `ACTIVE_HAND` bitmask `0x02` at metadata index `8`.
    pub const ACTIVE_HAND: BitMaskKey = BitMaskKey::new(8, 0x02, false);
    /// `IS_RIPTIDE_SPIN_ATTACK` bitmask `0x04` at metadata index `8`.
    pub const IS_RIPTIDE_SPIN_ATTACK: BitMaskKey = BitMaskKey::new(8, 0x04, false);
    /// `HEALTH` at metadata index `9` (`Metadata::Float`).
    pub const HEALTH: MetadataKey<f32> = MetadataKey::new(9, conv::to_float, conv::from_float);
    /// `POTION_EFFECT_PARTICLES` at metadata index `10` (`Metadata::ParticleList`).
    pub const POTION_EFFECT_PARTICLES: MetadataKey<Vec<ParticleEntry>> =
        MetadataKey::new(10, conv::to_particle_list, conv::from_particle_list);
    /// `IS_POTION_EFFECT_AMBIANT` at metadata index `11` (`Metadata::Boolean`).
    pub const IS_POTION_EFFECT_AMBIANT: MetadataKey<bool> =
        MetadataKey::new(11, conv::to_bool, conv::from_bool);
    /// `NUMBER_OF_ARROWS` at metadata index `12` (`Metadata::VarInt`).
    pub const NUMBER_OF_ARROWS: MetadataKey<i32> =
        MetadataKey::new(12, conv::to_var_int, conv::from_var_int);
    /// `NUMBER_OF_BEE_STINGERS` at metadata index `13` (`Metadata::VarInt`).
    pub const NUMBER_OF_BEE_STINGERS: MetadataKey<i32> =
        MetadataKey::new(13, conv::to_var_int, conv::from_var_int);
    /// `LOCATION_OF_BED` at metadata index `14` (`Metadata::OptBlockPosition`).
    pub const LOCATION_OF_BED: MetadataKey<Option<BlockVec>> =
        MetadataKey::new(14, conv::to_opt_block_pos, conv::from_opt_block_pos);
}

/// Metadata keys for `Avatar` (extends `living_entity`, indices `15..17`).
pub mod avatar {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`15`).
    pub const START_INDEX: u8 = 15;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `MAIN_HAND` at metadata index `15` (`Metadata::MainHand`).
    pub const MAIN_HAND: MetadataKey<MainHand> =
        MetadataKey::new(15, conv::to_main_hand, conv::from_main_hand);
    /// `DISPLAYED_MODEL_PARTS_FLAGS` at metadata index `16` (`Metadata::Byte`).
    pub const DISPLAYED_MODEL_PARTS_FLAGS: MetadataKey<i8> =
        MetadataKey::new(16, conv::to_byte, conv::from_byte);
    /// `IS_CAPE_ENABLED` bitmask `0x01` at metadata index `16`.
    pub const IS_CAPE_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x01, false);
    /// `IS_JACKET_ENABLED` bitmask `0x02` at metadata index `16`.
    pub const IS_JACKET_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x02, false);
    /// `IS_LEFT_SLEEVE_ENABLED` bitmask `0x04` at metadata index `16`.
    pub const IS_LEFT_SLEEVE_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x04, false);
    /// `IS_RIGHT_SLEEVE_ENABLED` bitmask `0x08` at metadata index `16`.
    pub const IS_RIGHT_SLEEVE_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x08, false);
    /// `IS_LEFT_PANTS_LEG_ENABLED` bitmask `0x10` at metadata index `16`.
    pub const IS_LEFT_PANTS_LEG_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x10, false);
    /// `IS_RIGHT_PANTS_LEG_ENABLED` bitmask `0x20` at metadata index `16`.
    pub const IS_RIGHT_PANTS_LEG_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x20, false);
    /// `IS_HAT_ENABLED` bitmask `0x40` at metadata index `16`.
    pub const IS_HAT_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x40, false);
}

/// Metadata keys for `Player` (extends `avatar`, indices `17..21`).
pub mod player {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`17`).
    pub const START_INDEX: u8 = 17;
    /// Exclusive end metadata index of this class (`21`).
    pub const END_INDEX: u8 = 21;
    /// `ADDITIONAL_HEARTS` at metadata index `17` (`Metadata::Float`).
    pub const ADDITIONAL_HEARTS: MetadataKey<f32> =
        MetadataKey::new(17, conv::to_float, conv::from_float);
    /// `SCORE` at metadata index `18` (`Metadata::VarInt`).
    pub const SCORE: MetadataKey<i32> = MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
    /// `LEFT_SHOULDER_ENTITY_DATA` at metadata index `19` (`Metadata::OptVarInt`).
    pub const LEFT_SHOULDER_ENTITY_DATA: MetadataKey<Option<i32>> =
        MetadataKey::new(19, conv::to_opt_var_int, conv::from_opt_var_int);
    /// `RIGHT_SHOULDER_ENTITY_DATA` at metadata index `20` (`Metadata::OptVarInt`).
    pub const RIGHT_SHOULDER_ENTITY_DATA: MetadataKey<Option<i32>> =
        MetadataKey::new(20, conv::to_opt_var_int, conv::from_opt_var_int);
}

/// Metadata keys for `Mannequin` (extends `avatar`, indices `17..20`).
pub mod mannequin {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`17`).
    pub const START_INDEX: u8 = 17;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `DISPLAYED_MODEL_PARTS_FLAGS` at metadata index `16` (`Metadata::Byte`).
    pub const DISPLAYED_MODEL_PARTS_FLAGS: MetadataKey<i8> =
        MetadataKey::new(16, conv::to_byte, conv::from_byte);
    /// `IS_CAPE_ENABLED` bitmask `0x01` at metadata index `16`.
    pub const IS_CAPE_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x01, true);
    /// `IS_JACKET_ENABLED` bitmask `0x02` at metadata index `16`.
    pub const IS_JACKET_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x02, true);
    /// `IS_LEFT_SLEEVE_ENABLED` bitmask `0x04` at metadata index `16`.
    pub const IS_LEFT_SLEEVE_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x04, true);
    /// `IS_RIGHT_SLEEVE_ENABLED` bitmask `0x08` at metadata index `16`.
    pub const IS_RIGHT_SLEEVE_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x08, true);
    /// `IS_LEFT_PANTS_LEG_ENABLED` bitmask `0x10` at metadata index `16`.
    pub const IS_LEFT_PANTS_LEG_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x10, true);
    /// `IS_RIGHT_PANTS_LEG_ENABLED` bitmask `0x20` at metadata index `16`.
    pub const IS_RIGHT_PANTS_LEG_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x20, true);
    /// `IS_HAT_ENABLED` bitmask `0x40` at metadata index `16`.
    pub const IS_HAT_ENABLED: BitMaskKey = BitMaskKey::new(16, 0x40, true);
    /// `PROFILE` at metadata index `17` (`Metadata::ResolvableProfile`).
    pub const PROFILE: MetadataKey<ResolvableProfile> =
        MetadataKey::new(17, conv::to_profile, conv::from_profile);
    /// `IMMOVABLE` at metadata index `18` (`Metadata::Boolean`).
    pub const IMMOVABLE: MetadataKey<bool> = MetadataKey::new(18, conv::to_bool, conv::from_bool);
    /// `DESCRIPTION` at metadata index `19` (`Metadata::OptComponent`).
    pub const DESCRIPTION: MetadataKey<Option<Component>> =
        MetadataKey::new(19, conv::to_opt_component, conv::from_opt_component);
}

/// Metadata keys for `ArmorStand` (extends `living_entity`, indices `15..22`).
pub mod armor_stand {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`15`).
    pub const START_INDEX: u8 = 15;
    /// Exclusive end metadata index of this class (`22`).
    pub const END_INDEX: u8 = 22;
    /// `ARMOR_STAND_FLAGS` at metadata index `15` (`Metadata::Byte`).
    pub const ARMOR_STAND_FLAGS: MetadataKey<i8> =
        MetadataKey::new(15, conv::to_byte, conv::from_byte);
    /// `IS_SMALL` bitmask `0x01` at metadata index `15`.
    pub const IS_SMALL: BitMaskKey = BitMaskKey::new(15, 0x01, false);
    /// `HAS_ARMS` bitmask `0x04` at metadata index `15`.
    pub const HAS_ARMS: BitMaskKey = BitMaskKey::new(15, 0x04, false);
    /// `HAS_NO_BASE_PLATE` bitmask `0x08` at metadata index `15`.
    pub const HAS_NO_BASE_PLATE: BitMaskKey = BitMaskKey::new(15, 0x08, false);
    /// `IS_MARKER` bitmask `0x10` at metadata index `15`.
    pub const IS_MARKER: BitMaskKey = BitMaskKey::new(15, 0x10, false);
    /// `HEAD_ROTATION` at metadata index `16` (`Metadata::Rotation`).
    pub const HEAD_ROTATION: MetadataKey<[f32; 3]> =
        MetadataKey::new(16, conv::to_rotation, conv::from_rotation);
    /// `BODY_ROTATION` at metadata index `17` (`Metadata::Rotation`).
    pub const BODY_ROTATION: MetadataKey<[f32; 3]> =
        MetadataKey::new(17, conv::to_rotation, conv::from_rotation);
    /// `LEFT_ARM_ROTATION` at metadata index `18` (`Metadata::Rotation`).
    pub const LEFT_ARM_ROTATION: MetadataKey<[f32; 3]> =
        MetadataKey::new(18, conv::to_rotation, conv::from_rotation);
    /// `RIGHT_ARM_ROTATION` at metadata index `19` (`Metadata::Rotation`).
    pub const RIGHT_ARM_ROTATION: MetadataKey<[f32; 3]> =
        MetadataKey::new(19, conv::to_rotation, conv::from_rotation);
    /// `LEFT_LEG_ROTATION` at metadata index `20` (`Metadata::Rotation`).
    pub const LEFT_LEG_ROTATION: MetadataKey<[f32; 3]> =
        MetadataKey::new(20, conv::to_rotation, conv::from_rotation);
    /// `RIGHT_LEG_ROTATION` at metadata index `21` (`Metadata::Rotation`).
    pub const RIGHT_LEG_ROTATION: MetadataKey<[f32; 3]> =
        MetadataKey::new(21, conv::to_rotation, conv::from_rotation);
}

/// Metadata keys for `Mob` (extends `living_entity`, indices `15..16`).
pub mod mob {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`15`).
    pub const START_INDEX: u8 = 15;
    /// Exclusive end metadata index of this class (`16`).
    pub const END_INDEX: u8 = 16;
    /// `MOB_FLAGS` at metadata index `15` (`Metadata::Byte`).
    pub const MOB_FLAGS: MetadataKey<i8> = MetadataKey::new(15, conv::to_byte, conv::from_byte);
    /// `NO_AI` bitmask `0x01` at metadata index `15`.
    pub const NO_AI: BitMaskKey = BitMaskKey::new(15, 0x01, false);
    /// `IS_LEFT_HANDED` bitmask `0x02` at metadata index `15`.
    pub const IS_LEFT_HANDED: BitMaskKey = BitMaskKey::new(15, 0x02, false);
    /// `IS_AGGRESSIVE` bitmask `0x04` at metadata index `15`.
    pub const IS_AGGRESSIVE: BitMaskKey = BitMaskKey::new(15, 0x04, false);
}

/// Metadata keys for `Allay` (extends `mob`, indices `16..18`).
pub mod allay {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`18`).
    pub const END_INDEX: u8 = 18;
    /// `IS_DANCING` at metadata index `16` (`Metadata::Boolean`).
    pub const IS_DANCING: MetadataKey<bool> = MetadataKey::new(16, conv::to_bool, conv::from_bool);
    /// `CAN_DUPLICATE` at metadata index `17` (`Metadata::Boolean`).
    pub const CAN_DUPLICATE: MetadataKey<bool> =
        MetadataKey::new(17, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Armadillo` (extends `ageable_mob`, indices `18..19`).
pub mod armadillo {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `STATE` at metadata index `18` (`Metadata::ArmadilloState`).
    pub const STATE: MetadataKey<ArmadilloState> =
        MetadataKey::new(18, conv::to_armadillo_state, conv::from_armadillo_state);
}

/// Metadata keys for `Bat` (extends `mob`, indices `16..17`).
pub mod bat {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `BAT_FLAGS` at metadata index `16` (`Metadata::Byte`).
    pub const BAT_FLAGS: MetadataKey<i8> = MetadataKey::new(16, conv::to_byte, conv::from_byte);
    /// `IS_HANGING` bitmask `0x01` at metadata index `16`.
    pub const IS_HANGING: BitMaskKey = BitMaskKey::new(16, 0x01, false);
}

/// Metadata keys for `Dolphin` (extends `ageable_mob`, indices `18..20`).
pub mod dolphin {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `HAS_FISH` at metadata index `18` (`Metadata::Boolean`).
    pub const HAS_FISH: MetadataKey<bool> = MetadataKey::new(18, conv::to_bool, conv::from_bool);
    /// `MOISTURE_LEVEL` at metadata index `19` (`Metadata::VarInt`).
    pub const MOISTURE_LEVEL: MetadataKey<i32> =
        MetadataKey::new(19, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `AbstractFish` (extends `mob`, indices `16..17`).
pub mod abstract_fish {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `FROM_BUCKET` at metadata index `16` (`Metadata::Boolean`).
    pub const FROM_BUCKET: MetadataKey<bool> = MetadataKey::new(16, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `PufferFish` (extends `abstract_fish`, indices `17..18`).
pub mod puffer_fish {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`17`).
    pub const START_INDEX: u8 = 17;
    /// Exclusive end metadata index of this class (`18`).
    pub const END_INDEX: u8 = 18;
    /// `PUFF_STATE` at metadata index `17` (`Metadata::VarInt`).
    pub const PUFF_STATE: MetadataKey<i32> =
        MetadataKey::new(17, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Salmon` (extends `abstract_fish`, indices `17..18`).
pub mod salmon {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`17`).
    pub const START_INDEX: u8 = 17;
    /// Exclusive end metadata index of this class (`18`).
    pub const END_INDEX: u8 = 18;
    /// `SIZE` at metadata index `17` (`Metadata::VarInt`).
    pub const SIZE: MetadataKey<i32> = MetadataKey::new(17, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `TropicalFish` (extends `abstract_fish`, indices `17..18`).
pub mod tropical_fish {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`17`).
    pub const START_INDEX: u8 = 17;
    /// Exclusive end metadata index of this class (`18`).
    pub const END_INDEX: u8 = 18;
    /// `VARIANT` at metadata index `17` (`Metadata::VarInt`).
    pub const VARIANT: MetadataKey<i32> =
        MetadataKey::new(17, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Tadpole` (extends `abstract_fish`, indices `17..18`).
pub mod tadpole {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`17`).
    pub const START_INDEX: u8 = 17;
    /// Exclusive end metadata index of this class (`18`).
    pub const END_INDEX: u8 = 18;
    /// `AGE_LOCKED` at metadata index `17` (`Metadata::Boolean`).
    pub const AGE_LOCKED: MetadataKey<bool> = MetadataKey::new(17, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `AgeableMob` (extends `mob`, indices `16..18`).
pub mod ageable_mob {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`18`).
    pub const END_INDEX: u8 = 18;
    /// `IS_BABY` at metadata index `16` (`Metadata::Boolean`).
    pub const IS_BABY: MetadataKey<bool> = MetadataKey::new(16, conv::to_bool, conv::from_bool);
    /// `AGE_LOCKED` at metadata index `17` (`Metadata::Boolean`).
    pub const AGE_LOCKED: MetadataKey<bool> = MetadataKey::new(17, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `AbstractCubeMob` (extends `ageable_mob`, indices `18..19`).
pub mod abstract_cube_mob {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `SIZE` at metadata index `18` (`Metadata::VarInt`).
    pub const SIZE: MetadataKey<i32> = MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `SulfurCube` (extends `abstract_cube_mob`, indices `19..21`).
pub mod sulfur_cube {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`19`).
    pub const START_INDEX: u8 = 19;
    /// Exclusive end metadata index of this class (`21`).
    pub const END_INDEX: u8 = 21;
    /// `MAX_FUSE` at metadata index `19` (`Metadata::VarInt`).
    pub const MAX_FUSE: MetadataKey<i32> =
        MetadataKey::new(19, conv::to_var_int, conv::from_var_int);
    /// `FROM_BUCKET` at metadata index `20` (`Metadata::Boolean`).
    pub const FROM_BUCKET: MetadataKey<bool> = MetadataKey::new(20, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Sniffer` (extends `ageable_mob`, indices `18..20`).
pub mod sniffer {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `STATE` at metadata index `18` (`Metadata::SnifferState`).
    pub const STATE: MetadataKey<SnifferState> =
        MetadataKey::new(18, conv::to_sniffer_state, conv::from_sniffer_state);
    /// `DROP_SEED_AT_TICK` at metadata index `19` (`Metadata::VarInt`).
    pub const DROP_SEED_AT_TICK: MetadataKey<i32> =
        MetadataKey::new(19, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `AbstractHorse` (extends `ageable_mob`, indices `18..19`).
pub mod abstract_horse {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `ABSTRACT_HORSE_FLAGS` at metadata index `18` (`Metadata::Byte`).
    pub const ABSTRACT_HORSE_FLAGS: MetadataKey<i8> =
        MetadataKey::new(18, conv::to_byte, conv::from_byte);
    /// `IS_TAME` bitmask `0x02` at metadata index `18`.
    pub const IS_TAME: BitMaskKey = BitMaskKey::new(18, 0x02, false);
    /// `HAS_BRED` bitmask `0x08` at metadata index `18`.
    pub const HAS_BRED: BitMaskKey = BitMaskKey::new(18, 0x08, false);
    /// `IS_EATING` bitmask `0x10` at metadata index `18`.
    pub const IS_EATING: BitMaskKey = BitMaskKey::new(18, 0x10, false);
    /// `IS_REARING` bitmask `0x20` at metadata index `18`.
    pub const IS_REARING: BitMaskKey = BitMaskKey::new(18, 0x20, false);
    /// `IS_MOUTH_OPEN` bitmask `0x40` at metadata index `18`.
    pub const IS_MOUTH_OPEN: BitMaskKey = BitMaskKey::new(18, 0x40, false);
}

/// Metadata keys for `Horse` (extends `abstract_horse`, indices `19..20`).
pub mod horse {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`19`).
    pub const START_INDEX: u8 = 19;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `VARIANT` at metadata index `19` (`Metadata::VarInt`).
    pub const VARIANT: MetadataKey<i32> =
        MetadataKey::new(19, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Camel` (extends `abstract_horse`, indices `19..21`).
pub mod camel {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`19`).
    pub const START_INDEX: u8 = 19;
    /// Exclusive end metadata index of this class (`21`).
    pub const END_INDEX: u8 = 21;
    /// `DASHING` at metadata index `19` (`Metadata::Boolean`).
    pub const DASHING: MetadataKey<bool> = MetadataKey::new(19, conv::to_bool, conv::from_bool);
    /// `LAST_POSE_CHANGE_TICK` at metadata index `20` (`Metadata::VarLong`).
    pub const LAST_POSE_CHANGE_TICK: MetadataKey<i64> =
        MetadataKey::new(20, conv::to_var_long, conv::from_var_long);
}

/// Metadata keys for `ChestedHorse` (extends `abstract_horse`, indices `19..20`).
pub mod chested_horse {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`19`).
    pub const START_INDEX: u8 = 19;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `HAS_CHEST` at metadata index `19` (`Metadata::Boolean`).
    pub const HAS_CHEST: MetadataKey<bool> = MetadataKey::new(19, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Llama` (extends `chested_horse`, indices `20..22`).
pub mod llama {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`20`).
    pub const START_INDEX: u8 = 20;
    /// Exclusive end metadata index of this class (`22`).
    pub const END_INDEX: u8 = 22;
    /// `STRENGTH` at metadata index `20` (`Metadata::VarInt`).
    pub const STRENGTH: MetadataKey<i32> =
        MetadataKey::new(20, conv::to_var_int, conv::from_var_int);
    /// `VARIANT` at metadata index `21` (`Metadata::VarInt`).
    pub const VARIANT: MetadataKey<i32> =
        MetadataKey::new(21, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Axolotl` (extends `ageable_mob`, indices `18..21`).
pub mod axolotl {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`21`).
    pub const END_INDEX: u8 = 21;
    /// `VARIANT` at metadata index `18` (`Metadata::VarInt`).
    pub const VARIANT: MetadataKey<i32> =
        MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
    /// `IS_PLAYING_DEAD` at metadata index `19` (`Metadata::Boolean`).
    pub const IS_PLAYING_DEAD: MetadataKey<bool> =
        MetadataKey::new(19, conv::to_bool, conv::from_bool);
    /// `IS_FROM_BUCKET` at metadata index `20` (`Metadata::Boolean`).
    pub const IS_FROM_BUCKET: MetadataKey<bool> =
        MetadataKey::new(20, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Bee` (extends `ageable_mob`, indices `18..20`).
pub mod bee {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `BEE_FLAGS` at metadata index `18` (`Metadata::Byte`).
    pub const BEE_FLAGS: MetadataKey<i8> = MetadataKey::new(18, conv::to_byte, conv::from_byte);
    /// `IS_ROLLING` bitmask `0x02` at metadata index `18`.
    pub const IS_ROLLING: BitMaskKey = BitMaskKey::new(18, 0x02, false);
    /// `HAS_STUNG` bitmask `0x04` at metadata index `18`.
    pub const HAS_STUNG: BitMaskKey = BitMaskKey::new(18, 0x04, false);
    /// `HAS_NECTAR` bitmask `0x08` at metadata index `18`.
    pub const HAS_NECTAR: BitMaskKey = BitMaskKey::new(18, 0x08, false);
    /// `ANGER_END_TIME` at metadata index `19` (`Metadata::VarLong`).
    pub const ANGER_END_TIME: MetadataKey<i64> =
        MetadataKey::new(19, conv::to_var_long, conv::from_var_long);
}

/// Metadata keys for `GlowSquid` (extends `ageable_mob`, indices `18..19`).
pub mod glow_squid {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `DARK_TICKS_REMAINING` at metadata index `18` (`Metadata::VarInt`).
    pub const DARK_TICKS_REMAINING: MetadataKey<i32> =
        MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Fox` (extends `ageable_mob`, indices `18..22`).
pub mod fox {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`22`).
    pub const END_INDEX: u8 = 22;
    /// `VARIANT` at metadata index `18` (`Metadata::VarInt`).
    pub const VARIANT: MetadataKey<i32> =
        MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
    /// `FOX_FLAGS` at metadata index `19` (`Metadata::Byte`).
    pub const FOX_FLAGS: MetadataKey<i8> = MetadataKey::new(19, conv::to_byte, conv::from_byte);
    /// `IS_SITTING` bitmask `0x01` at metadata index `19`.
    pub const IS_SITTING: BitMaskKey = BitMaskKey::new(19, 0x01, false);
    /// `IS_CROUCHING` bitmask `0x04` at metadata index `19`.
    pub const IS_CROUCHING: BitMaskKey = BitMaskKey::new(19, 0x04, false);
    /// `IS_INTERESTED` bitmask `0x08` at metadata index `19`.
    pub const IS_INTERESTED: BitMaskKey = BitMaskKey::new(19, 0x08, false);
    /// `IS_POUNCING` bitmask `0x10` at metadata index `19`.
    pub const IS_POUNCING: BitMaskKey = BitMaskKey::new(19, 0x10, false);
    /// `IS_SLEEPING` bitmask `0x20` at metadata index `19`.
    pub const IS_SLEEPING: BitMaskKey = BitMaskKey::new(19, 0x20, false);
    /// `IS_FACEPLANTED` bitmask `0x40` at metadata index `19`.
    pub const IS_FACEPLANTED: BitMaskKey = BitMaskKey::new(19, 0x40, false);
    /// `IS_DEFENDING` bitmask `0x80` at metadata index `19`.
    pub const IS_DEFENDING: BitMaskKey = BitMaskKey::new(19, 0x80, false);
    /// `FIRST_UUID` at metadata index `20` (`Metadata::OptUUID`).
    pub const FIRST_UUID: MetadataKey<Option<Uuid>> =
        MetadataKey::new(20, conv::to_opt_uuid, conv::from_opt_uuid);
    /// `SECOND_UUID` at metadata index `21` (`Metadata::OptUUID`).
    pub const SECOND_UUID: MetadataKey<Option<Uuid>> =
        MetadataKey::new(21, conv::to_opt_uuid, conv::from_opt_uuid);
}

/// Metadata keys for `Frog` (extends `ageable_mob`, indices `18..20`).
pub mod frog {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `VARIANT` at metadata index `18` (`Metadata::FrogVariant`).
    pub const VARIANT: MetadataKey<u32> =
        MetadataKey::new(18, conv::to_frog_variant, conv::from_frog_variant);
    /// `TONGUE_TARGET` at metadata index `19` (`Metadata::OptVarInt`).
    pub const TONGUE_TARGET: MetadataKey<Option<i32>> =
        MetadataKey::new(19, conv::to_opt_var_int, conv::from_opt_var_int);
}

/// Metadata keys for `Ocelot` (extends `ageable_mob`, indices `18..19`).
pub mod ocelot {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `IS_TRUSTING` at metadata index `18` (`Metadata::Boolean`).
    pub const IS_TRUSTING: MetadataKey<bool> = MetadataKey::new(18, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Panda` (extends `ageable_mob`, indices `18..24`).
pub mod panda {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`24`).
    pub const END_INDEX: u8 = 24;
    /// `BREED_TIMER` at metadata index `18` (`Metadata::VarInt`).
    pub const BREED_TIMER: MetadataKey<i32> =
        MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
    /// `SNEEZE_TIMER` at metadata index `19` (`Metadata::VarInt`).
    pub const SNEEZE_TIMER: MetadataKey<i32> =
        MetadataKey::new(19, conv::to_var_int, conv::from_var_int);
    /// `EAT_TIMER` at metadata index `20` (`Metadata::VarInt`).
    pub const EAT_TIMER: MetadataKey<i32> =
        MetadataKey::new(20, conv::to_var_int, conv::from_var_int);
    /// `MAIN_GENE` at metadata index `21` (`Metadata::Byte`).
    pub const MAIN_GENE: MetadataKey<i8> = MetadataKey::new(21, conv::to_byte, conv::from_byte);
    /// `HIDDEN_GENE` at metadata index `22` (`Metadata::Byte`).
    pub const HIDDEN_GENE: MetadataKey<i8> = MetadataKey::new(22, conv::to_byte, conv::from_byte);
    /// `PANDA_FLAGS` at metadata index `23` (`Metadata::Byte`).
    pub const PANDA_FLAGS: MetadataKey<i8> = MetadataKey::new(23, conv::to_byte, conv::from_byte);
    /// `IS_SNEEZING` bitmask `0x02` at metadata index `23`.
    pub const IS_SNEEZING: BitMaskKey = BitMaskKey::new(23, 0x02, false);
    /// `IS_ROLLING` bitmask `0x04` at metadata index `23`.
    pub const IS_ROLLING: BitMaskKey = BitMaskKey::new(23, 0x04, false);
    /// `IS_SITTING` bitmask `0x08` at metadata index `23`.
    pub const IS_SITTING: BitMaskKey = BitMaskKey::new(23, 0x08, false);
    /// `IS_ON_BACK` bitmask `0x10` at metadata index `23`.
    pub const IS_ON_BACK: BitMaskKey = BitMaskKey::new(23, 0x10, false);
}

/// Metadata keys for `Chicken` (extends `ageable_mob`, indices `18..20`).
pub mod chicken {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `VARIANT` at metadata index `18` (`Metadata::ChickenVariant`).
    pub const VARIANT: MetadataKey<u32> =
        MetadataKey::new(18, conv::to_chicken_variant, conv::from_chicken_variant);
    /// `SOUND_VARIANT` at metadata index `19` (`Metadata::ChickenSoundVariant`).
    pub const SOUND_VARIANT: MetadataKey<u32> = MetadataKey::new(
        19,
        conv::to_chicken_sound_variant,
        conv::from_chicken_sound_variant,
    );
}

/// Metadata keys for `Cow` (extends `ageable_mob`, indices `18..20`).
pub mod cow {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `VARIANT` at metadata index `18` (`Metadata::CowVariant`).
    pub const VARIANT: MetadataKey<u32> =
        MetadataKey::new(18, conv::to_cow_variant, conv::from_cow_variant);
    /// `SOUND_VARIANT` at metadata index `19` (`Metadata::CowSoundVariant`).
    pub const SOUND_VARIANT: MetadataKey<u32> =
        MetadataKey::new(19, conv::to_cow_sound_variant, conv::from_cow_sound_variant);
}

/// Metadata keys for `Pig` (extends `ageable_mob`, indices `18..21`).
pub mod pig {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`21`).
    pub const END_INDEX: u8 = 21;
    /// `BOOST_TIME` at metadata index `18` (`Metadata::VarInt`).
    pub const BOOST_TIME: MetadataKey<i32> =
        MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
    /// `VARIANT` at metadata index `19` (`Metadata::PigVariant`).
    pub const VARIANT: MetadataKey<u32> =
        MetadataKey::new(19, conv::to_pig_variant, conv::from_pig_variant);
    /// `SOUND_VARIANT` at metadata index `20` (`Metadata::PigSoundVariant`).
    pub const SOUND_VARIANT: MetadataKey<u32> =
        MetadataKey::new(20, conv::to_pig_sound_variant, conv::from_pig_sound_variant);
}

/// Metadata keys for `Rabbit` (extends `ageable_mob`, indices `18..19`).
pub mod rabbit {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `TYPE` at metadata index `18` (`Metadata::VarInt`).
    pub const TYPE: MetadataKey<i32> = MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Turtle` (extends `ageable_mob`, indices `18..20`).
pub mod turtle {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `HAS_EGG` at metadata index `18` (`Metadata::Boolean`).
    pub const HAS_EGG: MetadataKey<bool> = MetadataKey::new(18, conv::to_bool, conv::from_bool);
    /// `IS_LAYING_EGG` at metadata index `19` (`Metadata::Boolean`).
    pub const IS_LAYING_EGG: MetadataKey<bool> =
        MetadataKey::new(19, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `PolarBear` (extends `ageable_mob`, indices `18..19`).
pub mod polar_bear {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `IS_STANDING_UP` at metadata index `18` (`Metadata::Boolean`).
    pub const IS_STANDING_UP: MetadataKey<bool> =
        MetadataKey::new(18, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Mooshroom` (extends `ageable_mob`, indices `18..19`).
pub mod mooshroom {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `VARIANT` at metadata index `18` (`Metadata::VarInt`).
    pub const VARIANT: MetadataKey<i32> =
        MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Hoglin` (extends `ageable_mob`, indices `18..19`).
pub mod hoglin {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `IMMUNE_ZOMBIFICATION` at metadata index `18` (`Metadata::Boolean`).
    pub const IMMUNE_ZOMBIFICATION: MetadataKey<bool> =
        MetadataKey::new(18, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Sheep` (extends `ageable_mob`, indices `18..19`).
pub mod sheep {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `SHEEP_FLAGS` at metadata index `18` (`Metadata::Byte`).
    pub const SHEEP_FLAGS: MetadataKey<i8> = MetadataKey::new(18, conv::to_byte, conv::from_byte);
    /// `COLOR_ID` byte-mask `0x0F >> 0` at metadata index `18`.
    pub const COLOR_ID: ByteMaskKey = ByteMaskKey::new(18, 0x0F, 0, 0);
    /// `IS_SHEARED` bitmask `0x10` at metadata index `18`.
    pub const IS_SHEARED: BitMaskKey = BitMaskKey::new(18, 0x10, false);
}

/// Metadata keys for `Strider` (extends `ageable_mob`, indices `18..20`).
pub mod strider {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `FUNGUS_BOOST` at metadata index `18` (`Metadata::VarInt`).
    pub const FUNGUS_BOOST: MetadataKey<i32> =
        MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
    /// `IS_SHAKING` at metadata index `19` (`Metadata::Boolean`).
    pub const IS_SHAKING: MetadataKey<bool> = MetadataKey::new(19, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Goat` (extends `ageable_mob`, indices `18..21`).
pub mod goat {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`21`).
    pub const END_INDEX: u8 = 21;
    /// `IS_SCREAMING_GOAT` at metadata index `18` (`Metadata::Boolean`).
    pub const IS_SCREAMING_GOAT: MetadataKey<bool> =
        MetadataKey::new(18, conv::to_bool, conv::from_bool);
    /// `HAS_LEFT_HORN` at metadata index `19` (`Metadata::Boolean`).
    pub const HAS_LEFT_HORN: MetadataKey<bool> =
        MetadataKey::new(19, conv::to_bool, conv::from_bool);
    /// `HAS_RIGHT_HORN` at metadata index `20` (`Metadata::Boolean`).
    pub const HAS_RIGHT_HORN: MetadataKey<bool> =
        MetadataKey::new(20, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `TameableAnimal` (extends `ageable_mob`, indices `18..20`).
pub mod tameable_animal {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `TAMEABLE_ANIMAL_FLAGS` at metadata index `18` (`Metadata::Byte`).
    pub const TAMEABLE_ANIMAL_FLAGS: MetadataKey<i8> =
        MetadataKey::new(18, conv::to_byte, conv::from_byte);
    /// `IS_SITTING` bitmask `0x01` at metadata index `18`.
    pub const IS_SITTING: BitMaskKey = BitMaskKey::new(18, 0x01, false);
    /// `IS_TAMED` bitmask `0x04` at metadata index `18`.
    pub const IS_TAMED: BitMaskKey = BitMaskKey::new(18, 0x04, false);
    /// `OWNER` at metadata index `19` (`Metadata::OptUUID`).
    pub const OWNER: MetadataKey<Option<Uuid>> =
        MetadataKey::new(19, conv::to_opt_uuid, conv::from_opt_uuid);
}

/// Metadata keys for `Cat` (extends `tameable_animal`, indices `20..25`).
pub mod cat {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`20`).
    pub const START_INDEX: u8 = 20;
    /// Exclusive end metadata index of this class (`25`).
    pub const END_INDEX: u8 = 25;
    /// `VARIANT` at metadata index `20` (`Metadata::CatVariant`).
    pub const VARIANT: MetadataKey<u32> =
        MetadataKey::new(20, conv::to_cat_variant, conv::from_cat_variant);
    /// `IS_LYING` at metadata index `21` (`Metadata::Boolean`).
    pub const IS_LYING: MetadataKey<bool> = MetadataKey::new(21, conv::to_bool, conv::from_bool);
    /// `IS_RELAXED` at metadata index `22` (`Metadata::Boolean`).
    pub const IS_RELAXED: MetadataKey<bool> = MetadataKey::new(22, conv::to_bool, conv::from_bool);
    /// `COLLAR_COLOR` at metadata index `23` (`Metadata::VarInt`).
    pub const COLLAR_COLOR: MetadataKey<i32> =
        MetadataKey::new(23, conv::to_var_int, conv::from_var_int);
    /// `SOUND_VARIANT` at metadata index `24` (`Metadata::CatSoundVariant`).
    pub const SOUND_VARIANT: MetadataKey<u32> =
        MetadataKey::new(24, conv::to_cat_sound_variant, conv::from_cat_sound_variant);
}

/// Metadata keys for `Wolf` (extends `tameable_animal`, indices `20..25`).
pub mod wolf {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`20`).
    pub const START_INDEX: u8 = 20;
    /// Exclusive end metadata index of this class (`25`).
    pub const END_INDEX: u8 = 25;
    /// `IS_BEGGING` at metadata index `20` (`Metadata::Boolean`).
    pub const IS_BEGGING: MetadataKey<bool> = MetadataKey::new(20, conv::to_bool, conv::from_bool);
    /// `COLLAR_COLOR` at metadata index `21` (`Metadata::VarInt`).
    pub const COLLAR_COLOR: MetadataKey<i32> =
        MetadataKey::new(21, conv::to_var_int, conv::from_var_int);
    /// `ANGER_TIME` at metadata index `22` (`Metadata::VarLong`).
    pub const ANGER_TIME: MetadataKey<i64> =
        MetadataKey::new(22, conv::to_var_long, conv::from_var_long);
    /// `VARIANT` at metadata index `23` (`Metadata::WolfVariant`).
    pub const VARIANT: MetadataKey<u32> =
        MetadataKey::new(23, conv::to_wolf_variant, conv::from_wolf_variant);
    /// `SOUND_VARIANT` at metadata index `24` (`Metadata::WolfSoundVariant`).
    pub const SOUND_VARIANT: MetadataKey<u32> = MetadataKey::new(
        24,
        conv::to_wolf_sound_variant,
        conv::from_wolf_sound_variant,
    );
}

/// Metadata keys for `Parrot` (extends `tameable_animal`, indices `20..21`).
pub mod parrot {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`20`).
    pub const START_INDEX: u8 = 20;
    /// Exclusive end metadata index of this class (`21`).
    pub const END_INDEX: u8 = 21;
    /// `VARIANT` at metadata index `20` (`Metadata::VarInt`).
    pub const VARIANT: MetadataKey<i32> =
        MetadataKey::new(20, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `AbstractNautilus` (extends `tameable_animal`, indices `20..21`).
pub mod abstract_nautilus {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`20`).
    pub const START_INDEX: u8 = 20;
    /// Exclusive end metadata index of this class (`21`).
    pub const END_INDEX: u8 = 21;
    /// `DASH` at metadata index `20` (`Metadata::Boolean`).
    pub const DASH: MetadataKey<bool> = MetadataKey::new(20, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `ZombieNautilus` (extends `abstract_nautilus`, indices `21..22`).
pub mod zombie_nautilus {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`21`).
    pub const START_INDEX: u8 = 21;
    /// Exclusive end metadata index of this class (`22`).
    pub const END_INDEX: u8 = 22;
    /// `VARIANT` at metadata index `21` (`Metadata::ZombieNautilusVariant`).
    pub const VARIANT: MetadataKey<u32> = MetadataKey::new(
        21,
        conv::to_zombie_nautilus_variant,
        conv::from_zombie_nautilus_variant,
    );
}

/// Metadata keys for `AbstractVillager` (extends `ageable_mob`, indices `18..19`).
pub mod abstract_villager {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `HEAD_SHAKE_TIMER` at metadata index `18` (`Metadata::VarInt`).
    pub const HEAD_SHAKE_TIMER: MetadataKey<i32> =
        MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Villager` (extends `abstract_villager`, indices `19..21`).
pub mod villager {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`19`).
    pub const START_INDEX: u8 = 19;
    /// Exclusive end metadata index of this class (`21`).
    pub const END_INDEX: u8 = 21;
    /// `VARIANT` at metadata index `19` (`Metadata::VillagerData`).
    pub const VARIANT: MetadataKey<VillagerData> =
        MetadataKey::new(19, conv::to_villager_data, conv::from_villager_data);
    /// `IS_FINALIZED` at metadata index `20` (`Metadata::Boolean`).
    pub const IS_FINALIZED: MetadataKey<bool> =
        MetadataKey::new(20, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `HappyGhast` (extends `ageable_mob`, indices `18..20`).
pub mod happy_ghast {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`18`).
    pub const START_INDEX: u8 = 18;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `IS_LEASH_HOLDER` at metadata index `18` (`Metadata::Boolean`).
    pub const IS_LEASH_HOLDER: MetadataKey<bool> =
        MetadataKey::new(18, conv::to_bool, conv::from_bool);
    /// `STAYS_STILL` at metadata index `19` (`Metadata::Boolean`).
    pub const STAYS_STILL: MetadataKey<bool> = MetadataKey::new(19, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `IronGolem` (extends `mob`, indices `16..17`).
pub mod iron_golem {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `IRON_GOLEM_FLAGS` at metadata index `16` (`Metadata::Byte`).
    pub const IRON_GOLEM_FLAGS: MetadataKey<i8> =
        MetadataKey::new(16, conv::to_byte, conv::from_byte);
    /// `IS_PLAYER_CREATED` bitmask `0x01` at metadata index `16`.
    pub const IS_PLAYER_CREATED: BitMaskKey = BitMaskKey::new(16, 0x01, false);
}

/// Metadata keys for `SnowGolem` (extends `mob`, indices `16..17`).
pub mod snow_golem {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `SNOW_GOLEM_FLAGS` at metadata index `16` (`Metadata::Byte`).
    pub const SNOW_GOLEM_FLAGS: MetadataKey<i8> =
        MetadataKey::new(16, conv::to_byte, conv::from_byte);
    /// `PUMPKIN_HAT` bitmask `0x10` at metadata index `16`.
    pub const PUMPKIN_HAT: BitMaskKey = BitMaskKey::new(16, 0x10, true);
}

/// Metadata keys for `Shulker` (extends `mob`, indices `16..19`).
pub mod shulker {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `ATTACH_FACE` at metadata index `16` (`Metadata::Direction`).
    pub const ATTACH_FACE: MetadataKey<Direction> =
        MetadataKey::new(16, conv::to_direction, conv::from_direction);
    /// `SHIELD_HEIGHT` at metadata index `17` (`Metadata::Byte`).
    pub const SHIELD_HEIGHT: MetadataKey<i8> = MetadataKey::new(17, conv::to_byte, conv::from_byte);
    /// `COLOR` at metadata index `18` (`Metadata::Byte`).
    pub const COLOR: MetadataKey<i8> = MetadataKey::new(18, conv::to_byte, conv::from_byte);
}

/// Metadata keys for `CopperGolem` (extends `mob`, indices `16..18`).
pub mod copper_golem {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`18`).
    pub const END_INDEX: u8 = 18;
    /// `WEATHER_STATE` at metadata index `16` (`Metadata::WeatherState`).
    pub const WEATHER_STATE: MetadataKey<WeatherState> =
        MetadataKey::new(16, conv::to_weather_state, conv::from_weather_state);
    /// `STATE` at metadata index `17` (`Metadata::CopperGolemState`).
    pub const STATE: MetadataKey<CopperGolemState> = MetadataKey::new(
        17,
        conv::to_copper_golem_state,
        conv::from_copper_golem_state,
    );
}

/// Metadata keys for `BasePiglin` (extends `mob`, indices `16..17`).
pub mod base_piglin {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `IMMUNE_ZOMBIFICATION` at metadata index `16` (`Metadata::Boolean`).
    pub const IMMUNE_ZOMBIFICATION: MetadataKey<bool> =
        MetadataKey::new(16, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Piglin` (extends `base_piglin`, indices `17..20`).
pub mod piglin {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`17`).
    pub const START_INDEX: u8 = 17;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `IS_BABY` at metadata index `17` (`Metadata::Boolean`).
    pub const IS_BABY: MetadataKey<bool> = MetadataKey::new(17, conv::to_bool, conv::from_bool);
    /// `IS_CHARGING_CROSSBOW` at metadata index `18` (`Metadata::Boolean`).
    pub const IS_CHARGING_CROSSBOW: MetadataKey<bool> =
        MetadataKey::new(18, conv::to_bool, conv::from_bool);
    /// `IS_DANCING` at metadata index `19` (`Metadata::Boolean`).
    pub const IS_DANCING: MetadataKey<bool> = MetadataKey::new(19, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Blaze` (extends `mob`, indices `16..17`).
pub mod blaze {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `BLAZE_FLAGS` at metadata index `16` (`Metadata::Byte`).
    pub const BLAZE_FLAGS: MetadataKey<i8> = MetadataKey::new(16, conv::to_byte, conv::from_byte);
    /// `IS_ON_FIRE` bitmask `0x01` at metadata index `16`.
    pub const IS_ON_FIRE: BitMaskKey = BitMaskKey::new(16, 0x01, false);
}

/// Metadata keys for `Bogged` (extends `mob`, indices `16..17`).
pub mod bogged {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `IS_SHEARED` at metadata index `16` (`Metadata::Boolean`).
    pub const IS_SHEARED: MetadataKey<bool> = MetadataKey::new(16, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Creaking` (extends `mob`, indices `16..20`).
pub mod creaking {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `CAN_MOVE` at metadata index `16` (`Metadata::Boolean`).
    pub const CAN_MOVE: MetadataKey<bool> = MetadataKey::new(16, conv::to_bool, conv::from_bool);
    /// `IS_ACTIVE` at metadata index `17` (`Metadata::Boolean`).
    pub const IS_ACTIVE: MetadataKey<bool> = MetadataKey::new(17, conv::to_bool, conv::from_bool);
    /// `IS_TEARING_DOWN` at metadata index `18` (`Metadata::Boolean`).
    pub const IS_TEARING_DOWN: MetadataKey<bool> =
        MetadataKey::new(18, conv::to_bool, conv::from_bool);
    /// `HOME_POS` at metadata index `19` (`Metadata::OptBlockPosition`).
    pub const HOME_POS: MetadataKey<Option<BlockVec>> =
        MetadataKey::new(19, conv::to_opt_block_pos, conv::from_opt_block_pos);
}

/// Metadata keys for `Creeper` (extends `mob`, indices `16..19`).
pub mod creeper {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `STATE` at metadata index `16` (`Metadata::VarInt`).
    pub const STATE: MetadataKey<i32> = MetadataKey::new(16, conv::to_var_int, conv::from_var_int);
    /// `IS_CHARGED` at metadata index `17` (`Metadata::Boolean`).
    pub const IS_CHARGED: MetadataKey<bool> = MetadataKey::new(17, conv::to_bool, conv::from_bool);
    /// `IS_IGNITED` at metadata index `18` (`Metadata::Boolean`).
    pub const IS_IGNITED: MetadataKey<bool> = MetadataKey::new(18, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Guardian` (extends `mob`, indices `16..18`).
pub mod guardian {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`18`).
    pub const END_INDEX: u8 = 18;
    /// `IS_RETRACTING_SPIKES` at metadata index `16` (`Metadata::Boolean`).
    pub const IS_RETRACTING_SPIKES: MetadataKey<bool> =
        MetadataKey::new(16, conv::to_bool, conv::from_bool);
    /// `TARGET_EID` at metadata index `17` (`Metadata::VarInt`).
    pub const TARGET_EID: MetadataKey<i32> =
        MetadataKey::new(17, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Raider` (extends `mob`, indices `16..17`).
pub mod raider {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `IS_CELEBRATING` at metadata index `16` (`Metadata::Boolean`).
    pub const IS_CELEBRATING: MetadataKey<bool> =
        MetadataKey::new(16, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Pillager` (extends `raider`, indices `17..18`).
pub mod pillager {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`17`).
    pub const START_INDEX: u8 = 17;
    /// Exclusive end metadata index of this class (`18`).
    pub const END_INDEX: u8 = 18;
    /// `IS_CHARGING` at metadata index `17` (`Metadata::Boolean`).
    pub const IS_CHARGING: MetadataKey<bool> = MetadataKey::new(17, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `SpellcasterIllager` (extends `raider`, indices `17..18`).
pub mod spellcaster_illager {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`17`).
    pub const START_INDEX: u8 = 17;
    /// Exclusive end metadata index of this class (`18`).
    pub const END_INDEX: u8 = 18;
    /// `SPELL` at metadata index `17` (`Metadata::Byte`).
    pub const SPELL: MetadataKey<i8> = MetadataKey::new(17, conv::to_byte, conv::from_byte);
}

/// Metadata keys for `Witch` (extends `raider`, indices `17..18`).
pub mod witch {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`17`).
    pub const START_INDEX: u8 = 17;
    /// Exclusive end metadata index of this class (`18`).
    pub const END_INDEX: u8 = 18;
    /// `IS_DRINKING_POTION` at metadata index `17` (`Metadata::Boolean`).
    pub const IS_DRINKING_POTION: MetadataKey<bool> =
        MetadataKey::new(17, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Spider` (extends `mob`, indices `16..17`).
pub mod spider {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `SPIDER_FLAGS` at metadata index `16` (`Metadata::Byte`).
    pub const SPIDER_FLAGS: MetadataKey<i8> = MetadataKey::new(16, conv::to_byte, conv::from_byte);
    /// `IS_CLIMBING` bitmask `0x01` at metadata index `16`.
    pub const IS_CLIMBING: BitMaskKey = BitMaskKey::new(16, 0x01, false);
}

/// Metadata keys for `Vex` (extends `mob`, indices `16..17`).
pub mod vex {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `VEX_FLAGS` at metadata index `16` (`Metadata::Byte`).
    pub const VEX_FLAGS: MetadataKey<i8> = MetadataKey::new(16, conv::to_byte, conv::from_byte);
    /// `IS_ATTACKING` bitmask `0x01` at metadata index `16`.
    pub const IS_ATTACKING: BitMaskKey = BitMaskKey::new(16, 0x01, false);
}

/// Metadata keys for `Warden` (extends `mob`, indices `16..17`).
pub mod warden {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `ANGER_LEVEL` at metadata index `16` (`Metadata::VarInt`).
    pub const ANGER_LEVEL: MetadataKey<i32> =
        MetadataKey::new(16, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Wither` (extends `mob`, indices `16..20`).
pub mod wither {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`20`).
    pub const END_INDEX: u8 = 20;
    /// `CENTER_HEAD_TARGET` at metadata index `16` (`Metadata::VarInt`).
    pub const CENTER_HEAD_TARGET: MetadataKey<i32> =
        MetadataKey::new(16, conv::to_var_int, conv::from_var_int);
    /// `LEFT_HEAD_TARGET` at metadata index `17` (`Metadata::VarInt`).
    pub const LEFT_HEAD_TARGET: MetadataKey<i32> =
        MetadataKey::new(17, conv::to_var_int, conv::from_var_int);
    /// `RIGHT_HEAD_TARGET` at metadata index `18` (`Metadata::VarInt`).
    pub const RIGHT_HEAD_TARGET: MetadataKey<i32> =
        MetadataKey::new(18, conv::to_var_int, conv::from_var_int);
    /// `INVULNERABLE_TIME` at metadata index `19` (`Metadata::VarInt`).
    pub const INVULNERABLE_TIME: MetadataKey<i32> =
        MetadataKey::new(19, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Zoglin` (extends `mob`, indices `16..17`).
pub mod zoglin {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `IS_BABY` at metadata index `16` (`Metadata::Boolean`).
    pub const IS_BABY: MetadataKey<bool> = MetadataKey::new(16, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Zombie` (extends `mob`, indices `16..19`).
pub mod zombie {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `IS_BABY` at metadata index `16` (`Metadata::Boolean`).
    pub const IS_BABY: MetadataKey<bool> = MetadataKey::new(16, conv::to_bool, conv::from_bool);
    /// `SPECIAL_TYPE` at metadata index `17` (`Metadata::VarInt`).
    pub const SPECIAL_TYPE: MetadataKey<i32> =
        MetadataKey::new(17, conv::to_var_int, conv::from_var_int);
    /// `IS_BECOMING_DROWNED` at metadata index `18` (`Metadata::Boolean`).
    pub const IS_BECOMING_DROWNED: MetadataKey<bool> =
        MetadataKey::new(18, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `ZombieVillager` (extends `zombie`, indices `19..22`).
pub mod zombie_villager {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`19`).
    pub const START_INDEX: u8 = 19;
    /// Exclusive end metadata index of this class (`22`).
    pub const END_INDEX: u8 = 22;
    /// `IS_CONVERTING` at metadata index `19` (`Metadata::Boolean`).
    pub const IS_CONVERTING: MetadataKey<bool> =
        MetadataKey::new(19, conv::to_bool, conv::from_bool);
    /// `VILLAGER_DATA` at metadata index `20` (`Metadata::VillagerData`).
    pub const VILLAGER_DATA: MetadataKey<VillagerData> =
        MetadataKey::new(20, conv::to_villager_data, conv::from_villager_data);
    /// `IS_FINALIZED` at metadata index `21` (`Metadata::Boolean`).
    pub const IS_FINALIZED: MetadataKey<bool> =
        MetadataKey::new(21, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Enderman` (extends `mob`, indices `16..19`).
pub mod enderman {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
    /// `CARRIED_BLOCK` at metadata index `16` (`Metadata::OptBlockState`).
    pub const CARRIED_BLOCK: MetadataKey<Option<Block>> =
        MetadataKey::new(16, conv::to_opt_block_state, conv::from_opt_block_state);
    /// `IS_SCREAMING` at metadata index `17` (`Metadata::Boolean`).
    pub const IS_SCREAMING: MetadataKey<bool> =
        MetadataKey::new(17, conv::to_bool, conv::from_bool);
    /// `IS_STARING` at metadata index `18` (`Metadata::Boolean`).
    pub const IS_STARING: MetadataKey<bool> = MetadataKey::new(18, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `EnderDragon` (extends `mob`, indices `16..17`).
pub mod ender_dragon {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `DRAGON_PHASE` at metadata index `16` (`Metadata::VarInt`).
    pub const DRAGON_PHASE: MetadataKey<i32> =
        MetadataKey::new(16, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Ghast` (extends `mob`, indices `16..17`).
pub mod ghast {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `IS_ATTACKING` at metadata index `16` (`Metadata::Boolean`).
    pub const IS_ATTACKING: MetadataKey<bool> =
        MetadataKey::new(16, conv::to_bool, conv::from_bool);
}

/// Metadata keys for `Phantom` (extends `mob`, indices `16..17`).
pub mod phantom {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`16`).
    pub const START_INDEX: u8 = 16;
    /// Exclusive end metadata index of this class (`17`).
    pub const END_INDEX: u8 = 17;
    /// `SIZE` at metadata index `16` (`Metadata::VarInt`).
    pub const SIZE: MetadataKey<i32> = MetadataKey::new(16, conv::to_var_int, conv::from_var_int);
}

/// Metadata keys for `Slime` (extends `abstract_cube_mob`, indices `19..19`).
pub mod slime {
    /// First metadata index owned by this class (`19`).
    pub const START_INDEX: u8 = 19;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
}

/// Metadata keys for `MagmaCube` (extends `abstract_cube_mob`, indices `19..19`).
pub mod magma_cube {
    /// First metadata index owned by this class (`19`).
    pub const START_INDEX: u8 = 19;
    /// Exclusive end metadata index of this class (`19`).
    pub const END_INDEX: u8 = 19;
}

/// Metadata keys for `PrimedTnt` (extends `entity`, indices `8..10`).
pub mod primed_tnt {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`10`).
    pub const END_INDEX: u8 = 10;
    /// `FUSE_TIME` at metadata index `8` (`Metadata::VarInt`).
    pub const FUSE_TIME: MetadataKey<i32> =
        MetadataKey::new(8, conv::to_var_int, conv::from_var_int);
    /// `BLOCK_STATE` at metadata index `9` (`Metadata::BlockState`).
    pub const BLOCK_STATE: MetadataKey<Block> =
        MetadataKey::new(9, conv::to_block_state, conv::from_block_state);
}

/// Metadata keys for `OminousItemSpawner` (extends `entity`, indices `8..9`).
pub mod ominous_item_spawner {
    #[allow(unused_imports)]
    use super::*;

    /// First metadata index owned by this class (`8`).
    pub const START_INDEX: u8 = 8;
    /// Exclusive end metadata index of this class (`9`).
    pub const END_INDEX: u8 = 9;
    /// `ITEM` at metadata index `8` (`Metadata::ItemStack`).
    pub const ITEM: MetadataKey<ItemStack> =
        MetadataKey::new(8, conv::to_item_stack, conv::from_item_stack);
}
