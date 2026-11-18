//! Entity `Attribute`, `AttributeModifier`, `AttributeInstance`, and sparse `AttributeMap`
//! (`net.minestom.server.entity.attribute` equivalent for MC 26.2).

use std::collections::BTreeMap;

use minestom_data::generated::attributes;
use minestom_protocol::entity::{AttributeProperty, AttributeWireModifier, AttributeWireOperation};

use crate::EntityError;

/// Static specification of a vanilla MC 26.2 entity attribute (`attribute.json`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttributeSpec {
    /// Numeric protocol ID (`0..40`).
    pub id: u32,
    /// Canonical `minecraft:...` resource key.
    pub key: &'static str,
    /// Default base value.
    pub default_value: f64,
    /// Minimum clamped final value.
    pub min_value: f64,
    /// Maximum clamped final value.
    pub max_value: f64,
    /// Whether changes are synchronized to clients via `EntityAttributesPacket`.
    pub client_sync: bool,
}

/// Complete MC 26.2 attribute specification table (`40` entries, indexed by protocol ID).
pub static ATTRIBUTE_SPECS: [AttributeSpec; attributes::ATTRIBUTE_COUNT] = [
    AttributeSpec {
        id: 0,
        key: "minecraft:air_drag_modifier",
        default_value: 1.0,
        min_value: 0.0,
        max_value: 2048.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 1,
        key: "minecraft:armor",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 30.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 2,
        key: "minecraft:armor_toughness",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 20.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 3,
        key: "minecraft:attack_damage",
        default_value: 2.0,
        min_value: 0.0,
        max_value: 2048.0,
        client_sync: false,
    },
    AttributeSpec {
        id: 4,
        key: "minecraft:attack_knockback",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 5.0,
        client_sync: false,
    },
    AttributeSpec {
        id: 5,
        key: "minecraft:attack_speed",
        default_value: 4.0,
        min_value: 0.0,
        max_value: 1024.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 6,
        key: "minecraft:below_name_distance",
        default_value: 10.0,
        min_value: 0.0,
        max_value: 512.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 7,
        key: "minecraft:block_break_speed",
        default_value: 1.0,
        min_value: 0.0,
        max_value: 1024.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 8,
        key: "minecraft:block_interaction_range",
        default_value: 4.5,
        min_value: 0.0,
        max_value: 64.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 9,
        key: "minecraft:bounciness",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 1.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 10,
        key: "minecraft:burning_time",
        default_value: 1.0,
        min_value: 0.0,
        max_value: 1024.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 11,
        key: "minecraft:camera_distance",
        default_value: 4.0,
        min_value: 0.0,
        max_value: 32.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 12,
        key: "minecraft:explosion_knockback_resistance",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 1.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 13,
        key: "minecraft:entity_interaction_range",
        default_value: 3.0,
        min_value: 0.0,
        max_value: 64.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 14,
        key: "minecraft:fall_damage_multiplier",
        default_value: 1.0,
        min_value: 0.0,
        max_value: 100.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 15,
        key: "minecraft:flying_speed",
        default_value: 0.4,
        min_value: 0.0,
        max_value: 1024.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 16,
        key: "minecraft:follow_range",
        default_value: 32.0,
        min_value: 0.0,
        max_value: 2048.0,
        client_sync: false,
    },
    AttributeSpec {
        id: 17,
        key: "minecraft:friction_modifier",
        default_value: 1.0,
        min_value: 0.0,
        max_value: 2048.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 18,
        key: "minecraft:gravity",
        default_value: 0.08,
        min_value: -1.0,
        max_value: 1.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 19,
        key: "minecraft:jump_strength",
        default_value: 0.42,
        min_value: 0.0,
        max_value: 32.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 20,
        key: "minecraft:knockback_resistance",
        default_value: 0.0,
        min_value: -2.0,
        max_value: 1.0,
        client_sync: false,
    },
    AttributeSpec {
        id: 21,
        key: "minecraft:luck",
        default_value: 0.0,
        min_value: -1024.0,
        max_value: 1024.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 22,
        key: "minecraft:max_absorption",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 2048.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 23,
        key: "minecraft:max_health",
        default_value: 20.0,
        min_value: 1.0,
        max_value: 1024.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 24,
        key: "minecraft:mining_efficiency",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 1024.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 25,
        key: "minecraft:movement_efficiency",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 1.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 26,
        key: "minecraft:movement_speed",
        default_value: 0.7,
        min_value: 0.0,
        max_value: 1024.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 27,
        key: "minecraft:name_tag_distance",
        default_value: 64.0,
        min_value: 0.0,
        max_value: 512.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 28,
        key: "minecraft:oxygen_bonus",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 1024.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 29,
        key: "minecraft:safe_fall_distance",
        default_value: 3.0,
        min_value: -1024.0,
        max_value: 1024.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 30,
        key: "minecraft:scale",
        default_value: 1.0,
        min_value: 0.0625,
        max_value: 16.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 31,
        key: "minecraft:sneaking_speed",
        default_value: 0.3,
        min_value: 0.0,
        max_value: 1.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 32,
        key: "minecraft:spawn_reinforcements",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 1.0,
        client_sync: false,
    },
    AttributeSpec {
        id: 33,
        key: "minecraft:step_height",
        default_value: 0.6,
        min_value: 0.0,
        max_value: 10.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 34,
        key: "minecraft:submerged_mining_speed",
        default_value: 0.2,
        min_value: 0.0,
        max_value: 20.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 35,
        key: "minecraft:sweeping_damage_ratio",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 1.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 36,
        key: "minecraft:tempt_range",
        default_value: 10.0,
        min_value: 0.0,
        max_value: 2048.0,
        client_sync: false,
    },
    AttributeSpec {
        id: 37,
        key: "minecraft:water_movement_efficiency",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 1.0,
        client_sync: true,
    },
    AttributeSpec {
        id: 38,
        key: "minecraft:waypoint_transmit_range",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 60_000_000.0,
        client_sync: false,
    },
    AttributeSpec {
        id: 39,
        key: "minecraft:waypoint_receive_range",
        default_value: 0.0,
        min_value: 0.0,
        max_value: 60_000_000.0,
        client_sync: false,
    },
];

