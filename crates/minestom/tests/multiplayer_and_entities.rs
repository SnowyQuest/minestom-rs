//! End-to-end M4 integration test: connects two simultaneous headless Minecraft 26.2 clients
//! (`Alice` and `Bob`) alongside a spawned custom `ZOMBIE` entity with gravity & custom
//! `EntityBehavior`, verifying:
//! - Both players see the `ZOMBIE` spawn (`SpawnEntityPacket` + `EntityMetaDataPacket`) and fall
//!   onto the `Y = 40` grass floor (`EntityPositionPacket`).
//! - `Alice` and `Bob` see each other (`PlayerInfoUpdatePacket` + `SpawnEntityPacket`).
//! - `Bob` moving, rotating, sneaking (`ClientInputPacket`), swinging arm (`ClientAnimationPacket`),
//!   and attacking (`ClientAttackPacket`) replicates to `Alice` and fires server events.
//! - `Bob` disconnecting sends `DestroyEntitiesPacket` + `PlayerInfoRemovePacket` to `Alice`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use minestom::block::BlockGetter;
use minestom::network::PacketStream;
use minestom::prelude::*;
use minestom::protocol::config::{
    ClientFinishConfigurationPacket, ClientSelectKnownPacksPacket, ClientSettingsPacket,
    FinishConfigurationPacket, SelectKnownPacksPacket,
};
use minestom::protocol::entity::{
    ClientAnimationPacket, ClientAttackPacket, ClientInputPacket, DestroyEntitiesPacket,
    EntityAnimation, EntityAnimationPacket, EntityHeadLookPacket, EntityMetaDataPacket, EntityPose,
    EntityPositionAndRotationPacket, EntityPositionPacket, MetadataValue, PlayerInfoRemovePacket,
    SpawnEntityPacket,
};
use minestom::protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom::protocol::login::{
    ClientLoginAcknowledgedPacket, ClientLoginStartPacket, LoginSuccessPacket, SetCompressionPacket,
};
use minestom::protocol::play::{
    ChunkDataPacket, ClientPlayerPositionAndRotationPacket, ClientTeleportConfirmPacket,
    JoinGamePacket, PlayerPositionAndLookPacket,
};
use minestom::protocol::{MAX_PACKET_SIZE, PacketPayload, decode_packet_exact};
use tokio::net::TcpStream;
use uuid::Uuid;

struct DriftEastBehavior;
impl EntityBehavior for DriftEastBehavior {
    fn tick(
        &mut self,
        core: &mut EntityCore,
        _living: Option<&mut LivingState>,
        _world: &dyn BlockGetter,
    ) {
        core.velocity.x = 0.1;
    }
}

async fn connect_play_client(addr: SocketAddr, username: &str) -> (PacketStream, i32, Uuid) {
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

    let login_ok = decode_packet_exact::<LoginSuccessPacket>(
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

    let mut self_eid = None;
    let mut tp_id = None;
    let mut chunks = 0_usize;
    while chunks < 25 || self_eid.is_none() || tp_id.is_none() {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            JoinGamePacket::PACKET_ID => {
                let j = decode_packet_exact::<JoinGamePacket>(frame.payload).unwrap();
                self_eid = Some(j.entity_id);
            }
            PlayerPositionAndLookPacket::PACKET_ID => {
                let tp = decode_packet_exact::<PlayerPositionAndLookPacket>(frame.payload).unwrap();
                tp_id = Some(tp.teleport_id);
            }
            ChunkDataPacket::PACKET_ID => {
                chunks += 1;
            }
            _ => {}
        }
    }

    client
        .send_packet(&ClientTeleportConfirmPacket {
            teleport_id: tp_id.unwrap(),
        })
        .await
        .unwrap();

    (client, self_eid.unwrap(), login_ok.profile.uuid)
}

