//! M3 Runnable Example: Brigadier commands (`/gamemode`, `/tp`, `/say`, `/warp` with
//! dynamic server-side tab completion), custom `PlayerChatEvent` formatting, and a
//! cancellable `PlayerMoveEvent` world boundary!
//!
//! Run with:
//! ```bash
//! cargo run -p minestom --example commands_and_events
//! ```

use minestom::prelude::*;

#[tokio::main]
async fn main() -> Result<(), minestom::Error> {
    tracing_subscriber::fmt()
        .with_env_filter("info,minestom=debug")
        .init();

    let server = MinecraftServer::builder()
        .motd(
            Component::text("minestom-rs 26.2 ")
                .color(NamedTextColor::Gold)
                .append(Component::text("Commands & Events Demo").color(NamedTextColor::Aqua)),
        )
        .build()?;

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
    });

    // Register /gamemode (/gm) <mode>
    let mode_arg = Argument::game_mode("mode");
    let mode_ref = mode_arg.clone();
    server
        .command_manager()
        .register(Command::new("gamemode").with_aliases(["gm"]).add_syntax(
            [mode_arg.untyped()],
            move |sender, ctx| {
                let mode = *ctx.get(&mode_ref).expect("validated GameMode");
                sender.set_game_mode(mode);
                sender.send_message(
                    Component::text(format!("Game mode updated to {mode:?}"))
                        .color(NamedTextColor::Green),
                );
            },
        ));

    // Register /tp <x y z> supporting relative `~` coordinates
    let pos_arg = Argument::vec3("destination");
    let pos_ref = pos_arg.clone();
    server
        .command_manager()
        .register(
            Command::new("tp").add_syntax([pos_arg.untyped()], move |sender, ctx| {
                let dest = *ctx.get(&pos_ref).expect("validated Vec3");
                let cur = sender.origin_position();
                sender.teleport(Pos::with_look(dest.x, dest.y, dest.z, cur.yaw, cur.pitch));
                sender.send_message(
                    Component::text(format!(
                        "Teleported to ({:.1}, {:.1}, {:.1})",
                        dest.x, dest.y, dest.z
                    ))
                    .color(NamedTextColor::Yellow),
                );
            }),
        );

    // Register /say <message...>
    let msg_arg = Argument::greedy_string("message");
    let msg_ref = msg_arg.clone();
    server
        .command_manager()
        .register(
            Command::new("say").add_syntax([msg_arg.untyped()], move |sender, ctx| {
                let text = ctx.get(&msg_ref).expect("validated greedy string");
                sender.broadcast(
                    Component::text(format!("[{}] {text}", sender.name()))
                        .color(NamedTextColor::LightPurple),
                );
            }),
        );

    // Register /warp <target> with dynamic suggestions & tooltips
    let warp_arg = Argument::word("target").with_suggestion_callback(|_, _, sug| {
        sug.add_entry(SuggestionEntry::with_tooltip(
            "spawn",
            "Center of the world",
        ));
        sug.add_entry(SuggestionEntry::with_tooltip(
            "sky",
            "High altitude platform",
        ));
    });
    let warp_ref = warp_arg.clone();
    server.command_manager().register(
        Command::new("warp")
            .with_default_executor(|sender, _| {
                sender.send_message("Try /warp spawn or /warp sky");
            })
            .add_syntax([warp_arg.untyped()], move |sender, ctx| {
                match ctx.get(&warp_ref).map(String::as_str) {
                    Some("spawn") => sender.teleport(Pos::new(0.0, 42.0, 0.0)),
                    Some("sky") => sender.teleport(Pos::new(0.0, 100.0, 0.0)),
                    Some(other) => {
                        sender.send_message(
                            Component::text(format!("Unknown warp `{other}`"))
                                .color(NamedTextColor::Red),
                        );
                    }
                    None => {}
                }
            }),
    );

    let events = server.global_event_handler();
    let spawn_instance = instance.clone();
    events.add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
        ev.set_spawning_instance(&spawn_instance);
        ev.player().set_respawn_point(Pos::new(0.0, 42.0, 0.0));
        ev.player().set_permission_level(4);
    });

    // Format chat messages with [Player] prefix
    events.add_listener(|ev: &mut PlayerChatEvent| {
        ev.formatted_message = Some(
            Component::text(format!("[{}] ", ev.username))
                .color(NamedTextColor::Aqua)
                .append(Component::text(&ev.raw_message).color(NamedTextColor::White)),
        );
    });

    // Keep players inside a 64-block arena radius around (0, 0)
    events.add_listener(|ev: &mut PlayerMoveEvent| {
        if ev.new_position.x.abs() > 64.0 || ev.new_position.z.abs() > 64.0 {
            ev.set_cancelled(true);
        }
    });

    server.start("0.0.0.0:25565").await?;
    Ok(())
}