/// Strongly typed `Copy` handle for a Minecraft 26.2 entity `Attribute` (`minecraft:attribute`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Attribute(u32);

impl Attribute {
    /// `minecraft:armor` (`1`).
    pub const ARMOR: Self = Self(attributes::ARMOR);
    /// `minecraft:armor_toughness` (`2`).
    pub const ARMOR_TOUGHNESS: Self = Self(attributes::ARMOR_TOUGHNESS);
    /// `minecraft:attack_damage` (`3`).
    pub const ATTACK_DAMAGE: Self = Self(attributes::ATTACK_DAMAGE);
    /// `minecraft:attack_knockback` (`4`).
    pub const ATTACK_KNOCKBACK: Self = Self(attributes::ATTACK_KNOCKBACK);
    /// `minecraft:attack_speed` (`5`).
    pub const ATTACK_SPEED: Self = Self(attributes::ATTACK_SPEED);
    /// `minecraft:block_break_speed` (`7`).
    pub const BLOCK_BREAK_SPEED: Self = Self(attributes::BLOCK_BREAK_SPEED);
    /// `minecraft:block_interaction_range` (`8`).
    pub const BLOCK_INTERACTION_RANGE: Self = Self(attributes::BLOCK_INTERACTION_RANGE);
    /// `minecraft:entity_interaction_range` (`13`).
    pub const ENTITY_INTERACTION_RANGE: Self = Self(attributes::ENTITY_INTERACTION_RANGE);
    /// `minecraft:fall_damage_multiplier` (`14`).
    pub const FALL_DAMAGE_MULTIPLIER: Self = Self(attributes::FALL_DAMAGE_MULTIPLIER);
    /// `minecraft:flying_speed` (`15`).
    pub const FLYING_SPEED: Self = Self(attributes::FLYING_SPEED);
    /// `minecraft:follow_range` (`16`).
    pub const FOLLOW_RANGE: Self = Self(attributes::FOLLOW_RANGE);
    /// `minecraft:gravity` (`18`).
    pub const GRAVITY: Self = Self(attributes::GRAVITY);
    /// `minecraft:jump_strength` (`19`).
    pub const JUMP_STRENGTH: Self = Self(attributes::JUMP_STRENGTH);
    /// `minecraft:knockback_resistance` (`20`).
    pub const KNOCKBACK_RESISTANCE: Self = Self(attributes::KNOCKBACK_RESISTANCE);
    /// `minecraft:luck` (`21`).
    pub const LUCK: Self = Self(attributes::LUCK);
    /// `minecraft:max_absorption` (`22`).
    pub const MAX_ABSORPTION: Self = Self(attributes::MAX_ABSORPTION);
    /// `minecraft:max_health` (`23`).
    pub const MAX_HEALTH: Self = Self(attributes::MAX_HEALTH);
    /// `minecraft:movement_speed` (`26`).
    pub const MOVEMENT_SPEED: Self = Self(attributes::MOVEMENT_SPEED);
    /// `minecraft:safe_fall_distance` (`29`).
    pub const SAFE_FALL_DISTANCE: Self = Self(attributes::SAFE_FALL_DISTANCE);
    /// `minecraft:scale` (`30`).
    pub const SCALE: Self = Self(attributes::SCALE);
    /// `minecraft:step_height` (`33`).
    pub const STEP_HEIGHT: Self = Self(attributes::STEP_HEIGHT);

