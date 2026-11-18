//! End-to-end M8 integration tests over real TCP sockets on `127.0.0.1:0`:
//! 1. Full Online Mode (RSA-1024 `EncryptionRequest`/`ClientEncryptionResponse` + streaming
//!    AES-128-CFB8 cipher + Zlib compression + local mock Mojang `hasJoined` HTTP server)!
//! 2. Velocity modern forwarding (`velocity:player_info` HMAC-SHA256 v1) valid & invalid secrets!
//! 3. `BungeeGuard` `\0` handshake forwarding, token-bucket `PacketRateLimiter`, and graceful
//!    shutdown kicking players + saving modified Anvil `.mca` chunks!

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use minestom::network::{
    PacketStream, encode_velocity_player_info, encrypt_rsa_pkcs1v15, mojang_sha1_server_id,
};
use minestom::prelude::*;
use minestom::protocol::config::{
    ClientFinishConfigurationPacket, ClientSelectKnownPacksPacket, ClientSettingsPacket,
    FinishConfigurationPacket, SelectKnownPacksPacket,
};
use minestom::protocol::entity::ClientAnimationPacket;
use minestom::protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom::protocol::login::{
    ClientEncryptionResponsePacket, ClientLoginAcknowledgedPacket, ClientLoginPluginResponsePacket,
    ClientLoginStartPacket, EncryptionRequestPacket, GameProfile, LoginDisconnectPacket,
    LoginPluginRequestPacket, LoginSuccessPacket, ProfileProperty, SetCompressionPacket,
};
use minestom::protocol::play::{
    ChunkDataPacket, ClientTeleportConfirmPacket, PlayDisconnectPacket, PlayerPositionAndLookPacket,
};
use minestom::protocol::{MAX_PACKET_SIZE, PacketPayload, decode_packet_exact};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

static AUTH_SEQ: AtomicU64 = AtomicU64::new(1);

async fn finish_config_and_enter_play(client: &mut PacketStream) {
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
}

/// Spawns a local mock Yggdrasil `hasJoined` HTTP server that approves `(expected_user, expected_hash)`.
async fn spawn_mock_mojang_server(
    approved_hash: Arc<Mutex<Option<String>>>,
    auth_uuid: Uuid,
) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let expected = approved_hash.lock().unwrap().clone();
            let mut req_buf = vec![0_u8; 2048];
            let Ok(n) = sock.read(&mut req_buf).await else {
                continue;
            };
            let req_text = String::from_utf8_lossy(&req_buf[..n]);
            let first_line = req_text.lines().next().unwrap_or_default();

            let matched = expected.as_ref().is_some_and(|exp_hash| {
                first_line.contains("username=OnlinePlayer")
                    && first_line.contains(&format!("serverId={exp_hash}"))
            });

            let http_resp = if matched {
                let body = format!(
                    "{{\"id\":\"{}\",\"name\":\"OnlinePlayer\",\"properties\":[{{\"name\":\"textures\",\"value\":\"c2tpbg==\",\"signature\":\"c2ln\"}}]}}",
                    auth_uuid.simple()
                );
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned()
            };
            let _ = sock.write_all(http_resp.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
    addr
}

