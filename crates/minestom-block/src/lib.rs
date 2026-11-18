//! `Block` value type (`1,196` vanilla blocks across `32,366` states), exact per-state
//! properties & sub-voxel collision/outline shapes, `BlockEntityType`, `Fluid`, `GameEvent`,
//! `BlockHandler`, `BlockPlacementRule` (`BlockManager::with_vanilla_rules()`), and
//! `BlockGetter` / `BlockSetter`.
//!
//! ```
//! use std::str::FromStr;
//! use minestom_block::Block;
//!
//! assert!(Block::AIR.is_air());
//! assert!(Block::GRASS_BLOCK.is_solid());
//! let stairs = Block::from_str("minecraft:oak_stairs[facing=east,half=top]").unwrap();
//! assert_eq!(stairs.get_property("facing"), Some("east"));
//! assert_eq!(stairs.get_property("half"), Some("top"));
//! assert_eq!(stairs.collision_shape().len(), 2);
//! ```

pub mod rules;

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use minestom_core::{BlockFace, BlockVec, Hand, InstanceId, PlayerId, Vec3};
use minestom_data::generated::{
    block_entity_types,
    block_states::{
        BLOCK_INFOS, BLOCK_SHAPES, STATE_FLAG_AIR, STATE_FLAG_BLOCKS_MOTION,
        STATE_FLAG_CAN_RESPAWN_IN, STATE_FLAG_FLAMMABLE, STATE_FLAG_FLUID, STATE_FLAG_LIQUID,
        STATE_FLAG_OCCLUDES, STATE_FLAG_REDSTONE_CONDUCTOR, STATE_FLAG_REPLACEABLE,
        STATE_FLAG_REQUIRES_TOOL, STATE_FLAG_SIGNAL_SOURCE, STATE_FLAG_SOLID, STATE_INFOS,
        TOTAL_BLOCK_STATES, lookup_block_default_state,
    },
    fluids, game_events,
};
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use minestom_data::generated::block_states::{BlockAabb, BlockInfo, PushReaction, StateInfo};
pub use minestom_data::generated::fluids::FluidInfo;
pub use minestom_data::generated::game_events::GameEventInfo;
pub use rules::{
    AnvilPlacementRule, AwayFacingPlacementRule, AxisPlacementRule, BlockPlacementRule,
    Directional6PlacementRule, DoorPlacementRule, FaceAttachedPlacementRule,
    FenceConnectPlacementRule, HorizontalFacingPlacementRule, LanternPlacementRule, PlacementState,
    ReplacementContext, Rotation16PlacementRule, SlabPlacementRule, SnowLayerPlacementRule,
    StairsPlacementRule, TrapdoorPlacementRule, UpdateState, WallConnectPlacementRule,
    register_vanilla_placement_rules,
};

/// Errors returned when manipulating block states, properties, or registry IDs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlockError {
    /// Unknown block state ID.
    #[error("unknown block state ID: {0}")]
    UnknownStateId(u16),
    /// Unknown block key or malformed state string (`"minecraft:foo[k=v]"`).
    #[error("invalid block state string: `{0}`")]
    InvalidStateString(String),
    /// Unknown property name or value for a block.
    #[error("invalid property `{property}={value}` on block `{block}`")]
    InvalidProperty {
        /// Block namespace key.
        block: String,
        /// Property name.
        property: String,
        /// Attempted property value.
        value: String,
    },
    /// Unknown vanilla block-entity type ID.
    #[error("unknown block entity type ID: {0}")]
    UnknownBlockEntityType(u32),
    /// Unknown vanilla fluid ID.
    #[error("unknown fluid ID: {0}")]
    UnknownFluid(u32),
    /// Unknown vanilla game event ID.
    #[error("unknown game event ID: {0}")]
    UnknownGameEvent(u32),
}

/// Compact `Copy` value representing a vanilla block state (`u16` protocol state ID).
///
/// Exposes all **1,196** vanilla MC 26.2 `Block::*` default-state constants alongside
/// O(1) per-state property queries and sub-voxel [`BlockAabb`] collision/outline shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Block(u16);

