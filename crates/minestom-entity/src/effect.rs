//! `PotionEffect`, `Potion`, and active `EffectMap` ticking
//! (`net.minestom.server.potion` equivalent for MC 26.2, `40` generated effects).

use std::collections::BTreeMap;
use std::fmt;

use minestom_data::generated::potion_effects::{self, POTION_EFFECT_INFOS, PotionEffectInfo};
use minestom_protocol::entity::{EntityEffectPacket, RemoveEntityEffectPacket};

use crate::EntityError;

/// Strongly typed `Copy` handle for all **40** Minecraft 26.2 `PotionEffect`s (`minecraft:mob_effect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PotionEffect(u32);

minestom_data::impl_potion_effect_constants!(PotionEffect);

impl PotionEffect {
    /// Constructs a [`PotionEffect`] from a validated protocol ID (`0..40`).
    pub const fn from_id(id: u32) -> Result<Self, EntityError> {
        if (id as usize) < potion_effects::POTION_EFFECT_COUNT {
            Ok(Self(id))
        } else {
            Err(EntityError::UnknownPotionEffect(id))
        }
    }

    /// Zero-allocation `O(log N)` lookup by `"minecraft:..."` or bare `"..."` key.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        potion_effects::lookup_key(key).map(Self)
    }

    /// Numeric protocol ID (`0..40`).
    #[must_use]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// Static generated [`PotionEffectInfo`] entry.
    #[must_use]
    pub const fn info(self) -> &'static PotionEffectInfo {
        &POTION_EFFECT_INFOS[self.0 as usize]
    }

    /// Canonical `"minecraft:..."` key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.info().key
    }

    /// Particle RGB color (`0xRRGGBB`).
    #[must_use]
    pub const fn color(self) -> u32 {
        self.info().color
    }

    /// Returns `true` if this effect applies instantaneously (`instant_health`, `instant_damage`, `saturation`).
    #[must_use]
    pub const fn is_instantaneous(self) -> bool {
        self.info().instantaneous
    }
}

impl fmt::Display for PotionEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
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

impl<'a> IntoIterator for &'a EffectMap {
    type Item = &'a Potion;
    type IntoIter = std::collections::btree_map::Values<'a, u32, Potion>;

    fn into_iter(self) -> Self::IntoIter {
        self.active.values()
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
        assert!(PotionEffect::INSTANT_HEALTH.is_instantaneous());
        assert_eq!(effects.tick().len(), 0);
        assert_eq!(effects.get(PotionEffect::SPEED).unwrap().duration, 1);
        assert_eq!(effects.tick(), vec![PotionEffect::SPEED]);
        assert!(!effects.has(PotionEffect::SPEED));
        assert!(effects.has(PotionEffect::NIGHT_VISION));
    }
}
