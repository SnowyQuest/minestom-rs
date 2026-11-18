//! Compositional `Entity`, `EntityType` (`158` vanilla types with generated hitboxes,
//! aerodynamics & default attributes), `EntityMetadata` + complete `metadata_def` (`110` classes,
//! `298` typed keys & bitmasks), sub-voxel `BoundingBox` physics, spatial `EntityTracker`,
//! and `EntityBehavior` / AI goal traits (ADR 0004).
//!
//! ```
//! use minestom_entity::{Attribute, Entity, EntityType, metadata_def};
//! use minestom_text::Component;
//!
//! let warden = Entity::creature(EntityType::WARDEN)
//!     .with_custom_name(Component::text("Deep Dark Guardian"))
//!     .with_meta(metadata_def::warden::ANGER_LEVEL, 80);
//! assert_eq!(warden.living.as_ref().unwrap().max_health, 500.0);
//! assert_eq!(warden.core.metadata.get_key(metadata_def::warden::ANGER_LEVEL), Some(80));
//! ```

pub mod ai;
pub mod attribute;
pub mod collision;
pub mod damage;
pub mod effect;
pub mod metadata;
pub mod metadata_def;
pub mod pathfinding;
pub mod tracker;

use std::fmt;
use std::str::FromStr;

use minestom_block::{Block, BlockGetter};
use minestom_core::{GameMode, Hand, InstanceId, PlayerId, Pos, TagMap, Vec3};
pub use minestom_data::generated::entity_types::{EntityPacketType, EntityTypeInfo};
use minestom_data::generated::{
    entity_types::{self, ENTITY_INFOS},
    painting_variants::{self, PAINTING_VARIANT_INFOS, PaintingVariantInfo},
    villager_professions,
};
use minestom_event::{CancellableEvent, Event, InstanceEvent, PlayerEvent};
use minestom_item::ItemStack;
use minestom_text::Component;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

pub use ai::{
    ClosestEntityTarget, EntityAIGroup, FollowTargetGoal, GoalContext, GoalSelector,
    LastEntityDamagerTarget, MeleeAttackGoal, PendingMeleeHit, RandomLookAroundGoal,
    RandomStrollGoal, TargetCandidate, TargetSelector,
};
pub use attribute::{
    ATTRIBUTE_SPECS, Attribute, AttributeInstance, AttributeMap, AttributeModifier,
    AttributeOperation, AttributeSpec,
};
pub use collision::{BoundingBox, PhysicsStepOutcome, step_entity_physics};
pub use damage::{
    Damage, DamageType, EntityDamageEvent, EntityDeathEvent, PlayerRespawnEvent,
    compute_knockback_velocity,
};
pub use effect::{EffectMap, Potion, PotionEffect};
pub use metadata::EntityMetadata;
pub use metadata_def::{BitMaskKey, ByteMaskKey, MetadataKey};
pub use pathfinding::{
    Navigator, PathSearchConfig, find_path, find_path_with_config, is_standable,
};
pub use tracker::{EntityTracker, ViewerReconcileDiff};

/// Errors returned by entity operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EntityError {
    /// Referenced entity handle is no longer alive.
    #[error("entity no longer exists")]
    Despawned,
    /// Unknown vanilla entity type protocol ID.
    #[error("unknown entity type ID: {0}")]
    UnknownEntityType(u32),
    /// Unknown vanilla entity type key.
    #[error("unknown entity type key: `{0}`")]
    UnknownEntityTypeKey(String),
    /// Unknown vanilla entity attribute protocol ID.
    #[error("unknown attribute ID: {0}")]
    UnknownAttribute(u32),
    /// Unknown vanilla potion effect protocol ID.
    #[error("unknown potion effect ID: {0}")]
    UnknownPotionEffect(u32),
    /// Unknown vanilla damage type protocol ID.
    #[error("unknown damage type ID: {0}")]
    UnknownDamageType(u32),
}

/// Strongly typed `Copy` handle for all **158** Minecraft 26.2 `EntityType`s (`minecraft:entity_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityType(u32);

minestom_data::impl_entity_type_constants!(EntityType);

impl EntityType {
    /// Constructs an [`EntityType`] from a validated protocol ID (`0..158`).
    pub const fn from_id(id: u32) -> Result<Self, EntityError> {
        if (id as usize) < ENTITY_INFOS.len() {
            Ok(Self(id))
        } else {
            Err(EntityError::UnknownEntityType(id))
        }
    }

