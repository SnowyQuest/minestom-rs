//! End-to-end M7 integration test: verifies deterministic `SchedulerManager` (`TickStart` & `TickEnd`
//! tasks + shutdown hook), `AnvilLoader` `.mca` pre-loading & non-blocking `save_instance_async`,
//! `SharedInstance` zero-copy block sharing with isolated player visibility & independent `WorldBorder`,
//! opt-in 3D BFS `LightingChunk` (`UpdateLightPacket` `0x30`), MC 26.2 `SetTimePacket` (`0x71`),
//! and `Weather` (`ChangeGameStatePacket`)!

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use minestom::network::PacketStream;
use minestom::prelude::*;
use minestom::protocol::block::BlockChangePacket;
use minestom::protocol::config::{
    ClientFinishConfigurationPacket, ClientSelectKnownPacksPacket, ClientSettingsPacket,
    FinishConfigurationPacket, SelectKnownPacksPacket,
};
use minestom::protocol::entity::SpawnEntityPacket;
use minestom::protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom::protocol::login::{
    ClientLoginAcknowledgedPacket, ClientLoginStartPacket, LoginSuccessPacket, SetCompressionPacket,
};
use minestom::protocol::play::{
    ChangeGameStatePacket, ChunkDataPacket, ClientTeleportConfirmPacket, GameStateReason,
    InitializeWorldBorderPacket, PlayerPositionAndLookPacket, SetTimePacket, UpdateLightPacket,
};
use minestom::protocol::{MAX_PACKET_SIZE, PacketPayload, decode_packet_exact};
use minestom::world::unpack_light_nibbles;
use tokio::net::TcpStream;
use uuid::Uuid;

static WORLD_SEQ: AtomicU64 = AtomicU64::new(1);

struct JoinCapture {
    border_diameter: f64,
    day_time: i64,
    spawned_other_entities: usize,
}

