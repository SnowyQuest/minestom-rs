//! Validated Minecraft resource identifier (`Key` / `NamespaceId`, e.g. `minecraft:grass_block`).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::CoreError;

/// Default namespace (`"minecraft"`).
pub const MINECRAFT_NAMESPACE: &str = "minecraft";

fn is_valid_namespace_char(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-')
}

fn is_valid_value_char(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-' | '/')
}

/// Validated `namespace:value` resource location, corresponding to Adventure `Key` and Minestom `NamespaceID`.
///
/// ```
/// use minestom_core::Key;
///
/// let grass = Key::parse("grass_block").unwrap();
/// assert_eq!(grass.namespace(), "minecraft");
/// assert_eq!(grass.value(), "grass_block");
/// assert_eq!(grass.as_str(), "minecraft:grass_block");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    full: String,
    colon_idx: usize,
}

/// Type alias matching Minestom's `NamespaceID` naming.
pub type NamespaceId = Key;

impl Key {
    /// Constructs a validated `Key` from explicit `namespace` and `value`.
    pub fn new(namespace: &str, value: &str) -> Result<Self, CoreError> {
        if namespace.is_empty()
            || value.is_empty()
            || !namespace.chars().all(is_valid_namespace_char)
            || !value.chars().all(is_valid_value_char)
        {
            return Err(CoreError::InvalidKey(format!("{namespace}:{value}")));
        }
        let mut full = String::with_capacity(namespace.len() + 1 + value.len());
        full.push_str(namespace);
        full.push(':');
        full.push_str(value);
        Ok(Self {
            full,
            colon_idx: namespace.len(),
        })
    }

    /// Constructs a validated `minecraft:<value>` key.
    pub fn minecraft(value: &str) -> Result<Self, CoreError> {
        Self::new(MINECRAFT_NAMESPACE, value)
    }

    /// Parses `"namespace:value"` or `"value"` (defaulting namespace to `"minecraft"`).
    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        if let Some((ns, val)) = raw.split_once(':') {
            Self::new(ns, val)
        } else {
            Self::minecraft(raw)
        }
    }

    /// Returns the namespace portion (e.g. `"minecraft"`).
    pub fn namespace(&self) -> &str {
        &self.full[..self.colon_idx]
    }

    /// Returns the path/value portion (e.g. `"grass_block"`).
    pub fn value(&self) -> &str {
        &self.full[(self.colon_idx + 1)..]
    }

    /// Returns the canonical `"namespace:value"` string slice.
    pub fn as_str(&self) -> &str {
        &self.full
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.full)
    }
}

impl FromStr for Key {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.full)
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn valid_keys_roundtrip(
            ns in "[a-z0-9_.-]{1,16}",
            val in "[a-z0-9_./-]{1,32}",
        ) {
            let key = Key::new(&ns, &val).expect("valid key");
            prop_assert_eq!(key.namespace(), ns);
            prop_assert_eq!(key.value(), val);
            let parsed = Key::parse(key.as_str()).expect("reparse key");
            prop_assert_eq!(parsed, key);
        }
    }

    #[test]
    fn uppercase_or_empty_rejected() {
        assert!(Key::parse("Minecraft:stone").is_err());
        assert!(Key::parse("minecraft:").is_err());
        assert!(Key::parse(":stone").is_err());
    }
}
