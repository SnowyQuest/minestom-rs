//! M8 example: Production Online-Mode / Velocity / `BungeeGuard` configuration, per-connection
//! packet rate limiting, and `Ctrl+C` graceful shutdown automatically kicking players & saving
//! Anvil `.mca` chunks!
//!
//! Run with:
//! ```bash
//! cargo run -p minestom --example production_network
//! # Or with Velocity forwarding:
//! VELOCITY_SECRET=dev-secret cargo run -p minestom --example production_network
//! ```

use minestom::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let auth = if let Ok(secret) = std::env::var("VELOCITY_SECRET") {
        AuthMode::velocity(secret)
    } else if let Ok(token) = std::env::var("BUNGEEGUARD_TOKEN") {
        AuthMode::bungee_guard([token])
    } else {
        AuthMode::online()
    };

    let motd = Component::text("minestom-rs 26.2")
        .color(NamedTextColor::Gold)
        .bold(true)
        .append(Component::text(" - Production Network & Auth").color(NamedTextColor::Green));

    let server = MinecraftServer::builder()
        .motd(motd)
        .auth(auth)
        .max_packets_per_second(Some(400))
        .build()?;

    let world_dir = std::env::temp_dir().join("minestom_rs_prod_world");
    let instance = server.instance_manager().create_instance_container();
    instance.set_chunk_loader(AnvilLoader::with_dimension(
        &world_dir,
        "minecraft:overworld",
    ));
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::STONE);
        unit.modifier().fill_height(40, 41, Block::GRASS_BLOCK);
    });

    let spawn_inst = instance.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&spawn_inst);
            ev.player().set_respawn_point(Pos::new(0.5, 42.0, 0.5));
            ev.player().set_game_mode(GameMode::Survival);
        });

    let shutdown_handle = server.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            shutdown_handle.stop_cleanly();
        }
    });

    server.start("0.0.0.0:25565").await?;
    Ok(())
}
