//! Dedicated 20 TPS game tick thread (`Ms-TickScheduler`) managing active Play sessions,
//! movement chunk streaming, chat, Brigadier commands, and `KeepAlive` tracking (ADR 0002).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bytes::Bytes;
use flume::{Receiver, Sender};
use minestom_command::{CommandManager, CommandSender, PlayerSenderInfo, SenderEffect};
use minestom_core::{ChunkPos, GameMode, PlayerId, Point, Pos, Vec3};
use minestom_event::{CancellableEvent, GlobalEventHandler};
use minestom_protocol::RawPacketFrame;
use minestom_protocol::chat_command::{
    ClientChatMessagePacket, ClientCommandChatPacket, ClientSignedCommandChatPacket,
    ClientTabCompletePacket, SystemChatPacket, TabCompletePacket,
};
use minestom_protocol::login::GameProfile;
use minestom_protocol::play::{
    ChangeGameStatePacket, ChunkBatchFinishedPacket, ChunkBatchStartPacket,
    ClientPlayKeepAlivePacket, ClientPlayerPositionAndRotationPacket, ClientPlayerPositionPacket,
    ClientPlayerRotationPacket, ClientTeleportConfirmPacket, GameStateReason, JoinGamePacket,
    PlayDisconnectPacket, PlayKeepAlivePacket, PlayerAbilitiesPacket, PlayerInfoAddEntry,
    PlayerInfoUpdatePacket, PlayerPositionAndLookPacket, PlayerSpawnInfo, ServerDifficultyPacket,
    SpawnPositionPacket, UpdateHealthPacket, UpdateViewPositionPacket, WorldPos,
};
use minestom_protocol::{decode_packet_exact, encode_packet_payload};
use minestom_text::{Component, NamedTextColor};
use minestom_world::InstanceContainer;
use rustc_hash::FxHashSet;
use slotmap::SlotMap;
use tracing::{debug, trace_span, warn};

use crate::ServerConfig;
use crate::event::{
    PlayerChatEvent, PlayerCommandEvent, PlayerDisconnectEvent, PlayerMoveEvent, PlayerSpawnEvent,
};

/// Outbound command sent from the tick thread to a player's Tokio writer task.
pub(crate) enum OutboundCommand {
    /// Send a pre-encoded `[VarInt packet_id][payload]` body.
    Packet(Bytes),
    /// Send a final packet (if any) and close the TCP connection.
    Disconnect(Option<Bytes>),
}

/// Message sent from async connection tasks into the dedicated 20 TPS tick loop.
pub(crate) enum TickMessage {
    /// A player completed Configuration and is ready to join Play.
    RegisterPlayer {
        entity_id: i32,
        profile: GameProfile,
        game_mode: GameMode,
        permission_level: u8,
        spawn_pos: Pos,
        view_distance: u8,
        instance: InstanceContainer,
        outbound_tx: Sender<OutboundCommand>,
        player_id_reply: Sender<PlayerId>,
    },
    /// Inbound Play packet received from `player_id`.
    InboundPacket {
        player_id: PlayerId,
        frame: RawPacketFrame,
    },
    /// Socket reader closed for `player_id`.
    PlayerDisconnected { player_id: PlayerId },
    /// Stop the tick thread cleanly.
    Shutdown,
}

struct ActivePlayer {
    entity_id: i32,
    profile: GameProfile,
    game_mode: GameMode,
    permission_level: u8,
    position: Pos,
    chunk_pos: ChunkPos,
    view_distance: u8,
    instance: InstanceContainer,
    loaded_chunks: FxHashSet<ChunkPos>,
    pending_teleport_id: Option<i32>,
    next_teleport_id: i32,
    outbound_tx: Sender<OutboundCommand>,
    last_keep_alive_sent: Instant,
    pending_keep_alive_id: Option<(i64, Instant)>,
}

impl ActivePlayer {
    fn send_encoded(&self, payload: Bytes) -> bool {
        self.outbound_tx
            .try_send(OutboundCommand::Packet(payload))
            .is_ok()
    }

