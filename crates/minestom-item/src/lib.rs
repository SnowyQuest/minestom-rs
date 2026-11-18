//! `Material`, `ItemStack` with MC 26.2 Data Components, `PlayerInventory`, custom container
//! `Inventory` windows, `InventoryClickProcessor`, and item/inventory events.
//!
//! ```
//! use minestom_item::{ItemStack, Material};
//! use minestom_text::Component;
//!
//! let sword = ItemStack::single(Material::DIAMOND_SWORD)
//!     .with_custom_name(Component::text("Excalibur"))
//!     .with_unbreakable()
//!     .with_glowing(true);
//! assert_eq!(sword.max_stack_size(), 1);
//! assert!(sword.is_unbreakable());
//! ```

pub mod inventory;

use std::collections::{BTreeMap, BTreeSet};

use minestom_block::Block;
use minestom_core::{InstanceId, PlayerId, Tag, TagMap, tag::TagSerializable};
use minestom_data::generated::materials::{self, MATERIAL_INFOS, MaterialInfo};
use minestom_event::{CancellableEvent, Event, InstanceEvent, PlayerEvent};
use minestom_nbt::NbtCompound;
use minestom_text::Component;

pub use inventory::{
    ClickOutcome, Inventory, InventoryType, PlayerInventory, WindowClickMode,
    process_inventory_click,
};

/// Errors returned by item and inventory operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ItemError {
    /// Slot index out of bounds for an inventory.
    #[error("inventory slot {slot} out of bounds (size {size})")]
    SlotOutOfBounds {
        /// Requested slot index.
        slot: usize,
        /// Total slot capacity.
        size: usize,
    },
    /// Unknown vanilla item material protocol ID.
    #[error("unknown item material ID: {0}")]
    UnknownMaterialId(u32),
}

/// Strongly typed `Copy` handle for a Minecraft 26.2 `Material` (`minecraft:item`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Material(u32);

impl Material {
    /// `minecraft:air` (`id = 0`).
    pub const AIR: Self = Self(materials::AIR);
    /// `minecraft:stone`.
    pub const STONE: Self = Self(materials::STONE);
    /// `minecraft:grass_block`.
    pub const GRASS_BLOCK: Self = Self(materials::GRASS_BLOCK);
    /// `minecraft:dirt`.
    pub const DIRT: Self = Self(materials::DIRT);
    /// `minecraft:cobblestone`.
    pub const COBBLESTONE: Self = Self(materials::COBBLESTONE);
    /// `minecraft:oak_planks`.
    pub const OAK_PLANKS: Self = Self(materials::OAK_PLANKS);
    /// `minecraft:oak_log`.
    pub const OAK_LOG: Self = Self(materials::OAK_LOG);
    /// `minecraft:glass`.
    pub const GLASS: Self = Self(materials::GLASS);
    /// `minecraft:chest`.
    pub const CHEST: Self = Self(materials::CHEST);
    /// `minecraft:crafting_table`.
    pub const CRAFTING_TABLE: Self = Self(materials::CRAFTING_TABLE);
    /// `minecraft:furnace`.
    pub const FURNACE: Self = Self(materials::FURNACE);
    /// `minecraft:torch`.
    pub const TORCH: Self = Self(materials::TORCH);
    /// `minecraft:diamond`.
    pub const DIAMOND: Self = Self(materials::DIAMOND);
    /// `minecraft:iron_ingot`.
    pub const IRON_INGOT: Self = Self(materials::IRON_INGOT);
    /// `minecraft:gold_ingot`.
    pub const GOLD_INGOT: Self = Self(materials::GOLD_INGOT);
    /// `minecraft:emerald`.
    pub const EMERALD: Self = Self(materials::EMERALD);
    /// `minecraft:stick`.
    pub const STICK: Self = Self(materials::STICK);
    /// `minecraft:apple`.
    pub const APPLE: Self = Self(materials::APPLE);
    /// `minecraft:golden_apple`.
    pub const GOLDEN_APPLE: Self = Self(materials::GOLDEN_APPLE);
    /// `minecraft:bread`.
    pub const BREAD: Self = Self(materials::BREAD);
    /// `minecraft:cooked_beef`.
    pub const COOKED_BEEF: Self = Self(materials::COOKED_BEEF);
    /// `minecraft:diamond_sword`.
    pub const DIAMOND_SWORD: Self = Self(materials::DIAMOND_SWORD);
    /// `minecraft:iron_sword`.
    pub const IRON_SWORD: Self = Self(materials::IRON_SWORD);
    /// `minecraft:diamond_pickaxe`.
    pub const DIAMOND_PICKAXE: Self = Self(materials::DIAMOND_PICKAXE);
    /// `minecraft:bow`.
    pub const BOW: Self = Self(materials::BOW);
    /// `minecraft:arrow`.
    pub const ARROW: Self = Self(materials::ARROW);
    /// `minecraft:shield`.
    pub const SHIELD: Self = Self(materials::SHIELD);
    /// `minecraft:ender_pearl`.
    pub const ENDER_PEARL: Self = Self(materials::ENDER_PEARL);
    /// `minecraft:diamond_helmet`.
    pub const DIAMOND_HELMET: Self = Self(materials::DIAMOND_HELMET);
    /// `minecraft:diamond_chestplate`.
    pub const DIAMOND_CHESTPLATE: Self = Self(materials::DIAMOND_CHESTPLATE);
    /// `minecraft:diamond_leggings`.
    pub const DIAMOND_LEGGINGS: Self = Self(materials::DIAMOND_LEGGINGS);
    /// `minecraft:diamond_boots`.
    pub const DIAMOND_BOOTS: Self = Self(materials::DIAMOND_BOOTS);

