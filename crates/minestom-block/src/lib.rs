//! `Block` value type, state lookups, and `BlockGetter` world abstraction for `minestom-rs`.
//!
//! ```
//! use minestom_block::Block;
//!
//! assert!(Block::AIR.is_air());
//! assert!(Block::GRASS_BLOCK.is_solid());
//! assert!(!Block::GRASS_BLOCK.is_liquid());
//! assert!(Block::WATER.is_liquid());
//! ```

use minestom_core::BlockVec;
use minestom_data::generated::block_states::{
    self, BLOCK_INFOS, BlockInfo, STATE_IS_LIQUID, STATE_TO_BLOCK_ID, TOTAL_BLOCK_STATES,
};

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
    pub fn from_block_id(block_id: u16) -> Option<Self> {
        BLOCK_INFOS.get(usize::from(block_id)).map(|info| Self {
            state_id: info.default_state_id,
        })
    }

    /// Looks up a `Block` by its canonical `"minecraft:..."` key.
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
}

/// Read-only block query interface implemented by chunks and world instances.
pub trait BlockGetter {
    /// Returns the [`Block`] at `pos`, or [`Block::AIR`] if out of bounds / unloaded.
    fn get_block(&self, pos: BlockVec) -> Block;
}
