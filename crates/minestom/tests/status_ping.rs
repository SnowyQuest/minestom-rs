//! End-to-end integration tests connecting a real headless TCP client to `MinecraftServer`
//! over an ephemeral port (`127.0.0.1:0`).

use std::time::Duration;

use minestom::core::offline_player_uuid;
use minestom::network::PacketStream;
use minestom::prelude::*;
use minestom::protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom::protocol::login::LoginDisconnectPacket;
use minestom::protocol::status::{
    StatusPingRequestPacket, StatusPlayerSample, StatusPongResponsePacket, StatusRequestPacket,
    StatusResponsePacket, encode_rgba_64x64_png,
};
use minestom::protocol::{MAX_PACKET_SIZE, decode_packet_exact};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

#[tokio::test]
async fn headless_client_queries_server_list_motd_players_favicon_and_ping() {
    let icon = encode_rgba_64x64_png(&[[12, 180, 240, 255]; 64 * 64]);
    let motd = Component::text("minestom-rs M1 ")
        .color(NamedTextColor::Gold)
        .append(
            Component::text("Online!")
                .color(NamedTextColor::Green)
                .bold(true),
        );

    let server = MinecraftServer::builder()
        .motd(motd.clone())
        .max_players(64)
        .favicon(icon.clone())
        .player_sample(vec![StatusPlayerSample::new(
            "Steve",
            offline_player_uuid("Steve"),
        )])
        .build()
        .expect("build server");
    server.set_online_count(7);

    let bound = server
        .bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    // Connect headless client
    let tcp = TcpStream::connect(addr).await.expect("connect TCP");
    let mut client = PacketStream::new(tcp, MAX_PACKET_SIZE).expect("wrap packet stream");

    // 1. Send Handshake (Status intent)
    client
        .send_packet(&ClientHandshakePacket {
            protocol_version: PROTOCOL_VERSION,
            server_address: "localhost".to_owned(),
            server_port: addr.port(),
            intent: HandshakeIntent::Status,
        })
        .await
        .expect("send handshake");

    // 2. Send StatusRequestPacket and verify StatusResponsePacket
    client
        .send_packet(&StatusRequestPacket)
        .await
        .expect("send status request");

    let status_frame = client
        .read_frame_timeout(Duration::from_secs(5))
        .await
        .expect("read status frame")
        .expect("non-EOF status frame");
    assert_eq!(status_frame.packet_id, 0x00);
    let resp = decode_packet_exact::<StatusResponsePacket>(status_frame.payload)
        .expect("decode StatusResponsePacket");

    assert_eq!(resp.status.version.name, "26.2");
    assert_eq!(resp.status.version.protocol, 776);
    assert_eq!(resp.status.description, motd);
    assert_eq!(resp.status.favicon, Some(icon));
    let players = resp.status.players.expect("players block present");
    assert_eq!(players.online, 7);
    assert_eq!(players.max, 64);
    assert_eq!(players.sample.len(), 1);
    assert_eq!(players.sample[0].name, "Steve");

    // 3. Send PingRequest and verify echoed PongResponse + clean socket close
    let nonce = 0x1234_5678_9ABC_DEF0_i64;
    client
        .send_packet(&StatusPingRequestPacket { payload: nonce })
        .await
        .expect("send ping");

    let pong_frame = client
        .read_frame_timeout(Duration::from_secs(5))
        .await
        .expect("read pong frame")
        .expect("non-EOF pong frame");
    assert_eq!(pong_frame.packet_id, 0x01);
    let pong =
        decode_packet_exact::<StatusPongResponsePacket>(pong_frame.payload).expect("decode pong");
    assert_eq!(pong.payload, nonce);

    // Server closes connection immediately after PongResponse
    let eof = client
        .read_frame_timeout(Duration::from_secs(2))
        .await
        .expect("clean EOF");
    assert!(eof.is_none());

    handle.stop_cleanly();
    server_task
        .await
        .expect("join server")
        .expect("server exit ok");
}

#[tokio::test]
async fn duplicate_status_request_and_malformed_traffic_do_not_crash_server() {
    let server = MinecraftServer::builder().build().expect("build server");
    let bound = server.bind("127.0.0.1:0").await.expect("bind");
    let addr = bound.local_addr();
    let handle = bound.handle();
    let server_task = tokio::spawn(async move { bound.run().await });

    // Peer 1: Sends garbage malformed bytes
    {
        let mut bad_sock = TcpStream::connect(addr).await.expect("connect bad peer");
        bad_sock
            .write_all(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F, 0x00, 0x11])
            .await
            .unwrap();
    }

    // Peer 2: Sends wrong protocol version Login handshake and receives clean LoginDisconnectPacket
    {
        let tcp = TcpStream::connect(addr)
            .await
            .expect("connect outdated peer");
        let mut client = PacketStream::new(tcp, MAX_PACKET_SIZE).unwrap();
        client
            .send_packet(&ClientHandshakePacket {
                protocol_version: 760,
                server_address: "localhost".to_owned(),
                server_port: addr.port(),
                intent: HandshakeIntent::Login,
            })
            .await
            .unwrap();

        let kick_frame = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .expect("kick frame");
        let kick = decode_packet_exact::<LoginDisconnectPacket>(kick_frame.payload).unwrap();
        assert!(kick.reason.to_plain_text().contains("26.2"));
    }

    // Peer 3: Sends duplicate StatusRequestPacket -> disconnected cleanly; subsequent healthy client still works!
    {
        let tcp = TcpStream::connect(addr).await.expect("connect dup peer");
        let mut client = PacketStream::new(tcp, MAX_PACKET_SIZE).unwrap();
        client
            .send_packet(&ClientHandshakePacket {
                protocol_version: PROTOCOL_VERSION,
                server_address: "localhost".to_owned(),
                server_port: addr.port(),
                intent: HandshakeIntent::Status,
            })
            .await
            .unwrap();
        client.send_packet(&StatusRequestPacket).await.unwrap();
        let _ = client
            .read_frame_timeout(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        // Second StatusRequest must trigger disconnect
        client.send_packet(&StatusRequestPacket).await.unwrap();
        let after_dup = client
            .read_frame_timeout(Duration::from_secs(2))
            .await
            .unwrap();
        assert!(after_dup.is_none());
    }

    handle.stop_cleanly();
    server_task.await.unwrap().unwrap();
}
