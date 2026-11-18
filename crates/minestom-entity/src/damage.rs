//! `DamageType`, `Damage`, knockback math, and combat/death/respawn events
//! (`net.minestom.server.entity.damage` equivalent for MC 26.2, `51` generated damage types).

use std::fmt;

use minestom_core::{InstanceId, PlayerId, Pos, Vec3};
use minestom_data::generated::damage_types::{self, DAMAGE_TYPE_INFOS, DamageTypeInfo};
use minestom_event::{CancellableEvent, Event, InstanceEvent, PlayerEvent};
use minestom_protocol::entity::DamageEventPacket;

use crate::{EntityError, EntityType};

/// Strongly typed `Copy` handle for all **51** Minecraft 26.2 `DamageType`s (`minecraft:damage_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DamageType(u32);

minestom_data::impl_damage_type_constants!(DamageType);

impl DamageType {
    /// Constructs a [`DamageType`] from a validated dynamic-registry ID (`0..51`).
    pub const fn from_id(id: u32) -> Result<Self, EntityError> {
        if (id as usize) < damage_types::DAMAGE_TYPE_COUNT {
            Ok(Self(id))
        } else {
            Err(EntityError::UnknownDamageType(id))
        }
    }

    /// Zero-allocation `O(log N)` lookup by `"minecraft:..."` or bare `"..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        damage_types::lookup_key(key).map(Self)
    }

    /// Numeric dynamic-registry protocol ID (`0..51`).
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Static generated [`DamageTypeInfo`] entry.
    #[must_use]
    pub const fn info(self) -> &'static DamageTypeInfo {
        &DAMAGE_TYPE_INFOS[self.0 as usize]
    }

    /// Canonical `"minecraft:..."` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.info().key
    }
}

impl fmt::Display for DamageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
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
    /// Constructs damage of `damage_type` with no source entity.
    #[must_use]
    pub const fn of(damage_type: DamageType, amount: f32) -> Self {
        Self {
            damage_type,
            amount,
            source_entity_id: None,
            source_direct_id: None,
            source_pos: None,
        }
    }

    /// Computes vanilla Minecraft armor + armor toughness damage mitigation:
    /// `damage * (1.0 - clamp(max(armor / 5.0, armor - 4.0 * damage / (toughness + 8.0)), 0.0, 20.0) / 25.0)`.
    #[must_use]
    pub fn apply_armor_reduction(raw_damage: f32, armor: f32, toughness: f32) -> f32 {
        let min_def = armor * 0.2;
        let reduced_def = armor - (4.0 * raw_damage) / (toughness + 8.0);
        let defense = min_def.max(reduced_def).clamp(0.0, 20.0);
        raw_damage * (1.0 - defense / 25.0)
    }

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

/// Fired when a dead player clicks "Respawn" (`ClientStatusPacket::PerformRespawn` `0x0C`, `PlayerRespawnEvent` in Minestom).
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
    fn damage_type_lookup_and_knockback() {
        assert_eq!(
            DamageType::from_key("minecraft:player_attack"),
            Some(DamageType::PLAYER_ATTACK)
        );
        assert_eq!(DamageType::from_key("fall"), Some(DamageType::FALL));
        let v = compute_knockback_velocity(
            Vec3::ZERO,
            Pos::new(0.0, 40.0, 0.0),
            Pos::new(2.0, 40.0, 0.0),
            0.4,
            0.0,
            true,
        );
        assert!(v.x > 0.39);
        assert!((v.y - 0.4).abs() < 1e-6);
    }
}
