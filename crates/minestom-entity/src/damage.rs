//! `DamageType`, `Damage`, knockback math, and combat/death/respawn events
//! (`net.minestom.server.entity.damage` equivalent for MC 26.2).

use minestom_core::{InstanceId, PlayerId, Pos, Vec3};
use minestom_data::generated::dynamic_registries::CONFIGURATION_REGISTRIES;
use minestom_event::{CancellableEvent, Event, InstanceEvent, PlayerEvent};
use minestom_protocol::entity::DamageEventPacket;

use crate::EntityType;

/// Strongly typed `Copy` handle for a Minecraft 26.2 `DamageType` (`minecraft:damage_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DamageType(u32);

impl DamageType {
    /// `minecraft:arrow` (`0`).
    pub const ARROW: Self = Self(0);
    /// `minecraft:explosion` (`9`).
    pub const EXPLOSION: Self = Self(9);
    /// `minecraft:fall` (`10`).
    pub const FALL: Self = Self(10);
    /// `minecraft:generic` (`18`).
    pub const GENERIC: Self = Self(18);
    /// `minecraft:generic_kill` (`19`).
    pub const GENERIC_KILL: Self = Self(19);
    /// `minecraft:in_fire` (`21`).
    pub const IN_FIRE: Self = Self(21);
    /// `minecraft:lava` (`24`).
    pub const LAVA: Self = Self(24);
    /// `minecraft:magic` (`27`).
    pub const MAGIC: Self = Self(27);
    /// `minecraft:mob_attack` (`28`).
    pub const MOB_ATTACK: Self = Self(28);
    /// `minecraft:on_fire` (`31`).
    pub const ON_FIRE: Self = Self(31);
    /// `minecraft:out_of_world` (`32`).
    pub const OUT_OF_WORLD: Self = Self(32);
    /// `minecraft:player_attack` (`34`).
    pub const PLAYER_ATTACK: Self = Self(34);
    /// `minecraft:starve` (`40`).
    pub const STARVE: Self = Self(40);
    /// `minecraft:wither` (`49`).
    pub const WITHER: Self = Self(49);

    /// Looks up a [`DamageType`] by its `minecraft:...` key in `CONFIGURATION_REGISTRIES`.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        let reg = CONFIGURATION_REGISTRIES
            .iter()
            .find(|r| r.registry_id == "minecraft:damage_type")?;
        reg.entries
            .iter()
            .position(|(k, _)| *k == key)
            .map(|idx| Self(idx as u32))
    }

    /// Numeric dynamic-registry protocol ID (`0..51`).
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }
}

/// Damage descriptor passed into [`EntityDamageEvent`] and `server.damage_entity` / `server.damage_player`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Damage {
    /// Damage category (`minecraft:damage_type`).
    pub damage_type: DamageType,
    /// Raw health points to subtract.
    pub amount: f32,
    /// Responsible attacker entity network ID, if any.
    pub source_entity_id: Option<i32>,
    /// Direct damaging entity network ID (same as `source_entity_id` for melee).
    pub source_direct_id: Option<i32>,
    /// World origin of the damage (used for directional hit tilt & knockback).
    pub source_pos: Option<Pos>,
}

impl Damage {
    /// Constructs generic environmental/command damage with no source entity.
    #[must_use]
    pub const fn generic(amount: f32) -> Self {
        Self {
            damage_type: DamageType::GENERIC,
            amount,
            source_entity_id: None,
            source_direct_id: None,
            source_pos: None,
        }
    }

    /// Constructs melee `minecraft:player_attack` damage from `attacker_id` at `attacker_pos`.
    #[must_use]
    pub const fn from_player(attacker_id: i32, attacker_pos: Pos, amount: f32) -> Self {
        Self {
            damage_type: DamageType::PLAYER_ATTACK,
            amount,
            source_entity_id: Some(attacker_id),
            source_direct_id: Some(attacker_id),
            source_pos: Some(attacker_pos),
        }
    }

    /// Constructs melee `minecraft:mob_attack` damage from `attacker_id` at `attacker_pos`.
    #[must_use]
    pub const fn from_mob(attacker_id: i32, attacker_pos: Pos, amount: f32) -> Self {
        Self {
            damage_type: DamageType::MOB_ATTACK,
            amount,
            source_entity_id: Some(attacker_id),
            source_direct_id: Some(attacker_id),
            source_pos: Some(attacker_pos),
        }
    }

    /// Constructs `minecraft:fall` damage.
    #[must_use]
    pub const fn fall(amount: f32) -> Self {
        Self {
            damage_type: DamageType::FALL,
            amount,
            source_entity_id: None,
            source_direct_id: None,
            source_pos: None,
        }
    }

