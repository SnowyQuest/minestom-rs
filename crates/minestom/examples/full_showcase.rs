//! M10 Flagship Showcase: exercises virtually every `minestom-rs` subsystem in one server:
//! - Parallel chunk generation + 3D BFS sky/block lighting + custom `AbsoluteBlockBatch`
//! - `Sidebar` scoreboard + `TeamManager` colored prefix + `TabList` header/footer
//! - `BossBar` live health/progress bar + `Title` + `ActionBar`
//! - `Particle` effects + `Sound` playback + `ToastNotification` pop-up
//! - `RecipeManager` custom stonecutter recipe + MC 26.2 `Dialog::notice` modal (`/rules`)
//! - 20 TPS `Scheduler` task updating the sidebar & boss bar using lock-free `server.snapshot()`
//! - Autonomous A* `EntityAIGroup` Arena Guardian + melee combat!

use minestom::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let server = MinecraftServer::builder()
        .motd(
            Component::text("minestom-rs 26.2 ")
                .color(NamedTextColor::Gold)
                .bold(true)
                .append(Component::text("Full Parity Showcase").color(NamedTextColor::Aqua)),
        )
        .build()?;

    // 1. World generation + lighting + custom stonecutter recipe
    let instance = server.instance_manager().create_instance_container();
    instance.set_lighting_enabled(true);
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 39, Block::STONE);
        unit.modifier().fill_height(39, 40, Block::GRASS_BLOCK);
    });
    instance.set_block(BlockVec::new(0, 40, 4), Block::GLOWSTONE);

    server.recipe_manager().add_stonecutter_recipe(
        &[Material::COBBLESTONE],
        ItemStack::of(Material::DIAMOND, 1),
    );

    // 2. Global team, Sidebar, and BossBar
    server.create_team(
        "champions",
        TeamSettings {
            display_name: Component::text("Champions"),
            prefix: Component::text("[CHAMP] ").color(NamedTextColor::Gold),
            color: Some(TeamColor::Gold),
            ..TeamSettings::default()
        },
    );

    let sidebar = Sidebar::new(
        Component::text("MINESTOM-RS")
            .color(NamedTextColor::Gold)
            .bold(true),
    );
    server.set_sidebar_line(
        &sidebar,
        SidebarLine::new(
            "ver",
            Component::text("MC 26.2 (Proto 776)").color(NamedTextColor::Gray),
            3,
        ),
    );
    server.set_sidebar_line(
        &sidebar,
        SidebarLine::new(
            "online",
            Component::text("Online: 0").color(NamedTextColor::Green),
            2,
        ),
    );
    server.set_sidebar_line(
        &sidebar,
        SidebarLine::new(
            "tick",
            Component::text("Tick: 0").color(NamedTextColor::Aqua),
            1,
        ),
    );

    let boss_bar = BossBar::new(
        Component::text("Server Pulse").color(NamedTextColor::LightPurple),
        1.0,
        BossBarColor::Purple,
        BossBarOverlay::Notched10,
    );

    // 3. Periodic 1-second (20-tick) HUD refresh reading lock-free `server.snapshot()`
    let srv_sched = server.clone();
    let sb_sched = sidebar.clone();
    let bb_sched = boss_bar.clone();
    let _ = server
        .scheduler_manager()
        .build_task(move || {
            let snap = srv_sched.snapshot();
            srv_sched.set_sidebar_line(
                &sb_sched,
                SidebarLine::new(
                    "online",
                    Component::text(format!("Online: {}", snap.players.len()))
                        .color(NamedTextColor::Green),
                    2,
                ),
            );
            srv_sched.set_sidebar_line(
                &sb_sched,
                SidebarLine::new(
                    "tick",
                    Component::text(format!("Tick: {}", snap.tick_number))
                        .color(NamedTextColor::Aqua),
                    1,
                ),
            );
            #[allow(clippy::cast_precision_loss)]
            let pulse = ((snap.tick_number % 200) as f32) / 200.0;
            srv_sched.set_boss_bar_progress(&bb_sched, pulse);
        })
        .repeat(TaskSchedule::Tick(20))
        .schedule();

    // 4. `/rules` command opening an MC 26.2 modal Notice Dialog + particle & sound flourish
    let srv_cmd = server.clone();
    let inst_cmd = instance.clone();
    server
        .command_manager()
        .register(
            Command::new("rules").with_default_executor(move |sender, _ctx| {
                if let Some(p) = sender.as_player() {
                    let dlg = Dialog::notice(
                        Component::text("Server Rules").color(NamedTextColor::Gold),
                        Component::text(
                            "1. Safe Rust only (#![forbid(unsafe_code)]).\n2. Have fun!",
                        ),
                        Component::text("Understood"),
                    );
                    srv_cmd.show_dialog(p.player_id, &dlg);
                    srv_cmd.play_sound_to_player(
                        p.player_id,
                        &Sound::builtin(Sound::UI_BUTTON_CLICK, SoundSource::Ui, 1.0, 1.2),
                        p.position.as_vec(),
                    );
                    srv_cmd.spawn_particle(
                        &inst_cmd,
                        &Particle::HAPPY_VILLAGER.packet(
                            p.position.as_vec(),
                            (0.4, 0.6, 0.4),
                            0.02,
                            16,
                        ),
                    );
                }
            }),
        );

    // 5. Spawn an AI Guardian
    let ai = EntityAIGroup::new()
        .with_target_selector(LastEntityDamagerTarget::new(24.0))
        .with_goal_selector(MeleeAttackGoal::new(0.22, 2.0, 20))
        .with_goal_selector(RandomStrollGoal::new(5, 0.12, 80));
    server.spawn_entity(
        &instance,
        Entity::creature(EntityType::ZOMBIE)
            .with_custom_name(Component::text("Showcase Sentinel"))
            .with_behavior(ai),
        Pos::new(6.5, 40.0, 6.5),
    );

    // 6. Player join setup & welcome UI
    let inst_cfg = instance.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&inst_cfg);
            ev.player().set_respawn_point(Pos::new(0.5, 40.0, 0.5));
            ev.player().set_game_mode(GameMode::Survival);
        });

    let srv_join = server.clone();
    let sb_join = sidebar.clone();
    let bb_join = boss_bar.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut PlayerSpawnEvent| {
            if !ev.first_spawn {
                return;
            }
            let pid = ev.player_id;
            srv_join.add_team_member("champions", ev.username.clone());
            srv_join.add_sidebar_viewer(&sb_join, pid);
            srv_join.add_boss_bar_viewer(&bb_join, pid);
            srv_join.set_player_list_header_footer(
                pid,
                Component::text("Welcome to minestom-rs 26.2").color(NamedTextColor::Gold),
                Component::text("Type /rules for MC 26.2 Dialog").color(NamedTextColor::Gray),
            );
            srv_join.send_title(
                pid,
                &Title::new(
                    Component::text("Welcome!").color(NamedTextColor::Green),
                    Component::text("100% Safe Rust Minecraft 26.2").color(NamedTextColor::Yellow),
                ),
            );
            srv_join.send_action_bar(
                pid,
                Component::text("Use /rules to test Dialogs!").color(NamedTextColor::Aqua),
            );
            srv_join.send_toast(
                pid,
                &ToastNotification::new(
                    Component::text("Joined minestom-rs!"),
                    ItemStack::single(Material::DIAMOND_SWORD),
                    AdvancementFrameType::Goal,
                ),
            );
            srv_join.play_sound_to_player(
                pid,
                &Sound::builtin(Sound::PLAYER_LEVELUP, SoundSource::Player, 0.8, 1.0),
                ev.position.as_vec(),
            );
            srv_join.give_player_item(pid, ItemStack::single(Material::DIAMOND_SWORD));
        });

    server.start("0.0.0.0:25565").await?;
    Ok(())
}
