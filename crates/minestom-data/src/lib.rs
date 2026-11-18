//! Generated Minecraft 26.2 (protocol 776, data version 4903) constants and registry data.
//!
//! All modules under [`generated`] are deterministically produced by `cargo xtask codegen`
//! from `net.minestom:data:26.2-rv3` and committed to version control so normal builds
//! require no network access.

/// Deterministically generated data tables.
#[allow(clippy::unreadable_literal)]
pub mod generated {
    /// Generated Entity Attribute IDs (`minecraft:attribute`).
    pub mod attributes {
        include!("generated/attributes.rs");
    }
    /// Generated Block IDs (`minecraft:block`).
    pub mod blocks {
        include!("generated/blocks.rs");
    }
    /// Generated target Minecraft version and protocol constants.
    pub mod constants {
        include!("generated/constants.rs");
    }
    /// Generated `EntityType` IDs (`minecraft:entity_type`).
    pub mod entity_types {
        include!("generated/entity_types.rs");
    }
    /// Generated Fluid IDs (`minecraft:fluid`).
    pub mod fluids {
        include!("generated/fluids.rs");
    }
    /// Generated `GameEvent` IDs (`minecraft:game_event`).
    pub mod game_events {
        include!("generated/game_events.rs");
    }
    /// Generated Item Material IDs (`minecraft:item`).
    pub mod materials {
        include!("generated/materials.rs");
    }
    /// Generated Particle IDs (`minecraft:particle_type`).
    pub mod particles {
        include!("generated/particles.rs");
    }
    /// Generated Potion/`MobEffect` IDs (`minecraft:mob_effect`).
    pub mod potion_effects {
        include!("generated/potion_effects.rs");
    }
    /// Generated `SoundEvent` IDs (`minecraft:sound_event`).
    pub mod sounds {
        include!("generated/sounds.rs");
    }
}

pub use generated::constants::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_version_matches_minestom_26_2() {
        assert_eq!(VERSION_NAME, "26.2");
        assert_eq!(PROTOCOL_VERSION, 776);
        assert_eq!(DATA_VERSION, 4903);
        assert_eq!(RESOURCE_PACK_VERSION, (88, 0));
        assert_eq!(DATA_PACK_VERSION, (107, 1));
    }
}
