//! End-to-end M9 integration tests verifying:
//! - Entity Attributes & 3-stage `AttributeModifier` synchronization (`EntityAttributesPacket` `0x83`)
//! - Potion Effects (`EntityEffectPacket` `0x84`) & automatic tick expiration (`RemoveEntityEffectPacket` `0x4E`)
//! - 3D A* obstacle pathfinding (`Navigator`) + `EntityAIGroup` (`ClosestEntityTarget` + `MeleeAttackGoal`)
//!   routing around a 2-block solid wall to strike a Survival player (`DamageEventPacket` `0x19`,
//!   `HitAnimationPacket` `0x2A`, knockback `EntityVelocityPacket`, `UpdateHealthPacket`)
//! - Player melee retaliation (`ClientAttackPacket` `0x01`) + `DIAMOND_SWORD` + `STRENGTH` potion
//!   slaying the mob (`EntityDamageEvent`, `EntityDeathEvent`, `DestroyEntitiesPacket`)
//! - Fatal player damage -> `ClientStatusPacket::PerformRespawn` (`0x0C`) -> `PlayerRespawnEvent` ->
//!   `RespawnPacket` (`0x52`) + full health & teleport to overridden respawn destination!

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
use minestom::protocol::entity::{
    AttributeWireOperation, ClientAttackPacket, DamageEventPacket, DestroyEntitiesPacket,
    EntityAttributesPacket, EntityEffectPacket, EntityVelocityPacket, HitAnimationPacket,
    RemoveEntityEffectPacket,
};
use minestom::protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom::protocol::inventory::WindowItemsPacket;
use minestom::protocol::login::{
    ClientLoginAcknowledgedPacket, ClientLoginStartPacket, LoginSuccessPacket, SetCompressionPacket,
};
use minestom::protocol::play::{
    ClientPlayKeepAlivePacket, ClientStatusAction, ClientStatusPacket, ClientTeleportConfirmPacket,
    JoinGamePacket, PlayKeepAlivePacket, PlayerPositionAndLookPacket, RespawnPacket,
    UpdateHealthPacket,
};
use minestom::protocol::{MAX_PACKET_SIZE, PacketPayload, decode_packet_exact};
use tokio::net::TcpStream;
use uuid::Uuid;

