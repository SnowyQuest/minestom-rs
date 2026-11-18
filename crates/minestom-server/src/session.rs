//! Per-connection Tokio state machine driving `Handshake` → `Status` OR
//! `Login` (`Offline` / `Online` RSA+AES-CFB8 / `Velocity` HMAC-SHA256 / `BungeeGuard`) →
//! `Configuration` → `Play`, with per-connection token-bucket rate limiting.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;
use flume::Sender;
use minestom_core::{MainHand, offline_player_uuid};
use minestom_data::{PROTOCOL_VERSION, VERSION_NAME};
use minestom_entity::ConfiguringPlayer;
use minestom_event::CancellableEvent;
use minestom_network::{
    NetworkError, PRE_AUTH_SOCKET_TIMEOUT, PacketRateLimiter, PacketStream, ServerEncryptionKeys,
    VELOCITY_PLAYER_INFO_CHANNEL, mojang_sha1_server_id, parse_bungee_handshake,
    verify_velocity_player_info,
};
use minestom_protocol::config::{
    ClientConfigKeepAlivePacket, ClientConfigPluginMessagePacket, ClientFinishConfigurationPacket,
    ClientSelectKnownPacksPacket, ClientSettingsPacket, ConfigDisconnectPacket,
    ConfigPluginMessagePacket, ConfigTagsPacket, FinishConfigurationPacket, KnownPack,
    RegistryDataEntry, RegistryDataPacket, SelectKnownPacksPacket, TagBinding, TagRegistryGroup,
    UpdateEnabledFeaturesPacket,
};
use minestom_protocol::handshake::{ClientHandshakePacket, HandshakeIntent};
use minestom_protocol::login::{
    ClientEncryptionResponsePacket, ClientLoginAcknowledgedPacket, ClientLoginPluginResponsePacket,
    ClientLoginStartPacket, EncryptionRequestPacket, GameProfile, LoginDisconnectPacket,
    LoginPluginRequestPacket, LoginSuccessPacket, ProfileProperty, SetCompressionPacket,
};
use minestom_protocol::play::PlayDisconnectPacket;
use minestom_protocol::status::{
    StatusPingRequestPacket, StatusPongResponsePacket, StatusRequestPacket, StatusResponsePacket,
};
use minestom_protocol::{
    ConnectionState, MAX_PACKET_SIZE, MAX_PACKET_SIZE_PRE_AUTH, PacketPayload, ProtocolError,
    decode_packet_exact, encode_packet_payload,
};
use minestom_registry::Registries;
use minestom_text::{Component, NamedTextColor};
use tokio::net::TcpStream;
use uuid::Uuid;

use crate::event::{AsyncPlayerConfigurationEvent, ClientPingServerEvent, ServerListPingEvent};
use crate::tick::{OutboundCommand, TickMessage};
use crate::{AuthMode, ServerShared};

const PLAY_READ_TIMEOUT: Duration = Duration::from_secs(30);

