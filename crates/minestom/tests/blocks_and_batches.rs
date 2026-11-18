//! End-to-end M5 integration test: verifies block placing & breaking across multiple viewers,
//! `AcknowledgeBlockChangePacket`, `BlockChangePacket`, `BlockEntityDataPacket` (with NBT),
//! `MultiBlockChangePacket` via `RelativeBlockBatch`, `BlockHandler` (`on_place` / `on_interact` / `on_destroy`),
//! `BlockPlacementRule`, `Tag<T>` on blocks, and `PlayerBlockBreakEvent` cancellation revert!

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use minestom::network::PacketStream;
use minestom::prelude::*;
use minestom::protocol::block::{
    AcknowledgeBlockChangePacket, BlockChangePacket, BlockEntityDataPacket,
    ClientPlayerActionPacket, ClientPlayerBlockPlacementPacket, MultiBlockChangePacket,
    PlayerActionStatus,
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
    ChunkDataPacket, ClientTeleportConfirmPacket, PlayerPositionAndLookPacket,
};
use minestom::protocol::{MAX_PACKET_SIZE, PacketPayload, decode_packet_exact};
use tokio::net::TcpStream;
use uuid::Uuid;

struct LogOverrideRule;
impl BlockPlacementRule for LogOverrideRule {
    fn block_place(&self, state: &PlacementState<'_>) -> Option<Block> {
        let axis = match state.block_face {
            BlockFace::West | BlockFace::East => "x",
            BlockFace::Down | BlockFace::Up => "y",
            BlockFace::North | BlockFace::South => "z",
        };
        Block::OAK_LOG.with_property("axis", axis).ok()
    }
}

struct CountingChestHandler {
    placed: Arc<AtomicUsize>,
    interacted: Arc<AtomicUsize>,
    destroyed: Arc<AtomicUsize>,
}

impl BlockHandler for CountingChestHandler {
    fn key(&self) -> &'static str {
        "minecraft:chest"
    }

    fn on_place(&self, _ctx: &BlockPlacementContext) {
        self.placed.fetch_add(1, Ordering::SeqCst);
    }

    fn on_interact(&self, _ctx: &BlockInteractContext) -> bool {
        self.interacted.fetch_add(1, Ordering::SeqCst);
        false // Consume interaction so clicking chest does not place a block!
    }

    fn on_destroy(&self, _ctx: &BlockDestroyContext) {
        self.destroyed.fetch_add(1, Ordering::SeqCst);
    }
}

async fn connect_creative_player(addr: SocketAddr, name: &str) -> PacketStream {
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

    let mut tp_id = None;
    let mut chunks = 0_usize;
    while chunks < 25 || tp_id.is_none() {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            PlayerPositionAndLookPacket::PACKET_ID => {
                let tp = decode_packet_exact::<PlayerPositionAndLookPacket>(frame.payload).unwrap();
                tp_id = Some(tp.teleport_id);
            }
            ChunkDataPacket::PACKET_ID => chunks += 1,
            _ => {}
        }
    }

    client
        .send_packet(&ClientTeleportConfirmPacket {
            teleport_id: tp_id.unwrap(),
        })
        .await
        .unwrap();
    client
}