    fn send_packet<P: minestom_protocol::PacketPayload>(&self, pkt: &P) -> bool {
        let Ok(bytes) = encode_packet_payload(pkt) else {
            return false;
        };
        self.send_encoded(bytes)
    }

    fn command_sender(&self, pid: PlayerId) -> CommandSender {
        CommandSender::player(PlayerSenderInfo {
            player_id: pid,
            uuid: self.profile.uuid,
            username: self.profile.username.clone(),
            position: self.position,
            game_mode: self.game_mode,
            permission_level: self.permission_level,
        })
    }

    fn abilities_flags_for(mode: GameMode) -> u8 {
        match mode {
            GameMode::Creative => 0x01 | 0x04 | 0x08,
            GameMode::Spectator => 0x01 | 0x02 | 0x04,
            GameMode::Survival | GameMode::Adventure => 0x00,
        }
    }

    fn stream_chunks(&mut self, targets: &[ChunkPos]) {
        let missing_for_client: Vec<ChunkPos> = targets
            .iter()
            .copied()
            .filter(|c| !self.loaded_chunks.contains(c))
            .collect();
        if missing_for_client.is_empty() {
            return;
        }
        let Ok(payloads) = self.instance.chunk_packet_payloads(&missing_for_client) else {
            return;
        };
        let _ = self.send_packet(&ChunkBatchStartPacket);
        let mut count = 0_i32;
        for (cpos, bytes) in missing_for_client.into_iter().zip(payloads) {
            if self.send_encoded(bytes) {
                self.loaded_chunks.insert(cpos);
                count += 1;
            }
        }
        let _ = self.send_packet(&ChunkBatchFinishedPacket { batch_size: count });
    }

    fn teleport_server_side(&mut self, target: Pos) {
        let tp_id = self.next_teleport_id;
        self.next_teleport_id = self.next_teleport_id.wrapping_add(1).max(1);
        self.pending_teleport_id = Some(tp_id);
        update_player_position(self, target);
        let _ = self.send_packet(&PlayerPositionAndLookPacket {
            teleport_id: tp_id,
            position: target.as_vec(),
            delta: Vec3::ZERO,
            yaw: target.yaw,
            pitch: target.pitch,
            flags: 0,
        });
    }
}

pub(crate) fn spawn_tick_thread(
    config: ServerConfig,
    events: GlobalEventHandler,
    command_manager: CommandManager,
    online_count: Arc<AtomicU32>,
    inbox_rx: Receiver<TickMessage>,
    stop_flag: Arc<AtomicBool>,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("Ms-TickScheduler".to_owned())
        .spawn(move || {
            run_tick_loop(
                &config,
                &events,
                &command_manager,
                &online_count,
                &inbox_rx,
                &stop_flag,
            );
        })
}

fn emit_disconnect(pid: PlayerId, removed: &ActivePlayer, events: &GlobalEventHandler) {
    let mut ev = PlayerDisconnectEvent {
        player_id: pid,
        username: removed.profile.username.clone(),
        instance_id: removed.instance.id(),
    };
    events.call(&mut ev);
}

