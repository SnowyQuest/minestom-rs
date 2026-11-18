//! Typed `ServerConfig` with builder and TOML serialization support.

use minestom_core::Difficulty;
use minestom_protocol::status::StatusPlayerSample;
use minestom_text::Component;
use serde::{Deserialize, Serialize};

use crate::ServerError;

/// Strongly typed configuration for [`crate::MinecraftServer`], replacing Minestom's JVM `-D` flags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Simulation ticks per second (default `20`).
    pub ticks_per_second: u32,
    /// Chunk view distance in chunks (default `8`).
    pub chunk_view_distance: u8,
    /// Entity tracking view distance in chunks (default `5`).
    pub entity_view_distance: u8,
    /// Zlib packet compression threshold in bytes (`None` disables compression; default `Some(256)`).
    pub compression_threshold: Option<usize>,
    /// Server brand sent via `minecraft:brand` plugin message.
    pub brand_name: String,
    /// World difficulty advertised to clients.
    pub difficulty: Difficulty,
    /// Maximum player slots advertised in server status.
    pub max_players: u32,
    /// Server list MOTD description.
    pub motd: Component,
    /// Whether secure chat signatures are enforced.
    pub enforces_secure_chat: bool,
    /// Optional raw 64×64 PNG bytes for the server list favicon.
    #[serde(skip)]
    pub favicon: Option<Vec<u8>>,
    /// Optional hover sample entries shown in the server list.
    #[serde(default)]
    pub player_sample: Vec<StatusPlayerSample>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            ticks_per_second: 20,
            chunk_view_distance: 8,
            entity_view_distance: 5,
            compression_threshold: Some(256),
            brand_name: "minestom-rs".to_owned(),
            difficulty: Difficulty::Normal,
            max_players: 100,
            motd: Component::text("A minestom-rs Server"),
            enforces_secure_chat: false,
            favicon: None,
            player_sample: Vec::new(),
        }
    }
}

impl ServerConfig {
    /// Validates invariants on this configuration.
    pub fn validate(&self) -> Result<(), ServerError> {
        if self.ticks_per_second == 0 || self.ticks_per_second > 1000 {
            return Err(ServerError::Config(
                "ticks_per_second must be in 1..=1000".to_owned(),
            ));
        }
        if !(2..=32).contains(&self.chunk_view_distance) {
            return Err(ServerError::Config(
                "chunk_view_distance must be in 2..=32".to_owned(),
            ));
        }
        if !(1..=32).contains(&self.entity_view_distance) {
            return Err(ServerError::Config(
                "entity_view_distance must be in 1..=32".to_owned(),
            ));
        }
        Ok(())
    }

    /// Parses a `ServerConfig` from a TOML string and validates it.
    pub fn from_toml_str(toml_str: &str) -> Result<Self, ServerError> {
        let cfg: Self = toml::from_str(toml_str).map_err(|e| ServerError::Config(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Serializes this `ServerConfig` to a formatted TOML string.
    pub fn to_toml_string(&self) -> Result<String, ServerError> {
        toml::to_string_pretty(self).map_err(|e| ServerError::Config(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_toml_roundtrip() {
        let cfg = ServerConfig::default();
        let encoded = cfg.to_toml_string().expect("encode TOML");
        let decoded = ServerConfig::from_toml_str(&encoded).expect("decode TOML");
        assert_eq!(decoded, cfg);
    }
}
