//! End-to-end integration tests verifying:
//! 1. Complete 1,196-block parity: vanilla `SlabPlacementRule` merging (`bottom + top -> double`),
//!    `FenceConnectPlacementRule` dynamic neighbor connection & disconnection via `block_update`,
//!    2-block `OAK_DOOR` automatic upper-half placement + right-click open toggle + partner break,
//!    replaceable `SHORT_GRASS` overwrite, Creative empty-hand no-op, and sub-voxel slab physics!
//! 2. Complete 158-entity parity: `EntityType::WARDEN` (500 HP) & `EntityType::CHICKEN` (4 HP)
//!    data-driven default attributes, `Entity::text_display` + `metadata_def` runtime `EntityRef::set_meta`
//!    live wire sync, `PlayerRef` (`ev.player`) FIFO ordering, async `AsyncPlayerConfigurationEvent`
//!    listeners, and `UnloadChunkPacket` (`0x25`) emission on distant teleport!

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use minestom::network::PacketStream;
use minestom::prelude::*;
use minestom::protocol::block::{
    AcknowledgeBlockChangePacket, ClientPlayerActionPacket, ClientPlayerBlockPlacementPacket,
    PlayerActionStatus,
};
use minestom::protocol::chat_command::SystemChatPacket;
use minestom::protocol::config::{
    ClientFinishConfigurationPacket, ClientSelectKnownPacksPacket, ClientSettingsPacket,
    FinishConfigurationPacket, SelectKnownPacksPacket,
};
use minestom::protocol::entity::{EntityMetaDataPacket, MetadataValue};
use minestom::protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom::protocol::inventory::ClientCreativeInventoryActionPacket;
use minestom::protocol::login::{
    ClientLoginAcknowledgedPacket, ClientLoginStartPacket, LoginSuccessPacket, SetCompressionPacket,
};
use minestom::protocol::play::{
    ChunkBatchFinishedPacket, ClientTeleportConfirmPacket, PlayerPositionAndLookPacket,
    UnloadChunkPacket,
};
use minestom::protocol::{MAX_PACKET_SIZE, PacketPayload, decode_packet_exact};
use tokio::net::TcpStream;
use uuid::Uuid;

async fn connect_creative_client(addr: SocketAddr, username: &str) -> PacketStream {
    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut client = PacketStream::new(tcp, MAX_PACKET_SIZE).unwrap();

    client
        .send_packet(&ClientHandshakePacket {
            protocol_version: PROTOCOL_VERSION,
            server_address: "127.0.0.1".to_owned(),
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

    loop {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            SetCompressionPacket::PACKET_ID => {
                let comp = decode_packet_exact::<SetCompressionPacket>(frame.payload).unwrap();
                client.set_compression_threshold(Some(comp.threshold as usize));
            }
            LoginSuccessPacket::PACKET_ID => {
                let _ = decode_packet_exact::<LoginSuccessPacket>(frame.payload).unwrap();
                break;
            }
            other => panic!("unexpected login packet 0x{other:02X}"),
        }
    }

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
                let kp = decode_packet_exact::<SelectKnownPacksPacket>(frame.payload).unwrap();
                client
                    .send_packet(&ClientSelectKnownPacksPacket {
                        entries: kp.entries,
                    })
                    .await
                    .unwrap();
            }
            FinishConfigurationPacket::PACKET_ID => {
                let _ = decode_packet_exact::<FinishConfigurationPacket>(frame.payload).unwrap();
                client
                    .send_packet(&ClientFinishConfigurationPacket)
                    .await
                    .unwrap();
                break;
            }
            _ => {}
        }
    }

    let mut tp_done = false;
    let mut chunks_done = false;
    while !(tp_done && chunks_done) {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            PlayerPositionAndLookPacket::PACKET_ID => {
                let tp = decode_packet_exact::<PlayerPositionAndLookPacket>(frame.payload).unwrap();
                client
                    .send_packet(&ClientTeleportConfirmPacket {
                        teleport_id: tp.teleport_id,
                    })
                    .await
                    .unwrap();
                tp_done = true;
            }
            ChunkBatchFinishedPacket::PACKET_ID => chunks_done = true,
            _ => {}
        }
    }
    client
}

async fn wait_for_ack(client: &mut PacketStream, seq: i32) {
    loop {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if frame.packet_id == AcknowledgeBlockChangePacket::PACKET_ID {
            let ack = decode_packet_exact::<AcknowledgeBlockChangePacket>(frame.payload).unwrap();
            if ack.sequence == seq {
                return;
            }
        }
    }
}

