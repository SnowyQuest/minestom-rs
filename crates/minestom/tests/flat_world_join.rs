//! End-to-end M2 integration test: a headless Minecraft 26.2 client connects over TCP,
//! negotiates Offline Login + Zlib compression + Configuration (known packs, 27 dynamic
//! registries, 32 tag registries), enters Play, verifies flat `GRASS_BLOCK` chunks,
//! walks across chunk boundaries to receive streamed chunks, and answers `KeepAlive`!

use std::time::Duration;

use minestom::network::PacketStream;
use minestom::prelude::*;
use minestom::protocol::config::{
    ClientFinishConfigurationPacket, ClientSelectKnownPacksPacket, ClientSettingsPacket,
    ConfigTagsPacket, FinishConfigurationPacket, KnownPack, RegistryDataPacket,
    SelectKnownPacksPacket,
};
use minestom::protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom::protocol::login::{
    ClientLoginAcknowledgedPacket, ClientLoginStartPacket, LoginSuccessPacket, SetCompressionPacket,
};
use minestom::protocol::play::{
    ChangeGameStatePacket, ChunkDataPacket, ClientPlayKeepAlivePacket,
    ClientPlayerPositionAndRotationPacket, ClientTeleportConfirmPacket, GameStateReason,
    JoinGamePacket, PlayKeepAlivePacket, PlayerPositionAndLookPacket, UpdateViewPositionPacket,
};
use minestom::protocol::{MAX_PACKET_SIZE, PacketPayload, decode_packet_exact};
use tokio::net::TcpStream;
use uuid::Uuid;

