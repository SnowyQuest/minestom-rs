//! Compositional `Entity`, `EntityType`, `EntityMetadata`, `BoundingBox` physics,
//! spatial `EntityTracker`, and `EntityBehavior` traits (ADR 0004).
//!
//! ```
//! use minestom_entity::{Entity, EntityType};
//! use minestom_text::Component;
//!
//! let zombie = Entity::creature(EntityType::ZOMBIE)
//!     .with_custom_name(Component::text("Castle Guard"));
//! assert!(zombie.is_living());
//! assert_eq!(zombie.entity_type(), EntityType::ZOMBIE);
//! ```

pub mod ai;
pub mod attribute;
pub mod collision;
pub mod damage;
pub mod effect;
pub mod metadata;
pub mod pathfinding;
pub mod tracker;

use minestom_block::BlockGetter;
use minestom_core::{GameMode, Hand, InstanceId, PlayerId, Pos, TagMap, Vec3};
use minestom_data::generated::entity_types::{self, ENTITY_INFOS, EntityTypeInfo};
use minestom_event::{CancellableEvent, Event, InstanceEvent, PlayerEvent};
use minestom_item::ItemStack;
use minestom_protocol::entity::MetadataValue;
use minestom_text::Component;
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
pub use pathfinding::{
    Navigator, PathSearchConfig, find_path, find_path_with_config, is_standable,
};
pub use tracker::{EntityTracker, ViewerReconcileDiff};

/// Errors returned by entity operations.
#[derive(Debug, thiserror::Error)]
pub enum EntityError {
    /// Referenced entity handle is no longer alive.
    #[error("entity no longer exists")]
    Despawned,
    /// Unknown vanilla entity type protocol ID.
    #[error("unknown entity type ID: {0}")]
    UnknownEntityType(u32),
    /// Unknown vanilla entity attribute protocol ID.
    #[error("unknown attribute ID: {0}")]
    UnknownAttribute(u32),
    /// Unknown vanilla potion effect protocol ID.
    #[error("unknown potion effect ID: {0}")]
    UnknownPotionEffect(u32),
}

/// Strongly typed `Copy` handle for a Minecraft 26.2 `EntityType` (`minecraft:entity_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityType(u32);

impl EntityType {
    /// `minecraft:armor_stand`.
    pub const ARMOR_STAND: Self = Self(entity_types::ARMOR_STAND);
    /// `minecraft:arrow`.
    pub const ARROW: Self = Self(entity_types::ARROW);
    /// `minecraft:block_display`.
    pub const BLOCK_DISPLAY: Self = Self(entity_types::BLOCK_DISPLAY);
    /// `minecraft:chicken`.
    pub const CHICKEN: Self = Self(entity_types::CHICKEN);
    /// `minecraft:cow`.
    pub const COW: Self = Self(entity_types::COW);
    /// `minecraft:creeper`.
    pub const CREEPER: Self = Self(entity_types::CREEPER);
    /// `minecraft:enderman`.
    pub const ENDERMAN: Self = Self(entity_types::ENDERMAN);
    /// `minecraft:falling_block`.
    pub const FALLING_BLOCK: Self = Self(entity_types::FALLING_BLOCK);
    /// `minecraft:interaction`.
    pub const INTERACTION: Self = Self(entity_types::INTERACTION);
    /// `minecraft:iron_golem`.
    pub const IRON_GOLEM: Self = Self(entity_types::IRON_GOLEM);
    /// `minecraft:item`.
    pub const ITEM: Self = Self(entity_types::ITEM);
    /// `minecraft:item_display`.
    pub const ITEM_DISPLAY: Self = Self(entity_types::ITEM_DISPLAY);
    /// `minecraft:pig`.
    pub const PIG: Self = Self(entity_types::PIG);
    /// `minecraft:player`.
    pub const PLAYER: Self = Self(entity_types::PLAYER);
    /// `minecraft:sheep`.
    pub const SHEEP: Self = Self(entity_types::SHEEP);
    /// `minecraft:skeleton`.
    pub const SKELETON: Self = Self(entity_types::SKELETON);
    /// `minecraft:snowball`.
    pub const SNOWBALL: Self = Self(entity_types::SNOWBALL);
    /// `minecraft:spider`.
    pub const SPIDER: Self = Self(entity_types::SPIDER);
    /// `minecraft:text_display`.
    pub const TEXT_DISPLAY: Self = Self(entity_types::TEXT_DISPLAY);
    /// `minecraft:villager`.
    pub const VILLAGER: Self = Self(entity_types::VILLAGER);
    /// `minecraft:wolf`.
    pub const WOLF: Self = Self(entity_types::WOLF);
    /// `minecraft:zombie`.
    pub const ZOMBIE: Self = Self(entity_types::ZOMBIE);

    /// Constructs an [`EntityType`] from a validated protocol ID (`0..158`).
    pub const fn from_id(id: u32) -> Result<Self, EntityError> {
        if (id as usize) < ENTITY_INFOS.len() {
            Ok(Self(id))
        } else {
            Err(EntityError::UnknownEntityType(id))
        }
    }

    /// Looks up an [`EntityType`] by its `minecraft:...` resource key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        ENTITY_INFOS
            .iter()
            .find(|info| info.key == key)
            .map(|info| Self(info.id))
    }

    /// Numeric protocol ID for `SpawnEntityPacket`.
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Static generated [`EntityTypeInfo`] entry.
    #[must_use]
    pub const fn info(self) -> &'static EntityTypeInfo {
        &ENTITY_INFOS[self.0 as usize]
    }

    /// Canonical `minecraft:...` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.info().key
    }

    /// Default [`BoundingBox`] from `entity_type.json`.
    #[must_use]
    pub const fn bounding_box(self) -> BoundingBox {
        let info = self.info();
        BoundingBox::symmetric(info.width, info.height)
    }

    /// Whether this entity type extends `LivingEntity` in vanilla.
    #[must_use]
    pub const fn is_living(self) -> bool {
        self.info().living
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
    /// Constructs a new [`Entity`] of `entity_type` (automatically initializing [`LivingState`]
    /// if `entity_type.is_living()`).
    #[must_use]
    pub fn new(entity_type: EntityType) -> Self {
        let is_liv = entity_type.is_living();
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
                has_physics: true,
                object_data: 0,
                metadata: EntityMetadata::new(),
                tags: TagMap::new(),
                alive_ticks: 0,
                should_remove: false,
            },
            living: is_liv.then(LivingState::default),
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
            e.living = Some(LivingState::default());
        }
        e.role = EntityRole::Creature;
        e
    }

    /// Constructs a dropped world `ItemEntity` (`EntityType::ITEM`) carrying `item_stack`
    /// (`MetadataDef.ItemEntity.ITEM` at index `8`) and `pickup_delay_ticks`.
    #[must_use]
    pub fn item(item_stack: ItemStack, pickup_delay_ticks: u32) -> Self {
        let mut e = Self::new(EntityType::ITEM);
        e.core
            .metadata
            .set(8, MetadataValue::ItemStack(item_stack.clone()));
        e.role = EntityRole::DroppedItem {
            item_stack,
            pickup_delay_ticks,
        };
        e
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

    /// Enables or disables gravity (`HAS_NO_GRAVITY` metadata + physics flag).
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
                #[allow(clippy::cast_possible_truncation)]
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