fn is_valid_username(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Pre-encodes the 27 vanilla `RegistryDataPacket`s (`exclude_vanilla = true`) and `ConfigTagsPacket`
/// once at server startup so Configuration phase sends zero-allocation `Bytes` slices.
pub(crate) fn build_cached_vanilla_config_packets(
    registries: &Registries,
) -> Result<(Vec<Bytes>, Bytes), ProtocolError> {
    let mut reg_packets = Vec::with_capacity(registries.configuration_registries().len());
    for reg in registries.configuration_registries() {
        let entries = reg
            .entries
            .iter()
            .map(|(id, _)| RegistryDataEntry {
                id: (*id).to_owned(),
                data: None,
            })
            .collect();
        let pkt = RegistryDataPacket {
            registry_id: reg.registry_id.to_owned(),
            entries,
        };
        reg_packets.push(encode_packet_payload(&pkt)?);
    }

    let tag_groups = registries
        .client_tag_registries()
        .iter()
        .map(|(reg_id, tags)| TagRegistryGroup {
            registry: (*reg_id).to_owned(),
            tags: tags
                .iter()
                .map(|(tag_id, ids)| TagBinding {
                    identifier: (*tag_id).to_owned(),
                    entries: ids.iter().map(|&n| n as i32).collect(),
                })
                .collect(),
        })
        .collect();
    let tags_bytes = encode_packet_payload(&ConfigTagsPacket {
        registries: tag_groups,
    })?;

    Ok((reg_packets, tags_bytes))
}

fn build_full_nbt_registry_packets(registries: &Registries) -> Result<Vec<Bytes>, ProtocolError> {
    let mut out = Vec::with_capacity(registries.configuration_registries().len());
    for reg in registries.configuration_registries() {
        let mut entries = Vec::with_capacity(reg.entries.len());
        for (id, raw_json) in reg.entries {
            let nbt = Registries::decode_entry_nbt(raw_json)
                .map_err(|e| ProtocolError::InvalidField(e.to_string()))?;
            entries.push(RegistryDataEntry {
                id: (*id).to_owned(),
                data: Some(nbt),
            });
        }
        let pkt = RegistryDataPacket {
            registry_id: reg.registry_id.to_owned(),
            entries,
        };
        out.push(encode_packet_payload(&pkt)?);
    }
    Ok(out)
}

async fn login_kick(conn: &mut PacketStream, msg: &str) -> Result<(), NetworkError> {
    conn.send_packet(&LoginDisconnectPacket {
        reason: Component::text(msg).color(NamedTextColor::Red),
    })
    .await?;
    let _ = conn.shutdown().await;
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn run_client_session(
    tcp: TcpStream,
    shared: Arc<ServerShared>,
    tick_tx: Sender<TickMessage>,
) -> Result<(), NetworkError> {
    let mut conn = PacketStream::new(tcp, MAX_PACKET_SIZE_PRE_AUTH)?;
    if let Some(pps) = shared.config.max_packets_per_second {
        conn.set_rate_limiter(Some(PacketRateLimiter::new(
            pps,
            pps.saturating_mul(2).max(128),
        )));
    }

    // 1. Handshake
    let Some(first_frame) = conn.read_frame_timeout(PRE_AUTH_SOCKET_TIMEOUT).await? else {
        return Ok(());
    };
    if first_frame.packet_id != ClientHandshakePacket::PACKET_ID {
        return Err(NetworkError::Protocol(ProtocolError::UnknownPacketId {
            state: ConnectionState::Handshake,
            id: first_frame.packet_id,
        }));
    }
    let handshake = decode_packet_exact::<ClientHandshakePacket>(first_frame.payload)?;

    let mut bungee_forwarded: Option<(Uuid, Vec<ProfileProperty>)> = None;

    match handshake.intent {
        HandshakeIntent::Status => return handle_status_phase(&mut conn, &shared).await,
        HandshakeIntent::Transfer => {
            conn.send_packet(&LoginDisconnectPacket {
                reason: Component::translatable("multiplayer.disconnect.transfers_disabled"),
            })
            .await?;
            let _ = conn.shutdown().await;
            return Ok(());
        }
        HandshakeIntent::Login => {
            if handshake.protocol_version != PROTOCOL_VERSION {
                return login_kick(
                    &mut conn,
                    &format!("Invalid Version, please use {VERSION_NAME}"),
                )
                .await;
            }
            if let AuthMode::Bungee { guard_tokens } = &shared.config.auth {
                match parse_bungee_handshake(&handshake.server_address, guard_tokens.as_ref()) {
                    Ok((_, client_ip, buuid, bprops)) => {
                        conn.set_peer_ip(client_ip);
                        bungee_forwarded = Some((buuid, bprops));
                    }
                    Err(_) => {
                        return login_kick(
                            &mut conn,
                            "Invalid connection, please connect through the BungeeCord proxy.",
                        )
                        .await;
                    }
                }
            } else if handshake.server_address.len() > 255 {
                return login_kick(&mut conn, "Server address too long!").await;
            }
        }
    }

    // 2. Login Phase (`Offline`, `Bungee`, `Velocity`, or `Online`)
    let Some(login_frame) = conn.read_frame_timeout(PRE_AUTH_SOCKET_TIMEOUT).await? else {
        return Ok(());
    };
    if login_frame.packet_id != ClientLoginStartPacket::PACKET_ID {
        return Err(NetworkError::Protocol(ProtocolError::UnknownPacketId {
            state: ConnectionState::Login,
            id: login_frame.packet_id,
        }));
    }
    let login_start = decode_packet_exact::<ClientLoginStartPacket>(login_frame.payload)?;
    if !is_valid_username(&login_start.username) {
        return login_kick(&mut conn, "Malformed username!").await;
    }

    let profile = match &shared.config.auth {
        AuthMode::Offline => {
            let player_uuid = if login_start.profile_id.is_nil() {
                offline_player_uuid(&login_start.username)
            } else {
                login_start.profile_id
            };
            GameProfile {
                uuid: player_uuid,
                username: login_start.username,
                properties: Vec::new(),
            }
        }
        AuthMode::Bungee { .. } => {
            let Some((buuid, bprops)) = bungee_forwarded else {
                return login_kick(&mut conn, "Missing BungeeCord forwarding!").await;
            };
            GameProfile {
                uuid: buuid,
                username: login_start.username,
                properties: bprops,
            }
        }
        AuthMode::Velocity { secret } => {
            let query_id = 1_i32;
            conn.send_packet(&LoginPluginRequestPacket {
                message_id: query_id,
                channel: VELOCITY_PLAYER_INFO_CHANNEL.to_owned(),
                data: Vec::new(),
            })
            .await?;

            let Some(plug_frame) = conn.read_frame_timeout(PRE_AUTH_SOCKET_TIMEOUT).await? else {
                return Ok(());
            };
            if plug_frame.packet_id != ClientLoginPluginResponsePacket::PACKET_ID {
                return login_kick(&mut conn, "Expected Velocity plugin response!").await;
            }
            let resp = decode_packet_exact::<ClientLoginPluginResponsePacket>(plug_frame.payload)?;
            let Some(raw_data) = resp.data.filter(|_| resp.message_id == query_id) else {
                return login_kick(&mut conn, "Invalid proxy response!").await;
            };
            let Ok(forwarded) = verify_velocity_player_info(secret.as_bytes(), &raw_data) else {
                return login_kick(&mut conn, "Invalid proxy response!").await;
            };
            conn.set_peer_ip(forwarded.client_ip);
            forwarded.profile
        }
        AuthMode::Online { .. } => {
            let (Some(keys), Some(session_client)) =
                (&shared.encryption_keys, &shared.session_client)
            else {
                return login_kick(&mut conn, "Online encryption not initialized!").await;
            };
            let nonce = ServerEncryptionKeys::generate_verify_token();
            conn.send_packet(&EncryptionRequestPacket {
                server_id: String::new(),
                public_key: keys.public_key_der().to_vec(),
                verify_token: nonce.to_vec(),
                should_authenticate: true,
            })
            .await?;

            let Some(enc_frame) = conn.read_frame_timeout(PRE_AUTH_SOCKET_TIMEOUT).await? else {
                return Ok(());
            };
            if enc_frame.packet_id != ClientEncryptionResponsePacket::PACKET_ID {
                return login_kick(&mut conn, "Expected EncryptionResponse!").await;
            }
            let enc_resp =
                decode_packet_exact::<ClientEncryptionResponsePacket>(enc_frame.payload)?;
            let Ok(shared_secret) = keys.verify_encryption_response(
                &nonce,
                &enc_resp.shared_secret,
                &enc_resp.encrypted_verify_token,
            ) else {
                return login_kick(&mut conn, "Encryption verification failed!").await;
            };

            // Enable AES-128-CFB8 stream cipher immediately after ClientEncryptionResponsePacket!
            conn.enable_encryption(&shared_secret);

            let server_hash = mojang_sha1_server_id("", &shared_secret, keys.public_key_der());
            match session_client
                .verify_has_joined(
                    &login_start.username,
                    &server_hash,
                    Some(conn.peer_addr().ip()),
                )
                .await
            {
                Ok(verified_profile) => verified_profile,
                Err(_) => {
                    return login_kick(&mut conn, "Failed to verify username!").await;
                }
            }
        }
    };

    if let Some(threshold) = shared.config.compression_threshold
        && threshold > 0
    {
        conn.send_packet(&SetCompressionPacket {
            threshold: threshold as i32,
        })
        .await?;
        conn.set_compression_threshold(Some(threshold));
    }
    conn.set_max_inbound_size(MAX_PACKET_SIZE);

    conn.send_packet(&LoginSuccessPacket {
        profile: profile.clone(),
        session_id: Uuid::nil(),
    })
    .await?;

    let Some(ack_frame) = conn.read_frame_timeout(PRE_AUTH_SOCKET_TIMEOUT).await? else {
        return Ok(());
    };
    if ack_frame.packet_id != ClientLoginAcknowledgedPacket::PACKET_ID {
        return Err(NetworkError::Protocol(ProtocolError::UnknownPacketId {
            state: ConnectionState::Login,
            id: ack_frame.packet_id,
        }));
    }
    let _ = decode_packet_exact::<ClientLoginAcknowledgedPacket>(ack_frame.payload)?;

    // 3. Configuration Phase
    conn.send_packet(&ConfigPluginMessagePacket::brand(
        &shared.config.brand_name,
    )?)
    .await?;
    conn.send_packet(&SelectKnownPacksPacket {
        entries: vec![KnownPack::minecraft_core()],
    })
    .await?;

    let mut config_event = AsyncPlayerConfigurationEvent::new(ConfiguringPlayer::new(
        profile.uuid,
        profile.username.clone(),
    ));
    shared.events.call(&mut config_event);
    let (configured_player, spawning_instance_opt) = config_event.into_parts();

    let Some(spawning_instance) = spawning_instance_opt else {
        conn.send_packet(&ConfigDisconnectPacket {
            message: Component::text("No spawning instance configured for player!")
                .color(NamedTextColor::Red),
        })
        .await?;
        let _ = conn.shutdown().await;
        return Ok(());
    };

    conn.send_packet(&UpdateEnabledFeaturesPacket::default())
        .await?;

    let mut client_view_distance = shared.config.chunk_view_distance;
    let mut client_main_hand = MainHand::Right;
    let mut client_skin_parts = 0x7F_u8;
    let mut finish_sent = false;

    loop {
        let Some(cfg_frame) = conn.read_frame_timeout(PRE_AUTH_SOCKET_TIMEOUT).await? else {
            return Ok(());
        };
        match cfg_frame.packet_id {
            ClientSettingsPacket::PACKET_ID => {
                let settings = decode_packet_exact::<ClientSettingsPacket>(cfg_frame.payload)?;
                if settings.view_distance > 0 {
                    client_view_distance =
                        (settings.view_distance as u8).min(shared.config.chunk_view_distance);
                }
                client_main_hand = settings.main_hand;
                client_skin_parts = settings.displayed_skin_parts;
            }
            ClientConfigPluginMessagePacket::PACKET_ID => {
                let _ = decode_packet_exact::<ClientConfigPluginMessagePacket>(cfg_frame.payload)?;
            }
            ClientConfigKeepAlivePacket::PACKET_ID => {
                let _ = decode_packet_exact::<ClientConfigKeepAlivePacket>(cfg_frame.payload)?;
            }
            ClientSelectKnownPacksPacket::PACKET_ID if !finish_sent => {
                let kp = decode_packet_exact::<ClientSelectKnownPacksPacket>(cfg_frame.payload)?;
                let core_pack = KnownPack::minecraft_core();
                let exclude_vanilla = kp.entries.contains(&core_pack);

                if exclude_vanilla {
                    for body in &shared.cached_registry_packets {
                        conn.send_raw_body(body).await?;
                    }
                } else {
                    for body in build_full_nbt_registry_packets(&shared.registries)? {
                        conn.send_raw_body(&body).await?;
                    }
                }
                conn.send_raw_body(&shared.cached_tags_packet).await?;
                conn.send_packet(&FinishConfigurationPacket).await?;
                finish_sent = true;
            }
            ClientFinishConfigurationPacket::PACKET_ID if finish_sent => {
                let _ = decode_packet_exact::<ClientFinishConfigurationPacket>(cfg_frame.payload)?;
                break;
            }
            0x01 | 0x05 | 0x06 | 0x08 | 0x09 => {}
            other => {
                return Err(NetworkError::Protocol(ProtocolError::UnknownPacketId {
                    state: ConnectionState::Configuration,
                    id: other,
                }));
            }
        }
    }

    // 4. Transition to Play: split socket & register with 20 TPS tick loop
    let entity_id = shared.next_entity_id.fetch_add(1, Ordering::Relaxed);
    let (mut reader, mut writer) = conn.into_split();
    let (outbound_tx, outbound_rx) = flume::bounded::<OutboundCommand>(1024);
    let writer_kick_tx = outbound_tx.clone();
    let (pid_tx, pid_rx) = flume::bounded(1);

    if tick_tx
        .send_async(TickMessage::RegisterPlayer {
            entity_id,
            profile,
            game_mode: configured_player.game_mode(),
            permission_level: configured_player.permission_level(),
            main_hand: client_main_hand,
            displayed_skin_parts: client_skin_parts,
            spawn_pos: configured_player.respawn_point(),
            view_distance: client_view_distance,
            instance: spawning_instance,
            outbound_tx,
            player_id_reply: pid_tx,
        })
        .await
        .is_err()
    {
        return Ok(());
    }

    let Ok(player_id) = pid_rx.recv_async().await else {
        return Ok(());
    };

    let writer_handle = tokio::spawn(async move {
        while let Ok(cmd) = outbound_rx.recv_async().await {
            match cmd {
                OutboundCommand::Packet(bytes) => {
                    if writer.send_raw_body(&bytes).await.is_err() {
                        break;
                    }
                }
                OutboundCommand::Disconnect(maybe_kick) => {
                    if let Some(kick) = maybe_kick {
                        let _ = writer.send_raw_body(&kick).await;
                    }
                    let _ = writer.shutdown().await;
                    break;
                }
            }
        }
    });

    loop {
        match reader.read_frame_timeout(PLAY_READ_TIMEOUT).await {
            Ok(Some(frame)) => {
                if tick_tx
                    .send_async(TickMessage::InboundPacket { player_id, frame })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(NetworkError::RateLimited) => {
                let kick = encode_packet_payload(&PlayDisconnectPacket {
                    message: Component::text("Packet rate limit exceeded!")
                        .color(NamedTextColor::Red),
                })
                .ok();
                let _ = writer_kick_tx.try_send(OutboundCommand::Disconnect(kick));
                break;
            }
            Ok(None) | Err(_) => break,
        }
    }

    let _ = tick_tx
        .send_async(TickMessage::PlayerDisconnected { player_id })
        .await;
    let _ = writer_kick_tx.try_send(OutboundCommand::Disconnect(None));
    let _ = tokio::time::timeout(Duration::from_millis(100), writer_handle).await;
    Ok(())
}

async fn handle_status_phase(
    conn: &mut PacketStream,
    shared: &ServerShared,
) -> Result<(), NetworkError> {
    let mut status_requested = false;
    while let Some(frame) = conn.read_frame_timeout(PRE_AUTH_SOCKET_TIMEOUT).await? {
        match frame.packet_id {
            StatusRequestPacket::PACKET_ID => {
                if status_requested {
                    return Err(NetworkError::Closed);
                }
                let _ = decode_packet_exact::<StatusRequestPacket>(frame.payload)?;
                status_requested = true;
                let mut ev = ServerListPingEvent::new(shared.build_status());
                shared.events.call(&mut ev);
                if ev.is_cancelled() {
                    let _ = conn.shutdown().await;
                    return Ok(());
                }
                conn.send_packet(&StatusResponsePacket { status: ev.status })
                    .await?;
            }
            StatusPingRequestPacket::PACKET_ID => {
                let ping = decode_packet_exact::<StatusPingRequestPacket>(frame.payload)?;
                let mut ev = ClientPingServerEvent::new(ping.payload);
                shared.events.call(&mut ev);
                if !ev.is_cancelled() {
                    conn.send_packet(&StatusPongResponsePacket {
                        payload: ev.payload,
                    })
                    .await?;
                }
                let _ = conn.shutdown().await;
                return Ok(());
            }
            other => {
                return Err(NetworkError::Protocol(ProtocolError::UnknownPacketId {
                    state: ConnectionState::Status,
                    id: other,
                }));
            }
        }
    }
    Ok(())
}