minestom_data::impl_block_constants!(Block);

impl Block {
    /// Constructs a `Block` from a validated protocol `state_id` (`0..32_366`).
    pub const fn from_state_id(state_id: u16) -> Result<Self, BlockError> {
        if (state_id as usize) < TOTAL_BLOCK_STATES {
            Ok(Self(state_id))
        } else {
            Err(BlockError::UnknownStateId(state_id))
        }
    }

    /// `const`-friendly optional constructor from a protocol `state_id` (`0..32_366`).
    #[must_use]
    pub const fn from_state_id_opt(state_id: u16) -> Option<Self> {
        if (state_id as usize) < TOTAL_BLOCK_STATES {
            Some(Self(state_id))
        } else {
            None
        }
    }

    /// Constructs a `Block` from its numeric block registry ID (`0..1196`), returning its default state.
    #[must_use]
    pub fn from_block_id(block_id: u16) -> Option<Self> {
        BLOCK_INFOS
            .get(usize::from(block_id))
            .map(|info| Self(info.default_state_id))
    }

    /// Looks up a `Block`'s default state by `"minecraft:..."` or bare `"..."` key in
    /// zero-allocation `O(log N)` time.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        lookup_block_default_state(key).map(Self)
    }

    /// Parses either a plain block key (`"minecraft:stone"` / `"stone"`) or a full canonical
    /// state string (`"minecraft:oak_stairs[facing=east,half=top]"`).
    pub fn from_state_str(raw: &str) -> Result<Self, BlockError> {
        let trimmed = raw.trim();
        if let Some((block_part, rest)) = trimmed.split_once('[') {
            let inner = rest
                .strip_suffix(']')
                .ok_or_else(|| BlockError::InvalidStateString(raw.to_owned()))?;
            let mut block = Self::from_key(block_part)
                .ok_or_else(|| BlockError::InvalidStateString(raw.to_owned()))?;
            if !inner.is_empty() {
                for pair in inner.split(',') {
                    let (k, v) = pair
                        .split_once('=')
                        .ok_or_else(|| BlockError::InvalidStateString(raw.to_owned()))?;
                    block = block.with_property(k.trim(), v.trim())?;
                }
            }
            Ok(block)
        } else {
            Self::from_key(trimmed).ok_or_else(|| BlockError::InvalidStateString(raw.to_owned()))
        }
    }

    /// Returns the numeric protocol block state ID (`0..32_366`).
    #[must_use]
    pub const fn state_id(self) -> u16 {
        self.0
    }

    /// Returns the compact per-state [`StateInfo`] record in `O(1)`.
    #[must_use]
    pub const fn state_info(self) -> &'static StateInfo {
        &STATE_INFOS[self.0 as usize]
    }

    /// Returns the owning numeric block registry ID (`0..1196`).
    #[must_use]
    pub const fn block_id(self) -> u16 {
        self.state_info().block_id
    }

    /// Returns static [`BlockInfo`] metadata for this block in `O(1)`.
    #[must_use]
    pub const fn info(self) -> &'static BlockInfo {
        &BLOCK_INFOS[self.block_id() as usize]
    }

    /// Resets all properties on this block back to its vanilla default state (`defaultState` in Minestom).
    #[must_use]
    pub const fn default_state(self) -> Self {
        Self(self.info().default_state_id)
    }

    /// Returns `true` if `self` and `other` share the same underlying block type regardless of state
    /// (`block.compare(other)` in Minestom).
    #[must_use]
    pub const fn same_block(self, other: Self) -> bool {
        self.block_id() == other.block_id()
    }

    /// Zero-allocation iterator over every valid [`Block`] state belonging to this block
    /// (`possibleStates` in Minestom).
    pub fn possible_states(
        self,
    ) -> impl ExactSizeIterator<Item = Self> + DoubleEndedIterator + Clone {
        let info = self.info();
        let start = info.first_state_id;
        let end = start + info.state_count;
        (start..end).map(Self)
    }

    /// Canonical `"minecraft:..."` key of this block.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.info().key
    }

    /// Vanilla client translation key (e.g. `"block.minecraft.stone"`).
    #[must_use]
    pub const fn translation_key(self) -> &'static str {
        self.info().translation_key
    }

    /// Numeric `Material` ID of the item corresponding to this block, if any.
    #[must_use]
    pub const fn corresponding_item_id(self) -> Option<u16> {
        self.info().corresponding_item_id
    }

    /// Optional vanilla `minecraft:block_entity_type` protocol ID (e.g. for chests, signs, furnaces).
    #[must_use]
    pub const fn block_entity_id(self) -> Option<u16> {
        self.info().block_entity_id
    }

    /// Optional strongly-typed [`BlockEntityType`] associated with this block.
    #[must_use]
    pub const fn block_entity_type(self) -> Option<BlockEntityType> {
        match self.info().block_entity_id {
            Some(id) => Some(BlockEntityType(id as u32)),
            None => None,
        }
    }

    /// Returns `true` if this state is air (`air`, `cave_air`, `void_air`).
    #[must_use]
    pub const fn is_air(self) -> bool {
        (self.state_info().flags & STATE_FLAG_AIR) != 0
    }

    /// Returns `true` if this exact block state is solid (`StateInfo` per-state flag).
    #[must_use]
    pub const fn is_solid(self) -> bool {
        (self.state_info().flags & STATE_FLAG_SOLID) != 0
    }

    /// Returns `true` if this exact block state blocks motion (`blocksMotion` in `block.json`).
    #[must_use]
    pub const fn blocks_motion(self) -> bool {
        (self.state_info().flags & STATE_FLAG_BLOCKS_MOTION) != 0
    }

    /// Returns `true` if this block is a pure liquid (`minecraft:water` or `minecraft:lava`).
    #[must_use]
    pub const fn is_liquid(self) -> bool {
        (self.state_info().flags & STATE_FLAG_LIQUID) != 0
    }

    /// Returns `true` if this block state contains fluid (`water`, `lava`, `kelp`, `seagrass`,
    /// or any block state with `waterlogged=true`).
    #[must_use]
    pub const fn has_fluid(self) -> bool {
        (self.state_info().flags & STATE_FLAG_FLUID) != 0
    }

    /// Returns `true` if placing a block directly onto this state replaces it (`air`, `water`,
    /// `short_grass`, `tall_grass`, `fern`, `snow[layers=1]`, `vine`, `fire`, etc.).
    #[must_use]
    pub const fn is_replaceable(self) -> bool {
        (self.state_info().flags & STATE_FLAG_REPLACEABLE) != 0
    }

    /// Returns `true` if this block occludes neighbor faces (`occludes` in `block.json`).
    #[must_use]
    pub const fn occludes(self) -> bool {
        (self.state_info().flags & STATE_FLAG_OCCLUDES) != 0
    }

    /// Returns `true` if a proper mining tool is required to harvest drops.
    #[must_use]
    pub const fn requires_tool(self) -> bool {
        (self.state_info().flags & STATE_FLAG_REQUIRES_TOOL) != 0
    }

    /// Returns `true` if this exact block state can catch fire (`false` when `waterlogged=true`).
    #[must_use]
    pub const fn is_flammable(self) -> bool {
        (self.state_info().flags & STATE_FLAG_FLAMMABLE) != 0
    }

    /// Returns `true` if this exact block state conducts redstone power.
    #[must_use]
    pub const fn is_redstone_conductor(self) -> bool {
        (self.state_info().flags & STATE_FLAG_REDSTONE_CONDUCTOR) != 0
    }

    /// Returns `true` if this block can emit redstone signal.
    #[must_use]
    pub const fn is_signal_source(self) -> bool {
        (self.state_info().flags & STATE_FLAG_SIGNAL_SOURCE) != 0
    }

    /// Returns `true` if players/mobs can spawn inside this block state.
    #[must_use]
    pub const fn can_respawn_in(self) -> bool {
        (self.state_info().flags & STATE_FLAG_CAN_RESPAWN_IN) != 0
    }

    /// Returns `true` if this block falls when unsupported (`sand`, `gravel`, `anvil`, concrete powder).
    #[must_use]
    pub const fn has_gravity(self) -> bool {
        self.info().gravity
    }

    /// Mining hardness (`-1.0` for unbreakable blocks like `bedrock` and `barrier`).
    #[must_use]
    pub const fn hardness(self) -> f32 {
        self.info().hardness
    }

    /// Returns `true` if `hardness < 0.0` (`bedrock`, `barrier`, `command_block`, etc.).
    #[must_use]
    pub fn is_unbreakable(self) -> bool {
        self.hardness() < 0.0
    }

    /// Explosion blast resistance.
    #[must_use]
    pub const fn explosion_resistance(self) -> f32 {
        self.info().explosion_resistance
    }

    /// Slipperiness friction (`0.6` normal, `0.98` ice, `0.989` blue ice).
    #[must_use]
    pub const fn friction(self) -> f32 {
        self.info().friction
    }

    /// Entity horizontal speed multiplier (`0.4` soul sand / honey block, `1.0` default).
    #[must_use]
    pub const fn speed_factor(self) -> f32 {
        self.info().speed_factor
    }

    /// Entity jump velocity multiplier (`0.5` honey block, `1.0` default).
    #[must_use]
    pub const fn jump_factor(self) -> f32 {
        self.info().jump_factor
    }

    /// Piston [`PushReaction`].
    #[must_use]
    pub const fn push_reaction(self) -> PushReaction {
        self.info().push_reaction
    }

    /// Per-state light opacity (`0..=15`, `lightBlock` in `block.json`).
    #[must_use]
    pub const fn light_opacity(self) -> u8 {
        self.state_info().light_block
    }

    /// Per-state emitted block light (`0..=15`, `lightEmission` in `block.json`).
    #[must_use]
    pub const fn light_emission(self) -> u8 {
        self.state_info().light_emission
    }

    /// Per-state map color ID.
    #[must_use]
    pub const fn map_color_id(self) -> u8 {
        self.state_info().map_color_id
    }

    /// Exact sub-voxel physical collision AABBs in block-local `[0.0, 1.0]` coordinates
    /// (`&[]` for air, liquids, open doors, flowers, and torches; 2 boxes for stairs, etc.).
    #[must_use]
    pub const fn collision_shape(self) -> &'static [BlockAabb] {
        BLOCK_SHAPES[self.state_info().collision_shape_idx as usize]
    }

    /// Exact sub-voxel selection/raycast outline AABBs in block-local coordinates.
    #[must_use]
    pub const fn outline_shape(self) -> &'static [BlockAabb] {
        BLOCK_SHAPES[self.state_info().outline_shape_idx as usize]
    }

    /// Returns `true` if a world-space entity AABB `(min, max)` overlaps any sub-voxel box in
    /// `self.collision_shape()` when placed at `block_pos`.
    #[must_use]
    pub fn collides_world_aabb(self, block_pos: BlockVec, min: Vec3, max: Vec3) -> bool {
        const EPS: f64 = 1.0e-7;
        let boxes = self.collision_shape();
        if boxes.is_empty() {
            return false;
        }
        let bx = f64::from(block_pos.x);
        let by = f64::from(block_pos.y);
        let bz = f64::from(block_pos.z);
        for b in boxes {
            let b_min_x = bx + f64::from(b.min_x);
            let b_max_x = bx + f64::from(b.max_x);
            let b_min_y = by + f64::from(b.min_y);
            let b_max_y = by + f64::from(b.max_y);
            let b_min_z = bz + f64::from(b.min_z);
            let b_max_z = bz + f64::from(b.max_z);
            if min.x < b_max_x - EPS
                && max.x > b_min_x + EPS
                && min.y < b_max_y - EPS
                && max.y > b_min_y + EPS
                && min.z < b_max_z - EPS
                && max.z > b_min_z + EPS
            {
                return true;
            }
        }
        false
    }

    /// Reads the value of block state property `name` in O(properties) without allocating.
    #[must_use]
    pub fn get_property(self, name: &str) -> Option<&'static str> {
        let info = self.info();
        if info.properties.is_empty() {
            return None;
        }
        let mut rem = usize::from(self.0.saturating_sub(info.first_state_id));
        let mut decoded_indices = [0_usize; 16];
        for (idx, (_, allowed)) in info.properties.iter().enumerate().rev() {
            let radix = allowed.len().max(1);
            decoded_indices[idx] = rem % radix;
            rem /= radix;
        }
        for ((pname, allowed), &v_idx) in info.properties.iter().zip(&decoded_indices) {
            if *pname == name {
                return allowed.get(v_idx).copied();
            }
        }
        None
    }

    /// Returns `true` if this block defines a property named `name`.
    #[must_use]
    pub fn has_property(self, name: &str) -> bool {
        self.info()
            .properties
            .iter()
            .any(|(pname, _)| *pname == name)
    }

    /// Returns a new [`Block`] state with property `name` set to `value` in O(properties).
    pub fn with_property(self, name: &str, value: &str) -> Result<Self, BlockError> {
        let info = self.info();
        let mut rem = usize::from(self.0.saturating_sub(info.first_state_id));
        let mut decoded_indices = [0_usize; 16];
        for (idx, (_, allowed)) in info.properties.iter().enumerate().rev() {
            let radix = allowed.len().max(1);
            decoded_indices[idx] = rem % radix;
            rem /= radix;
        }

        let mut found = false;
        for (idx, (pname, allowed)) in info.properties.iter().enumerate() {
            if *pname == name {
                let Some(new_v_idx) = allowed.iter().position(|&v| v == value) else {
                    return Err(BlockError::InvalidProperty {
                        block: info.key.to_owned(),
                        property: name.to_owned(),
                        value: value.to_owned(),
                    });
                };
                decoded_indices[idx] = new_v_idx;
                found = true;
                break;
            }
        }
        if !found {
            return Err(BlockError::InvalidProperty {
                block: info.key.to_owned(),
                property: name.to_owned(),
                value: value.to_owned(),
            });
        }

        let mut new_offset = 0_usize;
        for (idx, (_, allowed)) in info.properties.iter().enumerate() {
            new_offset = new_offset * allowed.len() + decoded_indices[idx];
        }
        Ok(Self(info.first_state_id + new_offset as u16))
    }

    /// Applies multiple `(property_name, value)` updates in a single pass.
    pub fn with_properties<'a>(
        mut self,
        props: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, BlockError> {
        for (k, v) in props {
            self = self.with_property(k, v)?;
        }
        Ok(self)
    }

    /// Returns all `(property_name, value)` pairs on this block state.
    #[must_use]
    pub fn properties(self) -> Vec<(&'static str, &'static str)> {
        let info = self.info();
        if info.properties.is_empty() {
            return Vec::new();
        }
        let mut rem = usize::from(self.0.saturating_sub(info.first_state_id));
        let mut decoded_indices = [0_usize; 16];
        for (idx, (_, allowed)) in info.properties.iter().enumerate().rev() {
            let radix = allowed.len().max(1);
            decoded_indices[idx] = rem % radix;
            rem /= radix;
        }
        info.properties
            .iter()
            .enumerate()
            .map(|(idx, (pname, allowed))| (*pname, allowed[decoded_indices[idx]]))
            .collect()
    }
}

