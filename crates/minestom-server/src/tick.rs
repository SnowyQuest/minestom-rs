//! Dedicated 20 TPS game tick thread (`Ms-TickScheduler`) managing active Play sessions,
//! spatial `EntityTracker` multi-player & world-entity synchronization, 20 TPS entity physics,
//! movement chunk streaming, chat, Brigadier commands, and `KeepAlive` tracking (ADR 0002).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bytes::Bytes;
use flume::{Receiver, Sender};
use minestom_command::{CommandManager, CommandSender, PlayerSenderInfo, SenderEffect};
use minestom_core::{ChunkPos, GameMode, Hand, InstanceId, MainHand, PlayerId, Point, Pos, Vec3};
use minestom_entity::{
    Entity, EntityDespawnEvent, EntityMetadata, EntitySpawnEvent, EntityTracker, EntityType,
    PlayerAttackEntityEvent, PlayerEntityInteractEvent, PlayerHandAnimationEvent,
};
use minestom_event::{CancellableEvent, GlobalEventHandler};
use minestom_protocol::RawPacketFrame;
use minestom_protocol::chat_command::{
    ClientChatMessagePacket, ClientCommandChatPacket, ClientSignedCommandChatPacket,
    ClientTabCompletePacket, SystemChatPacket, TabCompletePacket,
};
use minestom_protocol::config::ClientSettingsPacket;
use minestom_protocol::entity::{
    ClientAnimationPacket, ClientAttackPacket, ClientEntityAction, ClientEntityActionPacket,
    ClientInputPacket, ClientInteractEntityPacket, DestroyEntitiesPacket, EntityAnimation,
    EntityAnimationPacket, EntityHeadLookPacket, EntityMetaDataPacket, EntityPose,
    EntityPositionAndRotationPacket, EntityPositionPacket, EntityPositionSyncPacket,
    EntityRotationPacket, EntityVelocityPacket, PlayerInfoRemovePacket, SpawnEntityPacket,
    delta_short_4096,
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
use rustc_hash::{FxHashMap, FxHashSet};
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

/// Cross-thread command mutating non-player world entities inside the 20 TPS tick loop.
pub(crate) enum EntityCommand {
    /// Spawn a new world entity.
    Spawn {
        network_id: i32,
        instance: InstanceContainer,
        entity: Box<Entity>,
        position: Pos,
    },
    /// Remove an existing world entity.
    Remove { network_id: i32 },
    /// Teleport an existing world entity.
    Teleport { network_id: i32, position: Pos },
    /// Set velocity on an existing world entity.
    SetVelocity { network_id: i32, velocity: Vec3 },
}

/// Message sent from async connection tasks into the dedicated 20 TPS tick loop.
pub(crate) enum TickMessage {
    /// A player completed Configuration and is ready to join Play.
    RegisterPlayer {
        entity_id: i32,
        profile: GameProfile,
        game_mode: GameMode,
        permission_level: u8,
        main_hand: MainHand,
        displayed_skin_parts: u8,
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
    last_synced_pos: Pos,
    on_ground: bool,
    metadata: EntityMetadata,
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
}

struct SpawnedWorldEntity {
    entity: Entity,
    instance: InstanceContainer,
    last_synced_pos: Pos,
    last_synced_head_yaw: f32,
    velocity_dirty: bool,
}

struct WorldSimState {
    players: SlotMap<PlayerId, ActivePlayer>,
    player_by_entity_id: FxHashMap<i32, PlayerId>,
    entities: FxHashMap<i32, SpawnedWorldEntity>,
    tracker: EntityTracker,
}

impl WorldSimState {
    fn new() -> Self {
        Self {
            players: SlotMap::with_key(),
            player_by_entity_id: FxHashMap::default(),
            entities: FxHashMap::default(),
            tracker: EntityTracker::new(),
        }
    }

    fn send_to_viewers<P: minestom_protocol::PacketPayload>(&self, entity_id: i32, pkt: &P) {
        let Some(viewers) = self.tracker.viewers(entity_id) else {
            return;
        };
        if viewers.is_empty() {
            return;
        }
        let Ok(bytes) = encode_packet_payload(pkt) else {
            return;
        };
        for &pid in viewers {
            if let Some(p) = self.players.get(pid) {
                let _ = p.send_encoded(bytes.clone());
            }
        }
    }

    fn send_entity_spawn_bundle(&self, viewer_pid: PlayerId, target_entity_id: i32) {
        let Some(viewer) = self.players.get(viewer_pid) else {
            return;
        };
        if let Some(&target_pid) = self.player_by_entity_id.get(&target_entity_id) {
            if let Some(target_player) = self.players.get(target_pid) {
                let _ = viewer.send_packet(&SpawnEntityPacket {
                    entity_id: target_player.entity_id,
                    uuid: target_player.profile.uuid,
                    entity_type: EntityType::PLAYER.id() as i32,
                    position: target_player.position,
                    head_rot: target_player.position.yaw,
                    data: 0,
                    velocity: Vec3::ZERO,
                });
                let _ = viewer.send_packet(&EntityHeadLookPacket {
                    entity_id: target_player.entity_id,
                    yaw: target_player.position.yaw,
                });
                let meta = target_player.metadata.all_entries();
                if !meta.is_empty() {
                    let _ = viewer.send_packet(&EntityMetaDataPacket {
                        entity_id: target_player.entity_id,
                        entries: meta,
                    });
                }
            }
        } else if let Some(we) = self.entities.get(&target_entity_id) {
            let core = &we.entity.core;
            let _ = viewer.send_packet(&SpawnEntityPacket {
                entity_id: core.network_id,
                uuid: core.uuid,
                entity_type: core.entity_type.id() as i32,
                position: core.position,
                head_rot: core.head_yaw,
                data: core.object_data,
                velocity: core.velocity,
            });
            let _ = viewer.send_packet(&EntityHeadLookPacket {
                entity_id: core.network_id,
                yaw: core.head_yaw,
            });
            let meta = core.metadata.all_entries();
            if !meta.is_empty() {
                let _ = viewer.send_packet(&EntityMetaDataPacket {
                    entity_id: core.network_id,
                    entries: meta,
                });
            }
        }
    }

    fn reconcile_single_player(&mut self, pid: PlayerId) {
        let Some((self_eid, inst_id, cpos, vd)) = self
            .players
            .get(pid)
            .map(|p| (p.entity_id, p.instance.id(), p.chunk_pos, p.view_distance))
        else {
            return;
        };
        let diff = self
            .tracker
            .reconcile_viewer(pid, self_eid, inst_id, cpos, vd);
        if !diff.exited.is_empty()
            && let Some(viewer) = self.players.get(pid)
        {
            let _ = viewer.send_packet(&DestroyEntitiesPacket {
                entity_ids: diff.exited,
            });
        }
        for entered_eid in diff.entered {
            self.send_entity_spawn_bundle(pid, entered_eid);
        }
    }

    fn reconcile_instance_players(&mut self, instance_id: InstanceId) {
        let pids: Vec<PlayerId> = self
            .players
            .iter()
            .filter_map(|(pid, p)| (p.instance.id() == instance_id).then_some(pid))
            .collect();
        for pid in pids {
            self.reconcile_single_player(pid);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn broadcast_entity_movement(
        &self,
        entity_id: i32,
        old_pos: Pos,
        new_pos: Pos,
        head_yaw: f32,
        old_head_yaw: f32,
        delta_vel: Vec3,
        on_ground: bool,
    ) -> Pos {
        let dx = (new_pos.x - old_pos.x).abs();
        let dy = (new_pos.y - old_pos.y).abs();
        let dz = (new_pos.z - old_pos.z).abs();
        let pos_changed = dx > 1.0e-6 || dy > 1.0e-6 || dz > 1.0e-6;
        let rot_changed = (new_pos.yaw - old_pos.yaw).abs() > 1.0e-3
            || (new_pos.pitch - old_pos.pitch).abs() > 1.0e-3;
        let head_changed = (head_yaw - old_head_yaw).abs() > 1.0e-3;

        if dx >= 7.5 || dy >= 7.5 || dz >= 7.5 {
            self.send_to_viewers(
                entity_id,
                &EntityPositionSyncPacket {
                    entity_id,
                    position: new_pos.as_vec(),
                    delta: delta_vel,
                    yaw: new_pos.yaw,
                    pitch: new_pos.pitch,
                    on_ground,
                },
            );
            self.send_to_viewers(
                entity_id,
                &EntityHeadLookPacket {
                    entity_id,
                    yaw: head_yaw,
                },
            );
            new_pos
        } else if pos_changed && rot_changed {
            let sx = delta_short_4096(new_pos.x, old_pos.x);
            let sy = delta_short_4096(new_pos.y, old_pos.y);
            let sz = delta_short_4096(new_pos.z, old_pos.z);
            self.send_to_viewers(
                entity_id,
                &EntityPositionAndRotationPacket {
                    entity_id,
                    delta_x: sx,
                    delta_y: sy,
                    delta_z: sz,
                    yaw: new_pos.yaw,
                    pitch: new_pos.pitch,
                    on_ground,
                },
            );
            self.send_to_viewers(
                entity_id,
                &EntityHeadLookPacket {
                    entity_id,
                    yaw: head_yaw,
                },
            );
            Pos::with_look(
                old_pos.x + f64::from(sx) / 4096.0,
                old_pos.y + f64::from(sy) / 4096.0,
                old_pos.z + f64::from(sz) / 4096.0,
                new_pos.yaw,
                new_pos.pitch,
            )
        } else if pos_changed {
            let sx = delta_short_4096(new_pos.x, old_pos.x);
            let sy = delta_short_4096(new_pos.y, old_pos.y);
            let sz = delta_short_4096(new_pos.z, old_pos.z);
            self.send_to_viewers(
                entity_id,
                &EntityPositionPacket {
                    entity_id,
                    delta_x: sx,
                    delta_y: sy,
                    delta_z: sz,
                    on_ground,
                },
            );
            if head_changed {
                self.send_to_viewers(
                    entity_id,
                    &EntityHeadLookPacket {
                        entity_id,
                        yaw: head_yaw,
                    },
                );
            }
            Pos::with_look(
                old_pos.x + f64::from(sx) / 4096.0,
                old_pos.y + f64::from(sy) / 4096.0,
                old_pos.z + f64::from(sz) / 4096.0,
                old_pos.yaw,
                old_pos.pitch,
            )
        } else if rot_changed {
            self.send_to_viewers(
                entity_id,
                &EntityRotationPacket {
                    entity_id,
                    yaw: new_pos.yaw,
                    pitch: new_pos.pitch,
                    on_ground,
                },
            );
            self.send_to_viewers(
                entity_id,
                &EntityHeadLookPacket {
                    entity_id,
                    yaw: head_yaw,
                },
            );
            old_pos.with_yaw(new_pos.yaw).with_pitch(new_pos.pitch)
        } else {
            if head_changed {
                self.send_to_viewers(
                    entity_id,
                    &EntityHeadLookPacket {
                        entity_id,
                        yaw: head_yaw,
                    },
                );
            }
            old_pos
        }
    }

    fn remove_and_cleanup_player(&mut self, pid: PlayerId, events: &GlobalEventHandler) {
        let Some(removed) = self.players.remove(pid) else {
            return;
        };
        self.player_by_entity_id.remove(&removed.entity_id);
        self.tracker.remove_viewer(pid);
        let former_viewers = self.tracker.unregister(removed.entity_id);

        if !former_viewers.is_empty()
            && let Ok(destroy_bytes) = encode_packet_payload(&DestroyEntitiesPacket {
                entity_ids: vec![removed.entity_id],
            })
        {
            for vpid in former_viewers {
                if let Some(vp) = self.players.get(vpid) {
                    let _ = vp.send_encoded(destroy_bytes.clone());
                }
            }
        }

        if let Ok(info_rm_bytes) = encode_packet_payload(&PlayerInfoRemovePacket {
            uuids: vec![removed.profile.uuid],
        }) {
            for p in self.players.values() {
                let _ = p.send_encoded(info_rm_bytes.clone());
            }
        }

        let mut ev = PlayerDisconnectEvent {
            player_id: pid,
            username: removed.profile.username,
            instance_id: removed.instance.id(),
        };
        events.call(&mut ev);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_tick_thread(
    config: ServerConfig,
    events: GlobalEventHandler,
    command_manager: CommandManager,
    online_count: Arc<AtomicU32>,
    inbox_rx: Receiver<TickMessage>,
    entity_cmd_rx: Receiver<EntityCommand>,
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
                &entity_cmd_rx,
                &stop_flag,
            );
        })
}

fn run_tick_loop(
    config: &ServerConfig,
    events: &GlobalEventHandler,
    command_manager: &CommandManager,
    online_count: &AtomicU32,
    inbox_rx: &Receiver<TickMessage>,
    entity_cmd_rx: &Receiver<EntityCommand>,
    stop_flag: &AtomicBool,
) {
    let tick_duration = Duration::from_nanos(1_000_000_000 / u64::from(config.ticks_per_second));
    let keep_alive_interval = Duration::from_millis(config.keep_alive_interval_ms);
    let keep_alive_timeout = Duration::from_millis(config.keep_alive_timeout_ms);

    let mut sim = WorldSimState::new();
    let mut tick_num = 0_u64;
    let mut next_tick_deadline = Instant::now() + tick_duration;

    while !stop_flag.load(Ordering::Acquire) {
        let _span = trace_span!("tick", tick_num).entered();
        let now = Instant::now();

        // 1. Drain queued world EntityCommands (Spawn / Remove / Teleport / SetVelocity)
        drain_entity_commands(&mut sim, entity_cmd_rx, events);

        // 2. Drain cross-thread player messages & inbound packets
        while let Ok(msg) = inbox_rx.try_recv() {
            match msg {
                TickMessage::RegisterPlayer {
                    entity_id,
                    profile,
                    game_mode,
                    permission_level,
                    main_hand,
                    displayed_skin_parts,
                    spawn_pos,
                    view_distance,
                    instance,
                    outbound_tx,
                    player_id_reply,
                } => {
                    let effective_vd = view_distance.clamp(2, config.chunk_view_distance);
                    let center = spawn_pos.chunk_pos();
                    let mut metadata = EntityMetadata::new();
                    metadata.set_main_hand(main_hand);
                    metadata.set_displayed_skin_parts(displayed_skin_parts);
                    metadata.clear_dirty();

                    let pid = sim.players.insert(ActivePlayer {
                        entity_id,
                        profile: profile.clone(),
                        game_mode,
                        permission_level,
                        position: spawn_pos,
                        last_synced_pos: spawn_pos,
                        on_ground: true,
                        metadata,
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
                        sim.players.remove(pid);
                        continue;
                    }
                    sim.player_by_entity_id.insert(entity_id, pid);
                    online_count.store(sim.players.len() as u32, Ordering::Relaxed);

                    initialize_joining_player(pid, &mut sim, config, events, command_manager);
                }
                TickMessage::InboundPacket { player_id, frame } => {
                    if !sim.players.contains_key(player_id) {
                        continue;
                    }
                    if !handle_play_packet(player_id, &mut sim, events, command_manager, frame) {
                        if let Some(p) = sim.players.get(player_id) {
                            let _ = p.outbound_tx.try_send(OutboundCommand::Disconnect(None));
                        }
                        sim.remove_and_cleanup_player(player_id, events);
                    }
                    online_count.store(sim.players.len() as u32, Ordering::Relaxed);
                }
                TickMessage::PlayerDisconnected { player_id } => {
                    sim.remove_and_cleanup_player(player_id, events);
                    online_count.store(sim.players.len() as u32, Ordering::Relaxed);
                }
                TickMessage::Shutdown => {
                    let pids: Vec<PlayerId> = sim.players.keys().collect();
                    for pid in pids {
                        if let Some(p) = sim.players.get(pid) {
                            let kick = encode_packet_payload(&PlayDisconnectPacket {
                                message: Component::text("Server closed")
                                    .color(NamedTextColor::Red),
                            })
                            .ok();
                            let _ = p.outbound_tx.try_send(OutboundCommand::Disconnect(kick));
                        }
                        sim.remove_and_cleanup_player(pid, events);
                    }
                    online_count.store(0, Ordering::Relaxed);
                    return;
                }
            }
        }

        // Drain any EntityCommands emitted inside event/command handlers this tick
        drain_entity_commands(&mut sim, entity_cmd_rx, events);

        // 3. Step 20 TPS World Entities (Behavior + Gravity/Collision + Viewer Delta Sync)
        tick_world_entities(&mut sim, events);

        // 4. Per-tick KeepAlive check
        let mut timed_out = Vec::new();
        for (pid, player) in &mut sim.players {
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
            if let Some(p) = sim.players.get(pid) {
                warn!(username = %p.profile.username, "player timed out on KeepAlive");
                let kick = encode_packet_payload(&PlayDisconnectPacket {
                    message: Component::translatable("disconnect.timeout"),
                })
                .ok();
                let _ = p.outbound_tx.try_send(OutboundCommand::Disconnect(kick));
            }
            sim.remove_and_cleanup_player(pid, events);
            online_count.store(sim.players.len() as u32, Ordering::Relaxed);
        }

        tick_num = tick_num.wrapping_add(1);

        let after = Instant::now();
        if after < next_tick_deadline {
            thread::sleep(next_tick_deadline - after);
            next_tick_deadline += tick_duration;
        } else {
            next_tick_deadline = after + tick_duration;
        }
    }
}

fn drain_entity_commands(
    sim: &mut WorldSimState,
    entity_cmd_rx: &Receiver<EntityCommand>,
    events: &GlobalEventHandler,
) {
    while let Ok(cmd) = entity_cmd_rx.try_recv() {
        match cmd {
            EntityCommand::Spawn {
                network_id,
                instance,
                mut entity,
                position,
            } => {
                entity.core.network_id = network_id;
                entity.core.position = position;
                entity.core.head_yaw = position.yaw;
                let _ = instance.chunk_packet_payloads(&[position.chunk_pos()]);
                entity.notify_spawn();
                entity.core.metadata.clear_dirty();

                let inst_id = instance.id();
                let etype = entity.entity_type();
                sim.tracker.register(network_id, inst_id, position);
                sim.entities.insert(
                    network_id,
                    SpawnedWorldEntity {
                        entity: *entity,
                        instance,
                        last_synced_pos: position,
                        last_synced_head_yaw: position.yaw,
                        velocity_dirty: false,
                    },
                );

                let mut ev = EntitySpawnEvent {
                    network_id,
                    entity_type: etype,
                    instance_id: inst_id,
                    position,
                };
                events.call(&mut ev);
                sim.reconcile_instance_players(inst_id);
            }
            EntityCommand::Remove { network_id } => {
                despawn_world_entity(sim, network_id, events);
            }
            EntityCommand::Teleport {
                network_id,
                position,
            } => {
                let mut changed_inst = None;
                if let Some(we) = sim.entities.get_mut(&network_id) {
                    we.entity.core.position = position;
                    we.entity.core.head_yaw = position.yaw;
                    we.last_synced_pos = position;
                    we.last_synced_head_yaw = position.yaw;
                    changed_inst = Some((we.instance.id(), we.entity.core.on_ground));
                }
                if let Some((inst_id, on_ground)) = changed_inst {
                    let crossed = sim.tracker.update_position(network_id, inst_id, position);
                    if crossed {
                        sim.reconcile_instance_players(inst_id);
                    }
                    sim.send_to_viewers(
                        network_id,
                        &EntityPositionSyncPacket {
                            entity_id: network_id,
                            position: position.as_vec(),
                            delta: Vec3::ZERO,
                            yaw: position.yaw,
                            pitch: position.pitch,
                            on_ground,
                        },
                    );
                    sim.send_to_viewers(
                        network_id,
                        &EntityHeadLookPacket {
                            entity_id: network_id,
                            yaw: position.yaw,
                        },
                    );
                }
            }
            EntityCommand::SetVelocity {
                network_id,
                velocity,
            } => {
                if let Some(we) = sim.entities.get_mut(&network_id) {
                    we.entity.core.velocity = velocity;
                    we.velocity_dirty = true;
                }
            }
        }
    }
}

fn despawn_world_entity(sim: &mut WorldSimState, network_id: i32, events: &GlobalEventHandler) {
    let Some(removed) = sim.entities.remove(&network_id) else {
        return;
    };
    let viewers = sim.tracker.unregister(network_id);
    if !viewers.is_empty()
        && let Ok(bytes) = encode_packet_payload(&DestroyEntitiesPacket {
            entity_ids: vec![network_id],
        })
    {
        for pid in viewers {
            if let Some(p) = sim.players.get(pid) {
                let _ = p.send_encoded(bytes.clone());
            }
        }
    }
    let mut ev = EntityDespawnEvent {
        network_id,
        entity_type: removed.entity.entity_type(),
        instance_id: removed.instance.id(),
    };
    events.call(&mut ev);
}

fn tick_world_entities(sim: &mut WorldSimState, events: &GlobalEventHandler) {
    let eids: Vec<i32> = sim.entities.keys().copied().collect();
    let mut to_remove = Vec::new();
    let mut instances_needing_reconcile = FxHashSet::default();

    for eid in eids {
        let mut sync_step = None;
        let mut dirty_meta = None;
        let mut send_vel = None;

        if let Some(we) = sim.entities.get_mut(&eid) {
            let _ = we
                .instance
                .chunk_packet_payloads(&[we.entity.core.position.chunk_pos()]);
            we.entity.tick(&we.instance);

            if we.entity.core.should_remove {
                to_remove.push(eid);
                continue;
            }

            let new_pos = we.entity.core.position;
            let head_yaw = we.entity.core.head_yaw;
            let vel = we.entity.core.velocity;
            let on_ground = we.entity.core.on_ground;
            let inst_id = we.instance.id();

            sync_step = Some((
                inst_id,
                we.last_synced_pos,
                new_pos,
                head_yaw,
                we.last_synced_head_yaw,
                vel,
                on_ground,
            ));

            if we.entity.core.metadata.has_dirty() {
                dirty_meta = Some(we.entity.core.metadata.take_dirty());
            }
            if we.velocity_dirty {
                we.velocity_dirty = false;
                send_vel = Some(vel);
            }
        }

        if let Some((inst_id, old_pos, new_pos, head_yaw, old_head_yaw, vel, on_ground)) = sync_step
        {
            if sim.tracker.update_position(eid, inst_id, new_pos) {
                instances_needing_reconcile.insert(inst_id);
            }
            let updated_anchor = sim.broadcast_entity_movement(
                eid,
                old_pos,
                new_pos,
                head_yaw,
                old_head_yaw,
                vel,
                on_ground,
            );
            if let Some(we) = sim.entities.get_mut(&eid) {
                we.last_synced_pos = updated_anchor;
                we.last_synced_head_yaw = head_yaw;
            }
        }

        if let Some(vel) = send_vel {
            sim.send_to_viewers(
                eid,
                &EntityVelocityPacket {
                    entity_id: eid,
                    velocity: vel,
                },
            );
        }

        if let Some(entries) = dirty_meta {
            sim.send_to_viewers(
                eid,
                &EntityMetaDataPacket {
                    entity_id: eid,
                    entries,
                },
            );
        }
    }

    for inst_id in instances_needing_reconcile {
        sim.reconcile_instance_players(inst_id);
    }
    for eid in to_remove {
        despawn_world_entity(sim, eid, events);
    }
}

fn initialize_joining_player(
    pid: PlayerId,
    sim: &mut WorldSimState,
    config: &ServerConfig,
    events: &GlobalEventHandler,
    command_manager: &CommandManager,
) {
    let all_entries: Vec<PlayerInfoAddEntry> = sim
        .players
        .values()
        .map(|p| PlayerInfoAddEntry {
            uuid: p.profile.uuid,
            username: p.profile.username.clone(),
            properties: p.profile.properties.clone(),
            game_mode: p.game_mode,
            listed: true,
        })
        .collect();

    let Some(joining_entry) = sim.players.get(pid).map(|p| PlayerInfoAddEntry {
        uuid: p.profile.uuid,
        username: p.profile.username.clone(),
        properties: p.profile.properties.clone(),
        game_mode: p.game_mode,
        listed: true,
    }) else {
        return;
    };

    // Broadcast PlayerInfoUpdate to existing players BEFORE spawning player entity!
    if let Ok(new_info_bytes) = encode_packet_payload(&PlayerInfoUpdatePacket {
        entries: vec![joining_entry],
    }) {
        for (other_pid, other) in &sim.players {
            if other_pid != pid {
                let _ = other.send_encoded(new_info_bytes.clone());
            }
        }
    }

    let (player_eid, inst_id, spawn_pos, spawn_username, spawn_instance) = {
        let Some(player) = sim.players.get_mut(pid) else {
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

        let self_meta = player.metadata.all_entries();
        if !self_meta.is_empty() {
            let _ = player.send_packet(&EntityMetaDataPacket {
                entity_id: player.entity_id,
                entries: self_meta,
            });
        }

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

        (
            player.entity_id,
            player.instance.id(),
            spawn_pos,
            player.profile.username.clone(),
            player.instance.clone(),
        )
    };

    // Register in spatial tracker and reconcile bidirectional visibility for all players in instance!
    sim.tracker.register(player_eid, inst_id, spawn_pos);
    sim.reconcile_instance_players(inst_id);

    let mut spawn_ev = PlayerSpawnEvent {
        player_id: pid,
        username: spawn_username,
        position: spawn_pos,
        instance: spawn_instance,
        first_spawn: true,
    };
    events.call(&mut spawn_ev);
}

fn update_player_position_and_sync(
    sim: &mut WorldSimState,
    pid: PlayerId,
    new_pos: Pos,
    on_ground: bool,
) {
    let Some((eid, inst_id, old_synced, old_yaw)) = sim.players.get(pid).map(|p| {
        (
            p.entity_id,
            p.instance.id(),
            p.last_synced_pos,
            p.last_synced_pos.yaw,
        )
    }) else {
        return;
    };

    if let Some(player) = sim.players.get_mut(pid) {
        player.position = new_pos;
        player.on_ground = on_ground;
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

    let crossed = sim.tracker.update_position(eid, inst_id, new_pos);
    if crossed {
        sim.reconcile_instance_players(inst_id);
    }

    let updated_anchor = sim.broadcast_entity_movement(
        eid,
        old_synced,
        new_pos,
        new_pos.yaw,
        old_yaw,
        Vec3::ZERO,
        on_ground,
    );
    if let Some(player) = sim.players.get_mut(pid) {
        player.last_synced_pos = updated_anchor;
    }
}

fn teleport_player_server_side(sim: &mut WorldSimState, pid: PlayerId, target: Pos) {
    let Some(tp_id) = sim.players.get_mut(pid).map(|p| {
        let id = p.next_teleport_id;
        p.next_teleport_id = p.next_teleport_id.wrapping_add(1).max(1);
        p.pending_teleport_id = Some(id);
        id
    }) else {
        return;
    };
    update_player_position_and_sync(sim, pid, target, false);
    if let Some(p) = sim.players.get(pid) {
        let _ = p.send_packet(&PlayerPositionAndLookPacket {
            teleport_id: tp_id,
            position: target.as_vec(),
            delta: Vec3::ZERO,
            yaw: target.yaw,
            pitch: target.pitch,
            flags: 0,
        });
    }
}

fn process_movement(
    pid: PlayerId,
    sim: &mut WorldSimState,
    events: &GlobalEventHandler,
    proposed: Pos,
    on_ground: bool,
) {
    let Some(player) = sim.players.get(pid) else {
        return;
    };
    if player.pending_teleport_id.is_some() {
        return;
    }
    let old_pos = player.position;
    if old_pos == proposed && player.on_ground == on_ground {
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

    if ev.is_cancelled() {
        teleport_player_server_side(sim, pid, old_pos);
    } else if ev.new_position != proposed {
        teleport_player_server_side(sim, pid, ev.new_position);
    } else {
        update_player_position_and_sync(sim, pid, ev.new_position, on_ground);
    }
}

fn dispatch_player_command(
    pid: PlayerId,
    sim: &mut WorldSimState,
    events: &GlobalEventHandler,
    command_manager: &CommandManager,
    raw_command: String,
) {
    let Some(username) = sim.players.get(pid).map(|p| p.profile.username.clone()) else {
        return;
    };
    let mut cmd_ev = PlayerCommandEvent::new(pid, username, raw_command);
    events.call(&mut cmd_ev);
    if cmd_ev.is_cancelled() {
        return;
    }

    let Some(mut sender) = sim.players.get(pid).map(|p| p.command_sender(pid)) else {
        return;
    };
    let result = command_manager.execute(&mut sender, &cmd_ev.command);
    let replies = sender.take_messages();
    let effects = sender.take_effects();

    if let Err(err) = result
        && replies.is_empty()
        && let Some(p) = sim.players.get(pid)
    {
        let _ = p.send_packet(&SystemChatPacket {
            message: Component::text(err.to_string()).color(NamedTextColor::Red),
            overlay: false,
        });
    }

    if let Some(p) = sim.players.get(pid) {
        for msg in replies {
            let _ = p.send_packet(&SystemChatPacket {
                message: msg,
                overlay: false,
            });
        }
    }

    let mut pending_broadcasts: Vec<Component> = Vec::new();
    let mut pending_tp: Option<Pos> = None;
    let mut kick_reason: Option<Component> = None;

    if let Some(p) = sim.players.get_mut(pid) {
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
                SenderEffect::Teleport(pos) => pending_tp = Some(pos),
                SenderEffect::Broadcast(msg) => pending_broadcasts.push(msg),
                SenderEffect::Kick(reason) => kick_reason = Some(reason),
            }
        }
    }

    if let Some(tp_pos) = pending_tp {
        teleport_player_server_side(sim, pid, tp_pos);
    }

    for bcast in pending_broadcasts {
        if let Ok(encoded) = encode_packet_payload(&SystemChatPacket {
            message: bcast,
            overlay: false,
        }) {
            for target in sim.players.values() {
                let _ = target.send_encoded(encoded.clone());
            }
        }
    }

    if let Some(reason) = kick_reason {
        if let Some(p) = sim.players.get(pid) {
            let kick = encode_packet_payload(&PlayDisconnectPacket { message: reason }).ok();
            let _ = p.outbound_tx.try_send(OutboundCommand::Disconnect(kick));
        }
        sim.remove_and_cleanup_player(pid, events);
    }
}

fn sync_player_dirty_metadata(sim: &mut WorldSimState, pid: PlayerId) {
    let Some((eid, dirty)) = sim.players.get_mut(pid).and_then(|p| {
        p.metadata
            .has_dirty()
            .then(|| (p.entity_id, p.metadata.take_dirty()))
    }) else {
        return;
    };
    sim.send_to_viewers(
        eid,
        &EntityMetaDataPacket {
            entity_id: eid,
            entries: dirty,
        },
    );
}

fn handle_play_packet(
    pid: PlayerId,
    sim: &mut WorldSimState,
    events: &GlobalEventHandler,
    command_manager: &CommandManager,
    frame: RawPacketFrame,
) -> bool {
    match frame.packet_id {
        0x00 => {
            let Ok(pkt) = decode_packet_exact::<ClientTeleportConfirmPacket>(frame.payload) else {
                return false;
            };
            if let Some(player) = sim.players.get_mut(pid)
                && player.pending_teleport_id == Some(pkt.teleport_id)
            {
                player.pending_teleport_id = None;
            }
            true
        }
        0x01 => {
            let Ok(pkt) = decode_packet_exact::<ClientAttackPacket>(frame.payload) else {
                return false;
            };
            let Some((attacker_eid, inst_id)) =
                sim.players.get(pid).map(|p| (p.entity_id, p.instance.id()))
            else {
                return true;
            };
            let mut ev = PlayerAttackEntityEvent::new(pid, attacker_eid, pkt.target_id, inst_id);
            events.call(&mut ev);
            true
        }
        0x07 => {
            let Ok(pkt) = decode_packet_exact::<ClientCommandChatPacket>(frame.payload) else {
                return false;
            };
            dispatch_player_command(pid, sim, events, command_manager, pkt.message);
            true
        }
        0x08 => {
            let Ok(pkt) = decode_packet_exact::<ClientSignedCommandChatPacket>(frame.payload)
            else {
                return false;
            };
            dispatch_player_command(pid, sim, events, command_manager, pkt.message);
            true
        }
        0x09 => {
            let Ok(pkt) = decode_packet_exact::<ClientChatMessagePacket>(frame.payload) else {
                return false;
            };
            let Some(username) = sim.players.get(pid).map(|p| p.profile.username.clone()) else {
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
                            if let Some(target) = sim.players.get(target_id) {
                                let _ = target.send_encoded(encoded.clone());
                            }
                        }
                    } else {
                        for target in sim.players.values() {
                            let _ = target.send_encoded(encoded.clone());
                        }
                    }
                }
            }
            true
        }
        0x0E => {
            // ClientSettingsPacket in Play (`0x0E`): update skin parts / main hand dynamically!
            let Ok(settings) = decode_packet_exact::<ClientSettingsPacket>(frame.payload) else {
                return false;
            };
            if let Some(p) = sim.players.get_mut(pid) {
                p.metadata.set_main_hand(settings.main_hand);
                p.metadata
                    .set_displayed_skin_parts(settings.displayed_skin_parts);
            }
            sync_player_dirty_metadata(sim, pid);
            true
        }
        0x0F => {
            let Ok(pkt) = decode_packet_exact::<ClientTabCompletePacket>(frame.payload) else {
                return false;
            };
            if let Some(player) = sim.players.get(pid) {
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
        0x1A => {
            let Ok(pkt) = decode_packet_exact::<ClientInteractEntityPacket>(frame.payload) else {
                return false;
            };
            if let Some(inst_id) = sim.players.get(pid).map(|p| p.instance.id()) {
                let mut ev = PlayerEntityInteractEvent {
                    player_id: pid,
                    target_network_id: pkt.target_id,
                    hand: pkt.hand,
                    interact_position: pkt.location,
                    instance_id: inst_id,
                };
                events.call(&mut ev);
            }
            true
        }
        0x1C => {
            let Ok(pkt) = decode_packet_exact::<ClientPlayKeepAlivePacket>(frame.payload) else {
                return false;
            };
            let Some(player) = sim.players.get_mut(pid) else {
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
            if let Some(cur) = sim.players.get(pid).map(|p| p.position) {
                let proposed = cur
                    .with_x(pkt.position.x)
                    .with_y(pkt.position.y)
                    .with_z(pkt.position.z);
                process_movement(pid, sim, events, proposed, (pkt.flags & 0x01) != 0);
            }
            true
        }
        0x1F => {
            let Ok(pkt) =
                decode_packet_exact::<ClientPlayerPositionAndRotationPacket>(frame.payload)
            else {
                return false;
            };
            process_movement(pid, sim, events, pkt.position, (pkt.flags & 0x01) != 0);
            true
        }
        0x20 => {
            let Ok(pkt) = decode_packet_exact::<ClientPlayerRotationPacket>(frame.payload) else {
                return false;
            };
            if let Some(cur) = sim.players.get(pid).map(|p| p.position) {
                let proposed = cur.with_yaw(pkt.yaw).with_pitch(pkt.pitch);
                process_movement(pid, sim, events, proposed, (pkt.flags & 0x01) != 0);
            }
            true
        }
        0x2A => {
            let Ok(pkt) = decode_packet_exact::<ClientEntityActionPacket>(frame.payload) else {
                return false;
            };
            if let Some(p) = sim.players.get_mut(pid) {
                match pkt.action {
                    ClientEntityAction::StartSprinting => p.metadata.set_sprinting(true),
                    ClientEntityAction::StopSprinting => p.metadata.set_sprinting(false),
                    _ => {}
                }
            }
            sync_player_dirty_metadata(sim, pid);
            true
        }
        0x2B => {
            let Ok(pkt) = decode_packet_exact::<ClientInputPacket>(frame.payload) else {
                return false;
            };
            if let Some(p) = sim.players.get_mut(pid) {
                let sneaking = pkt.shift();
                if p.metadata.is_sneaking() != sneaking {
                    p.metadata.set_sneaking(sneaking);
                    p.metadata.set_pose(if sneaking {
                        EntityPose::Sneaking
                    } else {
                        EntityPose::Standing
                    });
                }
            }
            sync_player_dirty_metadata(sim, pid);
            true
        }
        0x3F => {
            let Ok(pkt) = decode_packet_exact::<ClientAnimationPacket>(frame.payload) else {
                return false;
            };
            let Some((eid, inst_id)) = sim.players.get(pid).map(|p| (p.entity_id, p.instance.id()))
            else {
                return true;
            };
            let mut ev = PlayerHandAnimationEvent::new(pid, pkt.hand, inst_id);
            events.call(&mut ev);
            if !ev.is_cancelled() {
                let animation = match pkt.hand {
                    Hand::Main => EntityAnimation::SwingMainArm,
                    Hand::Off => EntityAnimation::SwingOffHand,
                };
                sim.send_to_viewers(
                    eid,
                    &EntityAnimationPacket {
                        entity_id: eid,
                        animation,
                    },
                );
            }
            true
        }
        _ => true,
    }
}