    /// Constructs a `Material` from a validated protocol ID.
    pub const fn from_id(id: u32) -> Result<Self, ItemError> {
        if (id as usize) < MATERIAL_INFOS.len() {
            Ok(Self(id))
        } else {
            Err(ItemError::UnknownMaterialId(id))
        }
    }

    /// Looks up a `Material` by its `"minecraft:..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        let full = if key.contains(':') {
            key.to_owned()
        } else {
            format!("minecraft:{key}")
        };
        MATERIAL_INFOS
            .iter()
            .find(|info| info.key == full)
            .map(|info| Self(info.id))
    }

    /// Numeric protocol ID.
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Generated [`MaterialInfo`] metadata.
    #[must_use]
    pub const fn info(self) -> &'static MaterialInfo {
        &MATERIAL_INFOS[self.0 as usize]
    }

    /// Canonical `"minecraft:..."` resource key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.info().key
    }

    /// Default max stack size (`1`, `16`, or `64`).
    #[must_use]
    pub const fn default_max_stack_size(self) -> u32 {
        self.info().max_stack_size as u32
    }

    /// Default durability (`0` if not damageable).
    #[must_use]
    pub const fn default_max_damage(self) -> u32 {
        self.info().max_damage as u32
    }

    /// Corresponding [`Block`] placed by this item, if any.
    #[must_use]
    pub fn corresponding_block(self) -> Option<Block> {
        self.info().corresponding_block.and_then(Block::from_key)
    }

    /// Returns `true` if this material is `minecraft:air` (`0`).
    #[must_use]
    pub const fn is_air(self) -> bool {
        self.0 == 0
    }
}

/// Equipment slot discriminant (`EquipmentSlot.legacyProtocolId` in `EntityEquipmentPacket`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum EquipmentSlot {
    /// `0` — Main hand.
    MainHand = 0,
    /// `1` — Off hand.
    OffHand = 1,
    /// `2` — Boots.
    Boots = 2,
    /// `3` — Leggings.
    Leggings = 3,
    /// `4` — Chestplate.
    Chestplate = 4,
    /// `5` — Helmet.
    Helmet = 5,
    /// `6` — Body (wolf/horse armor).
    Body = 6,
    /// `7` — Saddle.
    Saddle = 7,
}

/// Item rarity (`DataComponents.RARITY`, ID `12` in MC 26.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ItemRarity {
    /// `0` — White.
    Common = 0,
    /// `1` — Yellow.
    Uncommon = 1,
    /// `2` — Aqua.
    Rare = 2,
    /// `3` — Light purple.
    Epic = 3,
}

