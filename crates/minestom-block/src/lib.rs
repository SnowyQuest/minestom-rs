//! `Block` value type, mixed-radix O(1) property lookups (`get_property` / `with_property`),
//! `BlockHandler`, `BlockPlacementRule`, `BlockManager`, and `BlockGetter` / `BlockSetter`.
//!
//! ```
//! use minestom_block::Block;
//!
//! assert!(Block::AIR.is_air());
//! assert!(Block::GRASS_BLOCK.is_solid());
//! let snowy = Block::GRASS_BLOCK.with_property("snowy", "true").unwrap();
//! assert_eq!(snowy.get_property("snowy"), Some("true"));
//! assert_eq!(Block::OAK_LOG.get_property("axis"), Some("y"));
//! ```

use std::sync::Arc;

use minestom_core::{BlockFace, BlockVec, Hand, InstanceId, PlayerId, Pos, Vec3};
use minestom_data::generated::block_states::{
    self, BLOCK_INFOS, BlockInfo, STATE_IS_LIQUID, STATE_TO_BLOCK_ID, TOTAL_BLOCK_STATES,
};
use parking_lot::RwLock;
use rustc_hash::FxHashMap;

/// Errors returned when manipulating block states or properties.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlockError {
    /// Unknown block state ID.
    #[error("unknown block state ID: {0}")]
    UnknownStateId(u16),
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
}

/// Compact `Copy` value representing a vanilla block state (`u16` protocol state ID).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Block {
    state_id: u16,
}

impl Block {
    /// `minecraft:air` (`state_id = 0`).
    pub const AIR: Self = Self::from_raw_const(block_states::STATE_AIR);
    /// `minecraft:stone`.
    pub const STONE: Self = Self::from_raw_const(block_states::STATE_STONE);
    /// `minecraft:granite`.
    pub const GRANITE: Self = Self::from_raw_const(block_states::STATE_GRANITE);
    /// `minecraft:diorite`.
    pub const DIORITE: Self = Self::from_raw_const(block_states::STATE_DIORITE);
    /// `minecraft:andesite`.
    pub const ANDESITE: Self = Self::from_raw_const(block_states::STATE_ANDESITE);
    /// `minecraft:grass_block` (default `snowy=false`).
    pub const GRASS_BLOCK: Self = Self::from_raw_const(block_states::STATE_GRASS_BLOCK);
    /// `minecraft:dirt`.
    pub const DIRT: Self = Self::from_raw_const(block_states::STATE_DIRT);
    /// `minecraft:cobblestone`.
    pub const COBBLESTONE: Self = Self::from_raw_const(block_states::STATE_COBBLESTONE);
    /// `minecraft:oak_planks`.
    pub const OAK_PLANKS: Self = Self::from_raw_const(block_states::STATE_OAK_PLANKS);
    /// `minecraft:bedrock`.
    pub const BEDROCK: Self = Self::from_raw_const(block_states::STATE_BEDROCK);
    /// `minecraft:water`.
    pub const WATER: Self = Self::from_raw_const(block_states::STATE_WATER);
    /// `minecraft:lava`.
    pub const LAVA: Self = Self::from_raw_const(block_states::STATE_LAVA);
    /// `minecraft:sand`.
    pub const SAND: Self = Self::from_raw_const(block_states::STATE_SAND);
    /// `minecraft:gravel`.
    pub const GRAVEL: Self = Self::from_raw_const(block_states::STATE_GRAVEL);
    /// `minecraft:oak_log`.
    pub const OAK_LOG: Self = Self::from_raw_const(block_states::STATE_OAK_LOG);
    /// `minecraft:oak_leaves`.
    pub const OAK_LEAVES: Self = Self::from_raw_const(block_states::STATE_OAK_LEAVES);
    /// `minecraft:glass`.
    pub const GLASS: Self = Self::from_raw_const(block_states::STATE_GLASS);
    /// `minecraft:torch`.
    pub const TORCH: Self = Self::from_raw_const(block_states::STATE_TORCH);
    /// `minecraft:glowstone`.
    pub const GLOWSTONE: Self = Self::from_raw_const(block_states::STATE_GLOWSTONE);
    /// `minecraft:chest`.
    pub const CHEST: Self = Self::from_raw_const(block_states::STATE_CHEST);
    /// `minecraft:crafting_table`.
    pub const CRAFTING_TABLE: Self = Self::from_raw_const(block_states::STATE_CRAFTING_TABLE);
    /// `minecraft:furnace`.
    pub const FURNACE: Self = Self::from_raw_const(block_states::STATE_FURNACE);
    /// `minecraft:oak_sign`.
    pub const OAK_SIGN: Self = Self::from_raw_const(block_states::STATE_OAK_SIGN);
    /// `minecraft:oak_stairs`.
    pub const OAK_STAIRS: Self = Self::from_raw_const(block_states::STATE_OAK_STAIRS);
    /// `minecraft:diamond_block`.
    pub const DIAMOND_BLOCK: Self = Self::from_raw_const(block_states::STATE_DIAMOND_BLOCK);
    /// `minecraft:gold_block`.
    pub const GOLD_BLOCK: Self = Self::from_raw_const(block_states::STATE_GOLD_BLOCK);
    /// `minecraft:iron_block`.
    pub const IRON_BLOCK: Self = Self::from_raw_const(block_states::STATE_IRON_BLOCK);
    /// `minecraft:emerald_block`.
    pub const EMERALD_BLOCK: Self = Self::from_raw_const(block_states::STATE_EMERALD_BLOCK);
    /// `minecraft:obsidian`.
    pub const OBSIDIAN: Self = Self::from_raw_const(block_states::STATE_OBSIDIAN);
    /// `minecraft:tnt`.
    pub const TNT: Self = Self::from_raw_const(block_states::STATE_TNT);
    /// `minecraft:redstone_lamp`.
    pub const REDSTONE_LAMP: Self = Self::from_raw_const(block_states::STATE_REDSTONE_LAMP);
    /// `minecraft:lantern`.
    pub const LANTERN: Self = Self::from_raw_const(block_states::STATE_LANTERN);

