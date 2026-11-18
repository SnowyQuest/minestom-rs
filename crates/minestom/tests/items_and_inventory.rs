//! End-to-end M6 integration test: verifies MC 26.2 `ItemStack` Data Components,
//! `WindowItemsPacket`, `EntityEquipmentPacket` across 2 simultaneous TCP clients,
//! `/chest` opening `OpenWindowPacket`, shift-click (`QuickMove`) + paint-drag (`QuickCraft`)
//! via `ClientClickWindowPacket` (`HashedStack`), `InventoryPreClickEvent` slot lock cancellation,
//! `DropItem` -> `ItemEntity` spawn + `CollectItemPacket` (`0x7C`) pickup, and Survival block consumption!

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use minestom::network::PacketStream;
use minestom::prelude::*;
use minestom::protocol::block::{
    ClientPlayerActionPacket, ClientPlayerBlockPlacementPacket, PlayerActionStatus,
};
use minestom::protocol::chat_command::ClientCommandChatPacket;
use minestom::protocol::config::{
    ClientFinishConfigurationPacket, ClientSelectKnownPacksPacket, ClientSettingsPacket,
    FinishConfigurationPacket, SelectKnownPacksPacket,
};
use minestom::protocol::entity::{EntityMetaDataPacket, MetadataValue, SpawnEntityPacket};
use minestom::protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom::protocol::inventory::{
    ClientClickWindowPacket, ClientCloseWindowPacket, ClientHeldItemChangePacket,
    CollectItemPacket, EntityEquipmentPacket, HashedStack, OpenWindowPacket, WindowItemsPacket,
};
use minestom::protocol::login::{
    ClientLoginAcknowledgedPacket, ClientLoginStartPacket, LoginSuccessPacket, SetCompressionPacket,
};
use minestom::protocol::play::{ClientTeleportConfirmPacket, PlayerPositionAndLookPacket};
use minestom::protocol::{MAX_PACKET_SIZE, PacketPayload, decode_packet_exact};
use tokio::net::TcpStream;
use uuid::Uuid;

