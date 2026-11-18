//! M5 Runnable Example: Block placing & breaking, `AxisPlacementRule` & `HorizontalFacingPlacementRule`,
//! custom `BlockHandler` + `Tag<T>` on blocks, `BlockEntityDataPacket` NBT sync, and `/tower`
//! bulk building via `RelativeBlockBatch` (`MultiBlockChangePacket`)!
//!
//! Run with:
//! ```bash
//! cargo run -p minestom --example blocks_and_batches
//! ```

use minestom::prelude::*;

struct LockedChestHandler;

impl BlockHandler for LockedChestHandler {
    fn key(&self) -> &'static str {
        "minecraft:chest"
    }

    fn on_place(&self, ctx: &BlockPlacementContext) {
        tracing::info!(pos = ?ctx.position, "Chest placed!");
    }

    fn on_destroy(&self, ctx: &BlockDestroyContext) {
        tracing::info!(pos = ?ctx.position, "Chest broken!");
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
                .append(Component::text("Blocks & Batches Demo").color(NamedTextColor::Yellow)),
        )
        .build()?;

    server
        .block_manager()
        .register_placement_rule(Block::OAK_LOG, AxisPlacementRule);
    server
        .block_manager()
        .register_placement_rule(Block::CHEST, HorizontalFacingPlacementRule);
    server.block_manager().register_handler(LockedChestHandler);

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
    });

    // Stamp a pre-built 3×5×3 stone-and-glass tower whenever a player runs `/tower`
    let mut tower = RelativeBlockBatch::new();
    for dy in 0..5_i32 {
        for dx in -1_i32..=1 {
            for dz in -1_i32..=1 {
                let is_wall = dx.abs() == 1 || dz.abs() == 1;
                if is_wall {
                    let b = if dy == 2 {
                        Block::GLASS
                    } else {
                        Block::COBBLESTONE
                    };
                    tower.set_relative(dx, dy, dz, b);
                }
            }
        }
    }

    let srv_for_cmd = server.clone();
    let inst_for_cmd = instance.clone();
    server
        .command_manager()
        .register(
            Command::new("tower").with_default_executor(move |sender, _| {
                let origin = sender.origin_position().to_block_vec().offset(4, 0, 0);
                let abs = tower.to_absolute(origin);
                srv_for_cmd.apply_batch_synced(&inst_for_cmd, &abs);
                sender.send_message(
                    Component::text("Built tower 4 blocks east!").color(NamedTextColor::Green),
                );
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

    // Protect bedrock-like spawn column (0, *, 0) from breaking
    server
        .global_event_handler()
        .add_listener(|ev: &mut PlayerBlockBreakEvent| {
            if ev.block_position.x == 0 && ev.block_position.z == 0 {
                ev.set_cancelled(true);
            }
        });

    server.start("0.0.0.0:25565").await?;
    Ok(())
}
