//! M9 Showcase: Entity AI (`EntityAIGroup`, `MeleeAttackGoal`, `ClosestEntityTarget`,
//! `RandomStrollGoal`), 3D A* obstacle pathfinding (`Navigator`), Entity Attributes &
//! Modifiers, Potion Effects, Combat Damage/Knockback, and Death/Respawn.

use minestom::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let server = MinecraftServer::builder()
        .motd(Component::text("minestom-rs 26.2 — Entity AI & Combat").color(NamedTextColor::Red))
        .build()?;

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::STONE);
    });

    // Build an obstacle wall between spawn and the guardian zombie so A* routes around it!
    let mut wall = AbsoluteBlockBatch::new();
    for z in -3..=3 {
        wall.set_block(BlockVec::new(4, 40, z), Block::COBBLESTONE);
        wall.set_block(BlockVec::new(4, 41, z), Block::COBBLESTONE);
    }
    server.apply_batch_synced(&instance, &wall);

    // Spawn an AI Arena Zombie on the far side of the wall
    let ai = EntityAIGroup::new()
        .with_target_selector(LastEntityDamagerTarget::new(24.0))
        .with_target_selector(ClosestEntityTarget::players(20.0))
        .with_goal_selector(MeleeAttackGoal::new(0.24, 2.1, 20))
        .with_goal_selector(RandomStrollGoal::new(6, 0.14, 60))
        .with_goal_selector(RandomLookAroundGoal::new(40));

    let zombie = Entity::creature(EntityType::ZOMBIE)
        .with_custom_name(Component::text("Arena Guardian").color(NamedTextColor::Gold))
        .with_max_health(30.0)
        .with_attribute(Attribute::ATTACK_DAMAGE, 4.0)
        .with_behavior(ai);
    server.spawn_entity(&instance, zombie, Pos::new(8.5, 40.0, 0.5));

    // Register `/buff` command granting Speed II, Strength I, and +20% Movement Speed modifier
    let srv_cmd = server.clone();
    server
        .command_manager()
        .register(
            Command::new("buff").with_default_executor(move |sender, _ctx| {
                if let Some(pid) = sender.as_player().map(|p| p.player_id) {
                    srv_cmd.add_player_effect(pid, Potion::new(PotionEffect::SPEED, 1, 600));
                    srv_cmd.add_player_effect(pid, Potion::new(PotionEffect::STRENGTH, 0, 600));
                    srv_cmd.add_player_attribute_modifier(
                        pid,
                        Attribute::MOVEMENT_SPEED,
                        AttributeModifier::new(
                            "arena:adrenaline",
                            0.20,
                            AttributeOperation::AddMultipliedTotal,
                        ),
                    );
                    sender.send_message(
                        Component::text("Granted Speed II, Strength I, and +20% Adrenaline!")
                            .color(NamedTextColor::Green),
                    );
                }
            }),
        );

    let inst_cfg = instance.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&inst_cfg);
            ev.player().set_respawn_point(Pos::new(0.5, 40.0, 0.5));
            ev.player().set_game_mode(GameMode::Survival);
        });

    let srv_spawn = server.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut PlayerSpawnEvent| {
            if ev.first_spawn {
                srv_spawn
                    .give_player_item(ev.player_id, ItemStack::single(Material::DIAMOND_SWORD));
            }
        });

    server
        .global_event_handler()
        .add_listener(|ev: &mut PlayerRespawnEvent| {
            ev.respawn_position = Pos::new(-4.5, 40.0, 0.5);
        });

    server.start("0.0.0.0:25565").await?;
    Ok(())
}