/// Protocol IDs for MC 26.2 Data Components (`DataComponents.java` registration order).
pub mod component_ids {
    /// `0` — `minecraft:custom_data`.
    pub const CUSTOM_DATA: i32 = 0;
    /// `1` — `minecraft:max_stack_size`.
    pub const MAX_STACK_SIZE: i32 = 1;
    /// `2` — `minecraft:max_damage`.
    pub const MAX_DAMAGE: i32 = 2;
    /// `3` — `minecraft:damage`.
    pub const DAMAGE: i32 = 3;
    /// `4` — `minecraft:unbreakable`.
    pub const UNBREAKABLE: i32 = 4;
    /// `6` — `minecraft:custom_name`.
    pub const CUSTOM_NAME: i32 = 6;
    /// `9` — `minecraft:item_name`.
    pub const ITEM_NAME: i32 = 9;
    /// `10` — `minecraft:item_model`.
    pub const ITEM_MODEL: i32 = 10;
    /// `11` — `minecraft:lore`.
    pub const LORE: i32 = 11;
    /// `12` — `minecraft:rarity`.
    pub const RARITY: i32 = 12;
    /// `13` — `minecraft:enchantments`.
    pub const ENCHANTMENTS: i32 = 13;
    /// `19` — `minecraft:repair_cost`.
    pub const REPAIR_COST: i32 = 19;
    /// `20` — `minecraft:creative_slot_lock`.
    pub const CREATIVE_SLOT_LOCK: i32 = 20;
    /// `21` — `minecraft:enchantment_glint_override`.
    pub const ENCHANTMENT_GLINT_OVERRIDE: i32 = 21;
}

/// Typed MC 26.2 item data component value (`DataComponent.Value` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub enum DataComponentValue {
    /// `0` — `minecraft:custom_data` (`NbtCompound`).
    CustomData(NbtCompound),
    /// `1` — `minecraft:max_stack_size` (`1..=99`).
    MaxStackSize(i32),
    /// `2` — `minecraft:max_damage`.
    MaxDamage(i32),
    /// `3` — `minecraft:damage`.
    Damage(i32),
    /// `4` — `minecraft:unbreakable` (`Unit`).
    Unbreakable,
    /// `6` — `minecraft:custom_name` (`Component`).
    CustomName(Component),
    /// `9` — `minecraft:item_name` (`Component`).
    ItemName(Component),
    /// `10` — `minecraft:item_model` (`String`).
    ItemModel(String),
    /// `11` — `minecraft:lore` (`Vec<Component>`).
    Lore(Vec<Component>),
    /// `12` — `minecraft:rarity` (`ItemRarity`).
    Rarity(ItemRarity),
    /// `13` — `minecraft:enchantments` (`enchantment_id -> level`).
    Enchantments(BTreeMap<i32, i32>),
    /// `19` — `minecraft:repair_cost`.
    RepairCost(i32),
    /// `20` — `minecraft:creative_slot_lock` (`Unit`).
    CreativeSlotLock,
    /// `21` — `minecraft:enchantment_glint_override`.
    EnchantmentGlintOverride(bool),
    /// Length-prefixed raw payload preserved verbatim from creative inventory packets.
    RawOpaque {
        /// Component protocol ID.
        id: i32,
        /// Raw payload bytes.
        bytes: Vec<u8>,
    },
}

impl DataComponentValue {
    /// Returns the MC 26.2 protocol component ID.
    #[must_use]
    pub const fn component_id(&self) -> i32 {
        match self {
            Self::CustomData(_) => component_ids::CUSTOM_DATA,
            Self::MaxStackSize(_) => component_ids::MAX_STACK_SIZE,
            Self::MaxDamage(_) => component_ids::MAX_DAMAGE,
            Self::Damage(_) => component_ids::DAMAGE,
            Self::Unbreakable => component_ids::UNBREAKABLE,
            Self::CustomName(_) => component_ids::CUSTOM_NAME,
            Self::ItemName(_) => component_ids::ITEM_NAME,
            Self::ItemModel(_) => component_ids::ITEM_MODEL,
            Self::Lore(_) => component_ids::LORE,
            Self::Rarity(_) => component_ids::RARITY,
            Self::Enchantments(_) => component_ids::ENCHANTMENTS,
            Self::RepairCost(_) => component_ids::REPAIR_COST,
            Self::CreativeSlotLock => component_ids::CREATIVE_SLOT_LOCK,
            Self::EnchantmentGlintOverride(_) => component_ids::ENCHANTMENT_GLINT_OVERRIDE,
            Self::RawOpaque { id, .. } => *id,
        }
    }
}