    /// Constructs an [`Attribute`] from a validated protocol ID (`0..40`).
    pub const fn from_id(id: u32) -> Result<Self, EntityError> {
        if (id as usize) < ATTRIBUTE_SPECS.len() {
            Ok(Self(id))
        } else {
            Err(EntityError::UnknownAttribute(id))
        }
    }

    /// Looks up an [`Attribute`] by its `minecraft:...` resource key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        ATTRIBUTE_SPECS
            .iter()
            .find(|s| s.key == key)
            .map(|s| Self(s.id))
    }

    /// Numeric protocol ID.
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Static [`AttributeSpec`] entry.
    #[must_use]
    pub const fn spec(self) -> &'static AttributeSpec {
        &ATTRIBUTE_SPECS[self.0 as usize]
    }

    /// Canonical `minecraft:...` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.spec().key
    }

    /// Default vanilla base value.
    #[must_use]
    pub const fn default_value(self) -> f64 {
        self.spec().default_value
    }

    /// Whether clients receive `EntityAttributesPacket` updates for this attribute.
    #[must_use]
    pub const fn is_client_synced(self) -> bool {
        self.spec().client_sync
    }
}

/// Modifier stage applied when computing an [`AttributeInstance`]'s final value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttributeOperation {
    /// `ADD_VALUE`: adds `amount` directly to `base_value`.
    AddValue,
    /// `ADD_MULTIPLIED_BASE`: adds `sum_after_add * amount`.
    AddMultipliedBase,
    /// `ADD_MULTIPLIED_TOTAL`: multiplies the running total by `(1.0 + amount)`.
    AddMultipliedTotal,
}

impl From<AttributeOperation> for AttributeWireOperation {
    fn from(op: AttributeOperation) -> Self {
        match op {
            AttributeOperation::AddValue => Self::AddValue,
            AttributeOperation::AddMultipliedBase => Self::AddMultipliedBase,
            AttributeOperation::AddMultipliedTotal => Self::AddMultipliedTotal,
        }
    }
}

impl From<AttributeWireOperation> for AttributeOperation {
    fn from(op: AttributeWireOperation) -> Self {
        match op {
            AttributeWireOperation::AddValue => Self::AddValue,
            AttributeWireOperation::AddMultipliedBase => Self::AddMultipliedBase,
            AttributeWireOperation::AddMultipliedTotal => Self::AddMultipliedTotal,
        }
    }
}

/// Named scalar modifier attached to an [`AttributeInstance`].
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeModifier {
    /// Namespaced modifier identifier (e.g. `"minecraft:sprinting"`).
    pub id: String,
    /// Modifier amount.
    pub amount: f64,
    /// Operation stage.
    pub operation: AttributeOperation,
}