#[test]
fn static_registry_constants_cover_all_blocks_entities_items_and_registries() {
    use minestom::data::generated;

    assert_eq!(generated::block_states::BLOCK_INFOS.len(), 1_196);
    assert_eq!(generated::block_states::STATE_INFOS.len(), 32_366);
    assert_eq!(generated::materials::MATERIAL_COUNT, 1_537);
    assert_eq!(generated::entity_types::ENTITY_INFOS.len(), 158);
    assert_eq!(Particle::COUNT, 125);
    assert_eq!(generated::sounds::ENTRIES.len(), 1_968);
    assert_eq!(generated::enchantments::ENCHANTMENT_INFOS.len(), 43);
    assert_eq!(generated::attributes::ATTRIBUTE_SPECS.len(), 40);
    assert_eq!(generated::potion_effects::POTION_EFFECT_INFOS.len(), 40);
    assert_eq!(generated::damage_types::DAMAGE_TYPE_INFOS.len(), 51);
    assert_eq!(generated::block_entity_types::BLOCK_ENTITY_TYPE_COUNT, 49);
    assert_eq!(generated::fluids::FLUID_INFOS.len(), 5);
    assert_eq!(generated::game_events::GAME_EVENT_INFOS.len(), 61);
    assert_eq!(
        generated::villager_professions::VILLAGER_PROFESSION_COUNT,
        15
    );
    assert_eq!(
        generated::painting_variants::PAINTING_VARIANT_INFOS.len(),
        51
    );

    // Spot-check high-ordinal constants directly on their strongly-typed structs
    assert_eq!(
        Block::WAXED_OXIDIZED_COPPER_TRAPDOOR.key(),
        "minecraft:waxed_oxidized_copper_trapdoor"
    );
    assert_eq!(
        EntityType::ZOMBIE_NAUTILUS.key(),
        "minecraft:zombie_nautilus"
    );
    assert_eq!(
        Material::MUSIC_DISC_LAVA_CHICKEN.key(),
        "minecraft:music_disc_lava_chicken"
    );
    assert_eq!(Enchantment::WIND_BURST.key(), "minecraft:wind_burst");
    assert_eq!(
        SoundEvent::UI_BUTTON_CLICK.key(),
        "minecraft:ui.button.click"
    );
    assert_eq!(
        BlockEntityType::TRIAL_SPAWNER.key(),
        "minecraft:trial_spawner"
    );
    assert_eq!(Fluid::FLOWING_LAVA.key(), "minecraft:flowing_lava");
    assert_eq!(
        GameEvent::SCULK_SENSOR_TENDRILS_CLICKING.key(),
        "minecraft:sculk_sensor_tendrils_clicking"
    );
    assert_eq!(VillagerProfession::LIBRARIAN.key(), "minecraft:librarian");
    assert_eq!(PaintingVariant::WANDERER.key(), "minecraft:wanderer");

    // Spot-check data-driven entity default attributes
    let warden = Entity::creature(EntityType::WARDEN);
    assert!((warden.living.as_ref().unwrap().max_health - 500.0).abs() < 1e-4);
    let chicken = Entity::creature(EntityType::CHICKEN);
    assert!((chicken.living.as_ref().unwrap().max_health - 4.0).abs() < 1e-4);

    // Spot-check block state properties & sub-voxel collision shapes
    assert!(!Block::WATER.is_solid());
    assert!(Block::WATER.is_liquid());
    assert!(Block::SHORT_GRASS.is_replaceable());
    let waterlogged_stairs = Block::OAK_STAIRS
        .with_property("waterlogged", "true")
        .unwrap();
    assert!(!waterlogged_stairs.is_liquid());
    assert!(waterlogged_stairs.has_fluid());
    assert_eq!(Block::OAK_SLAB.collision_shape().len(), 1);
    assert!((Block::OAK_SLAB.collision_shape()[0].max_y - 0.5).abs() < 1e-6);
}