    /// Builds the clientbound [`DamageEventPacket`] for `target_entity_id`.
    #[must_use]
    pub const fn to_packet(self, target_entity_id: i32) -> DamageEventPacket {
        DamageEventPacket {
            target_entity_id,
            damage_type_id: self.damage_type.id() as i32,
            source_entity_id: self.source_entity_id,
            source_direct_id: self.source_direct_id,
            source_pos: match self.source_pos {
                Some(p) => Some(Vec3::new(p.x, p.y, p.z)),
                None => None,
            },
        }
    }
}

/// Computes post-knockback velocity (`LivingEntity.takeKnockback` in Minestom / vanilla).
#[must_use]
pub fn compute_knockback_velocity(
    current: Vec3,
    attacker_pos: Pos,
    target_pos: Pos,
    base_strength: f64,
    knockback_resistance: f64,
    on_ground: bool,
) -> Vec3 {
    let effective = base_strength * (1.0 - knockback_resistance).max(0.0);
    if effective <= 0.0 {
        return current;
    }
    let mut dx = attacker_pos.x - target_pos.x;
    let mut dz = attacker_pos.z - target_pos.z;
    let mut dist = dx.hypot(dz);
    if dist < 1e-4 {
        dx = 1.0;
        dz = 0.0;
        dist = 1.0;
    }
    let nx = dx / dist;
    let nz = dz / dist;
    let vy = if on_ground {
        (current.y * 0.5 + effective).min(0.4)
    } else {
        current.y
    };
    Vec3::new(
        current.x * 0.5 - nx * effective,
        vy,
        current.z * 0.5 - nz * effective,
    )
}

/// Fired before a living entity or player takes damage (`EntityDamageEvent` in Minestom).
#[derive(Debug, Clone)]
pub struct EntityDamageEvent {
    /// Target protocol entity ID.
    pub target_network_id: i32,
    /// Target [`PlayerId`] if the victim is a player.
    pub target_player_id: Option<PlayerId>,
    /// Instance ID.
    pub instance_id: InstanceId,
    /// Mutable [`Damage`] payload (listeners may scale or replace `damage.amount`).
    pub damage: Damage,
    /// Whether to broadcast `DamageEventPacket` / `HitAnimationPacket`.
    pub animation: bool,
    cancelled: bool,
}

impl EntityDamageEvent {
    /// Creates a new [`EntityDamageEvent`].
    #[must_use]
    pub const fn new(
        target_network_id: i32,
        target_player_id: Option<PlayerId>,
        instance_id: InstanceId,
        damage: Damage,
    ) -> Self {
        Self {
            target_network_id,
            target_player_id,
            instance_id,
            damage,
            animation: true,
            cancelled: false,
        }
    }
}

impl Event for EntityDamageEvent {}

impl CancellableEvent for EntityDamageEvent {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

impl InstanceEvent for EntityDamageEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
}

/// Fired when a living entity or player reaches `0.0` health (`EntityDeathEvent` / `PlayerDeathEvent` in Minestom).
#[derive(Debug, Clone, Copy)]
pub struct EntityDeathEvent {
    /// Deceased entity network ID.
    pub network_id: i32,
    /// Deceased [`PlayerId`], if a player died.
    pub player_id: Option<PlayerId>,
    /// Entity type (`EntityType::PLAYER` for players).
    pub entity_type: EntityType,
    /// Instance ID.
    pub instance_id: InstanceId,
    /// Position at death.
    pub position: Pos,
}

impl Event for EntityDeathEvent {}

impl InstanceEvent for EntityDeathEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
}

/// Fired when a dead player clicks Respawn (`ClientStatusAction::PerformRespawn`, `PlayerRespawnEvent` in Minestom).
#[derive(Debug, Clone, Copy)]
pub struct PlayerRespawnEvent {
    /// Respawning [`PlayerId`].
    pub player_id: PlayerId,
    /// Instance ID.
    pub instance_id: InstanceId,
    /// Mutable respawn destination (defaults to `player.respawn_point`).
    pub respawn_position: Pos,
}

impl Event for PlayerRespawnEvent {}

impl PlayerEvent for PlayerRespawnEvent {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }
}

impl InstanceEvent for PlayerRespawnEvent {
    fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_type_constants_match_dynamic_registry() {
        assert_eq!(
            DamageType::from_key("minecraft:generic"),
            Some(DamageType::GENERIC)
        );
        assert_eq!(
            DamageType::from_key("minecraft:player_attack"),
            Some(DamageType::PLAYER_ATTACK)
        );
        assert_eq!(
            DamageType::from_key("minecraft:mob_attack"),
            Some(DamageType::MOB_ATTACK)
        );
        assert_eq!(
            DamageType::from_key("minecraft:fall"),
            Some(DamageType::FALL)
        );
        assert_eq!(
            DamageType::from_key("minecraft:out_of_world"),
            Some(DamageType::OUT_OF_WORLD)
        );
    }
}