fn run_tick_loop(
    config: &ServerConfig,
    events: &GlobalEventHandler,
    command_manager: &CommandManager,
    online_count: &AtomicU32,
    inbox_rx: &Receiver<TickMessage>,
    stop_flag: &AtomicBool,
) {
    let tick_duration = Duration::from_nanos(1_000_000_000 / u64::from(config.ticks_per_second));
    let keep_alive_interval = Duration::from_millis(config.keep_alive_interval_ms);
    let keep_alive_timeout = Duration::from_millis(config.keep_alive_timeout_ms);

    let mut players: SlotMap<PlayerId, ActivePlayer> = SlotMap::with_key();
    let mut tick_num = 0_u64;
    let mut next_tick_deadline = Instant::now() + tick_duration;

    while !stop_flag.load(Ordering::Acquire) {
        let _span = trace_span!("tick", tick_num).entered();
        let now = Instant::now();

        // 1. Drain cross-thread messages & inbound packets
        while let Ok(msg) = inbox_rx.try_recv() {
            match msg {
                TickMessage::RegisterPlayer {
                    entity_id,
                    profile,
                    game_mode,
                    permission_level,
                    spawn_pos,
                    view_distance,
                    instance,
                    outbound_tx,
                    player_id_reply,
                } => {
                    let effective_vd = view_distance.clamp(2, config.chunk_view_distance);
                    let center = spawn_pos.chunk_pos();
                    let pid = players.insert(ActivePlayer {
                        entity_id,
                        profile: profile.clone(),
                        game_mode,
                        permission_level,
                        position: spawn_pos,
                        chunk_pos: center,
                        view_distance: effective_vd,
                        instance: instance.clone(),
                        loaded_chunks: FxHashSet::default(),
                        pending_teleport_id: Some(1),
                        next_teleport_id: 2,
                        outbound_tx,
                        last_keep_alive_sent: now,
                        pending_keep_alive_id: None,
                    });
                    if player_id_reply.send(pid).is_err() {
                        players.remove(pid);
                        continue;
                    }
                    online_count.store(players.len() as u32, Ordering::Relaxed);

                    initialize_joining_player(pid, &mut players, config, events, command_manager);
                }
                TickMessage::InboundPacket { player_id, frame } => {
                    if !players.contains_key(player_id) {
                        continue;
                    }
                    if !handle_play_packet(player_id, &mut players, events, command_manager, frame)
                        && let Some(removed) = players.remove(player_id)
                    {
                        let _ = removed
                            .outbound_tx
                            .try_send(OutboundCommand::Disconnect(None));
                        online_count.store(players.len() as u32, Ordering::Relaxed);
                        emit_disconnect(player_id, &removed, events);
                    }
                }
                TickMessage::PlayerDisconnected { player_id } => {
                    if let Some(removed) = players.remove(player_id) {
                        online_count.store(players.len() as u32, Ordering::Relaxed);
                        emit_disconnect(player_id, &removed, events);
                    }
                }
                TickMessage::Shutdown => {
                    for (pid, p) in players.drain() {
                        let kick = encode_packet_payload(&PlayDisconnectPacket {
                            message: Component::text("Server closed").color(NamedTextColor::Red),
                        })
                        .ok();
                        let _ = p.outbound_tx.try_send(OutboundCommand::Disconnect(kick));
                        emit_disconnect(pid, &p, events);
                    }
                    online_count.store(0, Ordering::Relaxed);
                    return;
                }
            }
        }

        // 2. Per-tick KeepAlive check
        let mut timed_out = Vec::new();
        for (pid, player) in &mut players {
            if let Some((_, sent_at)) = player.pending_keep_alive_id {
                if now.duration_since(sent_at) >= keep_alive_timeout {
                    timed_out.push(pid);
                }
            } else if now.duration_since(player.last_keep_alive_sent) >= keep_alive_interval {
                let challenge = (tick_num as i64).wrapping_mul(0x517C_C1B7_2722_0A95).max(1);
                if player.send_packet(&PlayKeepAlivePacket { id: challenge }) {
                    player.last_keep_alive_sent = now;
                    player.pending_keep_alive_id = Some((challenge, now));
                } else {
                    timed_out.push(pid);
                }
            }
        }

        for pid in timed_out {
            if let Some(p) = players.remove(pid) {
                warn!(username = %p.profile.username, "player timed out on KeepAlive");
                let kick = encode_packet_payload(&PlayDisconnectPacket {
                    message: Component::translatable("disconnect.timeout"),
                })
                .ok();
                let _ = p.outbound_tx.try_send(OutboundCommand::Disconnect(kick));
                emit_disconnect(pid, &p, events);
            }
            online_count.store(players.len() as u32, Ordering::Relaxed);
        }

        tick_num = tick_num.wrapping_add(1);

        let after = Instant::now();
        if after < next_tick_deadline {
            let remaining = next_tick_deadline - after;
            thread::sleep(remaining);
            next_tick_deadline += tick_duration;
        } else {
            next_tick_deadline = after + tick_duration;
        }
    }
}

