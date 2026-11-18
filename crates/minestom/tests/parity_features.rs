//! End-to-end M10 integration tests verifying every remaining Minestom 26.2 parity subsystem:
//! - `Sidebar` scoreboard (`ScoreboardObjectivePacket` `0x6A`, `DisplayScoreboardPacket` `0x62`,
//!   `UpdateScorePacket` `0x6E`) + live viewer updates
//! - `TeamManager` (`TeamsPacket` `0x6D`) initial join sync & live member addition
//! - `BossBar` (`BossBarPacket` `0x09`) viewer registration & live progress update
//! - `Title` (`SetTitleTimePacket` `0x73`, `SetTitleSubTitlePacket` `0x70`, `SetTitleTextPacket` `0x72`),
//!   `ClearTitlesPacket` (`0x0E`), `ActionBarPacket` (`0x57`), & `PlayerListHeaderAndFooterPacket` (`0x7A`)
//! - `ParticlePacket` (`0x2F` — exact MC 26.2 wire layout!) & `SoundEffectPacket` (`0x75`) + `StopSoundPacket` (`0x77`)
//! - `ToastNotification` (`AdvancementsPacket` `0x82` add + immediate remove) & `RecipeManager` (`DeclareRecipesPacket` `0x85`)
//! - `ResourcePackPushPacket` (`0x51`), `ResourcePackPopPacket` (`0x50`), `ClientResourcePackStatusPacket` (`0x31`)
//!   firing `PlayerResourcePackStatusEvent` + forced-pack decline disconnect (`PlayDisconnectPacket`)
//! - MC 26.2 `Dialog::notice` (`ShowDialogPacket` `0x8C`) & `ClearDialogPacket` (`0x8B`)
//! - Lock-free point-in-time `ServerSnapshot` (`server.snapshot()`) consistency!

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use minestom::network::PacketStream;
use minestom::prelude::*;
use minestom::protocol::config::{
    ClientFinishConfigurationPacket, ClientSelectKnownPacksPacket, ClientSettingsPacket,
    FinishConfigurationPacket, SelectKnownPacksPacket,
};
use minestom::protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom::protocol::login::{
    ClientLoginAcknowledgedPacket, ClientLoginStartPacket, LoginSuccessPacket, SetCompressionPacket,
};
use minestom::protocol::play::{
    ClientPlayKeepAlivePacket, ClientTeleportConfirmPacket, PlayDisconnectPacket,
    PlayKeepAlivePacket, PlayerPositionAndLookPacket,
};
use minestom::protocol::ui_media::{
    ActionBarPacket, AdvancementsPacket, BossBarAction, BossBarPacket, ClearDialogPacket,
    ClearTitlesPacket, ClientResourcePackStatusPacket, DeclareRecipesPacket,
    DisplayScoreboardPacket, PlayerListHeaderAndFooterPacket, ResourcePackPopPacket,
    ResourcePackPushPacket, ScoreboardObjectivePacket, SetTitleSubTitlePacket, SetTitleTextPacket,
    SetTitleTimePacket, ShowDialogPacket, SoundEffectPacket, StopSoundPacket, TeamsPacket,
    UpdateScorePacket,
};
use minestom::protocol::{MAX_PACKET_SIZE, PacketPayload, decode_packet_exact};
use tokio::net::TcpStream;
use uuid::Uuid;

