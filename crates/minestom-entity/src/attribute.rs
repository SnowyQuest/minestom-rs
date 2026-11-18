//! Entity `Attribute`, `AttributeModifier`, `AttributeInstance`, and sparse `AttributeMap`
//! (`net.minestom.server.entity.attribute` equivalent for MC 26.2, `40` generated attributes).

use std::collections::BTreeMap;
use std::fmt;

use minestom_data::generated::attributes;
use minestom_protocol::entity::{AttributeProperty, AttributeWireModifier, AttributeWireOperation};

pub use minestom_data::generated::attributes::{
    ATTRIBUTE_SPECS, AttributeSpecInfo as AttributeSpec,
};

use crate::EntityError;

/// Strongly typed `Copy` handle for all **40** Minecraft 26.2 entity `Attribute`s (`minecraft:attribute`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Attribute(u32);

minestom_data::impl_attribute_constants!(Attribute);

impl Attribute {
    /// Constructs an [`Attribute`] from a validated protocol ID (`0..40`).
    pub const fn from_id(id: u32) -> Result<Self, EntityError> {
        if (id as usize) < ATTRIBUTE_SPECS.len() {
            Ok(Self(id))
        } else {
            Err(EntityError::UnknownAttribute(id))
        }
    }

    /// Zero-allocation `O(log N)` lookup by `"minecraft:..."` or bare `"..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        attributes::lookup_key(key).map(Self)
    }

    /// Numeric protocol ID (`0..40`).
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Static generated [`AttributeSpec`] entry.
    #[must_use]
    pub const fn spec(self) -> &'static AttributeSpec {
        &ATTRIBUTE_SPECS[self.0 as usize]
    }

    /// Canonical `"minecraft:..."` key.
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

impl fmt::Display for Attribute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
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

    /// Creates an [`AttributeMap`] populated from `default_attributes` `(attribute_id, base_value)`
    /// generated from `entity_type.json`.
    #[must_use]
    pub fn from_defaults(defaults: &[(u16, f64)]) -> Self {
        let mut map = Self::new();
        for &(aid, base) in defaults {
            if let Ok(attr) = Attribute::from_id(u32::from(aid)) {
                map.set_base_value(attr, base);
            }
        }
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

    /// Iterates over all customized [`AttributeInstance`]s.
    pub fn iter(&self) -> impl Iterator<Item = &AttributeInstance> {
        self.instances.values()
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

impl<'a> IntoIterator for &'a AttributeMap {
    type Item = &'a AttributeInstance;
    type IntoIter = std::collections::btree_map::Values<'a, u32, AttributeInstance>;

    fn into_iter(self) -> Self::IntoIter {
        self.instances.values()
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
        assert!((map.get_value(Attribute::MAX_HEALTH) - 45.0).abs() < 1e-6);

        let dirty = map.take_dirty_wire_properties();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].modifiers.len(), 3);
        assert_eq!(map.take_dirty_wire_properties().len(), 0);

        map.set_base_value(Attribute::ARMOR, 999.0);
        assert!((map.get_value(Attribute::ARMOR) - 30.0).abs() < 1e-6);
    }
}
