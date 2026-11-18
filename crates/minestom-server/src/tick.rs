//! Dedicated 20 TPS game tick thread (`Ms-TickScheduler`) managing active Play sessions,
//! movement chunk streaming, and `KeepAlive` tracking (ADR 0002).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bytes::Bytes;
use flume::{Receiver, Sender};
use minestom_core::{ChunkPos, GameMode, PlayerId, Point, Pos, Vec3};
use minestom_event::GlobalEventHandler;
use minestom_protocol::RawPacketFrame;
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
use crate::event::PlayerSpawnEvent;

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
}

pub(crate) fn spawn_tick_thread(
    config: ServerConfig,
    events: GlobalEventHandler,
    online_count: Arc<AtomicU32>,
    inbox_rx: Receiver<TickMessage>,
    stop_flag: Arc<AtomicBool>,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("Ms-TickScheduler".to_owned())
        .spawn(move || {
            run_tick_loop(&config, &events, &online_count, &inbox_rx, &stop_flag);
        })
}

fn run_tick_loop(
    config: &ServerConfig,
    events: &GlobalEventHandler,
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

                    initialize_joining_player(pid, &mut players, config, events);
                }
                TickMessage::InboundPacket { player_id, frame } => {
                    if let Some(player) = players.get_mut(player_id)
                        && !handle_play_packet(player, frame)
                    {
                        let _ = player
                            .outbound_tx
                            .try_send(OutboundCommand::Disconnect(None));
                        players.remove(player_id);
                        online_count.store(players.len() as u32, Ordering::Relaxed);
                    }
                }
                TickMessage::PlayerDisconnected { player_id } => {
                    if players.remove(player_id).is_some() {
                        online_count.store(players.len() as u32, Ordering::Relaxed);
                    }
                }
                TickMessage::Shutdown => {
                    for (_, p) in players.drain() {
                        let kick = encode_packet_payload(&PlayDisconnectPacket {
                            message: Component::text("Server closed").color(NamedTextColor::Red),
                        })
                        .ok();
                        let _ = p.outbound_tx.try_send(OutboundCommand::Disconnect(kick));
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
            }
            online_count.store(players.len() as u32, Ordering::Relaxed);
        }

        tick_num = tick_num.wrapping_add(1);

        let after = Instant::now();
        if after < next_tick_deadline {
            let remaining = next_tick_deadline - after;
            // Wake early if Shutdown arrives, or sleep remaining sub-50ms tick interval
            thread::sleep(remaining);
            next_tick_deadline += tick_duration;
        } else {
            // Prevent spiral of death if tick ever ran behind
            next_tick_deadline = after + tick_duration;
        }
    }
}

fn initialize_joining_player(
    pid: PlayerId,
    players: &mut SlotMap<PlayerId, ActivePlayer>,
    config: &ServerConfig,
    events: &GlobalEventHandler,
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

    let abilities_flags = match player.game_mode {
        GameMode::Creative => 0x01 | 0x04 | 0x08,
        GameMode::Spectator => 0x01 | 0x02 | 0x04,
        GameMode::Survival | GameMode::Adventure => 0x00,
    };

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

fn handle_play_packet(player: &mut ActivePlayer, frame: RawPacketFrame) -> bool {
    match frame.packet_id {
        0x00 => {
            let Ok(pkt) = decode_packet_exact::<ClientTeleportConfirmPacket>(frame.payload) else {
                return false;
            };
            if player.pending_teleport_id == Some(pkt.teleport_id) {
                player.pending_teleport_id = None;
            }
            let _ = player.next_teleport_id;
            true
        }
        0x1C => {
            let Ok(pkt) = decode_packet_exact::<ClientPlayKeepAlivePacket>(frame.payload) else {
                return false;
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
            if player.pending_teleport_id.is_none() {
                let updated = player
                    .position
                    .with_x(pkt.position.x)
                    .with_y(pkt.position.y)
                    .with_z(pkt.position.z);
                update_player_position(player, updated);
            }
            true
        }
        0x1F => {
            let Ok(pkt) =
                decode_packet_exact::<ClientPlayerPositionAndRotationPacket>(frame.payload)
            else {
                return false;
            };
            if player.pending_teleport_id.is_none() {
                update_player_position(player, pkt.position);
            }
            true
        }
        0x20 => {
            let Ok(pkt) = decode_packet_exact::<ClientPlayerRotationPacket>(frame.payload) else {
                return false;
            };
            if player.pending_teleport_id.is_none() {
                player.position = player.position.with_yaw(pkt.yaw).with_pitch(pkt.pitch);
            }
            true
        }
        // Benign Play packets ignored gracefully in M2 (ChunkBatchReceived, TickEnd, Settings, PluginMessage, PositionStatus, Abilities, PlayerLoaded, etc.)
        _ => true,
    }
}
