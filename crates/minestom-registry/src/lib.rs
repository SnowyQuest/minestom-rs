//! Static and dynamic Minecraft 26.2 registries (`DimensionType`, `Biome`, `ChatType`,
//! `DamageType`, `Dialog`, `Timeline`, `WorldClock`, entity variants, and tags).

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