    /// Zero-allocation `O(log N)` lookup by `"minecraft:..."` or bare `"..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        entity_types::lookup_key(key).map(Self)
    }

    /// Numeric protocol ID for `SpawnEntityPacket` (`0..158`).
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Static generated [`EntityTypeInfo`] entry.
    #[must_use]
    pub const fn info(self) -> &'static EntityTypeInfo {
        &ENTITY_INFOS[self.0 as usize]
    }

    /// Canonical `"minecraft:..."` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.info().key
    }

    /// Vanilla client translation key.
    #[must_use]
    pub const fn translation_key(self) -> &'static str {
        self.info().translation_key
    }

    /// Spawn packet category (`Base`, `Living`, `Player`, `ExperienceOrb`, `Painting`).
    #[must_use]
    pub const fn packet_type(self) -> EntityPacketType {
        self.info().packet_type
    }

    /// Default [`BoundingBox`] from `entity_type.json`.
    #[must_use]
    pub const fn bounding_box(self) -> BoundingBox {
        let info = self.info();
        BoundingBox::symmetric(info.width, info.height)
    }

    /// Default eye height offset from feet.
    #[must_use]
    pub const fn eye_height(self) -> f64 {
        self.info().eye_height
    }

    /// Whether this entity type extends `LivingEntity` in vanilla.
    #[must_use]
    pub const fn is_living(self) -> bool {
        self.info().living
    }

    /// Whether this entity type is immune to fire and lava damage.
    #[must_use]
    pub const fn is_fire_immune(self) -> bool {
        self.info().fire_immune
    }

    /// Exact vanilla client tracking range in chunks.
    #[must_use]
    pub const fn tracking_range_chunks(self) -> u8 {
        self.info().tracking_range_chunks
    }
}

impl TryFrom<u32> for EntityType {
    type Error = EntityError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::from_id(value)
    }
}

impl From<EntityType> for u32 {
    fn from(value: EntityType) -> Self {
        value.0
    }
}

impl fmt::Display for EntityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

impl FromStr for EntityType {
    type Err = EntityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_key(s).ok_or_else(|| EntityError::UnknownEntityTypeKey(s.to_owned()))
    }
}

impl Serialize for EntityType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

impl<'de> Deserialize<'de> for EntityType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_key(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown entity type `{s}`")))
    }
}

/// Strongly typed `Copy` handle for all **15** vanilla `VillagerProfession`s (`minecraft:villager_profession`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VillagerProfession(pub u32);

minestom_data::impl_villager_profession_constants!(VillagerProfession);

impl VillagerProfession {
    /// Zero-allocation `O(log N)` lookup by `"minecraft:..."` or bare `"..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        villager_professions::lookup_key(key).map(Self)
    }

    /// Canonical `"minecraft:..."` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        villager_professions::ENTRIES[self.0 as usize].1
    }
}

/// Strongly typed `Copy` handle for all **51** vanilla `PaintingVariant`s (`minecraft:painting_variant`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PaintingVariant(pub u32);

minestom_data::impl_painting_variant_constants!(PaintingVariant);

impl PaintingVariant {
    /// Zero-allocation `O(log N)` lookup by `"minecraft:..."` or bare `"..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        painting_variants::lookup_key(key).map(Self)
    }

    /// Static [`PaintingVariantInfo`] entry.
    #[must_use]
    pub const fn info(self) -> &'static PaintingVariantInfo {
        &PAINTING_VARIANT_INFOS[self.0 as usize]
    }

    /// Canonical `"minecraft:..."` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.info().key
    }
}

/// Mutable player configuration state exposed during `AsyncPlayerConfigurationEvent`
/// and initial spawn setup.
#[derive(Debug, Clone)]
pub struct ConfiguringPlayer {
    uuid: Uuid,
    username: String,
    respawn_point: Pos,
    game_mode: GameMode,
    permission_level: u8,
}

impl ConfiguringPlayer {
    /// Creates a new `ConfiguringPlayer` defaulting to `Pos(0, 40, 0)` and `GameMode::Survival`.
    #[must_use]
    pub const fn new(uuid: Uuid, username: String) -> Self {
        Self {
            uuid,
            username,
            respawn_point: Pos::new(0.0, 40.0, 0.0),
            game_mode: GameMode::Survival,
            permission_level: 0,
        }
    }

