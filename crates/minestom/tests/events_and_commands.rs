//! End-to-end M3 integration test: verifies `ServerListPingEvent`, `DeclareCommandsPacket`,
//! server-side tab completion (`ClientTabCompletePacket` -> `TabCompletePacket`),
//! `/command` execution (`ClientCommandChatPacket` + `PlayerCommandEvent`),
//! chat formatting & broadcasting (`ClientChatMessagePacket` + `PlayerChatEvent` -> `SystemChatPacket`),
//! `PlayerMoveEvent` cancellation snapping back via `PlayerPositionAndLookPacket`,
//! and `PlayerDisconnectEvent`!

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use minestom::network::PacketStream;
use minestom::prelude::*;
use minestom::protocol::chat_command::{
    ClientChatMessagePacket, ClientCommandChatPacket, ClientTabCompletePacket,
    DeclareCommandsPacket, SystemChatPacket, TabCompletePacket,
};
use minestom::protocol::config::{
    ClientFinishConfigurationPacket, ClientSelectKnownPacksPacket, ClientSettingsPacket,
    FinishConfigurationPacket, SelectKnownPacksPacket,
};
use minestom::protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom::protocol::login::{
    ClientLoginAcknowledgedPacket, ClientLoginStartPacket, LoginSuccessPacket, SetCompressionPacket,
};
use minestom::protocol::play::{
    ChangeGameStatePacket, ChunkDataPacket, ClientPlayerPositionAndRotationPacket,
    ClientTeleportConfirmPacket, GameStateReason, PlayerPositionAndLookPacket,
};
use minestom::protocol::status::{StatusRequestPacket, StatusResponsePacket};
use minestom::protocol::{MAX_PACKET_SIZE, PacketPayload, decode_packet_exact};
use tokio::net::TcpStream;
use uuid::Uuid;