async fn connect_survival_player(addr: SocketAddr, username: &str) -> (PacketStream, i32) {
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
            view_distance: 4,
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

    let mut entity_id = 0;
    loop {
        let frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        if frame.packet_id == JoinGamePacket::PACKET_ID {
            let join = decode_packet_exact::<JoinGamePacket>(frame.payload).unwrap();
            entity_id = join.entity_id;
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
    (client, entity_id)
}

async fn wait_for_packet<P: PacketPayload>(stream: &mut PacketStream) -> P {
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            let frame = stream
                .read_frame_timeout(Duration::from_secs(6))
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
async fn attributes_potions_astar_ai_combat_and_respawn_end_to_end() {
    let server = MinecraftServer::builder()
        .chunk_view_distance(4)
        .build()
        .unwrap();

    let instance = server.instance_manager().create_instance_container();
    instance.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::STONE);
    });

    // Build a 2-block-high solid wall between x=3, z=-1..=1 so the AI zombie MUST pathfind around z=+-2!
    let mut wall = AbsoluteBlockBatch::new();
    for z in -1..=1 {
        wall.set_block(BlockVec::new(3, 40, z), Block::OBSIDIAN);
        wall.set_block(BlockVec::new(3, 41, z), Block::OBSIDIAN);
    }
    server.apply_batch_synced(&instance, &wall);

    // Verify direct A* path routes cleanly around the obsidian wall
    let precheck = find_path(&instance, BlockVec::new(5, 40, 0), BlockVec::new(0, 40, 0))
        .expect("A* must route around wall");
    assert!(
        precheck
            .iter()
            .all(|p| !(p.x.floor() as i32 == 3 && (-1..=1).contains(&(p.z.floor() as i32))))
    );

    let damage_events = Arc::new(AtomicUsize::new(0));
    let death_events = Arc::new(AtomicUsize::new(0));
    let dmg_cnt = Arc::clone(&damage_events);
    server
        .global_event_handler()
        .add_listener(move |_ev: &mut EntityDamageEvent| {
            dmg_cnt.fetch_add(1, Ordering::SeqCst);
        });
    let dth_cnt = Arc::clone(&death_events);
    server
        .global_event_handler()
        .add_listener(move |_ev: &mut EntityDeathEvent| {
            dth_cnt.fetch_add(1, Ordering::SeqCst);
        });

    let inst_cfg = instance.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&inst_cfg);
            ev.player().set_respawn_point(Pos::new(0.5, 40.0, 0.5));
            ev.player().set_game_mode(GameMode::Survival);
        });

    let player_id_slot = Arc::new(std::sync::Mutex::new(None));
    let pid_writer = Arc::clone(&player_id_slot);
    let srv_spawn = server.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut PlayerSpawnEvent| {
            if ev.first_spawn {
                *pid_writer.lock().unwrap() = Some(ev.player_id);
                srv_spawn
                    .give_player_item(ev.player_id, ItemStack::single(Material::DIAMOND_SWORD));
            }
        });

    let custom_respawn = Pos::new(-6.5, 40.0, 3.5);
    server
        .global_event_handler()
        .add_listener(move |ev: &mut PlayerRespawnEvent| {
            ev.respawn_position = custom_respawn;
        });

    let bound = server.bind("127.0.0.1:0").await.unwrap();
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    let (mut hero, hero_eid) = connect_survival_player(addr, "Hero").await;
    // Wait until sword arrives in hotbar slot 0 (window slot 36)
    loop {
        let win = wait_for_packet::<WindowItemsPacket>(&mut hero).await;
        if win
            .items
            .get(36)
            .is_some_and(|i| i.material() == Material::DIAMOND_SWORD)
        {
            break;
        }
    }
    let hero_pid = player_id_slot.lock().unwrap().unwrap();

    // 1. Test Attribute modifier sync (`EntityAttributesPacket` `0x83`) & finite Potion Effect
    //    sync (`EntityEffectPacket` `0x84` -> `RemoveEntityEffectPacket` `0x4E`)!
    handle.add_player_attribute_modifier(
        hero_pid,
        Attribute::MOVEMENT_SPEED,
        AttributeModifier::new(
            "test:sprint_boost",
            0.30,
            AttributeOperation::AddMultipliedTotal,
        ),
    );
    let attr_pkt = wait_for_packet::<EntityAttributesPacket>(&mut hero).await;
    assert_eq!(attr_pkt.entity_id, hero_eid);
    assert_eq!(attr_pkt.properties.len(), 1);
    assert_eq!(
        attr_pkt.properties[0].attribute_id,
        Attribute::MOVEMENT_SPEED.id() as i32
    );
    assert!((attr_pkt.properties[0].base_value - 0.1).abs() < 1e-6);
    assert_eq!(
        attr_pkt.properties[0].modifiers[0].operation,
        AttributeWireOperation::AddMultipliedTotal
    );

    handle.add_player_effect(hero_pid, Potion::new(PotionEffect::SPEED, 1, 3));
    handle.add_player_effect(hero_pid, Potion::new(PotionEffect::STRENGTH, 0, 400));
    let eff_pkt = wait_for_packet::<EntityEffectPacket>(&mut hero).await;
    assert_eq!(eff_pkt.entity_id, hero_eid);
    assert_eq!(eff_pkt.effect_id, PotionEffect::SPEED.id() as i32);
    assert_eq!(eff_pkt.amplifier, 1);

    // Wait 3 ticks -> `SPEED` expires and server sends `RemoveEntityEffectPacket`!
    let rem_pkt = wait_for_packet::<RemoveEntityEffectPacket>(&mut hero).await;
    assert_eq!(rem_pkt.entity_id, hero_eid);
    assert_eq!(rem_pkt.potion_effect_id, PotionEffect::SPEED.id() as i32);

    // 2. Spawn an AI Zombie behind the obsidian wall at `(5.5, 40.0, 0.5)` with `10.0` HP.
    //    It must navigate around the wall via A* and melee-strike `Hero`!
    let ai = EntityAIGroup::new()
        .with_target_selector(ClosestEntityTarget::players(20.0))
        .with_goal_selector(MeleeAttackGoal::new(0.45, 2.2, 15));
    let zombie = Entity::creature(EntityType::ZOMBIE)
        .with_max_health(10.0)
        .with_attribute(Attribute::ATTACK_DAMAGE, 4.0)
        .with_behavior(ai);
    let zombie_eid = handle
        .spawn_entity(&instance, zombie, Pos::new(5.5, 40.0, 0.5))
        .network_id();

    let dmg_on_hero = wait_for_packet::<DamageEventPacket>(&mut hero).await;
    assert_eq!(dmg_on_hero.target_entity_id, hero_eid);
    assert_eq!(
        dmg_on_hero.damage_type_id,
        DamageType::MOB_ATTACK.id() as i32
    );
    assert_eq!(dmg_on_hero.source_entity_id, Some(zombie_eid));

    let hit_anim = wait_for_packet::<HitAnimationPacket>(&mut hero).await;
    assert_eq!(hit_anim.entity_id, hero_eid);

    let kb_vel = wait_for_packet::<EntityVelocityPacket>(&mut hero).await;
    assert_eq!(kb_vel.entity_id, hero_eid);

    let hp_after_hit = wait_for_packet::<UpdateHealthPacket>(&mut hero).await;
    assert!((hp_after_hit.health - 16.0).abs() < 1e-3);

    // 3. Hero strikes back with `ClientAttackPacket`!
    //    Base 2.0 + Diamond Sword 6.0 + Strength I 3.0 = 11.0 >= 10.0 -> one-shots the zombie!
    hero.send_packet(&ClientAttackPacket {
        target_id: zombie_eid,
    })
    .await
    .unwrap();

    loop {
        let destroy = wait_for_packet::<DestroyEntitiesPacket>(&mut hero).await;
        if destroy.entity_ids.contains(&zombie_eid) {
            break;
        }
    }
    assert!(damage_events.load(Ordering::SeqCst) >= 2);
    assert_eq!(death_events.load(Ordering::SeqCst), 1);

    // 4. Deal fatal `OUT_OF_WORLD` damage to Hero -> verify death + `ClientStatusAction::PerformRespawn`
    //    -> `PlayerRespawnEvent` -> `RespawnPacket` + full health + teleport to `custom_respawn`!
    handle.damage_player(
        hero_pid,
        Damage {
            damage_type: DamageType::OUT_OF_WORLD,
            amount: 50.0,
            source_entity_id: None,
            source_direct_id: None,
            source_pos: None,
        },
    );
    loop {
        let hp = wait_for_packet::<UpdateHealthPacket>(&mut hero).await;
        if hp.health <= 0.0 {
            break;
        }
    }
    assert_eq!(death_events.load(Ordering::SeqCst), 2);

    hero.send_packet(&ClientStatusPacket {
        action: ClientStatusAction::PerformRespawn,
    })
    .await
    .unwrap();

    let respawn_pkt = wait_for_packet::<RespawnPacket>(&mut hero).await;
    assert_eq!(respawn_pkt.copy_data, RespawnPacket::COPY_NONE);
    assert_eq!(respawn_pkt.spawn_info.game_mode, GameMode::Survival);

    let full_hp = wait_for_packet::<UpdateHealthPacket>(&mut hero).await;
    assert!((full_hp.health - 20.0).abs() < 1e-3);

    let tp = wait_for_packet::<PlayerPositionAndLookPacket>(&mut hero).await;
    assert!((tp.position.x - custom_respawn.x).abs() < 1e-6);
    assert!((tp.position.z - custom_respawn.z).abs() < 1e-6);

    drop(hero);
    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
}