    /// Returns the player's UUID.
    #[must_use]
    pub const fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Returns the player's username.
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Returns the configured initial spawn / respawn point.
    #[must_use]
    pub const fn respawn_point(&self) -> Pos {
        self.respawn_point
    }

    /// Sets the initial spawn and respawn position (`player.set_respawn_point(...)` in Minestom).
    pub const fn set_respawn_point(&mut self, pos: Pos) {
        self.respawn_point = pos;
    }

    /// Returns the player's initial [`GameMode`].
    #[must_use]
    pub const fn game_mode(&self) -> GameMode {
        self.game_mode
    }

    /// Sets the player's initial [`GameMode`].
    pub const fn set_game_mode(&mut self, game_mode: GameMode) {
        self.game_mode = game_mode;
    }

    /// Returns the player's command permission level (`0..=4`).
    #[must_use]
    pub const fn permission_level(&self) -> u8 {
        self.permission_level
    }

    /// Sets the player's command permission level (`0..=4`).
    pub const fn set_permission_level(&mut self, level: u8) {
        self.permission_level = level;
    }
}

/// Shared spatial, physics, metadata, and tag state present on every [`Entity`] (ADR 0004).
#[derive(Debug)]
pub struct EntityCore {
    /// Assigned runtime network entity ID (`0` until spawned into a server).
    pub network_id: i32,
    /// Entity UUID.
    pub uuid: Uuid,
    /// Vanilla [`EntityType`].
    pub entity_type: EntityType,
    /// Current world position and look angles.
    pub position: Pos,
    /// Head yaw angle in degrees.
    pub head_yaw: f32,
    /// Current velocity in blocks/tick.
    pub velocity: Vec3,
    /// Collision [`BoundingBox`].
    pub bounding_box: BoundingBox,
    /// Whether resting on a solid block.
    pub on_ground: bool,
    /// Whether server-side gravity & block collision physics run each tick.
    pub has_physics: bool,
    /// Type-specific `SpawnEntityPacket` object `data` (`0` for mobs).
    pub object_data: i32,
    /// Dirty-tracked [`EntityMetadata`].
    pub metadata: EntityMetadata,
    /// Arbitrary typed [`TagMap`] metadata.
    pub tags: TagMap,
    /// Number of server ticks elapsed since spawn.
    pub alive_ticks: u64,
    /// Set to `true` by behaviors or callers to despawn at the end of the tick.
    pub should_remove: bool,
}

/// Health, attributes, potion effects, and AI perception state present on living entities
/// and creatures (`LivingEntity` in Minestom).
#[derive(Debug, Clone)]
pub struct LivingState {
    /// Current health points.
    pub health: f32,
    /// Maximum health points.
    pub max_health: f32,
    /// Whether this entity has died (`health <= 0.0`).
    pub dead: bool,
    /// Sparse entity attributes (`MAX_HEALTH`, `MOVEMENT_SPEED`, `ATTACK_DAMAGE`, etc.).
    pub attributes: AttributeMap,
    /// Active potion effects ticking down at 20 TPS.
    pub effects: EffectMap,
    /// Post-hit invulnerability frames (`10` ticks = 0.5s in vanilla).
    pub invulnerable_ticks: u32,
    /// Network entity ID of the most recent attacker (`LastEntityDamagerTarget`).
    pub last_damager_network_id: Option<i32>,
    /// Snapshot of nearby living candidates populated by the server tick loop before AI ticks.
    pub nearby_targets: Vec<TargetCandidate>,
    /// Outgoing melee strikes queued by AI goals during `tick`.
    pub pending_melee_hits: Vec<PendingMeleeHit>,
    /// Potion effects that expired during the most recent `tick`.
    pub expired_effects: Vec<PotionEffect>,
}

impl Default for LivingState {
    fn default() -> Self {
        Self {
            health: 20.0,
            max_health: 20.0,
            dead: false,
            attributes: AttributeMap::new(),
            effects: EffectMap::new(),
            invulnerable_ticks: 0,
            last_damager_network_id: None,
            nearby_targets: Vec::new(),
            pending_melee_hits: Vec::new(),
            expired_effects: Vec::new(),
        }
    }
}