    const fn from_raw_const(state_id: u16) -> Self {
        Self { state_id }
    }

    /// Constructs a `Block` from a validated protocol `state_id`.
    pub const fn from_state_id(state_id: u16) -> Result<Self, BlockError> {
        if (state_id as usize) < TOTAL_BLOCK_STATES {
            Ok(Self { state_id })
        } else {
            Err(BlockError::UnknownStateId(state_id))
        }
    }

    /// Constructs a `Block` from its numeric block registry ID (returning its default state).
    #[must_use]
    pub fn from_block_id(block_id: u16) -> Option<Self> {
        BLOCK_INFOS.get(usize::from(block_id)).map(|info| Self {
            state_id: info.default_state_id,
        })
    }

    /// Looks up a `Block` by its canonical `"minecraft:..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        let full = if key.contains(':') {
            key.to_owned()
        } else {
            format!("minecraft:{key}")
        };
        BLOCK_INFOS
            .iter()
            .find(|info| info.key == full)
            .map(|info| Self {
                state_id: info.default_state_id,
            })
    }

    /// Returns the numeric protocol block state ID.
    #[must_use]
    pub const fn state_id(self) -> u16 {
        self.state_id
    }

    /// Returns the owning numeric block registry ID.
    #[must_use]
    pub fn block_id(self) -> u16 {
        STATE_TO_BLOCK_ID
            .get(usize::from(self.state_id))
            .copied()
            .unwrap_or(0)
    }

    /// Returns static [`BlockInfo`] metadata for this block.
    #[must_use]
    pub fn info(self) -> &'static BlockInfo {
        &BLOCK_INFOS[usize::from(self.block_id())]
    }

    /// Canonical `"minecraft:..."` key of this block.
    #[must_use]
    pub fn key(self) -> &'static str {
        self.info().key
    }

    /// Optional vanilla `minecraft:block_entity_type` protocol ID (e.g. for chests, signs, furnaces).
    #[must_use]
    pub fn block_entity_id(self) -> Option<u16> {
        self.info().block_entity_id
    }

    /// Returns `true` if this state is air (`air`, `cave_air`, `void_air`).
    #[must_use]
    pub fn is_air(self) -> bool {
        self.state_id == 0 || self.info().air
    }

    /// Returns `true` if this block state is solid.
    #[must_use]
    pub fn is_solid(self) -> bool {
        self.info().solid
    }

    /// Returns `true` if this block state contains fluid (`water`, `lava`, or `waterlogged=true`).
    #[must_use]
    pub fn is_liquid(self) -> bool {
        STATE_IS_LIQUID
            .get(usize::from(self.state_id))
            .copied()
            .unwrap_or(false)
    }

    /// Reads the value of block state property `name` in O(properties) without allocating!
    #[must_use]
    pub fn get_property(self, name: &str) -> Option<&'static str> {
        let info = self.info();
        if info.properties.is_empty() {
            return None;
        }
        let mut rem = usize::from(self.state_id.saturating_sub(info.first_state_id));
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

    /// Returns a new [`Block`] state with property `name` set to `value` in O(properties).
    pub fn with_property(self, name: &str, value: &str) -> Result<Self, BlockError> {
        let info = self.info();
        let mut rem = usize::from(self.state_id.saturating_sub(info.first_state_id));
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
        Ok(Self {
            state_id: info.first_state_id + new_offset as u16,
        })
    }

    /// Returns all `(property_name, value)` pairs on this block state.
    #[must_use]
    pub fn properties(self) -> Vec<(&'static str, &'static str)> {
        let info = self.info();
        if info.properties.is_empty() {
            return Vec::new();
        }
        let mut rem = usize::from(self.state_id.saturating_sub(info.first_state_id));
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
    /// Return `false` to consume the interaction and suppress adjacent block placement!
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

/// Input parameters passed to [`BlockPlacementRule::block_place`].
pub struct PlacementState<'a> {
    /// Read-only world block accessor.
    pub world: &'a dyn BlockGetter,
    /// Candidate default block being placed.
    pub block: Block,
    /// Face of the supporting block that was clicked.
    pub block_face: BlockFace,
    /// Target block position where the block will be placed.
    pub place_position: BlockVec,
    /// Sub-block cursor click offset (`0.0..=1.0`).
    pub cursor_position: Vec3,
    /// Placing player's position and look angles.
    pub player_position: Pos,
    /// Whether the player is holding sneak.
    pub is_player_shifting: bool,
}