impl AttributeModifier {
    /// Constructs a new [`AttributeModifier`].
    #[must_use]
    pub fn new(id: impl Into<String>, amount: f64, operation: AttributeOperation) -> Self {
        Self {
            id: id.into(),
            amount,
            operation,
        }
    }

    /// Converts to protocol [`AttributeWireModifier`].
    #[must_use]
    pub fn to_wire(&self) -> AttributeWireModifier {
        AttributeWireModifier {
            id: self.id.clone(),
            amount: self.amount,
            operation: self.operation.into(),
        }
    }
}

/// Mutable state of a single [`Attribute`] on an entity (`AttributeInstance` in Minestom).
#[derive(Debug, Clone)]
pub struct AttributeInstance {
    attribute: Attribute,
    base_value: f64,
    modifiers: BTreeMap<String, AttributeModifier>,
    cached_value: f64,
    dirty: bool,
}

impl AttributeInstance {
    /// Creates an [`AttributeInstance`] initialized to `attribute.default_value()`.
    #[must_use]
    pub fn new(attribute: Attribute) -> Self {
        let def = attribute.default_value();
        Self {
            attribute,
            base_value: def,
            modifiers: BTreeMap::new(),
            cached_value: def,
            dirty: false,
        }
    }

    /// Associated [`Attribute`].
    #[must_use]
    pub const fn attribute(&self) -> Attribute {
        self.attribute
    }

    /// Unmodified base value.
    #[must_use]
    pub const fn base_value(&self) -> f64 {
        self.base_value
    }

    /// Sets the base value and recomputes the clamped final value.
    pub fn set_base_value(&mut self, value: f64) {
        if (self.base_value - value).abs() > f64::EPSILON {
            self.base_value = value;
            self.recompute();
            self.dirty = true;
        }
    }

    /// Final computed & clamped attribute value.
    #[must_use]
    pub const fn value(&self) -> f64 {
        self.cached_value
    }

    /// Inserts or replaces a modifier by `modifier.id` and recomputes the final value.
    pub fn add_modifier(&mut self, modifier: AttributeModifier) {
        self.modifiers.insert(modifier.id.clone(), modifier);
        self.recompute();
        self.dirty = true;
    }

    /// Removes a modifier by `id`, returning `true` if one was present.
    pub fn remove_modifier(&mut self, id: &str) -> bool {
        if self.modifiers.remove(id).is_some() {
            self.recompute();
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Clears all modifiers on this attribute.
    pub fn clear_modifiers(&mut self) {
        if !self.modifiers.is_empty() {
            self.modifiers.clear();
            self.recompute();
            self.dirty = true;
        }
    }

    /// Active modifiers on this instance.
    pub fn modifiers(&self) -> impl Iterator<Item = &AttributeModifier> {
        self.modifiers.values()
    }

    /// Builds a wire [`AttributeProperty`] snapshot for `EntityAttributesPacket`.
    #[must_use]
    pub fn to_wire_property(&self) -> AttributeProperty {
        AttributeProperty {
            attribute_id: self.attribute.id() as i32,
            base_value: self.base_value,
            modifiers: self
                .modifiers
                .values()
                .map(AttributeModifier::to_wire)
                .collect(),
        }
    }

    fn recompute(&mut self) {
        let mut total = self.base_value;
        for m in self.modifiers.values() {
            if m.operation == AttributeOperation::AddValue {
                total += m.amount;
            }
        }
        let after_add = total;
        let mut mult_base = 0.0;
        for m in self.modifiers.values() {
            if m.operation == AttributeOperation::AddMultipliedBase {
                mult_base += m.amount;
            }
        }
        total = after_add * (1.0 + mult_base);
        for m in self.modifiers.values() {
            if m.operation == AttributeOperation::AddMultipliedTotal {
                total *= 1.0 + m.amount;
            }
        }
        let spec = self.attribute.spec();
        self.cached_value = total.clamp(spec.min_value, spec.max_value);
    }
}

/// Sparse per-entity map of modified [`AttributeInstance`]s.
#[derive(Debug, Clone, Default)]
pub struct AttributeMap {
    instances: BTreeMap<u32, AttributeInstance>,
}

impl AttributeMap {
    /// Creates an empty sparse [`AttributeMap`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            instances: BTreeMap::new(),
        }
    }

