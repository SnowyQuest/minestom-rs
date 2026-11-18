//! Common gameplay enums, RGB/ARGB colors, generational IDs, and offline UUID derivation.

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use slotmap::new_key_type;
use uuid::Uuid;

use crate::{CoreError, Vec3};

new_key_type! {
    /// Generational handle identifying an `Entity` inside an `Instance`.
    pub struct EntityId;
    /// Generational handle identifying a connected `Player`.
    pub struct PlayerId;
    /// Generational handle identifying a registered `Instance`.
    pub struct InstanceId;
}

/// Computes the deterministic offline-mode UUID for `username`, matching Java's
/// `UUID.nameUUIDFromBytes(("OfflinePlayer:" + username).getBytes(UTF_8))`.
#[must_use]
pub fn offline_player_uuid(username: &str) -> Uuid {
    let mut hasher = Md5::new();
    hasher.update(b"OfflinePlayer:");
    hasher.update(username.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&hasher.finalize());
    uuid::Builder::from_md5_bytes(bytes).into_uuid()
}

/// 24-bit RGB color (`0xRRGGBB`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Color(pub u32);

impl Color {
    /// Constructs a `Color` from `(r, g, b)` components.
    #[must_use]
    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    /// Red channel (`0..=255`).
    pub const fn r(self) -> u8 {
        ((self.0 >> 16) & 0xFF) as u8
    }

    /// Green channel (`0..=255`).
    pub const fn g(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    /// Blue channel (`0..=255`).
    pub const fn b(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    /// Formats as `"#rrggbb"`.
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("#{:06x}", self.0 & 0x00FF_FFFF)
    }
}

/// 32-bit ARGB color (`0xAARRGGBB`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct AlphaColor(pub u32);

impl AlphaColor {
    /// Constructs an `AlphaColor` from `(a, r, g, b)`.
    #[must_use]
    pub const fn from_argb(a: u8, r: u8, g: u8, b: u8) -> Self {
        Self(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }
}

/// Cardinal/vertical 6-way axis direction (`Down = 0` .. `East = 5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Direction {
    /// `-Y` (`0`).
    Down = 0,
    /// `+Y` (`1`).
    Up = 1,
    /// `-Z` (`2`).
    North = 2,
    /// `+Z` (`3`).
    South = 3,
    /// `-X` (`4`).
    West = 4,
    /// `+X` (`5`).
    East = 5,
}

/// Alias matching Minestom's `BlockFace`.
pub type BlockFace = Direction;

impl Direction {
    /// Converts a protocol face ID (`0..=5`) into a [`Direction`].
    pub const fn from_id(id: i32) -> Result<Self, CoreError> {
        match id {
            0 => Ok(Self::Down),
            1 => Ok(Self::Up),
            2 => Ok(Self::North),
            3 => Ok(Self::South),
            4 => Ok(Self::West),
            5 => Ok(Self::East),
            other => Err(CoreError::InvalidEnumId {
                kind: "Direction",
                value: other,
            }),
        }
    }

    /// Returns the opposite face.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Down => Self::Up,
            Self::Up => Self::Down,
            Self::North => Self::South,
            Self::South => Self::North,
            Self::West => Self::East,
            Self::East => Self::West,
        }
    }

    /// Unit normal vector pointing outward from this face.
    #[must_use]
    pub const fn normal(self) -> Vec3 {
        match self {
            Self::Down => Vec3::new(0.0, -1.0, 0.0),
            Self::Up => Vec3::new(0.0, 1.0, 0.0),
            Self::North => Vec3::new(0.0, 0.0, -1.0),
            Self::South => Vec3::new(0.0, 0.0, 1.0),
            Self::West => Vec3::new(-1.0, 0.0, 0.0),
            Self::East => Vec3::new(1.0, 0.0, 0.0),
        }
    }
}

/// Player game mode (`Survival = 0` .. `Spectator = 3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum GameMode {
    /// Survival (`0`).
    #[default]
    Survival = 0,
    /// Creative (`1`).
    Creative = 1,
    /// Adventure (`2`).
    Adventure = 2,
    /// Spectator (`3`).
    Spectator = 3,
}

impl GameMode {
    /// Converts a protocol ID (`0..=3`) into a [`GameMode`].
    pub const fn from_id(id: i32) -> Result<Self, CoreError> {
        match id {
            0 => Ok(Self::Survival),
            1 => Ok(Self::Creative),
            2 => Ok(Self::Adventure),
            3 => Ok(Self::Spectator),
            other => Err(CoreError::InvalidEnumId {
                kind: "GameMode",
                value: other,
            }),
        }
    }
}

/// Server world difficulty (`Peaceful = 0` .. `Hard = 3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum Difficulty {
    /// Peaceful (`0`).
    Peaceful = 0,
    /// Easy (`1`).
    Easy = 1,
    /// Normal (`2`).
    #[default]
    Normal = 2,
    /// Hard (`3`).
    Hard = 3,
}

impl Difficulty {
    /// Converts a protocol ID (`0..=3`) into a [`Difficulty`].
    pub const fn from_id(id: i32) -> Result<Self, CoreError> {
        match id {
            0 => Ok(Self::Peaceful),
            1 => Ok(Self::Easy),
            2 => Ok(Self::Normal),
            3 => Ok(Self::Hard),
            other => Err(CoreError::InvalidEnumId {
                kind: "Difficulty",
                value: other,
            }),
        }
    }
}

/// Active interaction hand (`Main = 0`, `Off = 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum Hand {
    /// Main hand (`0`).
    #[default]
    Main = 0,
    /// Off hand (`1`).
    Off = 1,
}

impl Hand {
    /// Converts a protocol ID (`0..=1`) into a [`Hand`].
    pub const fn from_id(id: i32) -> Result<Self, CoreError> {
        match id {
            0 => Ok(Self::Main),
            1 => Ok(Self::Off),
            other => Err(CoreError::InvalidEnumId {
                kind: "Hand",
                value: other,
            }),
        }
    }
}

/// Player dominant hand preference (`Left = 0`, `Right = 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum MainHand {
    /// Left-handed (`0`).
    Left = 0,
    /// Right-handed (`1`).
    #[default]
    Right = 1,
}

impl MainHand {
    /// Converts a protocol ID (`0..=1`) into a [`MainHand`].
    pub const fn from_id(id: i32) -> Result<Self, CoreError> {
        match id {
            0 => Ok(Self::Left),
            1 => Ok(Self::Right),
            other => Err(CoreError::InvalidEnumId {
                kind: "MainHand",
                value: other,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_java_notch_reference() {
        // Known reference: UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        // = b50ad385-829d-3141-a216-7e7d7539ba7f
        let uuid = offline_player_uuid("Notch");
        assert_eq!(uuid.get_version_num(), 3);
        assert_eq!(uuid.to_string(), "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }
}