impl TryFrom<u16> for Block {
    type Error = BlockError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::from_state_id(value)
    }
}

impl From<Block> for u16 {
    fn from(value: Block) -> Self {
        value.0
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let info = self.info();
        f.write_str(info.key)?;
        if !info.properties.is_empty() {
            f.write_str("[")?;
            let mut rem = usize::from(self.0.saturating_sub(info.first_state_id));
            let mut decoded_indices = [0_usize; 16];
            for (idx, (_, allowed)) in info.properties.iter().enumerate().rev() {
                let radix = allowed.len().max(1);
                decoded_indices[idx] = rem % radix;
                rem /= radix;
            }
            for (idx, (pname, allowed)) in info.properties.iter().enumerate() {
                if idx > 0 {
                    f.write_str(",")?;
                }
                f.write_str(pname)?;
                f.write_str("=")?;
                f.write_str(allowed[decoded_indices[idx]])?;
            }
            f.write_str("]")?;
        }
        Ok(())
    }
}

impl FromStr for Block {
    type Err = BlockError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_state_str(s)
    }
}

impl Serialize for Block {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Block {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_state_str(&s).map_err(serde::de::Error::custom)
    }
}

// ============================================================================
// BlockEntityType, Fluid, and GameEvent
// ============================================================================

/// Strongly typed `Copy` handle for a vanilla `minecraft:block_entity_type` (`49` entries).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockEntityType(u32);