    /// Creates an [`AttributeMap`] pre-configured with vanilla player movement speed (`0.1`).
    #[must_use]
    pub fn for_player() -> Self {
        let mut map = Self::new();
        map.set_base_value(Attribute::MOVEMENT_SPEED, 0.1);
        map.clear_dirty();
        map
    }

    /// Returns the computed value of `attribute` (falling back to `attribute.default_value()`).
    #[must_use]
    pub fn get_value(&self, attribute: Attribute) -> f64 {
        self.instances
            .get(&attribute.id())
            .map_or_else(|| attribute.default_value(), AttributeInstance::value)
    }

    /// Returns an immutable reference to `attribute`'s instance if customized.
    #[must_use]
    pub fn get(&self, attribute: Attribute) -> Option<&AttributeInstance> {
        self.instances.get(&attribute.id())
    }

    /// Returns a mutable reference to `attribute`'s [`AttributeInstance`], initializing on demand.
    pub fn instance_mut(&mut self, attribute: Attribute) -> &mut AttributeInstance {
        self.instances
            .entry(attribute.id())
            .or_insert_with(|| AttributeInstance::new(attribute))
    }

    /// Sets the base value of `attribute`.
    pub fn set_base_value(&mut self, attribute: Attribute, value: f64) {
        self.instance_mut(attribute).set_base_value(value);
    }

    /// Adds or replaces a modifier on `attribute`.
    pub fn add_modifier(&mut self, attribute: Attribute, modifier: AttributeModifier) {
        self.instance_mut(attribute).add_modifier(modifier);
    }

    /// Removes a modifier by `id` from `attribute`.
    pub fn remove_modifier(&mut self, attribute: Attribute, id: &str) -> bool {
        self.instances
            .get_mut(&attribute.id())
            .is_some_and(|inst| inst.remove_modifier(id))
    }

    /// Clears all dirty flags without emitting packets.
    pub fn clear_dirty(&mut self) {
        for inst in self.instances.values_mut() {
            inst.dirty = false;
        }
    }

    /// Drains all dirty client-synced [`AttributeProperty`] entries for `EntityAttributesPacket`.
    pub fn take_dirty_wire_properties(&mut self) -> Vec<AttributeProperty> {
        let mut out = Vec::new();
        for inst in self.instances.values_mut() {
            if inst.dirty {
                inst.dirty = false;
                if inst.attribute.is_client_synced() {
                    out.push(inst.to_wire_property());
                }
            }
        }
        out
    }

    /// Collects all customized client-synced [`AttributeProperty`] entries for a new viewer.
    #[must_use]
    pub fn all_synced_wire_properties(&self) -> Vec<AttributeProperty> {
        self.instances
            .values()
            .filter(|inst| inst.attribute.is_client_synced())
            .map(AttributeInstance::to_wire_property)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_three_stage_modifier_formula_and_clamping() {
        let mut map = AttributeMap::new();
        map.set_base_value(Attribute::MAX_HEALTH, 20.0);
        map.add_modifier(
            Attribute::MAX_HEALTH,
            AttributeModifier::new("test:flat", 4.0, AttributeOperation::AddValue),
        );
        map.add_modifier(
            Attribute::MAX_HEALTH,
            AttributeModifier::new("test:base_pct", 0.5, AttributeOperation::AddMultipliedBase),
        );
        map.add_modifier(
            Attribute::MAX_HEALTH,
            AttributeModifier::new(
                "test:total_pct",
                0.25,
                AttributeOperation::AddMultipliedTotal,
            ),
        );
        // (20 + 4) * (1 + 0.5) * (1 + 0.25) = 24 * 1.5 * 1.25 = 45.0
        assert!((map.get_value(Attribute::MAX_HEALTH) - 45.0).abs() < 1e-6);

        let dirty = map.take_dirty_wire_properties();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].modifiers.len(), 3);
        assert_eq!(map.take_dirty_wire_properties().len(), 0);

        // Clamping: ARMOR max is 30.0
        map.set_base_value(Attribute::ARMOR, 999.0);
        assert!((map.get_value(Attribute::ARMOR) - 30.0).abs() < 1e-6);
    }
}