async fn connect_and_capture_join(addr: SocketAddr, name: &str) -> (PacketStream, JoinCapture) {
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
            username: name.to_owned(),
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

    let mut border_diameter = None;
    let mut day_time = None;
    let mut tp_id = None;
    let mut chunks = 0_usize;
    let mut spawned_other_entities = 0_usize;

    while chunks < 25 || tp_id.is_none() || border_diameter.is_none() || day_time.is_none() {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            InitializeWorldBorderPacket::PACKET_ID => {
                let wb = decode_packet_exact::<InitializeWorldBorderPacket>(frame.payload).unwrap();
                border_diameter = Some(wb.new_diameter);
            }
            SetTimePacket::PACKET_ID => {
                let st = decode_packet_exact::<SetTimePacket>(frame.payload).unwrap();
                day_time = st.clocks.get(&0).map(|c| c.total_ticks);
            }
            PlayerPositionAndLookPacket::PACKET_ID => {
                let tp = decode_packet_exact::<PlayerPositionAndLookPacket>(frame.payload).unwrap();
                tp_id = Some(tp.teleport_id);
            }
            ChunkDataPacket::PACKET_ID => chunks += 1,
            SpawnEntityPacket::PACKET_ID => spawned_other_entities += 1,
            _ => {}
        }
    }

    client
        .send_packet(&ClientTeleportConfirmPacket {
            teleport_id: tp_id.unwrap(),
        })
        .await
        .unwrap();

    (
        client,
        JoinCapture {
            border_diameter: border_diameter.unwrap(),
            day_time: day_time.unwrap(),
            spawned_other_entities,
        },
    )
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn scheduler_anvil_shared_instance_lighting_border_time_and_weather_work() {
    let seq = WORLD_SEQ.fetch_add(1, Ordering::Relaxed);
    let world_dir =
        std::env::temp_dir().join(format!("minestom_rs_m7_it_{}_{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world_dir);

    // 1. Pre-seed chunk (0, 0) into an Anvil `.mca` region file on disk!
    let dim = DimensionType::overworld();
    {
        let pre_loader = AnvilLoader::with_dimension(&world_dir, &dim.key);
        let mut seed_chunk = Chunk::new(ChunkPos::new(0, 0), &dim);
        seed_chunk.fill_height(0, 40, Block::STONE);
        seed_chunk.set_block(3, 40, 3, Block::EMERALD_BLOCK);
        pre_loader.save_chunk(&seed_chunk, 100).unwrap();
    }

    let server = MinecraftServer::builder()
        .chunk_view_distance(2)
        .build()
        .unwrap();

    let parent_inst = server.instance_manager().create_instance_container();
    parent_inst.set_lighting_enabled(true);
    parent_inst.set_chunk_loader(AnvilLoader::with_dimension(&world_dir, &dim.key));
    parent_inst.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
    });
    parent_inst.set_world_time(WorldTime {
        world_age: 500,
        day_time: 6_000,
        clock_rate: 0.0,
    });

    // SharedInstance shares chunk storage with `parent_inst`, with its own 64-block border & midnight clock!
    let shared_inst = server
        .instance_manager()
        .create_shared_instance(&parent_inst);
    shared_inst.set_world_border(WorldBorder::with_diameter(0.0, 0.0, 64.0));
    shared_inst.set_world_time(WorldTime {
        world_age: 900,
        day_time: 18_000,
        clock_rate: 0.0,
    });

    // Track deterministic TickStart, TickEnd, and Shutdown scheduler callbacks
    let tick_start_runs = Arc::new(AtomicUsize::new(0));
    let ts_ref = Arc::clone(&tick_start_runs);
    server
        .scheduler_manager()
        .build_task(move || {
            ts_ref.fetch_add(1, Ordering::SeqCst);
        })
        .repeat(TaskSchedule::Tick(1))
        .execution_type(ExecutionType::TickStart)
        .schedule();

    let shutdown_runs = Arc::new(AtomicUsize::new(0));
    let sd_ref = Arc::clone(&shutdown_runs);
    server.scheduler_manager().add_shutdown_task(move || {
        sd_ref.fetch_add(1, Ordering::SeqCst);
    });

    let p_cfg = parent_inst.clone();
    let s_cfg = shared_inst.as_instance().clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            if ev.player().username() == "MainExplorer" {
                ev.set_spawning_instance(&p_cfg);
            } else {
                ev.set_spawning_instance(&s_cfg);
            }
            ev.player().set_respawn_point(Pos::new(0.5, 41.0, 0.5));
            ev.player().set_game_mode(GameMode::Creative);
        });

    let bound = server.bind("127.0.0.1:0").await.unwrap();
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    let (mut main_client, main_join) = connect_and_capture_join(addr, "MainExplorer").await;
    let (mut shared_client, shared_join) = connect_and_capture_join(addr, "SharedClone").await;

    // Verify independent WorldBorder & MC 26.2 SetTimePacket clocks AND entity isolation!
    assert!(main_join.border_diameter > 1_000_000.0);
    assert_eq!(main_join.day_time, 6_000);
    assert!((shared_join.border_diameter - 64.0).abs() < 1.0e-6);
    assert_eq!(shared_join.day_time, 18_000);
    assert_eq!(shared_join.spawned_other_entities, 0);

    // Verify chunk (0, 0) was loaded from our Anvil `.mca` file in both parent & shared instances!
    assert_eq!(
        parent_inst.get_block(BlockVec::new(3, 40, 3)),
        Block::EMERALD_BLOCK
    );
    assert_eq!(
        shared_inst.get_block(BlockVec::new(3, 40, 3)),
        Block::EMERALD_BLOCK
    );

    // 2. Schedule a `TickEnd` task placing `GLOWSTONE` at (6, 42, 6) in `parent_inst` ->
    // BOTH `main_client` AND `shared_client` receive `BlockChangePacket` + `UpdateLightPacket` (`0x30`)!
    let srv_task = server.clone();
    let p_task = parent_inst.clone();
    server
        .scheduler_manager()
        .build_task(move || {
            srv_task.set_block_synced(&p_task, BlockVec::new(6, 42, 6), Block::GLOWSTONE, None);
        })
        .delay(TaskSchedule::Tick(2))
        .execution_type(ExecutionType::TickEnd)
        .schedule();

    for client in [&mut main_client, &mut shared_client] {
        let mut saw_block = false;
        let mut saw_light = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !saw_block || !saw_light {
            let rem = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(
                !rem.is_zero(),
                "timed out waiting for BlockChange + UpdateLight"
            );
            let frame = client.read_frame_timeout(rem).await.unwrap().unwrap();
            if frame.packet_id == BlockChangePacket::PACKET_ID {
                let bc = decode_packet_exact::<BlockChangePacket>(frame.payload).unwrap();
                if bc.position == BlockVec::new(6, 42, 6) {
                    assert_eq!(bc.block_state_id, i32::from(Block::GLOWSTONE.state_id()));
                    saw_block = true;
                }
            } else if frame.packet_id == UpdateLightPacket::PACKET_ID {
                let ul = decode_packet_exact::<UpdateLightPacket>(frame.payload).unwrap();
                if ul.chunk_x == 0 && ul.chunk_z == 0 {
                    // Section for y = 42 (`min_y = -64` -> `rel_y = 106` -> `sec_idx = 6` -> `light_bit = 7`, `ly = 10`)
                    let light_bit = 7_usize;
                    assert_ne!(
                        ul.light_data.block_mask[light_bit / 64] & (1_u64 << (light_bit % 64)),
                        0
                    );
                    let mut idx = 0;
                    for b in 0..light_bit {
                        if (ul.light_data.block_mask[b / 64] & (1_u64 << (b % 64))) != 0 {
                            idx += 1;
                        }
                    }
                    let nibbles = unpack_light_nibbles(&ul.light_data.block_light[idx]);
                    assert_eq!(nibbles[(10 << 8) | (6 << 4) | 6], 15);
                    assert_eq!(nibbles[(10 << 8) | (6 << 4) | 7], 14);
                    saw_light = true;
                }
            }
        }
    }

    // 3. Live weather update on `shared_inst` -> `shared_client` receives thunderstorm game state!
    server.set_weather(&shared_inst, Weather::THUNDER);
    let mut saw_thunder = false;
    let weather_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !saw_thunder {
        let rem = weather_deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!rem.is_zero(), "timed out waiting for ThunderLevelChange");
        let frame = shared_client
            .read_frame_timeout(rem)
            .await
            .unwrap()
            .unwrap();
        if frame.packet_id == ChangeGameStatePacket::PACKET_ID {
            let gs = decode_packet_exact::<ChangeGameStatePacket>(frame.payload).unwrap();
            if gs.reason == GameStateReason::ThunderLevelChange && (gs.value - 1.0).abs() < 1.0e-5 {
                saw_thunder = true;
            }
        }
    }

    // 4. Non-blocking async save persists the newly placed GLOWSTONE into `.mca` on disk!
    let saved_count = server.save_instance_async(&parent_inst).await.unwrap();
    assert!(saved_count >= 1);

    let verify_loader = AnvilLoader::with_dimension(&world_dir, &dim.key);
    let reloaded = verify_loader
        .load_chunk(&dim, ChunkPos::new(0, 0))
        .unwrap()
        .unwrap();
    assert_eq!(reloaded.get_block(6, 42, 6), Block::GLOWSTONE);
    assert!(tick_start_runs.load(Ordering::SeqCst) >= 2);

    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
    assert_eq!(shutdown_runs.load(Ordering::SeqCst), 1);

    let _ = std::fs::remove_dir_all(&world_dir);
}
