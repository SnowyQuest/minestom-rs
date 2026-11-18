//! `Block` value type, state properties, `BlockHandler`, and `BlockPlacementRule` for `minestom-rs`.

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