impl LivingState {
    /// Initializes [`LivingState`] with exact vanilla `defaultAttributes` from `entity_type.json`
    /// (e.g. `Warden = 500 HP / 30 dmg`, `IronGolem = 100 HP`, `Chicken = 4 HP`, `Zombie = 20 HP / 3 dmg / 2 armor`).
    #[must_use]
    pub fn for_entity_type(entity_type: EntityType) -> Self {
        let attributes = AttributeMap::from_defaults(entity_type.info().default_attributes);
        let max_health = attributes.get_value(Attribute::MAX_HEALTH) as f32;
        Self {
            health: max_health,
            max_health,
            attributes,
            ..Self::default()
        }
    }
}

/// Role classification of an [`Entity`] (`Entity`, `LivingEntity`, `EntityCreature`, `ItemEntity` in Minestom).
#[derive(Debug, Clone, PartialEq)]
pub enum EntityRole {
    /// Non-living object (display entity, projectile, falling block, interaction).
    Object,
    /// Dropped world item (`ItemEntity` in Minestom) with pickup delay ticks.
    DroppedItem {
        /// Carried [`ItemStack`].
        item_stack: ItemStack,
        /// Remaining ticks before players can pick up this item (`10` = 0.5s, `40` = 2.0s).
        pickup_delay_ticks: u32,
    },
    /// Living entity (`ArmorStand`, passive/ambient living).
    Living,
    /// AI-capable mob/creature (`EntityCreature` in Minestom).
    Creature,
}

/// Trait for custom per-entity tick & spawn logic without Java class inheritance (`EntityCreature` / custom `Entity` in Minestom).
pub trait EntityBehavior: Send + Sync + 'static {
    /// Invoked once when the entity spawns into an instance.
    fn on_spawn(&mut self, _core: &mut EntityCore, _living: Option<&mut LivingState>) {}

    /// Invoked every server tick (20 TPS) before physics resolution.
    fn tick(
        &mut self,
        _core: &mut EntityCore,
        _living: Option<&mut LivingState>,
        _world: &dyn BlockGetter,
    ) {
    }
}

/// Compositional non-player world entity combining [`EntityCore`], optional [`LivingState`],
/// [`EntityRole`], and an optional [`EntityBehavior`] (ADR 0004).
pub struct Entity {
    /// Core spatial, physics, and metadata state.
    pub core: EntityCore,
    /// Health state for living entities and creatures.
    pub living: Option<LivingState>,
    /// Entity role classification.
    pub role: EntityRole,
    behavior: Option<Box<dyn EntityBehavior>>,
}

impl std::fmt::Debug for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entity")
            .field("network_id", &self.core.network_id)
            .field("type", &self.core.entity_type.key())
            .field("position", &self.core.position)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

impl Entity {
    /// Constructs a new [`Entity`] of `entity_type`, automatically populating [`LivingState`]
    /// with vanilla `defaultAttributes` when `entity_type.is_living()`.
    #[must_use]
    pub fn new(entity_type: EntityType) -> Self {
        let is_liv = entity_type.is_living();
        let living = is_liv.then(|| LivingState::for_entity_type(entity_type));
        let mut metadata = EntityMetadata::new();
        if let Some(liv) = &living {
            metadata.set_health(liv.health);
            metadata.clear_dirty();
        }
        Self {
            core: EntityCore {
                network_id: 0,
                uuid: Uuid::new_v4(),
                entity_type,
                position: Pos::ZERO,
                head_yaw: 0.0,
                velocity: Vec3::ZERO,
                bounding_box: entity_type.bounding_box(),
                on_ground: false,
                has_physics: entity_type.info().acceleration > 0.0 || is_liv,
                object_data: 0,
                metadata,
                tags: TagMap::new(),
                alive_ticks: 0,
                should_remove: false,
            },
            living,
            role: if is_liv {
                EntityRole::Living
            } else {
                EntityRole::Object
            },
            behavior: None,
        }
    }

    /// Constructs an AI/behavior-ready creature (`EntityCreature` equivalent).
    #[must_use]
    pub fn creature(entity_type: EntityType) -> Self {
        let mut e = Self::new(entity_type);
        if e.living.is_none() {
            e.living = Some(LivingState::for_entity_type(entity_type));
        }
        e.role = EntityRole::Creature;
        e
    }

    /// Constructs a dropped world `ItemEntity` (`EntityType::ITEM`) carrying `item_stack`
    /// (`metadata_def::item_entity::ITEM`) and `pickup_delay_ticks`.
    #[must_use]
    pub fn item(item_stack: ItemStack, pickup_delay_ticks: u32) -> Self {
        let mut e = Self::new(EntityType::ITEM);
        e.core.has_physics = true;
        e.core
            .metadata
            .set_key(metadata_def::item_entity::ITEM, item_stack.clone());
        e.role = EntityRole::DroppedItem {
            item_stack,
            pickup_delay_ticks,
        };
        e
    }

