//! Generated Minecraft 26.2 (protocol 776, data version 4903) constants and registry data.
//!
//! All modules under [`generated`] are deterministically produced by `cargo xtask codegen`
//! from `net.minestom:data:26.2-rv3` and committed to version control so normal builds
//! require no network access.

/// Deterministically generated data tables.
#[rustfmt::skip]
#[allow(clippy::unreadable_literal, clippy::approx_constant)]
pub mod generated {
    /// Generated Entity Attribute specs & IDs (`minecraft:attribute`, `40` entries).
    pub mod attributes {
        include!("generated/attributes.rs");
    }
    /// Generated Block Entity Type IDs (`minecraft:block_entity_type`, `49` entries).
    pub mod block_entity_types {
        include!("generated/block_entity_types.rs");
    }
    /// Generated block definitions, per-state tables, shapes, and default state constants (`1,196` blocks, `32,366` states).
    pub mod block_states {
        include!("generated/block_states.rs");
    }
    /// Generated Block IDs (`minecraft:block`, `1,196` entries).
    pub mod blocks {
        include!("generated/blocks.rs");
    }
    /// Generated target Minecraft version and protocol constants.
    pub mod constants {
        include!("generated/constants.rs");
    }
    /// Generated Damage Type specs & IDs (`minecraft:damage_type`, `51` entries).
    pub mod damage_types {
        include!("generated/damage_types.rs");
    }
    /// Generated MC 26.2 configuration dynamic registries (`27` registries).
    pub mod dynamic_registries {
        include!("generated/dynamic_registries.rs");
    }
    /// Generated Enchantment specs & IDs (`minecraft:enchantment`, `43` entries).
    pub mod enchantments {
        include!("generated/enchantments.rs");
    }
    /// Generated `EntityType` specs, aerodynamics, default attributes & IDs (`minecraft:entity_type`, `158` entries).
    pub mod entity_types {
        include!("generated/entity_types.rs");
    }
    /// Generated Fluid specs & IDs (`minecraft:fluid`, `5` entries).
    pub mod fluids {
        include!("generated/fluids.rs");
    }
    /// Generated `GameEvent` specs & IDs (`minecraft:game_event`, `61` entries).
    pub mod game_events {
        include!("generated/game_events.rs");
    }
    /// Generated Item Material specs, attribute modifiers & IDs (`minecraft:item`, `1,537` entries).
    pub mod materials {
        include!("generated/materials.rs");
    }
    /// Generated Painting Variant specs & IDs (`minecraft:painting_variant`, `51` entries).
    pub mod painting_variants {
        include!("generated/painting_variants.rs");
    }
    /// Generated Particle specs & IDs (`minecraft:particle_type`, `125` entries).
    pub mod particles {
        include!("generated/particles.rs");
    }
    /// Generated Potion/`MobEffect` specs & IDs (`minecraft:mob_effect`, `40` entries).
    pub mod potion_effects {
        include!("generated/potion_effects.rs");
    }
    /// Generated resolved tag lists for all 32 client tag registries.
    pub mod registry_tags {
        include!("generated/registry_tags.rs");
    }
    /// Generated `SoundEvent` IDs (`minecraft:sound_event`, `1,968` entries).
    pub mod sounds {
        include!("generated/sounds.rs");
    }
    /// Generated Villager Profession IDs (`minecraft:villager_profession`, `15` entries).
    pub mod villager_professions {
        include!("generated/villager_professions.rs");
    }
}

pub use generated::constants::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_version_and_counts_match_minestom_26_2() {
        assert_eq!(VERSION_NAME, "26.2");
        assert_eq!(PROTOCOL_VERSION, 776);
        assert_eq!(DATA_VERSION, 4903);
        assert_eq!(RESOURCE_PACK_VERSION, (88, 0));
        assert_eq!(DATA_PACK_VERSION, (107, 1));
        assert_eq!(
            generated::dynamic_registries::CONFIGURATION_REGISTRIES.len(),
            27
        );
        assert_eq!(generated::registry_tags::CLIENT_TAG_REGISTRIES.len(), 32);
        assert_eq!(generated::block_states::BLOCK_INFOS.len(), 1196);
        assert_eq!(generated::block_states::STATE_INFOS.len(), 32_366);
        assert_eq!(generated::materials::MATERIAL_INFOS.len(), 1537);
        assert_eq!(generated::entity_types::ENTITY_INFOS.len(), 158);
        assert_eq!(generated::attributes::ATTRIBUTE_COUNT, 40);
        assert_eq!(generated::potion_effects::POTION_EFFECT_COUNT, 40);
        assert_eq!(generated::damage_types::DAMAGE_TYPE_COUNT, 51);
        assert_eq!(generated::enchantments::ENCHANTMENT_COUNT, 43);
        assert_eq!(generated::particles::PARTICLE_COUNT, 125);
        assert_eq!(generated::sounds::SOUND_EVENT_COUNT, 1968);
        assert_eq!(generated::block_entity_types::BLOCK_ENTITY_TYPE_COUNT, 49);
        assert_eq!(generated::fluids::FLUID_COUNT, 5);
        assert_eq!(generated::game_events::GAME_EVENT_COUNT, 61);
        assert_eq!(
            generated::block_states::lookup_block_default_state("minecraft:stone"),
            generated::block_states::lookup_block_default_state("stone")
        );
        assert_eq!(
            generated::block_states::lookup_block_default_state("other:stone"),
            None
        );
    }
}
