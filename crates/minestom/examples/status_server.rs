//! M1 Demo: Vanilla Minecraft 26.2 Server List Status, Rich MOTD, Player Sample, and PNG Favicon.
//!
//! Run with:
//! `cargo run -p minestom --example status_server`

use minestom::core::offline_player_uuid;
use minestom::prelude::*;
use minestom::protocol::status::{StatusPlayerSample, encode_rgba_64x64_png};

fn generate_demo_favicon() -> Vec<u8> {
    let mut pixels = [[0_u8; 4]; 64 * 64];
    for y in 0..64 {
        for x in 0..64 {
            let border = !(4..60).contains(&x) || !(4..60).contains(&y);
            let r = if border { 255 } else { (x * 4) as u8 };
            let g = if border { 170 } else { (y * 3) as u8 };
            let b = if border { 0 } else { 90 };
            pixels[y * 64 + x] = [r, g, b, 255];
        }
    }
    encode_rgba_64x64_png(&pixels)
}

#[tokio::main]
async fn main() -> Result<(), minestom::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let motd = Component::text("⚡ minestom-rs ")
        .color(NamedTextColor::Gold)
        .bold(true)
        .append(
            Component::text(format!(
                "Minecraft {VERSION_NAME} (Protocol {PROTOCOL_VERSION})\n"
            ))
            .color(NamedTextColor::Yellow)
            .bold(false),
        )
        .append(Component::text("Safe, idiomatic Rust server library").color(NamedTextColor::Aqua));

    let server = MinecraftServer::builder()
        .motd(motd)
        .max_players(250)
        .favicon(generate_demo_favicon())
        .player_sample(vec![
            StatusPlayerSample::new("Ferris", offline_player_uuid("Ferris")),
            StatusPlayerSample::new("Minestom", offline_player_uuid("Minestom")),
        ])
        .build()?;
    server.set_online_count(2);

    let shutdown_handle = server.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            shutdown_handle.stop_cleanly();
        }
    });

    server.start("0.0.0.0:25565").await?;
    Ok(())
}
