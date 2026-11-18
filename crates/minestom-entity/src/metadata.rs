//! Dirty-tracked `EntityMetadata` container supporting both raw `(index, MetadataValue)` access
//! and zero-cost compile-time typed [`MetadataKey`], [`BitMaskKey`], and [`ByteMaskKey`] entries
//! from [`crate::metadata_def`].

use std::collections::{BTreeMap, BTreeSet};

use minestom_core::MainHand;
use minestom_protocol::entity::{EntityPose, MetadataValue};
use minestom_text::Component;

use crate::metadata_def::{self, BitMaskKey, ByteMaskKey, MetadataKey};

/// Dirty-tracked map of entity metadata indices (`0..255`) for `EntityMetaDataPacket`.
#[derive(Debug, Clone, Default)]
pub struct EntityMetadata {
    entries: BTreeMap<u8, MetadataValue>,
    dirty: BTreeSet<u8>,
}

impl EntityMetadata {
    /// Creates an empty [`EntityMetadata`] map.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            dirty: BTreeSet::new(),
        }
    }

    /// Sets raw metadata `index` to `value` and marks it dirty if changed.
    pub fn set(&mut self, index: u8, value: MetadataValue) {
        if self.entries.get(&index) != Some(&value) {
            self.entries.insert(index, value);
            self.dirty.insert(index);
        }
    }

    /// Reads the current raw value at `index`, if set.
    #[must_use]
    pub fn get(&self, index: u8) -> Option<&MetadataValue> {
        self.entries.get(&index)
    }

    /// Sets a typed [`MetadataKey<T>`] entry (from [`crate::metadata_def`]) and marks its wire index dirty.
    pub fn set_key<T>(&mut self, key: MetadataKey<T>, value: T) {
        self.set(key.index(), key.encode_value(value));
    }

    /// Reads a typed [`MetadataKey<T>`] entry, returning `None` if unset.
    #[must_use]
    pub fn get_key<T>(&self, key: MetadataKey<T>) -> Option<T> {
        self.entries
            .get(&key.index())
            .and_then(|raw| key.decode_value(raw))
    }

    /// Sets a boolean [`BitMaskKey`] flag inside a byte metadata entry.
    pub fn set_flag(&mut self, flag: BitMaskKey, enabled: bool) {
        let current = match self.entries.get(&flag.index()) {
            Some(MetadataValue::Byte(b)) => *b as u8,
            _ => {
                if flag.default_value() {
                    flag.mask()
                } else {
                    0
                }
            }
        };
        let next = if enabled {
            current | flag.mask()
        } else {
            current & !flag.mask()
        };
        self.set(flag.index(), MetadataValue::Byte(next as i8));
    }

    /// Reads a boolean [`BitMaskKey`] flag (returning `flag.default_value()` if unset).
    #[must_use]
    pub fn get_flag(&self, flag: BitMaskKey) -> bool {
        match self.entries.get(&flag.index()) {
            Some(MetadataValue::Byte(b)) => ((*b as u8) & flag.mask()) != 0,
            _ => flag.default_value(),
        }
    }

    /// Sets a multi-bit [`ByteMaskKey`] sub-field (e.g. `Sheep::COLOR_ID`, `TextDisplay::ALIGNMENT`).
    pub fn set_byte_mask(&mut self, field: ByteMaskKey, value: u8) {
        let current = match self.entries.get(&field.index()) {
            Some(MetadataValue::Byte(b)) => *b as u8,
            _ => (field.default_value() << field.shift()) & field.mask(),
        };
        let next = (current & !field.mask()) | ((value << field.shift()) & field.mask());
        self.set(field.index(), MetadataValue::Byte(next as i8));
    }

    /// Reads a multi-bit [`ByteMaskKey`] sub-field.
    #[must_use]
    pub fn get_byte_mask(&self, field: ByteMaskKey) -> u8 {
        match self.entries.get(&field.index()) {
            Some(MetadataValue::Byte(b)) => ((*b as u8) & field.mask()) >> field.shift(),
            _ => field.default_value(),
        }
    }

    /// Sets `IS_ON_FIRE` (`index 0`, bit `0x01`).
    pub fn set_on_fire(&mut self, on_fire: bool) {
        self.set_flag(metadata_def::entity::IS_ON_FIRE, on_fire);
    }

    /// Sets `IS_CROUCHING` (`index 0`, bit `0x02`).
    pub fn set_sneaking(&mut self, sneaking: bool) {
        self.set_flag(metadata_def::entity::IS_CROUCHING, sneaking);
    }

    /// Returns `true` if `IS_CROUCHING` is set.
    #[must_use]
    pub fn is_sneaking(&self) -> bool {
        self.get_flag(metadata_def::entity::IS_CROUCHING)
    }

    /// Sets `IS_SPRINTING` (`index 0`, bit `0x08`).
    pub fn set_sprinting(&mut self, sprinting: bool) {
        self.set_flag(metadata_def::entity::IS_SPRINTING, sprinting);
    }

    /// Returns `true` if `IS_SPRINTING` is set.
    #[must_use]
    pub fn is_sprinting(&self) -> bool {
        self.get_flag(metadata_def::entity::IS_SPRINTING)
    }

    /// Sets `IS_INVISIBLE` (`index 0`, bit `0x20`).
    pub fn set_invisible(&mut self, invisible: bool) {
        self.set_flag(metadata_def::entity::IS_INVISIBLE, invisible);
    }

    /// Sets `HAS_GLOWING_EFFECT` (`index 0`, bit `0x40`).
    pub fn set_glowing(&mut self, glowing: bool) {
        self.set_flag(metadata_def::entity::HAS_GLOWING_EFFECT, glowing);
    }

    /// Sets `CUSTOM_NAME` (`index 2`, `OptComponent`).
    pub fn set_custom_name(&mut self, name: Option<Component>) {
        self.set_key(metadata_def::entity::CUSTOM_NAME, name);
    }

    /// Sets `CUSTOM_NAME_VISIBLE` (`index 3`, `Boolean`).
    pub fn set_custom_name_visible(&mut self, visible: bool) {
        self.set_key(metadata_def::entity::CUSTOM_NAME_VISIBLE, visible);
    }

    /// Sets `IS_SILENT` (`index 4`, `Boolean`).
    pub fn set_silent(&mut self, silent: bool) {
        self.set_key(metadata_def::entity::IS_SILENT, silent);
    }

    /// Sets `HAS_NO_GRAVITY` (`index 5`, `Boolean`).
    pub fn set_no_gravity(&mut self, no_gravity: bool) {
        self.set_key(metadata_def::entity::HAS_NO_GRAVITY, no_gravity);
    }

    /// Returns `true` if `HAS_NO_GRAVITY` (`index 5`) is enabled.
    #[must_use]
    pub fn has_no_gravity(&self) -> bool {
        self.get_key(metadata_def::entity::HAS_NO_GRAVITY)
            .unwrap_or(false)
    }

    /// Sets `POSE` (`index 6`, `EntityPose`).
    pub fn set_pose(&mut self, pose: EntityPose) {
        self.set_key(metadata_def::entity::POSE, pose);
    }

    /// Sets `LivingEntity::HEALTH` (`index 9`, `Float`).
    pub fn set_health(&mut self, health: f32) {
        self.set_key(metadata_def::living_entity::HEALTH, health);
    }

    /// Sets `Avatar::MAIN_HAND` (`index 15`, `MainHand` in MC 26.2).
    pub fn set_main_hand(&mut self, hand: MainHand) {
        self.set_key(metadata_def::avatar::MAIN_HAND, hand);
    }

    /// Sets `Avatar::DISPLAYED_MODEL_PARTS_FLAGS` (`index 16`, `Byte` in MC 26.2).
    pub fn set_displayed_skin_parts(&mut self, flags: u8) {
        self.set_key(
            metadata_def::avatar::DISPLAYED_MODEL_PARTS_FLAGS,
            flags as i8,
        );
    }

    /// Sets `Mob::NO_AI` (`index 15`, bit `0x01`).
    pub fn set_no_ai(&mut self, no_ai: bool) {
        self.set_flag(metadata_def::mob::NO_AI, no_ai);
    }

    /// Sets `Mob::IS_AGGRESSIVE` (`index 15`, bit `0x04`).
    pub fn set_aggressive(&mut self, aggressive: bool) {
        self.set_flag(metadata_def::mob::IS_AGGRESSIVE, aggressive);
    }

    /// Returns `true` if at least one entry was modified since the last [`Self::take_dirty`] call.
    #[must_use]
    pub fn has_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Drains all modified metadata entries since the last sync, returning a map ready for
    /// `EntityMetaDataPacket`.
    pub fn take_dirty(&mut self) -> BTreeMap<u8, MetadataValue> {
        let mut out = BTreeMap::new();
        for idx in std::mem::take(&mut self.dirty) {
            if let Some(v) = self.entries.get(&idx) {
                out.insert(idx, v.clone());
            }
        }
        out
    }

    /// Clears dirty flags without returning a map (useful right after initial spawn).
    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    /// Clones all non-default metadata entries for a newly entering viewer.
    #[must_use]
    pub fn all_entries(&self) -> BTreeMap<u8, MetadataValue> {
        self.entries.clone()
    }
}