async fn connect_survival_player(addr: SocketAddr, name: &str) -> PacketStream {
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

    loop {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if frame.packet_id == PlayerPositionAndLookPacket::PACKET_ID {
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
    client
}

async fn wait_for_packet<P: PacketPayload>(client: &mut PacketStream) -> P {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let rem = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !rem.is_zero(),
            "timed out waiting for packet 0x{:02X}",
            P::PACKET_ID
        );
        let frame = client
            .read_frame_timeout(rem)
            .await
            .unwrap()
            .expect("connection closed prematurely");
        if frame.packet_id == P::PACKET_ID {
            return decode_packet_exact::<P>(frame.payload).unwrap();
        }
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn items_containers_equipment_drop_pickup_and_block_consume_work() {
    let server = MinecraftServer::builder()
        .chunk_view_distance(3)
        .build()
        .unwrap();

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::STONE);
    });

    let shared_chest = Inventory::new(InventoryType::Chest3Row, Component::text("Test Vault"));
    let lock_item = ItemStack::single(Material::EMERALD)
        .with_custom_name(Component::text("Locked Emblem"))
        .with_glowing(true);
    shared_chest.set_item(0, lock_item).unwrap();
    shared_chest
        .set_item(1, ItemStack::of(Material::DIAMOND, 16))
        .unwrap();

    let chest_for_cmd = shared_chest.clone();
    server
        .command_manager()
        .register(
            Command::new("chest").with_default_executor(move |sender, _| {
                sender.open_inventory(&chest_for_cmd);
            }),
        );

    let custom_tag: Tag<i32> = Tag::new("power_tier");

    let inst_for_cfg = instance.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&inst_for_cfg);
            ev.player().set_respawn_point(Pos::new(0.5, 40.0, 0.5));
            ev.player().set_game_mode(GameMode::Survival);
        });

    let srv_for_spawn = server.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut PlayerSpawnEvent| {
            if ev.username == "Alice" {
                let sword = ItemStack::single(Material::DIAMOND_SWORD)
                    .with_custom_name(Component::text("Excalibur").color(NamedTextColor::Aqua))
                    .with_lore([Component::text("Relic")])
                    .with_enchantment(16, 5)
                    .with_unbreakable()
                    .with_tag(&custom_tag, 9001);
                srv_for_spawn.give_player_item(ev.player_id, sword);
                srv_for_spawn.give_player_item(ev.player_id, ItemStack::of(Material::GLASS, 4));
            }
        });

    // Lock slot 0 of open container windows
    let cancelled_clicks = Arc::new(AtomicUsize::new(0));
    let cancelled_ref = Arc::clone(&cancelled_clicks);
    server
        .global_event_handler()
        .add_listener(move |ev: &mut InventoryPreClickEvent| {
            if ev.window_id > 0 && ev.slot == 0 {
                cancelled_ref.fetch_add(1, Ordering::SeqCst);
                ev.set_cancelled(true);
            }
        });

    let close_events = Arc::new(AtomicUsize::new(0));
    let close_ref = Arc::clone(&close_events);
    server
        .global_event_handler()
        .add_listener(move |_ev: &mut InventoryCloseEvent| {
            close_ref.fetch_add(1, Ordering::SeqCst);
        });

    let pickup_events = Arc::new(AtomicUsize::new(0));
    let pickup_ref = Arc::clone(&pickup_events);
    server
        .global_event_handler()
        .add_listener(move |_ev: &mut PickupItemEvent| {
            pickup_ref.fetch_add(1, Ordering::SeqCst);
        });

    let bound = server.bind("127.0.0.1:0").await.unwrap();
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    let mut alice = connect_survival_player(addr, "Alice").await;
    let mut bob = connect_survival_player(addr, "Bob").await;

    // 1. Verify Alice received her custom sword (slot 36) & 4 glass (slot 37) via WindowItemsPacket,
    // and Bob sees Alice holding the DIAMOND_SWORD via EntityEquipmentPacket!
    let mut alice_win0 = wait_for_packet::<WindowItemsPacket>(&mut alice).await;
    while alice_win0.items.get(37).is_none_or(ItemStack::is_air) {
        alice_win0 = wait_for_packet::<WindowItemsPacket>(&mut alice).await;
    }
    let rec_sword = &alice_win0.items[36];
    assert_eq!(rec_sword.material(), Material::DIAMOND_SWORD);
    assert!(rec_sword.is_unbreakable());
    assert_eq!(rec_sword.get_tag(&custom_tag), Some(9001));
    assert_eq!(alice_win0.items[37].material(), Material::GLASS);
    assert_eq!(alice_win0.items[37].amount(), 4);

    let bob_equip = wait_for_packet::<EntityEquipmentPacket>(&mut bob).await;
    assert_eq!(
        bob_equip
            .equipments
            .get(&EquipmentSlot::MainHand)
            .map(ItemStack::material),
        Some(Material::DIAMOND_SWORD)
    );

    // 2. Alice switches hotbar to slot 1 (GLASS) -> Bob receives EntityEquipmentPacket(GLASS),
    // and Alice places 1 GLASS block in Survival -> stack decrements from 4 to 3!
    alice
        .send_packet(&ClientHeldItemChangePacket { slot: 1 })
        .await
        .unwrap();
    let mut bob_equip_glass = wait_for_packet::<EntityEquipmentPacket>(&mut bob).await;
    while bob_equip_glass
        .equipments
        .get(&EquipmentSlot::MainHand)
        .map(ItemStack::material)
        != Some(Material::GLASS)
    {
        bob_equip_glass = wait_for_packet::<EntityEquipmentPacket>(&mut bob).await;
    }

    alice
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
    let after_place_win = wait_for_packet::<WindowItemsPacket>(&mut alice).await;
    assert_eq!(after_place_win.items[37].material(), Material::GLASS);
    assert_eq!(after_place_win.items[37].amount(), 3);
    assert_eq!(instance.get_block(BlockVec::new(2, 40, 0)), Block::GLASS);

    // 3. Alice executes `/chest` -> receives OpenWindowPacket + combined 63-slot WindowItemsPacket!
    alice
        .send_packet(&ClientCommandChatPacket {
            message: "chest".to_owned(),
        })
        .await
        .unwrap();
    let open_pkt = wait_for_packet::<OpenWindowPacket>(&mut alice).await;
    assert_eq!(
        open_pkt.window_type,
        InventoryType::Chest3Row.window_type_id()
    );
    let chest_wid = open_pkt.window_id;

    let mut chest_win = wait_for_packet::<WindowItemsPacket>(&mut alice).await;
    while chest_win.window_id != chest_wid {
        chest_win = wait_for_packet::<WindowItemsPacket>(&mut alice).await;
    }
    assert_eq!(chest_win.items.len(), 63);
    assert_eq!(chest_win.items[0].material(), Material::EMERALD);
    assert_eq!(chest_win.items[1].amount(), 16);

    // Clicking locked slot 0 is cancelled by InventoryPreClickEvent -> slot 0 stays EMERALD!
    alice
        .send_packet(&ClientClickWindowPacket {
            window_id: chest_wid,
            state_id: chest_win.state_id,
            slot: 0,
            button: 0,
            click_type: WindowClickMode::Pickup,
            changed_slots: BTreeMap::new(),
            clicked_item: HashedStack::Air,
        })
        .await
        .unwrap();
    let mut resynced = wait_for_packet::<WindowItemsPacket>(&mut alice).await;
    while resynced.window_id != chest_wid {
        resynced = wait_for_packet::<WindowItemsPacket>(&mut alice).await;
    }
    assert_eq!(cancelled_clicks.load(Ordering::SeqCst), 1);
    assert_eq!(
        shared_chest.get_item(0).unwrap().material(),
        Material::EMERALD
    );

    // Pick up 16 diamonds in slot 1, paint-drag evenly across chest slots 2 & 3 (8 each), then shift-click slot 2 into Alice's hotbar!
    for (slot, button, mode) in [
        (1_i16, 0_i8, WindowClickMode::Pickup),
        (-999, 0, WindowClickMode::QuickCraft),
        (2, 1, WindowClickMode::QuickCraft),
        (3, 1, WindowClickMode::QuickCraft),
        (-999, 2, WindowClickMode::QuickCraft),
        (2, 0, WindowClickMode::QuickMove),
    ] {
        alice
            .send_packet(&ClientClickWindowPacket {
                window_id: chest_wid,
                state_id: 1,
                slot,
                button,
                click_type: mode,
                changed_slots: BTreeMap::new(),
                clicked_item: HashedStack::Air,
            })
            .await
            .unwrap();
    }

    alice
        .send_packet(&ClientCloseWindowPacket {
            window_id: chest_wid,
        })
        .await
        .unwrap();

    // Wait until InventoryCloseEvent fires and verify chest slot 3 has 8 diamonds & slot 2 is empty
    let close_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while close_events.load(Ordering::SeqCst) == 0 {
        assert!(tokio::time::Instant::now() < close_deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(shared_chest.get_item(2).unwrap().is_air());
    assert_eq!(shared_chest.get_item(3).unwrap().amount(), 8);

    // 4. Alice drops 1 GLASS (`DropItem` on slot 1) -> spawns `EntityType::ITEM` with ItemStack metadata,
    // and after the 10-tick pickup delay expires, Alice/Bob collects it (`CollectItemPacket` + `PickupItemEvent`)!
    alice
        .send_packet(&ClientPlayerActionPacket {
            status: PlayerActionStatus::DropItem,
            position: BlockVec::ZERO,
            block_face: BlockFace::Down,
            sequence: 0,
        })
        .await
        .unwrap();

    // Bob observes SpawnEntityPacket (`EntityType::ITEM`) + EntityMetaDataPacket (`MetadataValue::ItemStack`)
    let mut item_eid = None;
    let spawn_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while item_eid.is_none() {
        let rem = spawn_deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !rem.is_zero(),
            "timed out waiting for dropped ItemEntity spawn"
        );
        let frame = bob.read_frame_timeout(rem).await.unwrap().unwrap();
        if frame.packet_id == SpawnEntityPacket::PACKET_ID {
            let sp = decode_packet_exact::<SpawnEntityPacket>(frame.payload).unwrap();
            if sp.entity_type == EntityType::ITEM.id() as i32 {
                item_eid = Some(sp.entity_id);
            }
        }
    }
    let item_eid = item_eid.unwrap();

    let mut saw_item_meta = false;
    while !saw_item_meta {
        let rem = spawn_deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!rem.is_zero(), "timed out waiting for ItemEntity metadata");
        let frame = bob.read_frame_timeout(rem).await.unwrap().unwrap();
        if frame.packet_id == EntityMetaDataPacket::PACKET_ID {
            let md = decode_packet_exact::<EntityMetaDataPacket>(frame.payload).unwrap();
            if md.entity_id == item_eid {
                assert!(matches!(
                    md.entries.get(&8),
                    Some(MetadataValue::ItemStack(s)) if s.material() == Material::GLASS && s.amount() == 1
                ));
                saw_item_meta = true;
            }
        }
    }

    // Wait for CollectItemPacket (`0x7C`) after the 10-tick (500ms) pickup delay!
    let collect = wait_for_packet::<CollectItemPacket>(&mut bob).await;
    assert_eq!(collect.collected_entity_id, item_eid);
    assert_eq!(collect.pickup_item_count, 1);
    assert!(pickup_events.load(Ordering::SeqCst) >= 1);

    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
}