/// Input parameters passed to [`BlockPlacementRule::block_update`].
pub struct UpdateState<'a> {
    /// Read-only world block accessor.
    pub world: &'a dyn BlockGetter,
    /// Position of the block receiving a neighbor update.
    pub block_position: BlockVec,
    /// Current state of this block.
    pub current_block: Block,
    /// Face toward the neighbor that changed.
    pub from_face: BlockFace,
}

/// Custom state computation rule when placing or neighbor-updating a block type
/// (`BlockPlacementRule` in Minestom).
pub trait BlockPlacementRule: Send + Sync + 'static {
    /// Computes the placed [`Block`] state, or returns `None` to disallow placement.
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block>;

    /// Computes an updated [`Block`] state when an adjacent neighbor changes.
    fn block_update(&self, state: &UpdateState<'_>) -> Block {
        state.current_block
    }
}

/// Built-in placement rule orienting `axis=x|y|z` pillar blocks (logs, quartz pillars, etc.)
/// along the clicked [`BlockFace`].
pub struct AxisPlacementRule;

impl BlockPlacementRule for AxisPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let axis = match state.block_face {
            BlockFace::West | BlockFace::East => "x",
            BlockFace::Down | BlockFace::Up => "y",
            BlockFace::North | BlockFace::South => "z",
        };
        Some(
            state
                .block
                .with_property("axis", axis)
                .unwrap_or(state.block),
        )
    }
}

/// Built-in placement rule orienting `facing=north|south|west|east` horizontal blocks
/// (furnaces, chests, stairs) opposite the placing player's yaw.
pub struct HorizontalFacingPlacementRule;

impl BlockPlacementRule for HorizontalFacingPlacementRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let mut deg = (state.player_position.yaw - 90.0) % 360.0;
        if deg < 0.0 {
            deg += 360.0;
        }
        // Face TOWARD the player (opposite of look direction)
        let facing = if deg < 45.0 || deg >= 315.0 {
            "east"
        } else if deg < 135.0 {
            "south"
        } else if deg < 225.0 {
            "west"
        } else {
            "north"
        };
        Some(
            state
                .block
                .with_property("facing", facing)
                .unwrap_or(state.block),
        )
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
        Self::new()
    }
}

impl BlockManager {
    /// Creates an empty [`BlockManager`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(BlockManagerInner {
                handlers: FxHashMap::default(),
                placement_rules: FxHashMap::default(),
            })),
        }
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

    /// Looks up the [`BlockPlacementRule`] registered for `block`, if any.
    #[must_use]
    pub fn get_placement_rule(&self, block: Block) -> Option<Arc<dyn BlockPlacementRule>> {
        self.inner
            .read()
            .placement_rules
            .get(&block.block_id())
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_radix_block_properties_and_block_entity_ids_work() {
        let log_x = Block::OAK_LOG.with_property("axis", "x").unwrap();
        assert_eq!(log_x.get_property("axis"), Some("x"));
        let log_z = log_x.with_property("axis", "z").unwrap();
        assert_eq!(log_z.get_property("axis"), Some("z"));

        let stairs = Block::OAK_STAIRS
            .with_property("facing", "south")
            .unwrap()
            .with_property("half", "top")
            .unwrap();
        assert_eq!(stairs.get_property("facing"), Some("south"));
        assert_eq!(stairs.get_property("half"), Some("top"));

        assert!(Block::CHEST.block_entity_id().is_some());
        assert!(Block::FURNACE.block_entity_id().is_some());
        assert!(Block::STONE.block_entity_id().is_none());
    }
}
