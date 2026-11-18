//! M6 example: custom named/enchanted `ItemStack`s with MC 26.2 Data Components,
//! shared 27-slot `Inventory` (`Chest3Row`) opened via `/chest` or right-clicking a chest block,
//! locked GUI slot via `InventoryPreClickEvent`, and dropped `ItemEntity` pickup!
//!
//! Run with:
//! ```bash
//! cargo run -p minestom --example inventory_demo
//! ```

use minestom::prelude::*;

struct SharedChestHandler {
    server: MinecraftServer,
    chest_inv: Inventory,
}

impl BlockHandler for SharedChestHandler {
    fn key(&self) -> &'static str {
        "minecraft:chest"
    }

    fn on_interact(&self, ctx: &BlockInteractContext) -> bool {
        self.server.open_inventory(ctx.player_id, &self.chest_inv);
        false
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let motd = Component::text("minestom-rs 26.2")
        .color(NamedTextColor::Gold)
        .bold(true)
        .append(Component::text(" - Items & Inventory Demo").color(NamedTextColor::Gray));
    let server = MinecraftServer::builder().motd(motd).build()?;

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::STONE);
        unit.modifier().fill_height(40, 41, Block::GRASS_BLOCK);
    });
    instance.set_block(BlockVec::new(2, 41, 0), Block::CHEST);

    let shared_chest = Inventory::new(
        InventoryType::Chest3Row,
        Component::text("Shared Vault").color(NamedTextColor::Gold),
    );
    let badge_tag: Tag<String> = Tag::new("ui_action");
    let locked_icon = ItemStack::single(Material::EMERALD)
        .with_custom_name(Component::text("Locked Vault Emblem").color(NamedTextColor::Green))
        .with_lore([Component::text("Slot 0 cannot be taken!").color(NamedTextColor::Gray)])
        .with_glowing(true)
        .with_tag(&badge_tag, "locked".to_owned());
    shared_chest.set_item(0, locked_icon)?;
    shared_chest.set_item(13, ItemStack::of(Material::DIAMOND, 32))?;
    shared_chest.set_item(14, ItemStack::of(Material::GOLDEN_APPLE, 8))?;

    server.block_manager().register_handler(SharedChestHandler {
        server: server.clone(),
        chest_inv: shared_chest.clone(),
    });

    let chest_for_cmd = shared_chest.clone();
    server
        .command_manager()
        .register(
            Command::new("chest").with_default_executor(move |sender, _| {
                sender.open_inventory(&chest_for_cmd);
                sender.send_message(
                    Component::text("Opened Shared Vault!").color(NamedTextColor::Aqua),
                );
            }),
        );

    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&instance);
            ev.player().set_respawn_point(Pos::new(0.5, 42.0, 0.5));
            ev.player().set_game_mode(GameMode::Survival);
        });

    let srv_for_spawn = server.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut PlayerSpawnEvent| {
            let blade = ItemStack::single(Material::DIAMOND_SWORD)
                .with_custom_name(Component::text("Rustblade").color(NamedTextColor::Aqua))
                .with_lore([Component::text("Forged in safe Rust").color(NamedTextColor::Gray)])
                .with_enchantment(20, 5)
                .with_unbreakable();
            srv_for_spawn.give_player_item(ev.player_id, blade);
            srv_for_spawn.give_player_item(ev.player_id, ItemStack::of(Material::GLASS, 16));
        });

    // Prevent taking the locked emblem in slot 0 of open container windows!
    server
        .global_event_handler()
        .add_listener(|ev: &mut InventoryPreClickEvent| {
            if ev.window_id > 0 && ev.slot == 0 {
                ev.set_cancelled(true);
            }
        });

    server.start("0.0.0.0:25565").await?;
    Ok(())
}