    /// Constructs a floating `TextDisplay` entity (`EntityType::TEXT_DISPLAY`) with physics disabled.
    #[must_use]
    pub fn text_display(text: impl Into<Component>) -> Self {
        Self::new(EntityType::TEXT_DISPLAY)
            .with_physics(false)
            .with_meta(metadata_def::text_display::TEXT, text.into())
    }

    /// Constructs a `BlockDisplay` entity (`EntityType::BLOCK_DISPLAY`) displaying `block`.
    #[must_use]
    pub fn block_display(block: Block) -> Self {
        Self::new(EntityType::BLOCK_DISPLAY)
            .with_physics(false)
            .with_meta(metadata_def::block_display::DISPLAYED_BLOCK_STATE, block)
    }

    /// Constructs an `ItemDisplay` entity (`EntityType::ITEM_DISPLAY`) displaying `item_stack`.
    #[must_use]
    pub fn item_display(item_stack: ItemStack) -> Self {
        Self::new(EntityType::ITEM_DISPLAY)
            .with_physics(false)
            .with_meta(metadata_def::item_display::DISPLAYED_ITEM, item_stack)
    }

    /// Constructs an invisible clickable/attackable `Interaction` hitbox (`EntityType::INTERACTION`).
    #[must_use]
    pub fn interaction(width: f32, height: f32) -> Self {
        let mut e = Self::new(EntityType::INTERACTION)
            .with_physics(false)
            .with_meta(metadata_def::interaction::WIDTH, width)
            .with_meta(metadata_def::interaction::HEIGHT, height)
            .with_meta(metadata_def::interaction::RESPONSIVE, true);
        e.core.bounding_box = BoundingBox::symmetric(f64::from(width), f64::from(height));
        e
    }

    /// Constructs an `ArmorStand` (`EntityType::ARMOR_STAND`).
    #[must_use]
    pub fn armor_stand() -> Self {
        Self::new(EntityType::ARMOR_STAND)
    }

    /// Sets a typed [`MetadataKey<T>`] entry on `self.core.metadata` in builder style.
    #[must_use]
    pub fn with_meta<T>(mut self, key: MetadataKey<T>, value: T) -> Self {
        self.core.metadata.set_key(key, value);
        self
    }

    /// Sets a boolean [`BitMaskKey`] flag on `self.core.metadata` in builder style.
    #[must_use]
    pub fn with_flag(mut self, flag: BitMaskKey, enabled: bool) -> Self {
        self.core.metadata.set_flag(flag, enabled);
        self
    }

    /// Attaches a custom [`EntityBehavior`] implementation (`on_spawn` + 20 TPS `tick`).
    #[must_use]
    pub fn with_behavior(mut self, behavior: impl EntityBehavior) -> Self {
        self.behavior = Some(Box::new(behavior));
        self
    }

    /// Sets a visible custom name plate above the entity.
    #[must_use]
    pub fn with_custom_name(mut self, name: impl Into<Component>) -> Self {
        self.core.metadata.set_custom_name(Some(name.into()));
        self.core.metadata.set_custom_name_visible(true);
        self
    }

    /// Enables or disables gravity (`HAS_NO_GRAVITY` metadata).
    #[must_use]
    pub fn with_no_gravity(mut self, no_gravity: bool) -> Self {
        self.core.metadata.set_no_gravity(no_gravity);
        self
    }

    /// Enables or disables server-side physics & block collision.
    #[must_use]
    pub const fn with_physics(mut self, enabled: bool) -> Self {
        self.core.has_physics = enabled;
        self
    }

    /// Sets initial velocity in blocks/tick.
    #[must_use]
    pub const fn with_velocity(mut self, velocity: Vec3) -> Self {
        self.core.velocity = velocity;
        self
    }

    /// Sets type-specific spawn `object_data` (e.g. block state ID for `FALLING_BLOCK`).
    #[must_use]
    pub const fn with_object_data(mut self, data: i32) -> Self {
        self.core.object_data = data;
        self
    }