/// Immutable-style Minecraft 26.2 `ItemStack` carrying [`Material`], stack `amount`, and
/// added/removed Data Component patches (`ItemStack` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemStack {
    material: Material,
    amount: u32,
    added_components: BTreeMap<i32, DataComponentValue>,
    removed_components: BTreeSet<i32>,
}

impl Default for ItemStack {
    fn default() -> Self {
        Self::AIR
    }
}

impl ItemStack {
    /// Empty `minecraft:air` stack (`amount = 0`).
    pub const AIR: Self = Self {
        material: Material::AIR,
        amount: 0,
        added_components: BTreeMap::new(),
        removed_components: BTreeSet::new(),
    };

    /// Creates an [`ItemStack`] of `material` with `amount` (`0` or `Material::AIR` normalizes to [`Self::AIR`]).
    #[must_use]
    pub fn of(material: Material, amount: u32) -> Self {
        if amount == 0 || material.is_air() {
            Self::AIR
        } else {
            Self {
                material,
                amount,
                added_components: BTreeMap::new(),
                removed_components: BTreeSet::new(),
            }
        }
    }

    /// Creates a single-item stack (`amount = 1`).
    #[must_use]
    pub fn single(material: Material) -> Self {
        Self::of(material, 1)
    }

    /// Constructs an [`ItemStack`] from raw decoded parts.
    #[must_use]
    pub fn from_parts(
        material: Material,
        amount: u32,
        added_components: BTreeMap<i32, DataComponentValue>,
        removed_components: BTreeSet<i32>,
    ) -> Self {
        if amount == 0 || material.is_air() {
            Self::AIR
        } else {
            Self {
                material,
                amount,
                added_components,
                removed_components,
            }
        }
    }

    pub(crate) fn normalized(self) -> Self {
        if self.amount == 0 || self.material.is_air() {
            Self::AIR
        } else {
            self
        }
    }

    /// Returns `true` if this stack is empty (`AIR` or `amount == 0`).
    #[must_use]
    pub const fn is_air(&self) -> bool {
        self.amount == 0 || self.material.is_air()
    }

    /// Item [`Material`].
    #[must_use]
    pub const fn material(&self) -> Material {
        self.material
    }

    /// Stack count (`0` for [`Self::AIR`]).
    #[must_use]
    pub const fn amount(&self) -> u32 {
        self.amount
    }

    /// Borrows added component overrides (`component_id -> value`).
    #[must_use]
    pub const fn added_components(&self) -> &BTreeMap<i32, DataComponentValue> {
        &self.added_components
    }

    /// Borrows removed prototype component IDs.
    #[must_use]
    pub const fn removed_components(&self) -> &BTreeSet<i32> {
        &self.removed_components
    }

    /// Returns `true` if `self` and `other` share the exact same [`Material`] and component patch
    /// (ignoring `amount`), matching Minestom's `ItemStack::isSimilar`.
    #[must_use]
    pub fn is_similar(&self, other: &Self) -> bool {
        if self.is_air() && other.is_air() {
            return true;
        }
        self.material == other.material
            && self.added_components == other.added_components
            && self.removed_components == other.removed_components
    }

    /// Effective maximum stack size (`MAX_STACK_SIZE` override or `material.default_max_stack_size()`).
    #[must_use]
    pub fn max_stack_size(&self) -> u32 {
        match self.added_components.get(&component_ids::MAX_STACK_SIZE) {
            Some(DataComponentValue::MaxStackSize(n)) if *n > 0 => *n as u32,
            _ => self.material.default_max_stack_size(),
        }
    }

    /// Returns a copy with updated `amount` (`0` becomes [`Self::AIR`]).
    #[must_use]
    pub fn with_amount(mut self, amount: u32) -> Self {
        if amount == 0 || self.material.is_air() {
            Self::AIR
        } else {
            self.amount = amount;
            self
        }
    }

    /// Inserts or replaces a [`DataComponentValue`].
    #[must_use]
    pub fn with_component(mut self, value: DataComponentValue) -> Self {
        if self.is_air() {
            return Self::AIR;
        }
        let cid = value.component_id();
        self.removed_components.remove(&cid);
        self.added_components.insert(cid, value);
        self
    }

