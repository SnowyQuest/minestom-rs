//! Typed `ServerConfig` with builder and TOML serialization support.

use std::collections::BTreeSet;

use minestom_core::Difficulty;
use minestom_network::DEFAULT_MOJANG_HAS_JOINED_URL;
use minestom_protocol::status::StatusPlayerSample;
use minestom_text::Component;
use serde::{Deserialize, Serialize};

use crate::ServerError;

/// Authentication and proxy forwarding mode (`Auth` in Minestom).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AuthMode {
    /// Offline mode (derives deterministic v3 UUID from username if client sends nil UUID).
    #[default]
    Offline,
    /// Mojang Online Mode (RSA-1024 + AES-128-CFB8 + `hasJoined` Yggdrasil session verification).
    Online {
        /// Session verification URL (defaults to [`DEFAULT_MOJANG_HAS_JOINED_URL`]).
        session_server_url: String,
        /// Whether to append `&ip=<client_ip>` to `hasJoined` queries.
        prevent_proxy_connections: bool,
    },
    /// Velocity modern login-plugin forwarding (`velocity:player_info` HMAC-SHA256 v1).
    Velocity {
        /// Shared `forwarding.secret` string configured in Velocity.
        secret: String,
    },
    /// `BungeeCord` / `BungeeGuard` legacy `\0`-delimited handshake forwarding.
    Bungee {
        /// Optional set of allowed `bungeeguard-token` secrets (`None` = standard unguarded `BungeeCord`).
        guard_tokens: Option<BTreeSet<String>>,
    },
}

impl AuthMode {
    /// Standard Mojang Online Mode using the official session server URL.
    #[must_use]
    pub fn online() -> Self {
        Self::Online {
            session_server_url: DEFAULT_MOJANG_HAS_JOINED_URL.to_owned(),
            prevent_proxy_connections: false,
        }
    }

    /// Online Mode pointing at a custom Yggdrasil/mock `hasJoined` endpoint URL.
    #[must_use]
    pub fn online_with_url(
        session_server_url: impl Into<String>,
        prevent_proxy_connections: bool,
    ) -> Self {
        Self::Online {
            session_server_url: session_server_url.into(),
            prevent_proxy_connections,
        }
    }

    /// Velocity modern forwarding with shared `secret`.
    #[must_use]
    pub fn velocity(secret: impl Into<String>) -> Self {
        Self::Velocity {
            secret: secret.into(),
        }
    }

    /// Standard `BungeeCord` IP forwarding without `BungeeGuard` token enforcement.
    #[must_use]
    pub const fn bungee() -> Self {
        Self::Bungee { guard_tokens: None }
    }

    /// `BungeeGuard` IP forwarding requiring one of `tokens` in `bungeeguard-token`.
    #[must_use]
    pub fn bungee_guard<I, S>(tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Bungee {
            guard_tokens: Some(tokens.into_iter().map(Into::into).collect()),
        }
    }
}

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
    /// Interval between Play `KeepAlive` challenges in milliseconds (default `10_000`).
    pub keep_alive_interval_ms: u64,
    /// Maximum time to wait for a `KeepAlive` reply before kicking in milliseconds (default `15_000`).
    pub keep_alive_timeout_ms: u64,
    /// Optional raw 64×64 PNG bytes for the server list favicon.
    #[serde(skip)]
    pub favicon: Option<Vec<u8>>,
    /// Optional hover sample entries shown in the server list.
    #[serde(default)]
    pub player_sample: Vec<StatusPlayerSample>,
    /// Player authentication & proxy forwarding mode (default [`AuthMode::Offline`]).
    #[serde(default)]
    pub auth: AuthMode,
    /// Optional sustained inbound packet rate limit per connection (`None` disables rate limiting; default `Some(500)`).
    #[serde(default)]
    pub max_packets_per_second: Option<u32>,
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
            keep_alive_interval_ms: 10_000,
            keep_alive_timeout_ms: 15_000,
            favicon: None,
            player_sample: Vec::new(),
            auth: AuthMode::Offline,
            max_packets_per_second: Some(500),
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
        if self.keep_alive_interval_ms == 0 || self.keep_alive_timeout_ms == 0 {
            return Err(ServerError::Config(
                "keep_alive intervals must be > 0".to_owned(),
            ));
        }
        match &self.auth {
            AuthMode::Velocity { secret } if secret.is_empty() => {
                return Err(ServerError::Config(
                    "Velocity forwarding secret cannot be empty".to_owned(),
                ));
            }
            AuthMode::Bungee {
                guard_tokens: Some(tokens),
            } if tokens.is_empty() => {
                return Err(ServerError::Config(
                    "BungeeGuard token set cannot be empty".to_owned(),
                ));
            }
            _ => {}
        }
        if self.max_packets_per_second == Some(0) {
            return Err(ServerError::Config(
                "max_packets_per_second must be > 0 when enabled".to_owned(),
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