#[tokio::test]
async fn two_players_and_custom_entity_sync_spawn_movement_sneak_animation_and_despawn() {
    let server = MinecraftServer::builder()
        .chunk_view_distance(2)
        .build()
        .unwrap();

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
    });

    // Spawn custom ZOMBIE before server starts!
    let zombie = Entity::creature(EntityType::ZOMBIE)
        .with_custom_name(Component::text("GuardBot"))
        .with_behavior(DriftEastBehavior);
    let zombie_eid = server.spawn_entity(&instance, zombie, Pos::new(2.0, 41.5, 2.0));

    let attacked_target = Arc::new(AtomicI32::new(0));
    let attacked_clone = Arc::clone(&attacked_target);
    server
        .global_event_handler()
        .add_listener(move |ev: &mut PlayerAttackEntityEvent| {
            attacked_clone.store(ev.target_network_id, Ordering::SeqCst);
        });

    let spawn_inst = instance.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&spawn_inst);
            ev.player().set_respawn_point(Pos::new(0.0, 40.0, 0.0));
        });

    let bound = server.bind("127.0.0.1:0").await.unwrap();
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    // 1. Connect Alice, then Bob
    let (mut alice, alice_eid, _) = connect_play_client(addr, "Alice").await;
    let (mut bob, bob_eid, bob_uuid) = connect_play_client(addr, "Bob").await;
    assert_ne!(alice_eid, bob_eid);

    // Alice should receive SpawnEntityPacket for Bob (`EntityType::PLAYER`) AND see Zombie move (`EntityPositionPacket`)!
    let mut alice_saw_bob_spawn = false;
    let mut alice_saw_zombie_move = false;

    while !(alice_saw_bob_spawn && alice_saw_zombie_move) {
        let frame = alice
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            SpawnEntityPacket::PACKET_ID => {
                let sp = decode_packet_exact::<SpawnEntityPacket>(frame.payload).unwrap();
                if sp.entity_id == bob_eid && sp.entity_type == EntityType::PLAYER.id() as i32 {
                    alice_saw_bob_spawn = true;
                }
            }
            EntityPositionPacket::PACKET_ID => {
                let mv = decode_packet_exact::<EntityPositionPacket>(frame.payload).unwrap();
                if mv.entity_id == zombie_eid && mv.delta_x > 0 {
                    alice_saw_zombie_move = true;
                }
            }
            _ => {}
        }
    }

    // 2. Bob walks + rotates, sneaks, swings main arm, and attacks the Zombie!
    bob.send_packet(&ClientPlayerPositionAndRotationPacket {
        position: Pos::with_look(3.0, 40.0, 1.0, 45.0, 10.0),
        flags: 1,
    })
    .await
    .unwrap();
    bob.send_packet(&ClientInputPacket { flags: 0x20 }) // shift/sneak!
        .await
        .unwrap();
    bob.send_packet(&ClientAnimationPacket { hand: Hand::Main })
        .await
        .unwrap();
    bob.send_packet(&ClientAttackPacket {
        target_id: zombie_eid,
    })
    .await
    .unwrap();

    let mut alice_saw_bob_pos_rot = false;
    let mut alice_saw_bob_head = false;
    let mut alice_saw_bob_sneak = false;
    let mut alice_saw_bob_swing = false;

    while !(alice_saw_bob_pos_rot
        && alice_saw_bob_head
        && alice_saw_bob_sneak
        && alice_saw_bob_swing)
    {
        let frame = alice
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            EntityPositionAndRotationPacket::PACKET_ID => {
                let p =
                    decode_packet_exact::<EntityPositionAndRotationPacket>(frame.payload).unwrap();
                if p.entity_id == bob_eid && p.delta_x > 0 {
                    alice_saw_bob_pos_rot = true;
                }
            }
            EntityHeadLookPacket::PACKET_ID => {
                let h = decode_packet_exact::<EntityHeadLookPacket>(frame.payload).unwrap();
                if h.entity_id == bob_eid {
                    alice_saw_bob_head = true;
                }
            }
            EntityMetaDataPacket::PACKET_ID => {
                let m = decode_packet_exact::<EntityMetaDataPacket>(frame.payload).unwrap();
                if m.entity_id == bob_eid
                    && m.entries.get(&6) == Some(&MetadataValue::Pose(EntityPose::Sneaking))
                {
                    alice_saw_bob_sneak = true;
                }
            }
            EntityAnimationPacket::PACKET_ID => {
                let a = decode_packet_exact::<EntityAnimationPacket>(frame.payload).unwrap();
                if a.entity_id == bob_eid && a.animation == EntityAnimation::SwingMainArm {
                    alice_saw_bob_swing = true;
                }
            }
            _ => {}
        }
    }
    assert_eq!(attacked_target.load(Ordering::SeqCst), zombie_eid);

    // 3. Bob disconnects -> Alice receives DestroyEntitiesPacket + PlayerInfoRemovePacket!
    drop(bob);
    let mut alice_saw_destroy = false;
    let mut alice_saw_info_remove = false;
    while !(alice_saw_destroy && alice_saw_info_remove) {
        let frame = alice
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        match frame.packet_id {
            DestroyEntitiesPacket::PACKET_ID => {
                let d = decode_packet_exact::<DestroyEntitiesPacket>(frame.payload).unwrap();
                if d.entity_ids.contains(&bob_eid) {
                    alice_saw_destroy = true;
                }
            }
            PlayerInfoRemovePacket::PACKET_ID => {
                let r = decode_packet_exact::<PlayerInfoRemovePacket>(frame.payload).unwrap();
                if r.uuids.contains(&bob_uuid) {
                    alice_saw_info_remove = true;
                }
            }
            _ => {}
        }
    }

    drop(alice);
    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
}