    /// Sets an initial [`Attribute`] base value on this living entity.
    #[must_use]
    pub fn with_attribute(mut self, attribute: Attribute, base_value: f64) -> Self {
        if let Some(liv) = &mut self.living {
            liv.attributes.set_base_value(attribute, base_value);
            if attribute == Attribute::MAX_HEALTH {
                let mh = liv.attributes.get_value(Attribute::MAX_HEALTH) as f32;
                liv.max_health = mh;
                liv.health = mh;
            }
        }
        self
    }

    /// Sets initial and maximum health on this living entity.
    #[must_use]
    pub fn with_max_health(self, max_health: f32) -> Self {
        self.with_attribute(Attribute::MAX_HEALTH, f64::from(max_health))
    }

    /// Adds an initial [`Potion`] effect to this living entity.
    #[must_use]
    pub fn with_potion(mut self, potion: Potion) -> Self {
        if let Some(liv) = &mut self.living {
            liv.effects.add(potion);
        }
        self
    }

    /// Returns this entity's [`EntityType`].
    #[must_use]
    pub const fn entity_type(&self) -> EntityType {
        self.core.entity_type
    }

    /// Returns `true` if this entity has [`LivingState`].
    #[must_use]
    pub const fn is_living(&self) -> bool {
        self.living.is_some()
    }

    /// Invokes `on_spawn` on the attached [`EntityBehavior`], if any.
    pub fn notify_spawn(&mut self) {
        if let Some(beh) = &mut self.behavior {
            beh.on_spawn(&mut self.core, self.living.as_mut());
        }
    }

    /// Advances custom behavior, potion timers, and server-side AABB physics by one 50ms tick.
    pub fn tick(&mut self, world: &dyn BlockGetter) {
        if let Some(liv) = &mut self.living {
            liv.invulnerable_ticks = liv.invulnerable_ticks.saturating_sub(1);
            liv.expired_effects = liv.effects.tick();
        }
        if let Some(beh) = &mut self.behavior {
            beh.tick(&mut self.core, self.living.as_mut(), world);
        }
        if let Some(liv) = &self.living {
            self.core.metadata.set_health(liv.health);
        }
        if let EntityRole::DroppedItem {
            pickup_delay_ticks, ..
        } = &mut self.role
        {
            *pickup_delay_ticks = pickup_delay_ticks.saturating_sub(1);
        }
        if self.core.has_physics {
            let no_grav = self.core.metadata.has_no_gravity();
            let outcome = step_entity_physics(
                world,
                self.core.bounding_box,
                self.core.position,
                self.core.velocity,
                no_grav,
            );
            self.core.position = outcome.position;
            self.core.velocity = outcome.velocity;
            self.core.on_ground = outcome.on_ground;
        }
        self.core.alive_ticks = self.core.alive_ticks.wrapping_add(1);
    }
}

/// Fired when a non-player [`Entity`] spawns into an instance (`EntitySpawnEvent` in Minestom).
#[derive(Debug, Clone, Copy)]
pub struct EntitySpawnEvent {
    /// Network entity ID.
    pub network_id: i32,
    /// Entity type.
    pub entity_type: EntityType,
    /// Target instance ID.
    pub instance_id: InstanceId,
    /// Spawn position.
    pub position: Pos,
}

impl Event for EntitySpawnEvent {}

impl InstanceEvent for EntitySpawnEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
}

/// Fired when a non-player [`Entity`] is removed from an instance (`EntityDespawnEvent` in Minestom).
#[derive(Debug, Clone, Copy)]
pub struct EntityDespawnEvent {
    /// Network entity ID.
    pub network_id: i32,
    /// Entity type.
    pub entity_type: EntityType,
    /// Instance ID.
    pub instance_id: InstanceId,
}

impl Event for EntityDespawnEvent {}

impl InstanceEvent for EntityDespawnEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
}

/// Fired when a player attacks an entity or another player (`ClientAttackPacket` `0x01`, `EntityAttackEvent` in Minestom).
#[derive(Debug, Clone, Copy)]
pub struct PlayerAttackEntityEvent {
    /// Attacking [`PlayerId`].
    pub attacker_player_id: PlayerId,
    /// Attacker's network entity ID.
    pub attacker_network_id: i32,
    /// Attacked target's network entity ID.
    pub target_network_id: i32,
    /// Instance ID.
    pub instance_id: InstanceId,
    cancelled: bool,
}

impl Event for PlayerAttackEntityEvent {}

impl CancellableEvent for PlayerAttackEntityEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

impl PlayerEvent for PlayerAttackEntityEvent {
    fn player_id(&self) -> PlayerId {
        self.attacker_player_id
    }
}