minestom_data::impl_block_entity_type_constants!(BlockEntityType);

impl BlockEntityType {
    /// Constructs a [`BlockEntityType`] from a validated protocol ID (`0..49`).
    pub const fn from_id(id: u32) -> Result<Self, BlockError> {
        if (id as usize) < block_entity_types::BLOCK_ENTITY_TYPE_COUNT {
            Ok(Self(id))
        } else {
            Err(BlockError::UnknownBlockEntityType(id))
        }
    }

    /// Zero-allocation O(log N) lookup by `"minecraft:..."` or bare `"..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        block_entity_types::lookup_key(key).map(Self)
    }

    /// Numeric protocol ID.
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Canonical `"minecraft:..."` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        block_entity_types::ENTRIES[self.0 as usize].1
    }
}

impl fmt::Display for BlockEntityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// Strongly typed `Copy` handle for a vanilla `minecraft:fluid` (`5` entries).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fluid(u32);

minestom_data::impl_fluid_constants!(Fluid);

impl Fluid {
    /// Constructs a [`Fluid`] from a validated protocol ID (`0..5`).
    pub const fn from_id(id: u32) -> Result<Self, BlockError> {
        if (id as usize) < fluids::FLUID_COUNT {
            Ok(Self(id))
        } else {
            Err(BlockError::UnknownFluid(id))
        }
    }