fn initialize_joining_player(
    pid: PlayerId,
    players: &mut SlotMap<PlayerId, ActivePlayer>,
    config: &ServerConfig,
    events: &GlobalEventHandler,
    command_manager: &CommandManager,
) {
    let all_entries: Vec<PlayerInfoAddEntry> = players
        .values()
        .map(|p| PlayerInfoAddEntry {
            uuid: p.profile.uuid,
            username: p.profile.username.clone(),
            properties: p.profile.properties.clone(),
            game_mode: p.game_mode,
            listed: true,
        })
        .collect();

    let Some(player) = players.get_mut(pid) else {
        return;
    };

    let dim = player.instance.dimension_type().clone();
    let spawn_pos = player.position;
    let center = player.chunk_pos;
    let vd = i32::from(player.view_distance);

    let join = JoinGamePacket {
        entity_id: player.entity_id,
        is_hardcore: false,
        worlds: vec![dim.key.clone()],
        max_players: 0,
        view_distance: vd,
        simulation_distance: vd,
        reduced_debug_info: false,
        enable_respawn_screen: true,
        do_limited_crafting: false,
        spawn_info: PlayerSpawnInfo {
            dimension_type: dim.protocol_id as i32,
            world: dim.key.clone(),
            hashed_seed: 0,
            game_mode: player.game_mode,
            previous_game_mode: None,
            is_debug: false,
            is_flat: true,
            death_location: None,
            portal_cooldown: 0,
            sea_level: 63,
        },
        online_mode: true,
        enforces_secure_chat: config.enforces_secure_chat,
    };

    let abilities_flags = ActivePlayer::abilities_flags_for(player.game_mode);

    let _ = player.send_packet(&join);
    let _ = player.send_packet(&ServerDifficultyPacket {
        difficulty: config.difficulty,
        locked: true,
    });
    let _ = player.send_packet(&SpawnPositionPacket {
        world_pos: WorldPos {
            dimension: dim.key,
            position: spawn_pos.to_block_vec(),
        },
        yaw: spawn_pos.yaw,
        pitch: spawn_pos.pitch,
    });
    let _ = player.send_packet(&PlayerInfoUpdatePacket {
        entries: all_entries,
    });
    let _ = player.send_packet(&PlayerAbilitiesPacket {
        flags: abilities_flags,
        ..PlayerAbilitiesPacket::default()
    });
    let sender_view = player.command_sender(pid);
    let declare_cmds = command_manager.build_declare_commands_packet(&sender_view);
    let _ = player.send_packet(&declare_cmds);

    let _ = player.send_packet(&UpdateHealthPacket {
        health: 20.0,
        food: 20,
        food_saturation: 5.0,
    });
    let _ = player.send_packet(&UpdateViewPositionPacket {
        chunk_x: center.x,
        chunk_z: center.z,
    });
    let _ = player.send_packet(&PlayerPositionAndLookPacket {
        teleport_id: 1,
        position: spawn_pos.as_vec(),
        delta: Vec3::ZERO,
        yaw: spawn_pos.yaw,
        pitch: spawn_pos.pitch,
        flags: 0,
    });
    let _ = player.send_packet(&ChangeGameStatePacket {
        reason: GameStateReason::LevelChunksLoadStart,
        value: 0.0,
    });

    let initial_chunks = center.chunks_in_range(player.view_distance);
    player.stream_chunks(&initial_chunks);

    let mut spawn_ev = PlayerSpawnEvent {
        player_id: pid,
        username: player.profile.username.clone(),
        position: spawn_pos,
        instance: player.instance.clone(),
        first_spawn: true,
    };
    events.call(&mut spawn_ev);
}

fn update_player_position(player: &mut ActivePlayer, new_pos: Pos) {
    player.position = new_pos;
    let new_chunk = new_pos.chunk_pos();
    if new_chunk != player.chunk_pos {
        player.chunk_pos = new_chunk;
        let _ = player.send_packet(&UpdateViewPositionPacket {
            chunk_x: new_chunk.x,
            chunk_z: new_chunk.z,
        });
        let targets = new_chunk.chunks_in_range(player.view_distance);
        player.stream_chunks(&targets);
    }
}

