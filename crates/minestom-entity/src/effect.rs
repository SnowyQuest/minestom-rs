//! `PotionEffect`, `Potion`, and active `EffectMap` ticking
//! (`net.minestom.server.potion` equivalent for MC 26.2).

use std::collections::BTreeMap;

use minestom_data::generated::potion_effects;
use minestom_protocol::entity::{EntityEffectPacket, RemoveEntityEffectPacket};

use crate::EntityError;

/// Strongly typed `Copy` handle for a Minecraft 26.2 `PotionEffect` (`minecraft:mob_effect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PotionEffect(u32);

impl PotionEffect {
    /// `minecraft:speed` (`0`).
    pub const SPEED: Self = Self(potion_effects::SPEED);
    /// `minecraft:slowness` (`1`).
    pub const SLOWNESS: Self = Self(potion_effects::SLOWNESS);
    /// `minecraft:haste` (`2`).
    pub const HASTE: Self = Self(potion_effects::HASTE);
    /// `minecraft:mining_fatigue` (`3`).
    pub const MINING_FATIGUE: Self = Self(potion_effects::MINING_FATIGUE);
    /// `minecraft:strength` (`4`).
    pub const STRENGTH: Self = Self(potion_effects::STRENGTH);
    /// `minecraft:instant_health` (`5`).
    pub const INSTANT_HEALTH: Self = Self(potion_effects::INSTANT_HEALTH);
    /// `minecraft:instant_damage` (`6`).
    pub const INSTANT_DAMAGE: Self = Self(potion_effects::INSTANT_DAMAGE);
    /// `minecraft:jump_boost` (`7`).
    pub const JUMP_BOOST: Self = Self(potion_effects::JUMP_BOOST);
    /// `minecraft:nausea` (`8`).
    pub const NAUSEA: Self = Self(potion_effects::NAUSEA);
    /// `minecraft:regeneration` (`9`).
    pub const REGENERATION: Self = Self(potion_effects::REGENERATION);
    /// `minecraft:resistance` (`10`).
    pub const RESISTANCE: Self = Self(potion_effects::RESISTANCE);
    /// `minecraft:fire_resistance` (`11`).
    pub const FIRE_RESISTANCE: Self = Self(potion_effects::FIRE_RESISTANCE);
    /// `minecraft:water_breathing` (`12`).
    pub const WATER_BREATHING: Self = Self(potion_effects::WATER_BREATHING);
    /// `minecraft:invisibility` (`13`).
    pub const INVISIBILITY: Self = Self(potion_effects::INVISIBILITY);
    /// `minecraft:blindness` (`14`).
    pub const BLINDNESS: Self = Self(potion_effects::BLINDNESS);
    /// `minecraft:night_vision` (`15`).
    pub const NIGHT_VISION: Self = Self(potion_effects::NIGHT_VISION);
    /// `minecraft:hunger` (`16`).
    pub const HUNGER: Self = Self(potion_effects::HUNGER);
    /// `minecraft:weakness` (`17`).
    pub const WEAKNESS: Self = Self(potion_effects::WEAKNESS);
    /// `minecraft:poison` (`18`).
    pub const POISON: Self = Self(potion_effects::POISON);
    /// `minecraft:wither` (`19`).
    pub const WITHER: Self = Self(potion_effects::WITHER);
    /// `minecraft:health_boost` (`20`).
    pub const HEALTH_BOOST: Self = Self(potion_effects::HEALTH_BOOST);
    /// `minecraft:absorption` (`21`).
    pub const ABSORPTION: Self = Self(potion_effects::ABSORPTION);
    /// `minecraft:saturation` (`22`).
    pub const SATURATION: Self = Self(potion_effects::SATURATION);
    /// `minecraft:glowing` (`23`).
    pub const GLOWING: Self = Self(potion_effects::GLOWING);
    /// `minecraft:levitation` (`24`).
    pub const LEVITATION: Self = Self(potion_effects::LEVITATION);
    /// `minecraft:slow_falling` (`27`).
    pub const SLOW_FALLING: Self = Self(potion_effects::SLOW_FALLING);
    /// `minecraft:darkness` (`32`).
    pub const DARKNESS: Self = Self(potion_effects::DARKNESS);

    /// Constructs a [`PotionEffect`] from a validated protocol ID (`0..40`).
    pub const fn from_id(id: u32) -> Result<Self, EntityError> {
        if (id as usize) < potion_effects::POTION_EFFECT_COUNT {
            Ok(Self(id))
        } else {
            Err(EntityError::UnknownPotionEffect(id))
        }
    }

    /// Looks up a [`PotionEffect`] by its `minecraft:...` resource key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        potion_effects::ENTRIES
            .iter()
            .find(|(_, k)| *k == key)
            .map(|(id, _)| Self(*id))
    }

    /// Numeric protocol ID.
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Canonical `minecraft:...` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        potion_effects::ENTRIES[self.0 as usize].1
    }
}