#[tokio::test]
async fn online_mode_rsa_aes128_cfb8_and_mock_mojang_session_server_work() {
    let approved_hash = Arc::new(Mutex::new(None));
    let expected_uuid = Uuid::new_v4();
    let mock_addr = spawn_mock_mojang_server(Arc::clone(&approved_hash), expected_uuid).await;
    let endpoint = format!("http://{mock_addr}/session/minecraft/hasJoined");

    let server = MinecraftServer::builder()
        .chunk_view_distance(2)
        .auth(AuthMode::online_with_url(endpoint, true))
        .build()
        .unwrap();

    let inst = server.instance_manager().create_instance_container();
    inst.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::STONE);
    });
    let spawn_inst = inst.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&spawn_inst);
            ev.player().set_respawn_point(Pos::new(0.5, 41.0, 0.5));
        });

    let bound = server.bind("127.0.0.1:0").await.unwrap();
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    // 1. Valid OnlinePlayer completes RSA-1024 + AES-128-CFB8 + Zlib + Play!
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
            username: "OnlinePlayer".to_owned(),
            profile_id: Uuid::nil(),
        })
        .await
        .unwrap();

    let enc_req = decode_packet_exact::<EncryptionRequestPacket>(
        client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    assert!(enc_req.should_authenticate);

    let shared_secret: [u8; 16] = [
        0x10, 0x22, 0x34, 0x46, 0x58, 0x6A, 0x7C, 0x8E, 0x91, 0xA3, 0xB5, 0xC7, 0xD9, 0xEB, 0xFD,
        0x0F,
    ];
    let server_hash =
        mojang_sha1_server_id(&enc_req.server_id, &shared_secret, &enc_req.public_key);
    *approved_hash.lock().unwrap() = Some(server_hash);

    let enc_secret = encrypt_rsa_pkcs1v15(&enc_req.public_key, &shared_secret).unwrap();
    let enc_token = encrypt_rsa_pkcs1v15(&enc_req.public_key, &enc_req.verify_token).unwrap();

    client
        .send_packet(&ClientEncryptionResponsePacket {
            shared_secret: enc_secret,
            encrypted_verify_token: enc_token,
        })
        .await
        .unwrap();
    // Enable AES-128-CFB8 on client immediately after sending ClientEncryptionResponsePacket!
    client.enable_encryption(&shared_secret);

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

    let success = decode_packet_exact::<LoginSuccessPacket>(
        client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    assert_eq!(success.profile.uuid, expected_uuid);
    assert_eq!(success.profile.properties.len(), 1);
    assert_eq!(success.profile.properties[0].name, "textures");

    finish_config_and_enter_play(&mut client).await;

    // 2. Unauthenticated user ("CrackedUser") gets kicked with encrypted LoginDisconnectPacket!
    let mut bad =
        PacketStream::new(TcpStream::connect(addr).await.unwrap(), MAX_PACKET_SIZE).unwrap();
    bad.send_packet(&ClientHandshakePacket {
        protocol_version: PROTOCOL_VERSION,
        server_address: "localhost".to_owned(),
        server_port: addr.port(),
        intent: HandshakeIntent::Login,
    })
    .await
    .unwrap();
    bad.send_packet(&ClientLoginStartPacket {
        username: "CrackedUser".to_owned(),
        profile_id: Uuid::nil(),
    })
    .await
    .unwrap();
    let req2 = decode_packet_exact::<EncryptionRequestPacket>(
        bad.read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    let sec2 = [7_u8; 16];
    bad.send_packet(&ClientEncryptionResponsePacket {
        shared_secret: encrypt_rsa_pkcs1v15(&req2.public_key, &sec2).unwrap(),
        encrypted_verify_token: encrypt_rsa_pkcs1v15(&req2.public_key, &req2.verify_token).unwrap(),
    })
    .await
    .unwrap();
    bad.enable_encryption(&sec2);
    let kick_frame = bad
        .read_frame_timeout(Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kick_frame.packet_id, LoginDisconnectPacket::PACKET_ID);

    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn velocity_modern_forwarding_hmac_sha256_accepts_valid_and_rejects_bad_secret() {
    let secret = "prod-velocity-secret-xyz";
    let server = MinecraftServer::builder()
        .chunk_view_distance(2)
        .auth(AuthMode::velocity(secret))
        .build()
        .unwrap();

    let inst = server.instance_manager().create_instance_container();
    let spawn_inst = inst.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&spawn_inst);
            ev.player().set_respawn_point(Pos::new(0.5, 41.0, 0.5));
        });

    let bound = server.bind("127.0.0.1:0").await.unwrap();
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    // 1. Valid Velocity HMAC-SHA256 login
    let fwd_uuid = Uuid::new_v4();
    let fwd_profile = GameProfile {
        uuid: fwd_uuid,
        username: "VeloUser".to_owned(),
        properties: vec![ProfileProperty {
            name: "textures".to_owned(),
            value: "vtex".to_owned(),
            signature: None,
        }],
    };
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
            username: "VeloUser".to_owned(),
            profile_id: Uuid::nil(),
        })
        .await
        .unwrap();

    let plug_req = decode_packet_exact::<LoginPluginRequestPacket>(
        client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    assert_eq!(plug_req.channel, "velocity:player_info");

    let signed_payload = encode_velocity_player_info(
        secret.as_bytes(),
        "198.51.100.99".parse::<IpAddr>().unwrap(),
        &fwd_profile,
    )
    .unwrap();
    client
        .send_packet(&ClientLoginPluginResponsePacket {
            message_id: plug_req.message_id,
            data: Some(signed_payload),
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
    let success = decode_packet_exact::<LoginSuccessPacket>(
        client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    assert_eq!(success.profile.uuid, fwd_uuid);
    finish_config_and_enter_play(&mut client).await;

    // 2. Wrong Velocity secret gets kicked immediately!
    let mut bad =
        PacketStream::new(TcpStream::connect(addr).await.unwrap(), MAX_PACKET_SIZE).unwrap();
    bad.send_packet(&ClientHandshakePacket {
        protocol_version: PROTOCOL_VERSION,
        server_address: "localhost".to_owned(),
        server_port: addr.port(),
        intent: HandshakeIntent::Login,
    })
    .await
    .unwrap();
    bad.send_packet(&ClientLoginStartPacket {
        username: "Attacker".to_owned(),
        profile_id: Uuid::nil(),
    })
    .await
    .unwrap();
    let req2 = decode_packet_exact::<LoginPluginRequestPacket>(
        bad.read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    let bad_payload = encode_velocity_player_info(
        b"wrong-secret",
        "198.51.100.1".parse().unwrap(),
        &fwd_profile,
    )
    .unwrap();
    bad.send_packet(&ClientLoginPluginResponsePacket {
        message_id: req2.message_id,
        data: Some(bad_payload),
    })
    .await
    .unwrap();
    let kick = bad
        .read_frame_timeout(Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kick.packet_id, LoginDisconnectPacket::PACKET_ID);

    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn bungeeguard_rate_limiting_and_graceful_shutdown_anvil_save_work() {
    let seq = AUTH_SEQ.fetch_add(1, Ordering::Relaxed);
    let world_dir = std::env::temp_dir().join(format!(
        "minestom_rs_m8_shutdown_{}_{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&world_dir);

    let dim = DimensionType::overworld();
    let server = MinecraftServer::builder()
        .chunk_view_distance(2)
        .auth(AuthMode::bungee_guard(["guard-secret-77"]))
        .max_packets_per_second(Some(15))
        .build()
        .unwrap();

    let inst = server.instance_manager().create_instance_container();
    inst.set_chunk_loader(AnvilLoader::with_dimension(&world_dir, &dim.key));
    inst.set_block(BlockVec::new(1, 45, 1), Block::GOLD_BLOCK);

    let spawn_inst = inst.clone();
    server
        .global_event_handler()
        .add_listener(move |ev: &mut AsyncPlayerConfigurationEvent| {
            ev.set_spawning_instance(&spawn_inst);
            ev.player().set_respawn_point(Pos::new(0.5, 46.0, 0.5));
        });

    let bound = server.bind("127.0.0.1:0").await.unwrap();
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    // 1. Direct connection without BungeeGuard token is kicked during Handshake!
    let mut direct =
        PacketStream::new(TcpStream::connect(addr).await.unwrap(), MAX_PACKET_SIZE).unwrap();
    direct
        .send_packet(&ClientHandshakePacket {
            protocol_version: PROTOCOL_VERSION,
            server_address: "localhost".to_owned(),
            server_port: addr.port(),
            intent: HandshakeIntent::Login,
        })
        .await
        .unwrap();
    let kick = direct
        .read_frame_timeout(Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kick.packet_id, LoginDisconnectPacket::PACKET_ID);

    // 2. Valid BungeeGuard client joins, floods >150 packets, and gets kicked by PacketRateLimiter!
    let flood_uuid = Uuid::new_v4();
    let bungee_addr = format!(
        "play.example.com\0203.0.113.5\0{}\0[{{\"name\":\"bungeeguard-token\",\"value\":\"guard-secret-77\"}}]",
        flood_uuid.simple()
    );
    let mut flooder =
        PacketStream::new(TcpStream::connect(addr).await.unwrap(), MAX_PACKET_SIZE).unwrap();
    flooder
        .send_packet(&ClientHandshakePacket {
            protocol_version: PROTOCOL_VERSION,
            server_address: bungee_addr.clone(),
            server_port: addr.port(),
            intent: HandshakeIntent::Login,
        })
        .await
        .unwrap();
    flooder
        .send_packet(&ClientLoginStartPacket {
            username: "FloodBot".to_owned(),
            profile_id: Uuid::nil(),
        })
        .await
        .unwrap();
    let comp = decode_packet_exact::<SetCompressionPacket>(
        flooder
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    flooder.set_compression_threshold(Some(comp.threshold as usize));
    let _ = decode_packet_exact::<LoginSuccessPacket>(
        flooder
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    finish_config_and_enter_play(&mut flooder).await;

    for _ in 0..200 {
        if flooder
            .send_packet(&ClientAnimationPacket { hand: Hand::Main })
            .await
            .is_err()
        {
            break;
        }
    }
    let mut saw_rate_kick = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match flooder.read_frame_timeout(Duration::from_secs(2)).await {
            Ok(Some(f)) if f.packet_id == PlayDisconnectPacket::PACKET_ID => {
                saw_rate_kick = true;
                break;
            }
            Ok(None) | Err(_) => break,
            _ => {}
        }
    }
    assert!(
        saw_rate_kick,
        "expected PlayDisconnectPacket on packet flood"
    );

    // 3. Connect a well-behaved BungeeGuard player, trigger `handle.stop_cleanly()`, and verify:
    //    (a) player receives `"Server closed"` `PlayDisconnectPacket`, AND
    //    (b) `GOLD_BLOCK` at (1, 45, 1) was automatically saved to `.mca` on shutdown!
    let good_addr = format!(
        "play.example.com\0203.0.113.6\0{}\0[{{\"name\":\"bungeeguard-token\",\"value\":\"guard-secret-77\"}}]",
        Uuid::new_v4().simple()
    );
    let mut good =
        PacketStream::new(TcpStream::connect(addr).await.unwrap(), MAX_PACKET_SIZE).unwrap();
    good.send_packet(&ClientHandshakePacket {
        protocol_version: PROTOCOL_VERSION,
        server_address: good_addr,
        server_port: addr.port(),
        intent: HandshakeIntent::Login,
    })
    .await
    .unwrap();
    good.send_packet(&ClientLoginStartPacket {
        username: "GoodPlayer".to_owned(),
        profile_id: Uuid::nil(),
    })
    .await
    .unwrap();
    let comp2 = decode_packet_exact::<SetCompressionPacket>(
        good.read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    good.set_compression_threshold(Some(comp2.threshold as usize));
    let _ = decode_packet_exact::<LoginSuccessPacket>(
        good.read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .payload,
    )
    .unwrap();
    finish_config_and_enter_play(&mut good).await;

    handle.stop_cleanly();

    let mut saw_shutdown_kick = false;
    while let Ok(Some(f)) = good.read_frame_timeout(Duration::from_secs(3)).await {
        if f.packet_id == PlayDisconnectPacket::PACKET_ID {
            saw_shutdown_kick = true;
            break;
        }
    }
    assert!(saw_shutdown_kick);
    server_task.await.unwrap().unwrap();

    let verify_loader = AnvilLoader::with_dimension(&world_dir, &dim.key);
    let saved_chunk = verify_loader
        .load_chunk(&dim, ChunkPos::new(0, 0))
        .unwrap()
        .expect("chunk saved on graceful shutdown");
    assert_eq!(saved_chunk.get_block(1, 45, 1), Block::GOLD_BLOCK);

    let _ = std::fs::remove_dir_all(&world_dir);
}