    /// Zero-allocation O(log N) lookup by `"minecraft:..."` or bare `"..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        fluids::lookup_key(key).map(Self)
    }

    /// Numeric protocol ID.
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Static [`FluidInfo`] entry.
    #[must_use]
    pub const fn info(self) -> &'static FluidInfo {
        &fluids::FLUID_INFOS[self.0 as usize]
    }

    /// Canonical `"minecraft:..."` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.info().key
    }
}

impl fmt::Display for Fluid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// Strongly typed `Copy` handle for a vanilla `minecraft:game_event` (`61` entries).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GameEvent(u32);

minestom_data::impl_game_event_constants!(GameEvent);

impl GameEvent {
    /// Constructs a [`GameEvent`] from a validated protocol ID (`0..61`).
    pub const fn from_id(id: u32) -> Result<Self, BlockError> {
        if (id as usize) < game_events::GAME_EVENT_COUNT {
            Ok(Self(id))
        } else {
            Err(BlockError::UnknownGameEvent(id))
        }
    }

    /// Zero-allocation O(log N) lookup by `"minecraft:..."` or bare `"..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        game_events::lookup_key(key).map(Self)
    }

    /// Numeric protocol ID.
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Static [`GameEventInfo`] entry.
    #[must_use]
    pub const fn info(self) -> &'static GameEventInfo {
        &game_events::GAME_EVENT_INFOS[self.0 as usize]
    }

    /// Canonical `"minecraft:..."` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.info().key
    }
}

