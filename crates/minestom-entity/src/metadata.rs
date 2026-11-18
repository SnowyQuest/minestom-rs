//! Dirty-tracked `EntityMetadata` container matching MC 26.2 `MetadataDef` indices
//! (`Entity` `0..=7`, `LivingEntity` `8..=14`, `Avatar` `15..=16`, `Player` `17..=20`, `Mob` `15`).

use std::collections::{BTreeMap, BTreeSet};

use minestom_core::MainHand;
use minestom_protocol::entity::{EntityPose, MetadataValue};
use minestom_text::Component;

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

    /// Reads the current value at `index`, if set.
    #[must_use]
    pub fn get(&self, index: u8) -> Option<&MetadataValue> {
        self.entries.get(&index)
    }

    fn set_bit(&mut self, index: u8, mask: i8, enabled: bool) {
        let current = match self.entries.get(&index) {
            Some(MetadataValue::Byte(b)) => *b,
            _ => 0,
        };
        let next = if enabled {
            current | mask
        } else {
            current & !mask
        };
        self.set(index, MetadataValue::Byte(next));
    }

    fn get_bit(&self, index: u8, mask: i8) -> bool {
        matches!(self.entries.get(&index), Some(MetadataValue::Byte(b)) if (*b & mask) != 0)
    }

    /// Sets `IS_ON_FIRE` (`index 0`, bit `0x01`).
    pub fn set_on_fire(&mut self, on_fire: bool) {
        self.set_bit(0, 0x01, on_fire);
    }

    /// Sets `IS_CROUCHING` (`index 0`, bit `0x02`).
    pub fn set_sneaking(&mut self, sneaking: bool) {
        self.set_bit(0, 0x02, sneaking);
    }

    /// Returns `true` if `IS_CROUCHING` is set.
    #[must_use]
    pub fn is_sneaking(&self) -> bool {
        self.get_bit(0, 0x02)
    }

    /// Sets `IS_SPRINTING` (`index 0`, bit `0x08`).
    pub fn set_sprinting(&mut self, sprinting: bool) {
        self.set_bit(0, 0x08, sprinting);
    }

    /// Returns `true` if `IS_SPRINTING` is set.
    #[must_use]
    pub fn is_sprinting(&self) -> bool {
        self.get_bit(0, 0x08)
    }

    /// Sets `IS_INVISIBLE` (`index 0`, bit `0x20`).
    pub fn set_invisible(&mut self, invisible: bool) {
        self.set_bit(0, 0x20, invisible);
    }

    /// Sets `HAS_GLOWING_EFFECT` (`index 0`, bit `0x40`).
    pub fn set_glowing(&mut self, glowing: bool) {
        self.set_bit(0, 0x40, glowing);
    }

    /// Sets `CUSTOM_NAME` (`index 2`, `OptComponent`).
    pub fn set_custom_name(&mut self, name: Option<Component>) {
        self.set(2, MetadataValue::OptComponent(name));
    }

    /// Sets `CUSTOM_NAME_VISIBLE` (`index 3`, `Boolean`).
    pub fn set_custom_name_visible(&mut self, visible: bool) {
        self.set(3, MetadataValue::Boolean(visible));
    }

    /// Sets `IS_SILENT` (`index 4`, `Boolean`).
    pub fn set_silent(&mut self, silent: bool) {
        self.set(4, MetadataValue::Boolean(silent));
    }

    /// Sets `HAS_NO_GRAVITY` (`index 5`, `Boolean`).
    pub fn set_no_gravity(&mut self, no_gravity: bool) {
        self.set(5, MetadataValue::Boolean(no_gravity));
    }

    /// Returns `true` if `HAS_NO_GRAVITY` (`index 5`) is enabled.
    #[must_use]
    pub fn has_no_gravity(&self) -> bool {
        matches!(self.entries.get(&5), Some(MetadataValue::Boolean(true)))
    }

    /// Sets `POSE` (`index 6`, `EntityPose`).
    pub fn set_pose(&mut self, pose: EntityPose) {
        self.set(6, MetadataValue::Pose(pose));
    }

    /// Sets `LivingEntity::HEALTH` (`index 9`, `Float`).
    pub fn set_health(&mut self, health: f32) {
        self.set(9, MetadataValue::Float(health));
    }

    /// Sets `Avatar::MAIN_HAND` (`index 15`, `MainHand` in MC 26.2).
    pub fn set_main_hand(&mut self, hand: MainHand) {
        self.set(15, MetadataValue::MainHand(hand));
    }

    /// Sets `Avatar::DISPLAYED_MODEL_PARTS_FLAGS` (`index 16`, `Byte` in MC 26.2).
    pub fn set_displayed_skin_parts(&mut self, flags: u8) {
        self.set(16, MetadataValue::Byte(flags as i8));
    }

    /// Sets `Mob::NO_AI` (`index 15`, bit `0x01`).
    pub fn set_no_ai(&mut self, no_ai: bool) {
        self.set_bit(15, 0x01, no_ai);
    }

    /// Sets `Mob::IS_AGGRESSIVE` (`index 15`, bit `0x04`).
    pub fn set_aggressive(&mut self, aggressive: bool) {
        self.set_bit(15, 0x04, aggressive);
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