#[tokio::test]
async fn headless_client_exercises_events_commands_chat_tab_complete_and_move_cancel() {
    let cfg = ServerConfig {
        chunk_view_distance: 2,
        ..ServerConfig::default()
    };
    let server = MinecraftServer::builder().config(cfg).build().unwrap();

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
    });

    // Register /gm <mode> and /warp <place>
    let mode_arg = Argument::game_mode("mode");
    let mode_ref = mode_arg.clone();
    server
        .command_manager()
        .register(Command::new("gamemode").with_aliases(["gm"]).add_syntax(
            [mode_arg.untyped()],
            move |sender, ctx| {
                let mode = *ctx.get(&mode_ref).unwrap();
                sender.set_game_mode(mode);
                sender.send_message(format!("Mode={mode:?}"));
            },
        ));

    let warp_arg = Argument::word("place").with_suggestion_callback(|_, _, sug| {
        sug.add_entry(SuggestionEntry::with_tooltip("spawn", "Hub"));
        sug.add("shop");
        sug.add("arena");
    });
    let warp_ref = warp_arg.clone();
    server
        .command_manager()
        .register(
            Command::new("warp").add_syntax([warp_arg.untyped()], move |sender, ctx| {
                let p = ctx.get(&warp_ref).unwrap();
                sender.send_message(format!("Warped:{p}"));
            }),
        );

    let events = server.global_event_handler();

    // 1. Dynamic MOTD via ServerListPingEvent with expire_count(1)
    events.add_listener_spec(
        EventListener::builder(|ev: &mut ServerListPingEvent| {
            ev.status.description = Component::text("One-Shot Event MOTD!");
        })
        .expire_count(1)
        .build(),
    );

    let spawn_instance = instance.clone();
    events.add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
        ev.set_spawning_instance(&spawn_instance);
        ev.player().set_respawn_point(Pos::new(0.0, 42.0, 0.0));
        ev.player().set_game_mode(GameMode::Survival);
    });

    // 2. Rewrite chat format via PlayerChatEvent
    events.add_listener(|ev: &mut PlayerChatEvent| {
        ev.formatted_message = Some(Component::text(format!(
            "[VIP {}] {}",
            ev.username, ev.raw_message
        )));
    });

    // 3. Track command invocations via PlayerCommandEvent
    let cmd_counter = Arc::new(AtomicUsize::new(0));
    let cmd_counter_clone = Arc::clone(&cmd_counter);
    events.add_listener(move |ev: &mut PlayerCommandEvent| {
        cmd_counter_clone.fetch_add(1, Ordering::SeqCst);
        if ev.command == "blocked" {
            ev.set_cancelled(true);
        }
    });

    // 4. Cancel moves past X > 50.0
    events.add_listener(|ev: &mut PlayerMoveEvent| {
        if ev.new_position.x > 50.0 {
            ev.set_cancelled(true);
        }
    });

    // 5. Observe PlayerDisconnectEvent
    let disconnected = Arc::new(AtomicBool::new(false));
    let disc_clone = Arc::clone(&disconnected);
    events.add_listener(move |_ev: &mut PlayerDisconnectEvent| {
        disc_clone.store(true, Ordering::SeqCst);
    });

    let bound = server.bind("127.0.0.1:0").await.unwrap();
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    // --- Part A: Verify ServerListPingEvent + expire_count(1) ---
    for expected in ["One-Shot Event MOTD!", "A minestom-rs Server"] {
        let mut status_conn =
            PacketStream::new(TcpStream::connect(addr).await.unwrap(), MAX_PACKET_SIZE).unwrap();
        status_conn
            .send_packet(&ClientHandshakePacket {
                protocol_version: PROTOCOL_VERSION,
                server_address: "localhost".to_owned(),
                server_port: addr.port(),
                intent: HandshakeIntent::Status,
            })
            .await
            .unwrap();
        status_conn.send_packet(&StatusRequestPacket).await.unwrap();
        let f = status_conn
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        let resp = decode_packet_exact::<StatusResponsePacket>(f.payload).unwrap();
        assert_eq!(resp.status.description.to_plain_text(), expected);
    }

    // --- Part B: Join Play ---
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
            username: "Steve".to_owned(),
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
        match frame.packet_id {
            SelectKnownPacksPacket::PACKET_ID => {
                let offered = decode_packet_exact::<SelectKnownPacksPacket>(frame.payload).unwrap();
                client
                    .send_packet(&ClientSelectKnownPacksPacket {
                        entries: offered.entries,
                    })
                    .await
                    .unwrap();
            }
            FinishConfigurationPacket::PACKET_ID => {
                client
                    .send_packet(&ClientFinishConfigurationPacket)
                    .await
                    .unwrap();
                break;
            }
            _ => {}
        }
    }

    let mut got_declare_commands = false;
    let mut initial_tp_id = None;
    let mut initial_chunks = 0_usize;

    while initial_chunks < 25 || !got_declare_commands || initial_tp_id.is_none() {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            DeclareCommandsPacket::PACKET_ID => {
                let decl = decode_packet_exact::<DeclareCommandsPacket>(frame.payload).unwrap();
                // Root has 3 top-level literals: gamemode, gm, warp
                assert_eq!(decl.nodes[0].children.len(), 3);
                got_declare_commands = true;
            }
            PlayerPositionAndLookPacket::PACKET_ID => {
                let tp = decode_packet_exact::<PlayerPositionAndLookPacket>(frame.payload).unwrap();
                initial_tp_id = Some(tp.teleport_id);
            }
            ChunkDataPacket::PACKET_ID => {
                initial_chunks += 1;
            }
            _ => {}
        }
    }

    client
        .send_packet(&ClientTeleportConfirmPacket {
            teleport_id: initial_tp_id.unwrap(),
        })
        .await
        .unwrap();

    // --- Part C: Tab Completion (`"/warp s"` -> `["spawn", "shop"]`) ---
    client
        .send_packet(&ClientTabCompletePacket {
            transaction_id: 42,
            text: "/warp s".to_owned(),
        })
        .await
        .unwrap();

    let tab_pkt = loop {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if frame.packet_id == TabCompletePacket::PACKET_ID {
            break decode_packet_exact::<TabCompletePacket>(frame.payload).unwrap();
        }
    };
    assert_eq!(tab_pkt.transaction_id, 42);
    assert_eq!(tab_pkt.start, 6);
    assert_eq!(tab_pkt.length, 1);
    assert_eq!(tab_pkt.matches.len(), 2);
    assert_eq!(tab_pkt.matches[0].match_str, "spawn");
    assert_eq!(
        tab_pkt.matches[0].tooltip.as_ref().unwrap().to_plain_text(),
        "Hub"
    );

    // --- Part D: Command Execution (`"gm creative"`) ---
    client
        .send_packet(&ClientCommandChatPacket {
            message: "gm creative".to_owned(),
        })
        .await
        .unwrap();

    let mut saw_gm_packet = false;
    let mut saw_gm_chat = false;
    while !(saw_gm_packet && saw_gm_chat) {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            ChangeGameStatePacket::PACKET_ID => {
                let gs = decode_packet_exact::<ChangeGameStatePacket>(frame.payload).unwrap();
                if gs.reason == GameStateReason::ChangeGameMode && (gs.value - 1.0).abs() < 1e-5 {
                    saw_gm_packet = true;
                }
            }
            SystemChatPacket::PACKET_ID => {
                let chat = decode_packet_exact::<SystemChatPacket>(frame.payload).unwrap();
                if chat.message.to_plain_text() == "Mode=Creative" {
                    saw_gm_chat = true;
                }
            }
            _ => {}
        }
    }
    assert_eq!(cmd_counter.load(Ordering::SeqCst), 1);

    // --- Part E: Chat Broadcast (`"Hello server!"` -> `"[VIP Steve] Hello server!"`) ---
    client
        .send_packet(&ClientChatMessagePacket {
            message: "Hello server!".to_owned(),
            timestamp: 1000,
            salt: 0,
            signature: None,
            ack_offset: 0,
            ack_bitset: [0, 0, 0],
            checksum: 0,
        })
        .await
        .unwrap();

    let vip_msg = loop {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if frame.packet_id == SystemChatPacket::PACKET_ID {
            let chat = decode_packet_exact::<SystemChatPacket>(frame.payload).unwrap();
            break chat.message.to_plain_text();
        }
    };
    assert_eq!(vip_msg, "[VIP Steve] Hello server!");

    // --- Part F: Cancellable PlayerMoveEvent snaps client back to X = 0.0 ---
    client
        .send_packet(&ClientPlayerPositionAndRotationPacket {
            position: Pos::with_look(100.0, 42.0, 0.0, 0.0, 0.0),
            flags: 1,
        })
        .await
        .unwrap();

    let snapback = loop {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if frame.packet_id == PlayerPositionAndLookPacket::PACKET_ID {
            break decode_packet_exact::<PlayerPositionAndLookPacket>(frame.payload).unwrap();
        }
    };
    assert!((snapback.position.x - 0.0).abs() < f64::EPSILON);

    // --- Part G: Disconnect triggers PlayerDisconnectEvent ---
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !disconnected.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .expect("PlayerDisconnectEvent fired");

    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
}