impl fmt::Display for GameEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

// ============================================================================
// BlockGetter, BlockSetter, BlockHandler, and BlockManager
// ============================================================================

/// Read-only block query interface implemented by chunks and world instances.
pub trait BlockGetter {
    /// Returns the [`Block`] at `pos`, or [`Block::AIR`] if out of bounds / unloaded.
    fn get_block(&self, pos: BlockVec) -> Block;
}

/// Mutable block write interface implemented by chunks, batches, and world instances.
pub trait BlockSetter {
    /// Writes `block` at `pos`.
    fn set_block(&mut self, pos: BlockVec, block: Block);
}

/// Details forwarded to [`BlockHandler::on_place`].
#[derive(Debug, Clone, Copy)]
pub struct BlockPlacementContext {
    /// Newly placed block state.
    pub block: Block,
    /// Previous block state at `position`.
    pub previous_block: Block,
    /// World instance ID.
    pub instance_id: InstanceId,
    /// Block position.
    pub position: BlockVec,
    /// Placing player, if placed by a player.
    pub player_id: Option<PlayerId>,
}

/// Details forwarded to [`BlockHandler::on_destroy`].
#[derive(Debug, Clone, Copy)]
pub struct BlockDestroyContext {
    /// Destroyed block state.
    pub block: Block,
    /// Replacement block state (usually [`Block::AIR`]).
    pub new_block: Block,
    /// World instance ID.
    pub instance_id: InstanceId,
    /// Block position.
    pub position: BlockVec,
    /// Breaking player, if broken by a player.
    pub player_id: Option<PlayerId>,
}

