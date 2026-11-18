//! M2 Demo: Target flat-world server where an unmodified Minecraft 26.2 vanilla client
//! can join in offline mode and walk around a generated `GRASS_BLOCK` world.
//!
//! Run with:
//! `cargo run -p minestom --example flat_world`

use minestom::prelude::*;

#[tokio::main]
async fn main() -> Result<(), minestom::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let server = MinecraftServer::builder().build()?;

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
    });

    server
        .global_event_handler()
        .add_listener(move |event: &mut AsyncPlayerConfigurationEvent| {
            event.set_spawning_instance(&instance);
            event.player().set_respawn_point(Pos::new(0.0, 42.0, 0.0));
            event.player().set_game_mode(GameMode::Creative);
        });

    let shutdown = server.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            shutdown.stop_cleanly();
        }
    });

    server.start("0.0.0.0:25565").await?;
    Ok(())
}