impl InstanceEvent for PlayerAttackEntityEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
}

impl PlayerAttackEntityEvent {
    /// Creates a new `PlayerAttackEntityEvent`.
    #[must_use]
    pub const fn new(
        attacker_player_id: PlayerId,
        attacker_network_id: i32,
        target_network_id: i32,
        instance_id: InstanceId,
    ) -> Self {
        Self {
            attacker_player_id,
            attacker_network_id,
            target_network_id,
            instance_id,
            cancelled: false,
        }
    }
}

/// Fired when a player right-clicks an entity (`ClientInteractEntityPacket` `0x1A`, `PlayerEntityInteractEvent` in Minestom).
#[derive(Debug, Clone, Copy)]
pub struct PlayerEntityInteractEvent {
    /// Interacting [`PlayerId`].
    pub player_id: PlayerId,
    /// Target network entity ID.
    pub target_network_id: i32,
    /// Hand used (`Main` or `Off`).
    pub hand: Hand,
    /// Hit location relative to target.
    pub interact_position: Vec3,
    /// Instance ID.
    pub instance_id: InstanceId,
}

impl Event for PlayerEntityInteractEvent {}

impl PlayerEvent for PlayerEntityInteractEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

impl InstanceEvent for PlayerEntityInteractEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
}

/// Fired when a player swings their arm (`ClientAnimationPacket` `0x3F`, `PlayerHandAnimationEvent` in Minestom).
#[derive(Debug, Clone, Copy)]
pub struct PlayerHandAnimationEvent {
    /// Swinging [`PlayerId`].
    pub player_id: PlayerId,
    /// Swung [`Hand`].
    pub hand: Hand,
    /// Instance ID.
    pub instance_id: InstanceId,
    cancelled: bool,
}

impl Event for PlayerHandAnimationEvent {}

impl CancellableEvent for PlayerHandAnimationEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

impl PlayerEvent for PlayerHandAnimationEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

impl PlayerHandAnimationEvent {
    /// Creates a new `PlayerHandAnimationEvent`.
    #[must_use]
    pub const fn new(player_id: PlayerId, hand: Hand, instance_id: InstanceId) -> Self {
        Self {
            player_id,
            hand,
            instance_id,
            cancelled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_158_entity_types_default_attributes_and_metadata_defs() {
        assert_eq!(ENTITY_INFOS.len(), 158);

        // Warden default attributes: 500 HP, 30 attack damage, fire immune!
        let warden = Entity::creature(EntityType::WARDEN);
        assert!(EntityType::WARDEN.is_fire_immune());
        let liv = warden.living.as_ref().unwrap();
        assert!((liv.max_health - 500.0).abs() < 1e-4);
        assert!((liv.health - 500.0).abs() < 1e-4);
        assert!((liv.attributes.get_value(Attribute::ATTACK_DAMAGE) - 30.0).abs() < 1e-4);

        // Chicken = 4 HP, IronGolem = 100 HP, Marker tracking range = 0 (not clamped!)
        assert_eq!(
            Entity::new(EntityType::CHICKEN).living.unwrap().max_health,
            4.0
        );
        assert_eq!(
            Entity::new(EntityType::IRON_GOLEM)
                .living
                .unwrap()
                .max_health,
            100.0
        );
        assert_eq!(EntityType::MARKER.tracking_range_chunks(), 0);

        // Typed MetadataDef across Display, ArmorStand, Creeper, Cat, Villager
        let td = Entity::text_display("Boss Arena")
            .with_meta(metadata_def::display::SCALE, [2.0, 2.0, 2.0])
            .with_flag(metadata_def::text_display::HAS_SHADOW, true);
        assert_eq!(
            td.core.metadata.get_key(metadata_def::display::SCALE),
            Some([2.0, 2.0, 2.0])
        );
        assert!(
            td.core
                .metadata
                .get_flag(metadata_def::text_display::HAS_SHADOW)
        );
        assert_eq!(metadata_def::text_display::TEXT.index(), 23);
        assert_eq!(metadata_def::creeper::IS_CHARGED.index(), 17);
        assert_eq!(metadata_def::cat::SOUND_VARIANT.index(), 24);
        assert_eq!(
            metadata_def::mannequin::DISPLAYED_MODEL_PARTS_FLAGS.index(),
            16
        );
        assert_eq!(metadata_def::mannequin::PROFILE.index(), 17);
    }
}