/// Details forwarded to [`BlockHandler::on_interact`].
#[derive(Debug, Clone, Copy)]
pub struct BlockInteractContext {
    /// Clicked block state.
    pub block: Block,
    /// World instance ID.
    pub instance_id: InstanceId,
    /// Clicked block position.
    pub position: BlockVec,
    /// Clicked [`BlockFace`].
    pub block_face: BlockFace,
    /// Sub-block cursor offset (`0.0..=1.0`).
    pub cursor_position: Vec3,
    /// Interacting player ID.
    pub player_id: PlayerId,
    /// Hand used.
    pub hand: Hand,
}

/// Custom server-side block behavior (`BlockHandler` in Minestom).
pub trait BlockHandler: Send + Sync + 'static {
    /// Canonical namespace key identifying this handler (e.g. `"minecraft:chest"`).
    fn key(&self) -> &str;

    /// Invoked when a block owning this handler is placed.
    fn on_place(&self, _ctx: &BlockPlacementContext) {}

    /// Invoked when a block owning this handler is broken or replaced.
    fn on_destroy(&self, _ctx: &BlockDestroyContext) {}

    /// Invoked when a player right-clicks this block while not sneaking.
    /// Return `false` to consume the interaction and suppress adjacent block placement.
    fn on_interact(&self, _ctx: &BlockInteractContext) -> bool {
        true
    }

    /// Invoked every server tick if [`Self::is_tickable`] returns `true`.
    fn tick(&self, _instance_id: InstanceId, _position: BlockVec, _block: Block) {}

    /// Whether this handler requires per-tick callbacks.
    fn is_tickable(&self) -> bool {
        false
    }
}

struct BlockManagerInner {
    handlers: FxHashMap<String, Arc<dyn BlockHandler>>,
    placement_rules: FxHashMap<u16, Arc<dyn BlockPlacementRule>>,
}

/// Thread-safe registry of [`BlockHandler`]s and [`BlockPlacementRule`]s (`BlockManager` in Minestom).
#[derive(Clone)]
pub struct BlockManager {
    inner: Arc<RwLock<BlockManagerInner>>,
}

impl std::fmt::Debug for BlockManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.inner.read();
        f.debug_struct("BlockManager")
            .field("handlers", &g.handlers.len())
            .field("placement_rules", &g.placement_rules.len())
            .finish_non_exhaustive()
    }
}