#[tokio::test]
async fn headless_client_joins_flat_world_walks_across_chunks_and_answers_keepalive() {
    let cfg = ServerConfig {
        chunk_view_distance: 2,
        keep_alive_interval_ms: 80,
        keep_alive_timeout_ms: 5_000,
        ..ServerConfig::default()
    };
    let server = MinecraftServer::builder()
        .config(cfg)
        .build()
        .expect("build server");

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
    });

    let spawn_instance = instance.clone();
    server
        .global_event_handler()
        .add_listener(move |event: &mut AsyncPlayerConfigurationEvent| {
            event.set_spawning_instance(&spawn_instance);
            event.player().set_respawn_point(Pos::new(0.0, 42.0, 0.0));
            event.player().set_game_mode(GameMode::Creative);
        });

    let bound = server.bind("127.0.0.1:0").await.expect("bind");
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    let tcp = TcpStream::connect(addr).await.expect("connect");
    let mut client = PacketStream::new(tcp, MAX_PACKET_SIZE).expect("packet stream");

    // 1. Handshake -> Login
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
            username: "Alex".to_owned(),
            profile_id: Uuid::nil(),
        })
        .await
        .unwrap();

    // Expect SetCompressionPacket (0x03), enable Zlib compression on client!
    let comp_frame = client
        .read_frame_timeout(Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(comp_frame.packet_id, SetCompressionPacket::PACKET_ID);
    let comp = decode_packet_exact::<SetCompressionPacket>(comp_frame.payload).unwrap();
    assert_eq!(comp.threshold, 256);
    client.set_compression_threshold(Some(comp.threshold as usize));

    // Expect LoginSuccessPacket (0x02)
    let success_frame = client
        .read_frame_timeout(Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(success_frame.packet_id, LoginSuccessPacket::PACKET_ID);
    let login_success = decode_packet_exact::<LoginSuccessPacket>(success_frame.payload).unwrap();
    assert_eq!(login_success.profile.username, "Alex");

    // Acknowledge Login -> transition to Configuration
    client
        .send_packet(&ClientLoginAcknowledgedPacket)
        .await
        .unwrap();

    // 2. Configuration Phase
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

    let mut registry_count = 0_usize;
    let mut got_tags = false;
    loop {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            SelectKnownPacksPacket::PACKET_ID => {
                let offered = decode_packet_exact::<SelectKnownPacksPacket>(frame.payload).unwrap();
                assert_eq!(offered.entries, vec![KnownPack::minecraft_core()]);
                client
                    .send_packet(&ClientSelectKnownPacksPacket {
                        entries: offered.entries,
                    })
                    .await
                    .unwrap();
            }
            RegistryDataPacket::PACKET_ID => {
                let reg = decode_packet_exact::<RegistryDataPacket>(frame.payload).unwrap();
                assert_ne!(reg.entries.len(), 0);
                registry_count += 1;
            }
            ConfigTagsPacket::PACKET_ID => {
                let tags = decode_packet_exact::<ConfigTagsPacket>(frame.payload).unwrap();
                assert_eq!(tags.registries.len(), 32);
                got_tags = true;
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
    assert_eq!(registry_count, 27);
    assert!(got_tags);

    // 3. Play Phase: verify JoinGame, Teleport @ (0, 42, 0), LevelChunksLoadStart, and 5x5 = 25 chunks!
    let mut got_join = false;
    let mut got_chunks_start_event = false;
    let mut teleport_id = None;
    let mut initial_chunks = 0_usize;

    while initial_chunks < 25 {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            JoinGamePacket::PACKET_ID => {
                let join = decode_packet_exact::<JoinGamePacket>(frame.payload).unwrap();
                assert_eq!(join.spawn_info.game_mode, GameMode::Creative);
                assert_eq!(join.spawn_info.world, "minecraft:overworld");
                got_join = true;
            }
            PlayerPositionAndLookPacket::PACKET_ID => {
                let tp = decode_packet_exact::<PlayerPositionAndLookPacket>(frame.payload).unwrap();
                assert!((tp.position.y - 42.0).abs() < f64::EPSILON);
                teleport_id = Some(tp.teleport_id);
            }
            ChangeGameStatePacket::PACKET_ID => {
                let gs = decode_packet_exact::<ChangeGameStatePacket>(frame.payload).unwrap();
                if gs.reason == GameStateReason::LevelChunksLoadStart {
                    got_chunks_start_event = true;
                }
            }
            ChunkDataPacket::PACKET_ID => {
                let chunk_pkt = decode_packet_exact::<ChunkDataPacket>(frame.payload).unwrap();
                let mut reader = bytes::Bytes::from(chunk_pkt.sections_data);
                // Decode sections -4..20 and check section 4 (world Y 0..16) is filled with GRASS_BLOCK!
                for sec_idx in 0..24 {
                    use bytes::Buf as _;
                    let block_count = reader.get_i16();
                    let liquid_count = reader.get_i16();
                    let blocks = Palette::read_network(&mut reader, 16, 4, 8, 15).unwrap();
                    let _biomes = Palette::read_network(&mut reader, 4, 1, 3, 6).unwrap();
                    if sec_idx == 4 || sec_idx == 5 {
                        // World Y 0..16 and 16..32 are completely filled with GRASS_BLOCK
                        assert_eq!(block_count, 4096);
                        assert_eq!(liquid_count, 0);
                        assert_eq!(
                            blocks.get(0, 0, 0),
                            u32::from(Block::GRASS_BLOCK.state_id())
                        );
                    }
                }
                initial_chunks += 1;
            }
            PlayKeepAlivePacket::PACKET_ID => {
                let ka = decode_packet_exact::<PlayKeepAlivePacket>(frame.payload).unwrap();
                client
                    .send_packet(&ClientPlayKeepAlivePacket { id: ka.id })
                    .await
                    .unwrap();
            }
            _ => {}
        }
    }

    assert!(got_join);
    assert!(got_chunks_start_event);
    assert_eq!(handle.online_count(), 1);

    // 4. Confirm initial teleport and walk east into chunk (1, 0) (`x = 20.0`) -> expect newly streamed chunks + KeepAlive!
    client
        .send_packet(&ClientTeleportConfirmPacket {
            teleport_id: teleport_id.expect("received initial teleport"),
        })
        .await
        .unwrap();

    client
        .send_packet(&ClientPlayerPositionAndRotationPacket {
            position: Pos::with_look(20.0, 42.0, 0.0, 90.0, 0.0),
            flags: 1,
        })
        .await
        .unwrap();

    let mut got_new_center = false;
    let mut streamed_after_walk = 0_usize;
    let mut answered_keepalive = false;

    while !(got_new_center && streamed_after_walk >= 5 && answered_keepalive) {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            UpdateViewPositionPacket::PACKET_ID => {
                let vp = decode_packet_exact::<UpdateViewPositionPacket>(frame.payload).unwrap();
                if vp.chunk_x == 1 && vp.chunk_z == 0 {
                    got_new_center = true;
                }
            }
            ChunkDataPacket::PACKET_ID => {
                let c = decode_packet_exact::<ChunkDataPacket>(frame.payload).unwrap();
                assert_eq!(c.chunk_x, 3); // Newly entered eastern column for radius=2 around chunk_x=1!
                streamed_after_walk += 1;
            }
            PlayKeepAlivePacket::PACKET_ID => {
                let ka = decode_packet_exact::<PlayKeepAlivePacket>(frame.payload).unwrap();
                client
                    .send_packet(&ClientPlayKeepAlivePacket { id: ka.id })
                    .await
                    .unwrap();
                answered_keepalive = true;
            }
            _ => {}
        }
    }

    drop(client);
    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
}