/// Active potion instance (`Potion` record in Minestom).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Potion {
    /// Underlying [`PotionEffect`].
    pub effect: PotionEffect,
    /// Effect level minus one (`0` = Level I).
    pub amplifier: i32,
    /// Remaining duration in ticks (`-1` = infinite).
    pub duration: i32,
    /// Bitflags (`AMBIENT_FLAG`, `PARTICLES_FLAG`, `ICON_FLAG`, `BLEND_FLAG`).
    pub flags: u8,
}

impl Potion {
    /// Beacon/ambient particle flag (`0x01`).
    pub const AMBIENT_FLAG: u8 = 0x01;
    /// Emit swirling particles (`0x02`).
    pub const PARTICLES_FLAG: u8 = 0x02;
    /// Display HUD status icon (`0x04`).
    pub const ICON_FLAG: u8 = 0x04;
    /// Smooth screen blend flag (`0x08`, darkness).
    pub const BLEND_FLAG: u8 = 0x08;
    /// Default flags (`PARTICLES_FLAG | ICON_FLAG`).
    pub const DEFAULT_FLAGS: u8 = Self::PARTICLES_FLAG | Self::ICON_FLAG;
    /// Infinite duration sentinel (`-1`).
    pub const INFINITE_DURATION: i32 = -1;

    /// Constructs a [`Potion`] with particles and HUD icon enabled.
    #[must_use]
    pub const fn new(effect: PotionEffect, amplifier: i32, duration: i32) -> Self {
        Self {
            effect,
            amplifier,
            duration,
            flags: Self::DEFAULT_FLAGS,
        }
    }

    /// Overrides the raw flag byte.
    #[must_use]
    pub const fn with_flags(mut self, flags: u8) -> Self {
        self.flags = flags;
        self
    }

    /// Builds the clientbound [`EntityEffectPacket`] for `entity_id`.
    #[must_use]
    pub const fn to_add_packet(self, entity_id: i32) -> EntityEffectPacket {
        EntityEffectPacket {
            entity_id,
            effect_id: self.effect.id() as i32,
            amplifier: self.amplifier,
            duration: self.duration,
            flags: self.flags,
        }
    }

    /// Builds the clientbound [`RemoveEntityEffectPacket`] for `entity_id`.
    #[must_use]
    pub const fn to_remove_packet(
        effect: PotionEffect,
        entity_id: i32,
    ) -> RemoveEntityEffectPacket {
        RemoveEntityEffectPacket {
            entity_id,
            potion_effect_id: effect.id() as i32,
        }
    }
}

/// Active potion effects map on an entity or player, ticking down finite durations at 20 TPS.
#[derive(Debug, Clone, Default)]
pub struct EffectMap {
    active: BTreeMap<u32, Potion>,
}

impl EffectMap {
    /// Creates an empty [`EffectMap`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active: BTreeMap::new(),
        }
    }

    /// Adds or replaces an active [`Potion`].
    pub fn add(&mut self, potion: Potion) {
        self.active.insert(potion.effect.id(), potion);
    }

    /// Removes `effect`, returning `true` if it was active.
    pub fn remove(&mut self, effect: PotionEffect) -> bool {
        self.active.remove(&effect.id()).is_some()
    }

    /// Clears all active effects and returns their [`PotionEffect`] types.
    pub fn clear(&mut self) -> Vec<PotionEffect> {
        let removed: Vec<PotionEffect> = self.active.values().map(|p| p.effect).collect();
        self.active.clear();
        removed
    }

    /// Returns the active [`Potion`] for `effect`, if present.
    #[must_use]
    pub fn get(&self, effect: PotionEffect) -> Option<&Potion> {
        self.active.get(&effect.id())
    }

    /// Returns `true` if `effect` is currently active.
    #[must_use]
    pub fn has(&self, effect: PotionEffect) -> bool {
        self.active.contains_key(&effect.id())
    }

    /// Iterates over all active [`Potion`] entries.
    pub fn iter(&self) -> impl Iterator<Item = &Potion> {
        self.active.values()
    }

    /// Advances finite durations by 1 tick and removes expired effects, returning expired types.
    pub fn tick(&mut self) -> Vec<PotionEffect> {
        let mut expired = Vec::new();
        self.active.retain(|_, potion| {
            if potion.duration == Potion::INFINITE_DURATION {
                true
            } else if potion.duration <= 1 {
                expired.push(potion.effect);
                false
            } else {
                potion.duration -= 1;
                true
            }
        });
        expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_map_ticks_down_and_preserves_infinite() {
        let mut effects = EffectMap::new();
        effects.add(Potion::new(PotionEffect::SPEED, 1, 2));
        effects.add(Potion::new(
            PotionEffect::NIGHT_VISION,
            0,
            Potion::INFINITE_DURATION,
        ));

        assert_eq!(effects.tick().len(), 0);
        assert_eq!(effects.get(PotionEffect::SPEED).unwrap().duration, 1);

        let expired = effects.tick();
        assert_eq!(expired, vec![PotionEffect::SPEED]);
        assert!(!effects.has(PotionEffect::SPEED));
        assert!(effects.has(PotionEffect::NIGHT_VISION));
    }
}
