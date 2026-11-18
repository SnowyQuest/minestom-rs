//! M4 Runnable Example: Multi-player visibility + compositional `Entity` with custom
//! 20 TPS `EntityBehavior` patrolling in a circle while landing on solid terrain via
//! AABB `BoundingBox` gravity & block collision!
//!
//! Run with:
//! ```bash
//! cargo run -p minestom --example multiplayer_entities
//! ```

use minestom::block::BlockGetter;
use minestom::prelude::*;

struct CirclePatrolBehavior {
    center: Vec3,
    radius: f64,
}

impl EntityBehavior for CirclePatrolBehavior {
    fn tick(
        &mut self,
        core: &mut EntityCore,
        _living: Option<&mut LivingState>,
        _world: &dyn BlockGetter,
    ) {
        let angle = (core.alive_ticks as f64) * 0.08;
        let target_x = self.center.x + self.radius * angle.cos();
        let target_z = self.center.z + self.radius * angle.sin();
        core.velocity.x = (target_x - core.position.x) * 0.25;
        core.velocity.z = (target_z - core.position.z) * 0.25;
        let yaw = (-(core.velocity.x).atan2(core.velocity.z)).to_degrees() as f32;
        core.position.yaw = yaw;
        core.head_yaw = yaw;
    }
}

#[tokio::main]
async fn main() -> Result<(), minestom::Error> {
    tracing_subscriber::fmt()
        .with_env_filter("info,minestom=debug")
        .init();

    let server = MinecraftServer::builder()
        .motd(
            Component::text("minestom-rs 26.2 ")
                .color(NamedTextColor::Gold)
                .append(
                    Component::text("Multiplayer & Entities Demo").color(NamedTextColor::Green),
                ),
        )
        .build()?;

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
    });

    // Spawn a patrolling zombie guard starting in mid-air at Y = 44.0 (lands cleanly on Y = 40.0)
    let guard = Entity::creature(EntityType::ZOMBIE)
        .with_custom_name(Component::text("Patrol Guard").color(NamedTextColor::Yellow))
        .with_behavior(CirclePatrolBehavior {
            center: Vec3::new(0.0, 40.0, 6.0),
            radius: 3.0,
        });
    server.spawn_entity(&instance, guard, Pos::new(3.0, 44.0, 6.0));

    // Register /summon <mob> command
    let mob_arg = Argument::choices("mob", ["zombie", "skeleton", "creeper", "pig", "cow"]);
    let mob_ref = mob_arg.clone();
    let srv_for_cmd = server.clone();
    let inst_for_cmd = instance.clone();
    server
        .command_manager()
        .register(
            Command::new("summon").add_syntax([mob_arg.untyped()], move |sender, ctx| {
                let choice = ctx.get(&mob_ref).map_or("zombie", String::as_str);
                let etype = match choice {
                    "skeleton" => EntityType::SKELETON,
                    "creeper" => EntityType::CREEPER,
                    "pig" => EntityType::PIG,
                    "cow" => EntityType::COW,
                    _ => EntityType::ZOMBIE,
                };
                let pos = sender.origin_position();
                let id = srv_for_cmd.spawn_entity(&inst_for_cmd, Entity::creature(etype), pos);
                sender.send_message(format!("Spawned {} (#{id})", etype.key()));
            }),
        );

    let spawn_inst = instance.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&spawn_inst);
            ev.player().set_respawn_point(Pos::new(0.0, 40.0, 0.0));
            ev.player().set_game_mode(GameMode::Creative);
        });

    server.start("0.0.0.0:25565").await?;
    Ok(())
}