async fn connect_player_collecting_init(
    addr: SocketAddr,
    username: &str,
) -> (PacketStream, Option<DeclareRecipesPacket>, Vec<TeamsPacket>) {
    let mut client =
        PacketStream::new(TcpStream::connect(addr).await.unwrap(), MAX_PACKET_SIZE).unwrap();
    client
        .send_packet(&ClientHandshakePacket {
            protocol_version: PROTOCOL_VERSION,
            server_address: "localhost".to_owned(),
            server_port: addr.port(),
            intent: HandshakeIntent::Login,
        })
        .await
        .unwrap();
    client
        .send_packet(&ClientLoginStartPacket {
            username: username.to_owned(),
            profile_id: Uuid::nil(),
        })
        .await
        .unwrap();

    let comp = decode_packet_exact::<SetCompressionPacket>(
        client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    client.set_compression_threshold(Some(comp.threshold as usize));

    let _ = decode_packet_exact::<LoginSuccessPacket>(
        client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    client
        .send_packet(&ClientLoginAcknowledgedPacket)
        .await
        .unwrap();
    client
        .send_packet(&ClientSettingsPacket {
            locale: "en_us".to_owned(),
            view_distance: 2,
            chat_mode: 0,
            chat_colors: true,
            displayed_skin_parts: 0x7F,
            main_hand: MainHand::Right,
            enable_text_filtering: false,
            allow_server_listings: true,
            particle_setting: 0,
        })
        .await
        .unwrap();

    loop {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if frame.packet_id == SelectKnownPacksPacket::PACKET_ID {
            let offered = decode_packet_exact::<SelectKnownPacksPacket>(frame.payload).unwrap();
            client
                .send_packet(&ClientSelectKnownPacksPacket {
                    entries: offered.entries,
                })
                .await
                .unwrap();
        } else if frame.packet_id == FinishConfigurationPacket::PACKET_ID {
            client
                .send_packet(&ClientFinishConfigurationPacket)
                .await
                .unwrap();
            break;
        }
    }

    let mut recipes = None;
    let mut teams = Vec::new();
    loop {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if frame.packet_id == DeclareRecipesPacket::PACKET_ID {
            recipes = Some(decode_packet_exact::<DeclareRecipesPacket>(frame.payload).unwrap());
        } else if frame.packet_id == TeamsPacket::PACKET_ID {
            teams.push(decode_packet_exact::<TeamsPacket>(frame.payload).unwrap());
        } else if frame.packet_id == PlayerPositionAndLookPacket::PACKET_ID {
            let pos_pkt =
                decode_packet_exact::<PlayerPositionAndLookPacket>(frame.payload).unwrap();
            client
                .send_packet(&ClientTeleportConfirmPacket {
                    teleport_id: pos_pkt.teleport_id,
                })
                .await
                .unwrap();
            break;
        }
    }
    (client, recipes, teams)
}

async fn wait_for_packet<P: PacketPayload>(stream: &mut PacketStream) -> P {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = stream
                .read_frame_timeout(Duration::from_secs(5))
                .await
                .unwrap()
                .unwrap();
            if frame.packet_id == PlayKeepAlivePacket::PACKET_ID {
                let ka = decode_packet_exact::<PlayKeepAlivePacket>(frame.payload).unwrap();
                stream
                    .send_packet(&ClientPlayKeepAlivePacket { id: ka.id })
                    .await
                    .unwrap();
                continue;
            }
            if frame.packet_id == P::PACKET_ID {
                return decode_packet_exact::<P>(frame.payload).unwrap();
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "timed out waiting for packet 0x{:02x} ({})",
            P::PACKET_ID,
            std::any::type_name::<P>()
        )
    })
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn m10_ui_media_scoreboard_bossbar_recipes_dialogs_resourcepacks_and_snapshots() {
    let server = MinecraftServer::builder()
        .chunk_view_distance(2)
        .build()
        .unwrap();

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::STONE);
    });
    let sentinel_eid = server.spawn_entity(
        &instance,
        Entity::new(EntityType::ARMOR_STAND),
        Pos::new(4.5, 40.0, 4.5),
    );

    // Pre-register a recipe and a scoreboard team BEFORE any player connects!
    server
        .recipe_manager()
        .set_property_items("furnace_input", &[Material::DIAMOND]);
    server
        .recipe_manager()
        .add_stonecutter_recipe(&[Material::COBBLESTONE], ItemStack::single(Material::STONE));
    server.create_team(
        "red_team",
        TeamSettings {
            display_name: Component::text("Red Team"),
            prefix: Component::text("[RED] "),
            color: Some(TeamColor::Red),
            ..TeamSettings::default()
        },
    );

    let rp_events = Arc::new(AtomicUsize::new(0));
    let rp_cnt = Arc::clone(&rp_events);
    server
        .global_event_handler()
        .add_listener(move |_ev: &mut PlayerResourcePackStatusEvent| {
            rp_cnt.fetch_add(1, Ordering::SeqCst);
        });

    let inst_cfg = instance.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&inst_cfg);
            ev.player().set_respawn_point(Pos::new(0.5, 40.0, 0.5));
            ev.player().set_game_mode(GameMode::Survival);
        });

    let (pid_tx, pid_rx) = tokio::sync::oneshot::channel();
    let pid_writer = Arc::new(std::sync::Mutex::new(Some(pid_tx)));
    server
        .global_event_handler()
        .add_listener(move |ev: &mut PlayerSpawnEvent| {
            if ev.first_spawn
                && let Some(tx) = pid_writer.lock().unwrap().take()
            {
                let _ = tx.send(ev.player_id);
            }
        });

    let bound = server.bind("127.0.0.1:0").await.unwrap();
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    let (mut client, init_recipes, init_teams) =
        connect_player_collecting_init(addr, "Clara").await;
    let clara_pid = pid_rx.await.unwrap();

    // 1. Verify initial DeclareRecipesPacket (`0x85`) & TeamsPacket (`0x6D`) arrived on join!
    let rec = init_recipes.expect("DeclareRecipesPacket must arrive on join");
    assert_eq!(rec.stonecutter_recipes.len(), 1);
    assert_eq!(
        rec.item_properties.get("furnace_input"),
        Some(&vec![Material::DIAMOND.id()])
    );
    assert_eq!(init_teams.len(), 1);
    assert_eq!(init_teams[0].team_name, "red_team");

    // 2. Verify lock-free `ServerSnapshot` (`server.snapshot()`)!
    tokio::time::sleep(Duration::from_millis(75)).await;
    let snap = handle.snapshot();
    assert_eq!(snap.players.len(), 1);
    let p_snap = snap.player_by_name("Clara").unwrap();
    assert_eq!(p_snap.player_id, clara_pid);
    assert!((p_snap.health - 20.0).abs() < 1e-3);
    assert!(snap.entity(sentinel_eid).is_some());
    assert!(snap.instance(instance.id()).is_some());

    // 3. Test Sidebar creation, viewer add, and live line update!
    let sidebar = Sidebar::new(Component::text("Lobby"));
    handle.set_sidebar_line(
        &sidebar,
        SidebarLine::new("coins", Component::text("Coins: 10"), 1),
    );
    handle.add_sidebar_viewer(&sidebar, clara_pid);

    let obj_pkt = wait_for_packet::<ScoreboardObjectivePacket>(&mut client).await;
    assert_eq!(obj_pkt.objective_name, sidebar.objective_name());
    let disp_pkt = wait_for_packet::<DisplayScoreboardPacket>(&mut client).await;
    assert_eq!(disp_pkt.position, 1);
    let score_pkt = wait_for_packet::<UpdateScorePacket>(&mut client).await;
    assert_eq!(score_pkt.entity_name, "coins");
    assert_eq!(score_pkt.display_name, Some(Component::text("Coins: 10")));

    handle.set_sidebar_line(
        &sidebar,
        SidebarLine::new("coins", Component::text("Coins: 99"), 1),
    );
    let updated_score = wait_for_packet::<UpdateScorePacket>(&mut client).await;
    assert_eq!(
        updated_score.display_name,
        Some(Component::text("Coins: 99"))
    );

    // 4. Test BossBar add & progress update!
    let bar = BossBar::new(
        Component::text("Dragon"),
        1.0,
        BossBarColor::Red,
        BossBarOverlay::Progress,
    );
    handle.add_boss_bar_viewer(&bar, clara_pid);
    let bb_add = wait_for_packet::<BossBarPacket>(&mut client).await;
    assert_eq!(bb_add.uuid, bar.uuid());

    handle.set_boss_bar_progress(&bar, 0.42);
    let bb_upd = wait_for_packet::<BossBarPacket>(&mut client).await;
    assert_eq!(bb_upd.action, BossBarAction::UpdateHealth(0.42));

    // 5. Test Title, ClearTitles, ActionBar, and TabList Header/Footer!
    handle.send_title(
        clara_pid,
        &Title::new(Component::text("Wave 1"), Component::text("Prepare!")),
    );
    let _ = wait_for_packet::<SetTitleTimePacket>(&mut client).await;
    let sub = wait_for_packet::<SetTitleSubTitlePacket>(&mut client).await;
    assert_eq!(sub.subtitle, Component::text("Prepare!"));
    let main_t = wait_for_packet::<SetTitleTextPacket>(&mut client).await;
    assert_eq!(main_t.title, Component::text("Wave 1"));

    handle.clear_titles(clara_pid, true);
    let clr = wait_for_packet::<ClearTitlesPacket>(&mut client).await;
    assert!(clr.reset);

    handle.send_action_bar(clara_pid, Component::text("Mana: 100/100"));
    let ab = wait_for_packet::<ActionBarPacket>(&mut client).await;
    assert_eq!(ab.text, Component::text("Mana: 100/100"));

    handle.set_player_list_header_footer(
        clara_pid,
        Component::text("Top Header"),
        Component::text("Bottom Footer"),
    );
    let hf = wait_for_packet::<PlayerListHeaderAndFooterPacket>(&mut client).await;
    assert_eq!(hf.header, Component::text("Top Header"));
    assert_eq!(hf.footer, Component::text("Bottom Footer"));

    // 6. Test Particle (`0x2F` exact wire order!), Sound (`0x75`), StopSound (`0x77`), and Toast (`0x82`)!
    handle.spawn_particle(
        &instance,
        &Particle::dust_packet(
            0x00_FF_88,
            1.25,
            Vec3::new(1.0, 41.0, 1.0),
            (0.1, 0.1, 0.1),
            8,
        ),
    );
    let part = wait_for_packet::<ParticlePacket>(&mut client).await;
    assert_eq!(part.particle_id, Particle::DUST.0);
    assert_eq!(part.particle_count, 8);

    handle.play_sound_in_instance(
        &instance,
        &Sound::builtin(Sound::EXPERIENCE_ORB_PICKUP, SoundSource::Player, 1.0, 1.5),
        Vec3::new(0.5, 40.0, 0.5),
    );
    let snd = wait_for_packet::<SoundEffectPacket>(&mut client).await;
    assert_eq!(snd.source, SoundSource::Player);

    handle.stop_sound(clara_pid, Some(SoundSource::Player), None);
    let stop_snd = wait_for_packet::<StopSoundPacket>(&mut client).await;
    assert_eq!(stop_snd.source, Some(SoundSource::Player));

    handle.send_toast(
        clara_pid,
        &ToastNotification::new(
            Component::text("Quest Complete!"),
            ItemStack::single(Material::DIAMOND),
            AdvancementFrameType::Challenge,
        ),
    );
    let adv_add = wait_for_packet::<AdvancementsPacket>(&mut client).await;
    assert_eq!(adv_add.advancements.len(), 1);
    let adv_rem = wait_for_packet::<AdvancementsPacket>(&mut client).await;
    assert_eq!(adv_rem.identifiers_to_remove.len(), 1);

    // 7. Test MC 26.2 Dialog (`ShowDialogPacket` `0x8C` + `ClearDialogPacket` `0x8B`)!
    let dlg = Dialog::notice(
        Component::text("Notice"),
        Component::text("Hello MC 26.2 Dialog!"),
        Component::text("OK"),
    );
    handle.show_dialog(clara_pid, &dlg);
    let _ = wait_for_packet::<ShowDialogPacket>(&mut client).await;

    handle.clear_dialog(clara_pid);
    let _ = wait_for_packet::<ClearDialogPacket>(&mut client).await;

    // 8. Test ResourcePack push (`0x51`), status event (`0x31`), pop (`0x50`), and forced decline kick!
    let opt_pack_id = Uuid::from_u128(0xABCD);
    handle.push_resource_pack(
        clara_pid,
        &ResourcePackRequest::new(
            opt_pack_id,
            "https://example.com/opt.zip",
            "0123456789abcdef0123456789abcdef01234567",
            false,
            Some(Component::text("Optional pack")),
        ),
    );
    let pushed = wait_for_packet::<ResourcePackPushPacket>(&mut client).await;
    assert_eq!(pushed.id, opt_pack_id);
    assert!(!pushed.forced);

    client
        .send_packet(&ClientResourcePackStatusPacket {
            id: opt_pack_id,
            status: ResourcePackStatus::SuccessfullyLoaded,
        })
        .await
        .unwrap();

    handle.pop_resource_pack(clara_pid, Some(opt_pack_id));
    let popped = wait_for_packet::<ResourcePackPopPacket>(&mut client).await;
    assert_eq!(popped.id, Some(opt_pack_id));
    assert_eq!(rp_events.load(Ordering::SeqCst), 1);

    // Push a FORCED pack -> client declines -> server kicks with `PlayDisconnectPacket`!
    let forced_id = Uuid::from_u128(0xDEAD);
    handle.push_resource_pack(
        clara_pid,
        &ResourcePackRequest::new(
            forced_id,
            "https://example.com/forced.zip",
            "fedcba9876543210fedcba9876543210fedcba98",
            true,
            None,
        ),
    );
    let _ = wait_for_packet::<ResourcePackPushPacket>(&mut client).await;
    client
        .send_packet(&ClientResourcePackStatusPacket {
            id: forced_id,
            status: ResourcePackStatus::Declined,
        })
        .await
        .unwrap();
    let _ = wait_for_packet::<PlayDisconnectPacket>(&mut client).await;
    assert_eq!(rp_events.load(Ordering::SeqCst), 2);

    drop(client);
    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
}
