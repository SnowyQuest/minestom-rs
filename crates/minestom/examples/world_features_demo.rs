//! M7 example: `SchedulerManager` (`TickStart`/`TickEnd` repeating tasks), `SharedInstance`,
//! Anvil `.mca` persistence (`AnvilLoader`), opt-in 3D BFS sky & block lighting (`LightingChunk`),
//! `WorldBorder`, MC 26.2 `WorldTime`, and `Weather`!
//!
//! Run with:
//! ```bash
//! cargo run -p minestom --example world_features_demo
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use minestom::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let motd = Component::text("minestom-rs 26.2")
        .color(NamedTextColor::Gold)
        .bold(true)
        .append(
            Component::text(" - Scheduler, Lighting, SharedInstance & Anvil")
                .color(NamedTextColor::Gray),
        );
    let server = MinecraftServer::builder().motd(motd).build()?;

    let world_dir = std::env::temp_dir().join("minestom_rs_demo_world");
    let main_instance = server.instance_manager().create_instance_container();
    main_instance.set_lighting_enabled(true);
    main_instance.set_chunk_loader(AnvilLoader::with_dimension(
        &world_dir,
        "minecraft:overworld",
    ));
    main_instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::STONE);
        unit.modifier().fill_height(40, 41, Block::GRASS_BLOCK);
    });
    main_instance.set_world_border(WorldBorder::with_diameter(0.0, 0.0, 128.0));
    main_instance.set_world_time(WorldTime {
        world_age: 0,
        day_time: 18_000, // Midnight so pulsing Glowstone block light stands out!
        clock_rate: 0.0,
    });

    // Zero-copy clone sharing all chunk data with `main_instance`!
    let _arena_clone = server
        .instance_manager()
        .create_shared_instance(&main_instance);

    // Repeating 20 TPS task toggling a Glowstone beacon every 20 ticks (1s) at TickEnd!
    let lit = Arc::new(AtomicBool::new(false));
    let srv_for_task = server.clone();
    let inst_for_task = main_instance.clone();
    server
        .scheduler_manager()
        .build_task(move || {
            let on = !lit.fetch_xor(true, Ordering::Relaxed);
            let blk = if on { Block::GLOWSTONE } else { Block::GLASS };
            srv_for_task.set_block_synced(&inst_for_task, BlockVec::new(4, 42, 0), blk, None);
        })
        .delay(TaskSchedule::Tick(20))
        .repeat(TaskSchedule::Tick(20))
        .execution_type(ExecutionType::TickEnd)
        .schedule();

    let srv_weather = server.clone();
    let inst_weather = main_instance.clone();
    let w_arg = Argument::choices("state", ["clear", "rain", "thunder"]);
    let w_ref = w_arg.clone();
    server
        .command_manager()
        .register(
            Command::new("weather").add_syntax([w_arg.untyped()], move |sender, ctx| {
                let choice = ctx.get(&w_ref).expect("validated choice").as_str();
                let w = match choice {
                    "rain" => Weather::RAIN,
                    "thunder" => Weather::THUNDER,
                    _ => Weather::CLEAR,
                };
                srv_weather.set_weather(&inst_weather, w);
                sender.send_message(format!("Updated weather to {choice}"));
            }),
        );

    let spawn_inst = main_instance.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&spawn_inst);
            ev.player().set_respawn_point(Pos::new(0.5, 42.0, 0.5));
            ev.player().set_game_mode(GameMode::Creative);
        });

    server.start("0.0.0.0:25565").await?;
    Ok(())
}