#[tokio::test]
async fn placing_breaking_block_entities_handlers_tags_and_batches_sync_across_clients() {
    let server = MinecraftServer::builder()
        .chunk_view_distance(2)
        .build()
        .unwrap();

    let placed = Arc::new(AtomicUsize::new(0));
    let interacted = Arc::new(AtomicUsize::new(0));
    let destroyed = Arc::new(AtomicUsize::new(0));
    server
        .block_manager()
        .register_handler(CountingChestHandler {
            placed: Arc::clone(&placed),
            interacted: Arc::clone(&interacted),
            destroyed: Arc::clone(&destroyed),
        });

    server
        .block_manager()
        .register_placement_rule(Block::STONE, LogOverrideRule);

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
    });

    let owner_tag: Tag<String> = Tag::new("owner");
    let inst_for_place = instance.clone();

    // When placing at (5, 40, 5), upgrade to a CHEST with custom NBT + Tag<String>!
    server
        .global_event_handler()
        .add_listener(move |ev: &mut PlayerBlockPlaceEvent| {
            if ev.block_position == BlockVec::new(5, 40, 5) {
                ev.block = Block::CHEST.with_property("facing", "south").unwrap();
                let mut nbt = NbtCompound::new();
                nbt.insert("CustomName", NbtTag::String("LootBox".to_owned()));
                ev.block_nbt = Some(nbt);
                inst_for_place.set_block_tag(ev.block_position, &owner_tag, ev.username.clone());
            }
        });

    // Protect (0, 39, 0) from breaking
    server
        .global_event_handler()
        .add_listener(|ev: &mut PlayerBlockBreakEvent| {
            if ev.block_position == BlockVec::new(0, 39, 0) {
                ev.set_cancelled(true);
            }
        });

    let spawn_inst = instance.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&spawn_inst);
            ev.player().set_respawn_point(Pos::new(0.0, 40.0, 0.0));
            ev.player().set_game_mode(GameMode::Creative);
        });

    let bound = server.bind("127.0.0.1:0").await.unwrap();
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    let mut builder = connect_creative_player(addr, "Builder").await;
    let mut observer = connect_creative_player(addr, "Observer").await;

    // Put STONE in Builder's active hotbar slot (`36`) before placing blocks!
    builder
        .send_packet(
            &minestom::protocol::inventory::ClientCreativeInventoryActionPacket {
                slot: 36,
                item: ItemStack::single(Material::STONE),
            },
        )
        .await
        .unwrap();

    // 1. Builder places block on East face of (3, 40, 0)? First place on Up face of (3, 39, 0) -> places at (3, 40, 0), then East face -> (4, 40, 0) with axis=x!
    builder
        .send_packet(&ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(3, 39, 0),
            block_face: BlockFace::Up,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 10,
        })
        .await
        .unwrap();

    builder
        .send_packet(&ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(3, 40, 0),
            block_face: BlockFace::East,
            cursor_x: 1.0,
            cursor_y: 0.5,
            cursor_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 11,
        })
        .await
        .unwrap();

    let mut builder_got_ack_11 = false;
    while !builder_got_ack_11 {
        let f = builder
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if f.packet_id == AcknowledgeBlockChangePacket::PACKET_ID {
            let ack = decode_packet_exact::<AcknowledgeBlockChangePacket>(f.payload).unwrap();
            if ack.sequence == 11 {
                builder_got_ack_11 = true;
            }
        }
    }

    let expected_log_x = Block::OAK_LOG.with_property("axis", "x").unwrap();
    assert_eq!(instance.get_block(BlockVec::new(4, 40, 0)), expected_log_x);

    // Observer sees BlockChangePacket for (4, 40, 0) = OAK_LOG[axis=x]!
    let mut observer_saw_log_x = false;
    while !observer_saw_log_x {
        let f = observer
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if f.packet_id == BlockChangePacket::PACKET_ID {
            let bc = decode_packet_exact::<BlockChangePacket>(f.payload).unwrap();
            if bc.position == BlockVec::new(4, 40, 0)
                && bc.block_state_id == i32::from(expected_log_x.state_id())
            {
                observer_saw_log_x = true;
            }
        }
    }

    // 2. Builder places CHEST at (5, 40, 5) by clicking Up face of (5, 39, 5) -> Observer receives BlockEntityDataPacket + handler on_place + Tag!
    builder
        .send_packet(&ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(5, 39, 5),
            block_face: BlockFace::Up,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 12,
        })
        .await
        .unwrap();

    let mut observer_saw_chest_nbt = false;
    while !observer_saw_chest_nbt {
        let f = observer
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if f.packet_id == BlockEntityDataPacket::PACKET_ID {
            let be = decode_packet_exact::<BlockEntityDataPacket>(f.payload).unwrap();
            if be.position == BlockVec::new(5, 40, 5) {
                let nbt = be.data.unwrap();
                assert_eq!(
                    nbt.get("CustomName"),
                    Some(&NbtTag::String("LootBox".to_owned()))
                );
                observer_saw_chest_nbt = true;
            }
        }
    }
    assert_eq!(placed.load(Ordering::SeqCst), 1);
    assert_eq!(
        instance.get_block_tag(BlockVec::new(5, 40, 5), &owner_tag),
        Some("Builder".to_owned())
    );

    // 3. Right-clicking the CHEST triggers `on_interact` (which returns `false`, suppressing adjacent placement)!
    builder
        .send_packet(&ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(5, 40, 5),
            block_face: BlockFace::Up,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 13,
        })
        .await
        .unwrap();

    // Break the CHEST -> triggers `on_destroy` and syncs AIR to Observer!
    builder
        .send_packet(&ClientPlayerActionPacket {
            status: PlayerActionStatus::StartedDigging,
            position: BlockVec::new(5, 40, 5),
            block_face: BlockFace::Up,
            sequence: 14,
        })
        .await
        .unwrap();

    // Also try breaking protected (0, 39, 0) -> cancelled, Builder receives revert BlockChangePacket(GRASS_BLOCK)!
    builder
        .send_packet(&ClientPlayerActionPacket {
            status: PlayerActionStatus::StartedDigging,
            position: BlockVec::new(0, 39, 0),
            block_face: BlockFace::Up,
            sequence: 15,
        })
        .await
        .unwrap();

    let mut builder_got_revert = false;
    while !builder_got_revert {
        let f = builder
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if f.packet_id == BlockChangePacket::PACKET_ID {
            let bc = decode_packet_exact::<BlockChangePacket>(f.payload).unwrap();
            if bc.position == BlockVec::new(0, 39, 0)
                && bc.block_state_id == i32::from(Block::GRASS_BLOCK.state_id())
            {
                builder_got_revert = true;
            }
        }
    }
    assert_eq!(interacted.load(Ordering::SeqCst), 1);
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(instance.get_block(BlockVec::new(5, 40, 5)), Block::AIR);

    // 4. Stamp RelativeBlockBatch via `apply_batch_synced` -> Observer receives MultiBlockChangePacket!
    let mut batch = RelativeBlockBatch::new();
    for dy in 0..4 {
        batch.set_relative(0, dy, 0, Block::GOLD_BLOCK);
    }
    handle.apply_batch_synced(&instance, &batch.to_absolute(BlockVec::new(-3, 40, -3)));

    let multi = loop {
        let f = observer
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if f.packet_id == MultiBlockChangePacket::PACKET_ID {
            break decode_packet_exact::<MultiBlockChangePacket>(f.payload).unwrap();
        }
    };
    assert_eq!(multi.blocks.len(), 4);
    assert_eq!(
        instance.get_block(BlockVec::new(-3, 43, -3)),
        Block::GOLD_BLOCK
    );

    drop(builder);
    drop(observer);
    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
}