    /// Marks `component_id` as explicitly removed from the item's prototype.
    #[must_use]
    pub fn without_component(mut self, component_id: i32) -> Self {
        if self.is_air() {
            return Self::AIR;
        }
        self.added_components.remove(&component_id);
        self.removed_components.insert(component_id);
        self
    }

    /// Sets `minecraft:custom_name` (`6`).
    #[must_use]
    pub fn with_custom_name(self, name: impl Into<Component>) -> Self {
        self.with_component(DataComponentValue::CustomName(name.into()))
    }

    /// Reads `minecraft:custom_name`, if set.
    #[must_use]
    pub fn custom_name(&self) -> Option<&Component> {
        match self.added_components.get(&component_ids::CUSTOM_NAME) {
            Some(DataComponentValue::CustomName(c)) => Some(c),
            _ => None,
        }
    }

    /// Sets `minecraft:item_name` (`9`).
    #[must_use]
    pub fn with_item_name(self, name: impl Into<Component>) -> Self {
        self.with_component(DataComponentValue::ItemName(name.into()))
    }

    /// Sets `minecraft:lore` (`11`).
    #[must_use]
    pub fn with_lore<I, C>(self, lines: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<Component>,
    {
        self.with_component(DataComponentValue::Lore(
            lines.into_iter().map(Into::into).collect(),
        ))
    }

    /// Reads `minecraft:lore`, if set.
    #[must_use]
    pub fn lore(&self) -> &[Component] {
        match self.added_components.get(&component_ids::LORE) {
            Some(DataComponentValue::Lore(v)) => v.as_slice(),
            _ => &[],
        }
    }

    /// Overrides `minecraft:max_stack_size` (`1..=99`).
    #[must_use]
    pub fn with_max_stack_size(self, max_stack: u32) -> Self {
        self.with_component(DataComponentValue::MaxStackSize(
            max_stack.clamp(1, 99) as i32
        ))
    }

    /// Sets `minecraft:damage` (`3`).
    #[must_use]
    pub fn with_damage(self, damage: u32) -> Self {
        self.with_component(DataComponentValue::Damage(damage as i32))
    }

    /// Sets `minecraft:max_damage` (`2`).
    #[must_use]
    pub fn with_max_damage(self, max_damage: u32) -> Self {
        self.with_component(DataComponentValue::MaxDamage(max_damage as i32))
    }

    /// Marks this item unbreakable (`minecraft:unbreakable`, ID `4`).
    #[must_use]
    pub fn with_unbreakable(self) -> Self {
        self.with_component(DataComponentValue::Unbreakable)
    }

    /// Returns `true` if `minecraft:unbreakable` is set.
    #[must_use]
    pub fn is_unbreakable(&self) -> bool {
        matches!(
            self.added_components.get(&component_ids::UNBREAKABLE),
            Some(DataComponentValue::Unbreakable)
        )
    }

    /// Overrides enchantment glint (`minecraft:enchantment_glint_override`, ID `21`).
    #[must_use]
    pub fn with_glowing(self, glowing: bool) -> Self {
        self.with_component(DataComponentValue::EnchantmentGlintOverride(glowing))
    }

    /// Adds or updates an enchantment (`enchantment_id`, `level`) in `minecraft:enchantments` (`13`).
    #[must_use]
    pub fn with_enchantment(mut self, enchantment_id: i32, level: i32) -> Self {
        if self.is_air() {
            return Self::AIR;
        }
        let mut map = match self.added_components.remove(&component_ids::ENCHANTMENTS) {
            Some(DataComponentValue::Enchantments(m)) => m,
            _ => BTreeMap::new(),
        };
        map.insert(enchantment_id, level);
        self.with_component(DataComponentValue::Enchantments(map))
    }

    /// Sets raw `minecraft:custom_data` (`0`) [`NbtCompound`].
    #[must_use]
    pub fn with_custom_data(self, compound: NbtCompound) -> Self {
        self.with_component(DataComponentValue::CustomData(compound))
    }

    /// Writes a typed [`Tag<T>`] inside `minecraft:custom_data` (`ItemStack::withTag` in Minestom).
    #[must_use]
    pub fn with_tag<T: TagSerializable>(mut self, tag: &Tag<T>, value: T) -> Self {
        if self.is_air() {
            return Self::AIR;
        }
        let existing = match self.added_components.remove(&component_ids::CUSTOM_DATA) {
            Some(DataComponentValue::CustomData(c)) => c,
            _ => NbtCompound::new(),
        };
        let mut tmap = TagMap::from_compound(existing);
        tmap.set_tag(tag, value);
        self.with_custom_data(tmap.as_compound().clone())
    }

    /// Reads a typed [`Tag<T>`] from `minecraft:custom_data`.
    #[must_use]
    pub fn get_tag<T: TagSerializable>(&self, tag: &Tag<T>) -> Option<T> {
        match self.added_components.get(&component_ids::CUSTOM_DATA) {
            Some(DataComponentValue::CustomData(c)) => {
                TagMap::from_compound(c.clone()).get_tag(tag)
            }
            _ => None,
        }
    }
}

/// Fired before an inventory click is processed (`InventoryPreClickEvent` in Minestom).
#[derive(Debug, Clone)]
pub struct InventoryPreClickEvent {
    /// Clicking player.
    pub player_id: PlayerId,
    /// Clicked window ID (`0` = player inventory).
    pub window_id: i32,
    /// Clicked slot (`-999` = outside window).
    pub slot: i16,
    /// Mouse/key button.
    pub button: i8,
    /// Click mode.
    pub mode: WindowClickMode,
    cancelled: bool,
}

impl Event for InventoryPreClickEvent {}

impl CancellableEvent for InventoryPreClickEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

impl PlayerEvent for InventoryPreClickEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

impl InventoryPreClickEvent {
    /// Creates a new `InventoryPreClickEvent`.
    #[must_use]
    pub const fn new(
        player_id: PlayerId,
        window_id: i32,
        slot: i16,
        button: i8,
        mode: WindowClickMode,
    ) -> Self {
        Self {
            player_id,
            window_id,
            slot,
            button,
            mode,
            cancelled: false,
        }
    }
}

/// Fired when a player drops an [`ItemStack`] (`ItemDropEvent` in Minestom).
#[derive(Debug, Clone)]
pub struct ItemDropEvent {
    /// Dropping player.
    pub player_id: PlayerId,
    /// Target instance.
    pub instance_id: InstanceId,
    /// Dropped [`ItemStack`].
    pub item_stack: ItemStack,
    cancelled: bool,
}

impl Event for ItemDropEvent {}

impl CancellableEvent for ItemDropEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

impl PlayerEvent for ItemDropEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

impl InstanceEvent for ItemDropEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
}

