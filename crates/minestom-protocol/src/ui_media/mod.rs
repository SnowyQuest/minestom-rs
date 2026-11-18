//! UI, HUD, media, scoreboard, teams, bossbar, titles, particles, sounds,
//! resource packs, advancements, recipes, and MC 26.2 dialog packets (protocol 776).

mod bossbar_title;
pub(crate) mod particle_sound;
mod recipe_advancement;
mod scoreboard;

pub use bossbar_title::{
    ActionBarPacket, BossBarAction, BossBarColor, BossBarOverlay, BossBarPacket, ClearTitlesPacket,
    PlayerListHeaderAndFooterPacket, SetTitleSubTitlePacket, SetTitleTextPacket,
    SetTitleTimePacket,
};
pub use particle_sound::{
    ClientResourcePackStatusPacket, ParticleData, ParticleEntry, ParticlePacket,
    ResourcePackPopPacket, ResourcePackPushPacket, ResourcePackStatus, SoundEffectPacket,
    SoundEvent, SoundHolder, SoundSource, StopSoundPacket,
};
pub use recipe_advancement::{
    AdvancementDisplayData, AdvancementEntry, AdvancementFrameType, AdvancementProgressEntry,
    AdvancementsPacket, ClearDialogPacket, DeclareRecipesPacket, DialogHolder, RecipePropertySet,
    ShowDialogPacket, StonecutterRecipeEntry,
};
pub use scoreboard::{
    CollisionRule, DisplayScoreboardPacket, NameTagVisibility, ObjectiveAction,
    ObjectiveRenderType, ResetScorePacket, ScoreboardObjectivePacket, SidebarNumberFormat,
    TeamColor, TeamSettings, TeamsAction, TeamsPacket, UpdateScorePacket,
};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use minestom_core::Vec3;
    use minestom_item::{ItemStack, Material};
    use minestom_nbt::{NbtCompound, NbtTag};
    use minestom_text::Component;
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::*;
    use crate::{decode_packet_exact, encode_packet_payload, read_var_int};

    #[test]
    #[allow(clippy::too_many_lines)]
    fn ui_media_packets_roundtrip() {
        let boss = BossBarPacket {
            uuid: Uuid::from_u128(99),
            action: BossBarAction::Add {
                title: Component::text("Raid Boss"),
                health: 0.75,
                color: BossBarColor::Purple,
                overlay: BossBarOverlay::Notched10,
                flags: 0x01,
            },
        };
        let mut b = encode_packet_payload(&boss).unwrap();
        assert_eq!(read_var_int(&mut b).unwrap(), 0x09);
        assert_eq!(decode_packet_exact::<BossBarPacket>(b).unwrap(), boss);

        let title = SetTitleTextPacket {
            title: Component::text("Victory"),
        };
        let mut tb = encode_packet_payload(&title).unwrap();
        assert_eq!(read_var_int(&mut tb).unwrap(), 0x72);
        assert_eq!(
            decode_packet_exact::<SetTitleTextPacket>(tb).unwrap(),
            title
        );

        let obj = ScoreboardObjectivePacket {
            objective_name: "sb-1".to_owned(),
            action: ObjectiveAction::Create {
                title: Component::text("Stats"),
                render_type: ObjectiveRenderType::Integer,
                number_format: Some(SidebarNumberFormat::Blank),
            },
        };
        let mut ob = encode_packet_payload(&obj).unwrap();
        assert_eq!(read_var_int(&mut ob).unwrap(), 0x6A);
        assert_eq!(
            decode_packet_exact::<ScoreboardObjectivePacket>(ob).unwrap(),
            obj
        );

        let team = TeamsPacket {
            team_name: "red".to_owned(),
            action: TeamsAction::Create {
                settings: TeamSettings {
                    display_name: Component::text("Red"),
                    prefix: Component::text("[RED] "),
                    suffix: Component::empty(),
                    name_tag_visibility: NameTagVisibility::Always,
                    collision_rule: CollisionRule::Never,
                    color: Some(TeamColor::Red),
                    friendly_flags: 0x02,
                },
                entities: vec!["Alice".to_owned()],
            },
        };
        let mut tmb = encode_packet_payload(&team).unwrap();
        assert_eq!(read_var_int(&mut tmb).unwrap(), 0x6D);
        assert_eq!(decode_packet_exact::<TeamsPacket>(tmb).unwrap(), team);

        let part = ParticlePacket {
            particle_id: 21,
            data: ParticleData::RgbScale {
                rgb: 0xFF_00_55,
                scale: 1.5,
            },
            override_limiter: true,
            long_distance: false,
            position: Vec3::new(1.0, 42.0, -3.0),
            offset: (0.2, 0.4, 0.2),
            max_speed: 0.05,
            particle_count: 12,
        };
        let mut pb = encode_packet_payload(&part).unwrap();
        assert_eq!(read_var_int(&mut pb).unwrap(), 0x2F);
        assert_eq!(decode_packet_exact::<ParticlePacket>(pb).unwrap(), part);

        let snd = SoundEffectPacket::at(
            SoundHolder::Builtin(SoundEvent::ENTITY_PLAYER_LEVELUP.id()),
            SoundSource::Master,
            Vec3::new(4.0, 40.0, 8.0),
            1.0,
            1.2,
            777,
        );
        let mut sb = encode_packet_payload(&snd).unwrap();
        assert_eq!(read_var_int(&mut sb).unwrap(), 0x75);
        assert_eq!(decode_packet_exact::<SoundEffectPacket>(sb).unwrap(), snd);

        let mut props = BTreeMap::new();
        props.insert(
            RecipePropertySet::FurnaceInput.into(),
            vec![Material::DIAMOND.id()],
        );
        let recipes = DeclareRecipesPacket {
            item_properties: props,
            stonecutter_recipes: vec![StonecutterRecipeEntry {
                input_item_ids: vec![Material::COBBLESTONE.id()],
                result: ItemStack::single(Material::STONE),
            }],
        };
        let mut rec_b = encode_packet_payload(&recipes).unwrap();
        assert_eq!(read_var_int(&mut rec_b).unwrap(), 0x85);
        assert_eq!(
            decode_packet_exact::<DeclareRecipesPacket>(rec_b).unwrap(),
            recipes
        );

        let mut dlg_nbt = NbtCompound::new();
        dlg_nbt.insert("type", NbtTag::String("minecraft:notice".to_owned()));
        let dlg = ShowDialogPacket {
            dialog: DialogHolder::Direct(NbtTag::Compound(dlg_nbt)),
        };
        let mut db = encode_packet_payload(&dlg).unwrap();
        assert_eq!(read_var_int(&mut db).unwrap(), 0x8C);
        assert_eq!(decode_packet_exact::<ShowDialogPacket>(db).unwrap(), dlg);
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic_ui_media_decoders(raw in prop::collection::vec(any::<u8>(), 0..256)) {
            let b = bytes::Bytes::from(raw);
            let _ = decode_packet_exact::<BossBarPacket>(b.clone());
            let _ = decode_packet_exact::<ClearTitlesPacket>(b.clone());
            let _ = decode_packet_exact::<ActionBarPacket>(b.clone());
            let _ = decode_packet_exact::<SetTitleSubTitlePacket>(b.clone());
            let _ = decode_packet_exact::<SetTitleTextPacket>(b.clone());
            let _ = decode_packet_exact::<SetTitleTimePacket>(b.clone());
            let _ = decode_packet_exact::<PlayerListHeaderAndFooterPacket>(b.clone());
            let _ = decode_packet_exact::<ScoreboardObjectivePacket>(b.clone());
            let _ = decode_packet_exact::<DisplayScoreboardPacket>(b.clone());
            let _ = decode_packet_exact::<UpdateScorePacket>(b.clone());
            let _ = decode_packet_exact::<ResetScorePacket>(b.clone());
            let _ = decode_packet_exact::<TeamsPacket>(b.clone());
            let _ = decode_packet_exact::<ParticlePacket>(b.clone());
            let _ = decode_packet_exact::<SoundEffectPacket>(b.clone());
            let _ = decode_packet_exact::<StopSoundPacket>(b.clone());
            let _ = decode_packet_exact::<ResourcePackPushPacket>(b.clone());
            let _ = decode_packet_exact::<ResourcePackPopPacket>(b.clone());
            let _ = decode_packet_exact::<ClientResourcePackStatusPacket>(b.clone());
            let _ = decode_packet_exact::<AdvancementsPacket>(b.clone());
            let _ = decode_packet_exact::<DeclareRecipesPacket>(b.clone());
            let _ = decode_packet_exact::<ClearDialogPacket>(b.clone());
            let _ = decode_packet_exact::<ShowDialogPacket>(b);
        }
    }
}
