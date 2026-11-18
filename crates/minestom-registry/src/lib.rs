//! Static and dynamic Minecraft 26.2 registries (`DimensionType`, `Biome`, `ChatType`,
//! `DamageType`, `Dialog`, `Timeline`, `WorldClock`, entity variants, and tags).

use minestom_data::generated::dynamic_registries::{
    BIOME_COUNT, CONFIGURATION_REGISTRIES, OVERWORLD_DIMENSION_TYPE_ID, PLAINS_BIOME_ID,
    VanillaDynamicRegistry,
};
use minestom_data::generated::registry_tags::{CLIENT_TAG_REGISTRIES, TagGroupSlice};
use minestom_nbt::{NbtTag, json_to_nbt};

/// Errors returned during registry lookup or registration.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// Entry was not found in the registry.
    #[error("unknown registry entry `{key}` in `{registry}`")]
    UnknownEntry {
        /// Registry identifier.
        registry: String,
        /// Missing entry key.
        key: String,
    },
    /// Invalid registry configuration or payload.
    #[error("invalid registry data: {0}")]
    InvalidData(String),
}

/// Dimension height and lighting geometry corresponding to Minestom's `DimensionType`.
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionType {
    /// Registry identifier (e.g. `"minecraft:overworld"`).
    pub key: String,
    /// Numeric protocol ID inside `minecraft:dimension_type`.
    pub protocol_id: u32,
    /// Minimum buildable Y coordinate (multiple of 16, `-64` for Overworld).
    pub min_y: i32,
    /// Total world height in blocks (multiple of 16, `384` for Overworld).
    pub height: i32,
    /// Logical height for chorus/portal placement (`384` for Overworld).
    pub logical_height: i32,
    /// Whether this dimension has sky light (`true` for Overworld).
    pub has_skylight: bool,
    /// Whether this dimension has a bedrock ceiling (`true` for Nether).
    pub has_ceiling: bool,
}

impl DimensionType {
    /// Standard vanilla `minecraft:overworld` (`min_y = -64`, `height = 384`, 24 sections).
    #[must_use]
    pub fn overworld() -> Self {
        Self {
            key: "minecraft:overworld".to_owned(),
            protocol_id: OVERWORLD_DIMENSION_TYPE_ID,
            min_y: -64,
            height: 384,
            logical_height: 384,
            has_skylight: true,
            has_ceiling: false,
        }
    }

    /// Exclusive upper Y bound (`min_y + height`, `320` for Overworld).
    #[must_use]
    pub const fn max_y(&self) -> i32 {
        self.min_y + self.height
    }

    /// Minimum 16-block section Y index (`-4` for Overworld).
    #[must_use]
    pub const fn min_section(&self) -> i32 {
        self.min_y >> 4
    }

    /// Exclusive maximum 16-block section Y index (`20` for Overworld).
    #[must_use]
    pub const fn max_section(&self) -> i32 {
        self.max_y() >> 4
    }

    /// Total number of vertical 16×16×16 sections per chunk (`24` for Overworld).
    #[must_use]
    pub const fn section_count(&self) -> usize {
        (self.height >> 4) as usize
    }
}

impl Default for DimensionType {
    fn default() -> Self {
        Self::overworld()
    }
}

/// Provides access to vanilla MC 26.2 configuration registries and tag sets.
#[derive(Debug, Clone, Default)]
pub struct Registries;

impl Registries {
    /// Creates a default vanilla `Registries` instance backed by `26.2-rv3` generated tables.
    #[must_use]
    pub const fn vanilla() -> Self {
        Self
    }

    /// Numeric biome ID for `minecraft:plains`.
    #[must_use]
    pub const fn plains_biome_id(&self) -> u32 {
        PLAINS_BIOME_ID
    }

    /// Total number of vanilla biomes (`BIOME_COUNT`).
    #[must_use]
    pub const fn biome_count(&self) -> usize {
        BIOME_COUNT
    }

    /// Returns all 27 MC 26.2 configuration dynamic registries in wire order.
    #[must_use]
    pub const fn configuration_registries(&self) -> &'static [VanillaDynamicRegistry] {
        CONFIGURATION_REGISTRIES
    }

    /// Decodes a vanilla dynamic registry JSON entry into an [`NbtTag`] when full NBT sync is needed
    /// (i.e. when a client did not advertise `minecraft:core:26.2` in `SelectKnownPacks`).
    pub fn decode_entry_nbt(raw_json: &str) -> Result<NbtTag, RegistryError> {
        let val: serde_json::Value = serde_json::from_str(raw_json)
            .map_err(|e| RegistryError::InvalidData(e.to_string()))?;
        Ok(json_to_nbt(&val))
    }

    /// Returns all 32 client tag registries `(registry_id, &[(tag_id, &[entry_ids])])`.
    #[must_use]
    pub const fn client_tag_registries(&self) -> &'static [(&'static str, &'static TagGroupSlice)] {
        CLIENT_TAG_REGISTRIES
    }
}