fn process_movement(
    pid: PlayerId,
    players: &mut SlotMap<PlayerId, ActivePlayer>,
    events: &GlobalEventHandler,
    proposed: Pos,
    on_ground: bool,
) {
    let Some(player) = players.get(pid) else {
        return;
    };
    if player.pending_teleport_id.is_some() {
        return;
    }
    let old_pos = player.position;
    if old_pos == proposed {
        return;
    }
    let mut ev = PlayerMoveEvent::new(
        pid,
        player.profile.username.clone(),
        player.instance.id(),
        old_pos,
        proposed,
        on_ground,
    );
    events.call(&mut ev);

    let Some(player) = players.get_mut(pid) else {
        return;
    };
    if ev.is_cancelled() {
        player.teleport_server_side(old_pos);
    } else if ev.new_position != proposed {
        player.teleport_server_side(ev.new_position);
    } else {
        update_player_position(player, ev.new_position);
    }
}

fn dispatch_player_command(
    pid: PlayerId,
    players: &mut SlotMap<PlayerId, ActivePlayer>,
    events: &GlobalEventHandler,
    command_manager: &CommandManager,
    raw_command: String,
) {
    let Some(username) = players.get(pid).map(|p| p.profile.username.clone()) else {
        return;
    };
    let mut cmd_ev = PlayerCommandEvent::new(pid, username, raw_command);
    events.call(&mut cmd_ev);
    if cmd_ev.is_cancelled() {
        return;
    }

    let Some(mut sender) = players.get(pid).map(|p| p.command_sender(pid)) else {
        return;
    };
    let result = command_manager.execute(&mut sender, &cmd_ev.command);
    let replies = sender.take_messages();
    let effects = sender.take_effects();

    if let Err(err) = result
        && replies.is_empty()
        && let Some(p) = players.get(pid)
    {
        let _ = p.send_packet(&SystemChatPacket {
            message: Component::text(err.to_string()).color(NamedTextColor::Red),
            overlay: false,
        });
    }

    if let Some(p) = players.get(pid) {
        for msg in replies {
            let _ = p.send_packet(&SystemChatPacket {
                message: msg,
                overlay: false,
            });
        }
    }

    let mut pending_broadcasts: Vec<Component> = Vec::new();
    let mut kick_reason: Option<Component> = None;

    if let Some(p) = players.get_mut(pid) {
        for effect in effects {
            match effect {
                SenderEffect::SetGameMode(mode) => {
                    p.game_mode = mode;
                    let _ = p.send_packet(&ChangeGameStatePacket {
                        reason: GameStateReason::ChangeGameMode,
                        value: f32::from(mode as u8),
                    });
                    let _ = p.send_packet(&PlayerAbilitiesPacket {
                        flags: ActivePlayer::abilities_flags_for(mode),
                        ..PlayerAbilitiesPacket::default()
                    });
                }
                SenderEffect::Teleport(pos) => {
                    p.teleport_server_side(pos);
                }
                SenderEffect::Broadcast(msg) => pending_broadcasts.push(msg),
                SenderEffect::Kick(reason) => kick_reason = Some(reason),
            }
        }
    }

    for bcast in pending_broadcasts {
        if let Ok(encoded) = encode_packet_payload(&SystemChatPacket {
            message: bcast,
            overlay: false,
        }) {
            for target in players.values() {
                let _ = target.send_encoded(encoded.clone());
            }
        }
    }

    if let Some(reason) = kick_reason
        && let Some(removed) = players.remove(pid)
    {
        let kick = encode_packet_payload(&PlayDisconnectPacket { message: reason }).ok();
        let _ = removed
            .outbound_tx
            .try_send(OutboundCommand::Disconnect(kick));
        emit_disconnect(pid, &removed, events);
    }
}