impl Default for BlockManager {
    fn default() -> Self {
        Self::with_vanilla_rules()
    }
}

impl BlockManager {
    /// Creates an empty [`BlockManager`] with zero placement rules registered.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            inner: Arc::new(RwLock::new(BlockManagerInner {
                handlers: FxHashMap::default(),
                placement_rules: FxHashMap::default(),
            })),
        }
    }

    /// Creates a [`BlockManager`] pre-populated with vanilla [`BlockPlacementRule`]s for all
    /// slabs, stairs, pillars/logs, directional & horizontal-facing blocks, doors, trapdoors,
    /// fences, glass panes, iron/copper bars, walls, buttons, levers, signs, banners, torches,
    /// lanterns, snow layers, and waterloggable blocks.
    #[must_use]
    pub fn with_vanilla_rules() -> Self {
        let mgr = Self::empty();
        register_vanilla_placement_rules(&mgr);
        mgr
    }

    /// Alias for [`Self::with_vanilla_rules`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_vanilla_rules()
    }

    /// Registers a [`BlockHandler`] under its [`BlockHandler::key`].
    pub fn register_handler(&self, handler: impl BlockHandler) {
        let arc: Arc<dyn BlockHandler> = Arc::new(handler);
        self.inner
            .write()
            .handlers
            .insert(arc.key().to_owned(), arc);
    }

    /// Looks up a registered [`BlockHandler`] by namespace key.
    #[must_use]
    pub fn get_handler(&self, key: &str) -> Option<Arc<dyn BlockHandler>> {
        self.inner.read().handlers.get(key).cloned()
    }

    /// Registers a [`BlockPlacementRule`] for `block`'s `block_id`.
    pub fn register_placement_rule(&self, block: Block, rule: impl BlockPlacementRule) {
        self.inner
            .write()
            .placement_rules
            .insert(block.block_id(), Arc::new(rule));
    }

    /// Registers a shared `Arc<dyn BlockPlacementRule>` for `block_id`.
    pub fn register_shared_placement_rule(&self, block_id: u16, rule: Arc<dyn BlockPlacementRule>) {
        self.inner.write().placement_rules.insert(block_id, rule);
    }

    /// Looks up the [`BlockPlacementRule`] registered for `block`, if any.
    #[must_use]
    pub fn get_placement_rule(&self, block: Block) -> Option<Arc<dyn BlockPlacementRule>> {
        self.inner
            .read()
            .placement_rules
            .get(&block.block_id())
            .cloned()
    }

    /// Returns the number of registered placement rules.
    #[must_use]
    pub fn placement_rule_count(&self) -> usize {
        self.inner.read().placement_rules.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_1196_blocks_and_32366_states_roundtrip() {
        assert_eq!(BLOCK_INFOS.len(), 1196);
        assert_eq!(Block::AIR.state_id(), 0);
        assert!(Block::AIR.is_air());
        assert!(Block::SHORT_GRASS.is_replaceable());
        assert!(Block::WATER.is_liquid());
        assert!(Block::WATER.has_fluid());
        assert!(Block::KELP.has_fluid());
        assert!(!Block::KELP.is_liquid());

        // Per-state light emission: unlit furnace = 0, lit furnace = 13!
        assert_eq!(Block::FURNACE.light_emission(), 0);
        let lit_furnace = Block::FURNACE.with_property("lit", "true").unwrap();
        assert_eq!(lit_furnace.light_emission(), 13);

        // Display & FromStr canonical state string round-trip
        let stair = Block::OAK_STAIRS
            .with_properties([("facing", "west"), ("half", "top"), ("waterlogged", "true")])
            .unwrap();
        assert!(stair.has_fluid());
        assert!(!stair.is_liquid());
        let rendered = stair.to_string();
        assert_eq!(Block::from_str(&rendered).unwrap(), stair);
        assert_eq!(Block::OAK_STAIRS.possible_states().len(), 80);
    }
}