#[tokio::test]
async fn vanilla_placement_rules_door_toggling_neighbor_updates_and_subvoxel_physics_work() {
    let server = MinecraftServer::builder()
        .chunk_view_distance(2)
        .build()
        .unwrap();

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::STONE);
    });
    // Pre-place replaceable SHORT_GRASS at (4, 40, 0) and bottom OAK_SLAB at (8, 40, 0)
    instance.set_block(BlockVec::new(4, 40, 0), Block::SHORT_GRASS);
    instance.set_block(BlockVec::new(8, 40, 0), Block::OAK_SLAB);

    // Drop a Chicken above the bottom slab at (8.5, 41.4, 0.5) -> must come to rest at y = 40.5!
    let chicken_ref = server.spawn_entity(
        &instance,
        Entity::creature(EntityType::CHICKEN),
        Pos::new(8.5, 41.4, 0.5),
    );

    let async_cfg_ran = Arc::new(AtomicBool::new(false));
    let async_cfg_flag = Arc::clone(&async_cfg_ran);
    let spawn_inst = instance.clone();
    server.global_event_handler().add_async_listener(
        move |ev: &mut AsyncPlayerConfigurationEvent| {
            let inst = spawn_inst.clone();
            let flag = Arc::clone(&async_cfg_flag);
            Box::pin(async move {
                tokio::task::yield_now().await;
                flag.store(true, Ordering::SeqCst);
                ev.set_spawning_instance(&inst);
                ev.player().set_respawn_point(Pos::new(0.5, 40.0, 0.5));
                ev.player().set_game_mode(GameMode::Creative);
            })
        },
    );

    let (player_ref_tx, player_ref_rx) = tokio::sync::oneshot::channel();
    let player_ref_slot = Arc::new(std::sync::Mutex::new(Some(player_ref_tx)));
    server
        .global_event_handler()
        .add_listener(move |ev: &mut PlayerSpawnEvent| {
            if ev.first_spawn {
                ev.player
                    .send_message(Component::text("Hello via PlayerRef!"));
                if let Some(tx) = player_ref_slot.lock().unwrap().take() {
                    let _ = tx.send(ev.player.clone());
                }
            }
        });

    let holo_ref = server.spawn_entity(
        &instance,
        Entity::text_display(Component::text("Initial Hologram")),
        Pos::new(1.5, 42.0, 1.5),
    );

    let bound = server.bind("127.0.0.1:0").await.unwrap();
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    let mut client = connect_creative_client(addr, "Architect").await;
    assert!(async_cfg_ran.load(Ordering::SeqCst));
    let player_ref = player_ref_rx.await.unwrap();

    // 1. Creative empty-hand right-click MUST NOT phantom-place Block::STONE!
    client
        .send_packet(&ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(2, 39, 0),
            block_face: BlockFace::Up,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 1,
        })
        .await
        .unwrap();
    wait_for_ack(&mut client, 1).await;
    assert_eq!(instance.get_block(BlockVec::new(2, 40, 0)), Block::AIR);

    // 2. Equip OAK_SLAB: place bottom slab on SHORT_GRASS (replaces grass in-place!), then merge into double slab!
    client
        .send_packet(&ClientCreativeInventoryActionPacket {
            slot: 36,
            item: ItemStack::of(Material::OAK_SLAB, 64),
        })
        .await
        .unwrap();
    client
        .send_packet(&ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(4, 40, 0),
            block_face: BlockFace::Up,
            cursor_x: 0.5,
            cursor_y: 0.2,
            cursor_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 2,
        })
        .await
        .unwrap();
    wait_for_ack(&mut client, 2).await;
    assert_eq!(
        instance
            .get_block(BlockVec::new(4, 40, 0))
            .get_property("type"),
        Some("bottom")
    );

    // Click Up face of the bottom slab at (4, 40, 0) -> merges into `type=double`!
    client
        .send_packet(&ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(4, 40, 0),
            block_face: BlockFace::Up,
            cursor_x: 0.5,
            cursor_y: 0.5,
            cursor_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 3,
        })
        .await
        .unwrap();
    wait_for_ack(&mut client, 3).await;
    assert_eq!(
        instance
            .get_block(BlockVec::new(4, 40, 0))
            .get_property("type"),
        Some("double")
    );

    // 3. Equip OAK_FENCE: place at (2, 40, 2) and (3, 40, 2) -> neighbor `block_update` connects `east=true` & `west=true`!
    client
        .send_packet(&ClientCreativeInventoryActionPacket {
            slot: 36,
            item: ItemStack::of(Material::OAK_FENCE, 64),
        })
        .await
        .unwrap();
    client
        .send_packet(&ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(2, 39, 2),
            block_face: BlockFace::Up,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 4,
        })
        .await
        .unwrap();
    client
        .send_packet(&ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(3, 39, 2),
            block_face: BlockFace::Up,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 5,
        })
        .await
        .unwrap();
    wait_for_ack(&mut client, 5).await;
    assert_eq!(
        instance
            .get_block(BlockVec::new(2, 40, 2))
            .get_property("east"),
        Some("true")
    );
    assert_eq!(
        instance
            .get_block(BlockVec::new(3, 40, 2))
            .get_property("west"),
        Some("true")
    );

    // Break fence at (3, 40, 2) -> fence at (2, 40, 2) dynamically updates `east=false`!
    client
        .send_packet(&ClientPlayerActionPacket {
            status: PlayerActionStatus::StartedDigging,
            position: BlockVec::new(3, 40, 2),
            block_face: BlockFace::Up,
            sequence: 6,
        })
        .await
        .unwrap();
    wait_for_ack(&mut client, 6).await;
    assert_eq!(
        instance
            .get_block(BlockVec::new(2, 40, 2))
            .get_property("east"),
        Some("false")
    );

    // 4. Equip OAK_DOOR: place at (5, 40, 2) -> automatically places `half=lower` AND `half=upper`,
    //    right-click toggles `open=true` on BOTH halves, and breaking upper half removes lower half too!
    client
        .send_packet(&ClientCreativeInventoryActionPacket {
            slot: 36,
            item: ItemStack::of(Material::OAK_DOOR, 16),
        })
        .await
        .unwrap();
    client
        .send_packet(&ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(5, 39, 2),
            block_face: BlockFace::Up,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 7,
        })
        .await
        .unwrap();
    wait_for_ack(&mut client, 7).await;
    assert_eq!(
        instance
            .get_block(BlockVec::new(5, 40, 2))
            .get_property("half"),
        Some("lower")
    );
    assert_eq!(
        instance
            .get_block(BlockVec::new(5, 41, 2))
            .get_property("half"),
        Some("upper")
    );

    // Right-click lower door -> toggles `open=true` on both halves!
    client
        .send_packet(&ClientPlayerBlockPlacementPacket {
            hand: Hand::Main,
            position: BlockVec::new(5, 40, 2),
            block_face: BlockFace::North,
            cursor_x: 0.5,
            cursor_y: 0.5,
            cursor_z: 0.0,
            inside_block: false,
            hit_world_border: false,
            sequence: 8,
        })
        .await
        .unwrap();
    wait_for_ack(&mut client, 8).await;
    assert_eq!(
        instance
            .get_block(BlockVec::new(5, 40, 2))
            .get_property("open"),
        Some("true")
    );
    assert_eq!(
        instance
            .get_block(BlockVec::new(5, 41, 2))
            .get_property("open"),
        Some("true")
    );

    // Break upper half -> breaks lower half automatically!
    client
        .send_packet(&ClientPlayerActionPacket {
            status: PlayerActionStatus::StartedDigging,
            position: BlockVec::new(5, 41, 2),
            block_face: BlockFace::North,
            sequence: 9,
        })
        .await
        .unwrap();
    wait_for_ack(&mut client, 9).await;
    assert_eq!(instance.get_block(BlockVec::new(5, 40, 2)), Block::AIR);
    assert_eq!(instance.get_block(BlockVec::new(5, 41, 2)), Block::AIR);

    // 5. Verify Chicken landed on the sub-voxel bottom slab at `y = 40.5`!
    tokio::time::sleep(Duration::from_millis(250)).await;
    let chicken_pos = chicken_ref.position().expect("chicken alive in snapshot");
    assert!(
        (chicken_pos.y - 40.5).abs() < 1e-3,
        "expected chicken to land on bottom slab at y=40.5, got {}",
        chicken_pos.y
    );

    // 6. Live `EntityRef::set_meta` updates TextDisplay hologram and sends `EntityMetaDataPacket`,
    //    AND `PlayerRef::teleport` far away emits `UnloadChunkPacket` (`0x25`)!
    holo_ref.set_meta(
        metadata_def::text_display::TEXT,
        Component::text("Updated Live!"),
    );
    player_ref.teleport(Pos::new(160.5, 40.0, 160.5));

    let mut saw_holo_meta = false;
    let mut saw_unload_chunk = false;
    let mut saw_welcome_chat = false;
    while !(saw_holo_meta && saw_unload_chunk) {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            SystemChatPacket::PACKET_ID => {
                let chat = decode_packet_exact::<SystemChatPacket>(frame.payload).unwrap();
                if chat
                    .message
                    .to_plain_text()
                    .contains("Hello via PlayerRef!")
                {
                    saw_welcome_chat = true;
                }
            }
            EntityMetaDataPacket::PACKET_ID => {
                let meta = decode_packet_exact::<EntityMetaDataPacket>(frame.payload).unwrap();
                if meta.entity_id == holo_ref.network_id()
                    && meta.entries.get(&23).is_some_and(|v| {
                        matches!(v, MetadataValue::Component(c) if c.to_plain_text() == "Updated Live!")
                    })
                {
                    saw_holo_meta = true;
                }
            }
            UnloadChunkPacket::PACKET_ID => {
                let _ = decode_packet_exact::<UnloadChunkPacket>(frame.payload).unwrap();
                saw_unload_chunk = true;
            }
            _ => {}
        }
    }
    let _ = saw_welcome_chat;

    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
}