impl ItemDropEvent {
    /// Creates a new `ItemDropEvent`.
    #[must_use]
    pub const fn new(player_id: PlayerId, instance_id: InstanceId, item_stack: ItemStack) -> Self {
        Self {
            player_id,
            instance_id,
            item_stack,
            cancelled: false,
        }
    }
}

/// Fired when a player walks over a dropped `ItemEntity` to pick it up (`PickupItemEvent` in Minestom).
#[derive(Debug, Clone)]
pub struct PickupItemEvent {
    /// Collector player.
    pub player_id: PlayerId,
    /// Instance ID.
    pub instance_id: InstanceId,
    /// Network ID of the dropped `ItemEntity`.
    pub item_entity_id: i32,
    /// Item stack being picked up.
    pub item_stack: ItemStack,
    cancelled: bool,
}

impl Event for PickupItemEvent {}

impl CancellableEvent for PickupItemEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

impl PlayerEvent for PickupItemEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

impl InstanceEvent for PickupItemEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
}

impl PickupItemEvent {
    /// Creates a new `PickupItemEvent`.
    #[must_use]
    pub const fn new(
        player_id: PlayerId,
        instance_id: InstanceId,
        item_entity_id: i32,
        item_stack: ItemStack,
    ) -> Self {
        Self {
            player_id,
            instance_id,
            item_entity_id,
            item_stack,
            cancelled: false,
        }
    }
}

/// Fired when a container window is closed (`InventoryCloseEvent` in Minestom).
#[derive(Debug, Clone, Copy)]
pub struct InventoryCloseEvent {
    /// Player who closed the window.
    pub player_id: PlayerId,
    /// Closed window ID.
    pub window_id: i32,
}

impl Event for InventoryCloseEvent {}

impl PlayerEvent for InventoryCloseEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}