fn handle_play_packet(
    pid: PlayerId,
    players: &mut SlotMap<PlayerId, ActivePlayer>,
    events: &GlobalEventHandler,
    command_manager: &CommandManager,
    frame: RawPacketFrame,
) -> bool {
    match frame.packet_id {
        0x00 => {
            let Ok(pkt) = decode_packet_exact::<ClientTeleportConfirmPacket>(frame.payload) else {
                return false;
            };
            if let Some(player) = players.get_mut(pid)
                && player.pending_teleport_id == Some(pkt.teleport_id)
            {
                player.pending_teleport_id = None;
            }
            true
        }
        0x07 => {
            let Ok(pkt) = decode_packet_exact::<ClientCommandChatPacket>(frame.payload) else {
                return false;
            };
            dispatch_player_command(pid, players, events, command_manager, pkt.message);
            true
        }
        0x08 => {
            let Ok(pkt) = decode_packet_exact::<ClientSignedCommandChatPacket>(frame.payload)
            else {
                return false;
            };
            dispatch_player_command(pid, players, events, command_manager, pkt.message);
            true
        }
        0x09 => {
            let Ok(pkt) = decode_packet_exact::<ClientChatMessagePacket>(frame.payload) else {
                return false;
            };
            let Some(username) = players.get(pid).map(|p| p.profile.username.clone()) else {
                return true;
            };
            let mut chat_ev = PlayerChatEvent::new(pid, username, pkt.message);
            events.call(&mut chat_ev);
            if !chat_ev.is_cancelled() {
                let component = chat_ev.build_component();
                if let Ok(encoded) = encode_packet_payload(&SystemChatPacket {
                    message: component,
                    overlay: false,
                }) {
                    if let Some(recipients) = &chat_ev.recipients {
                        for &target_id in recipients {
                            if let Some(target) = players.get(target_id) {
                                let _ = target.send_encoded(encoded.clone());
                            }
                        }
                    } else {
                        for target in players.values() {
                            let _ = target.send_encoded(encoded.clone());
                        }
                    }
                }
            }
            true
        }
        0x0F => {
            let Ok(pkt) = decode_packet_exact::<ClientTabCompletePacket>(frame.payload) else {
                return false;
            };
            if let Some(player) = players.get(pid) {
                let sender = player.command_sender(pid);
                let res = command_manager.complete(&sender, &pkt.text);
                let _ = player.send_packet(&TabCompletePacket {
                    transaction_id: pkt.transaction_id,
                    start: res.start,
                    length: res.length,
                    matches: res.matches,
                });
            }
            true
        }
        0x1C => {
            let Ok(pkt) = decode_packet_exact::<ClientPlayKeepAlivePacket>(frame.payload) else {
                return false;
            };
            let Some(player) = players.get_mut(pid) else {
                return true;
            };
            if let Some((expected, _)) = player.pending_keep_alive_id
                && pkt.id == expected
            {
                player.pending_keep_alive_id = None;
                true
            } else {
                debug!(username = %player.profile.username, "mismatched KeepAlive ID");
                false
            }
        }
        0x1E => {
            let Ok(pkt) = decode_packet_exact::<ClientPlayerPositionPacket>(frame.payload) else {
                return false;
            };
            if let Some(cur) = players.get(pid).map(|p| p.position) {
                let proposed = cur
                    .with_x(pkt.position.x)
                    .with_y(pkt.position.y)
                    .with_z(pkt.position.z);
                process_movement(pid, players, events, proposed, (pkt.flags & 0x01) != 0);
            }
            true
        }
        0x1F => {
            let Ok(pkt) =
                decode_packet_exact::<ClientPlayerPositionAndRotationPacket>(frame.payload)
            else {
                return false;
            };
            process_movement(pid, players, events, pkt.position, (pkt.flags & 0x01) != 0);
            true
        }
        0x20 => {
            let Ok(pkt) = decode_packet_exact::<ClientPlayerRotationPacket>(frame.payload) else {
                return false;
            };
            if let Some(cur) = players.get(pid).map(|p| p.position) {
                let proposed = cur.with_yaw(pkt.yaw).with_pitch(pkt.pitch);
                process_movement(pid, players, events, proposed, (pkt.flags & 0x01) != 0);
            }
            true
        }
        _ => true,
    }
}
