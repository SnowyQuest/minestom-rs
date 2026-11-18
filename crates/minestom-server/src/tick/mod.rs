//! Dedicated 20 TPS game tick thread (`Ms-TickScheduler`) managing active Play sessions,
//! spatial `EntityTracker` multi-player & world-entity synchronization, 20 TPS entity physics,
//! movement chunk streaming & unloading (`UnloadChunkPacket` `0x25`), chat, Brigadier commands,
//! and `KeepAlive` tracking (ADR 0002).

mod block_ops;
mod combat;
pub(crate) mod commands;
mod play_packets;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use bytes::Bytes;
use flume::{Receiver, Sender};
use minestom_block::BlockManager;
use minestom_command::{CommandManager, CommandSender, PlayerSenderInfo, SenderEffect};
use minestom_core::{ChunkPos, GameMode, InstanceId, PlayerId, Point, Pos, Vec3};
use minestom_entity::{
    Attribute, AttributeMap, AttributeModifier, AttributeOperation, EffectMap, Entity,
    EntityMetadata, EntityTracker, EntityType,
};
use minestom_event::{CancellableEvent, GlobalEventHandler};
use minestom_item::{EquipmentSlot, Inventory, ItemStack, PlayerInventory};
use minestom_protocol::chat_command::SystemChatPacket;
use minestom_protocol::encode_packet_payload;
use minestom_protocol::entity::{
    DestroyEntitiesPacket, EntityAttributesPacket, EntityHeadLookPacket, EntityMetaDataPacket,
    EntityPositionAndRotationPacket, EntityPositionPacket, EntityPositionSyncPacket,
    EntityRotationPacket, PlayerInfoRemovePacket, SpawnEntityPacket, delta_short_4096,
};
use minestom_protocol::inventory::{
    EntityEquipmentPacket, HeldItemChangePacket, WindowItemsPacket,
};
use minestom_protocol::login::GameProfile;
use minestom_protocol::play::{
    ChangeGameStatePacket, ChunkBatchFinishedPacket, ChunkBatchStartPacket, GameStateReason,
    JoinGamePacket, PlayDisconnectPacket, PlayKeepAlivePacket, PlayerAbilitiesPacket,
    PlayerInfoAddEntry, PlayerInfoGameModePacket, PlayerInfoUpdatePacket,
    PlayerPositionAndLookPacket, PlayerSpawnInfo, ServerDifficultyPacket, SpawnPositionPacket,
    UnloadChunkPacket, UpdateHealthPacket, UpdateViewPositionPacket, WorldPos,
};
use minestom_scheduler::SchedulerManager;
use minestom_text::{Component, NamedTextColor};
use minestom_world::{InstanceContainer, InstanceManager};
use rustc_hash::{FxHashMap, FxHashSet};
use slotmap::SlotMap;
use tracing::{trace_span, warn};
use uuid::Uuid;

pub(crate) use commands::{
    EntityCommand, OutboundCommand, PlayerControlCommand, PlayerInvCommand, ServerCommand,
    TickMessage, UiCommand, WorldBlockCommand, WorldEnvCommand,
};

use combat::{tick_players_gameplay, tick_world_entities};
use commands::drain_server_commands;
use play_packets::{drop_item_from_player, handle_play_packet, open_player_window};

use crate::event::{PlayerCommandEvent, PlayerDisconnectEvent, PlayerMoveEvent, PlayerSpawnEvent};
use crate::features::{RecipeManager, TeamManager};
use crate::handle::PlayerRef;
use crate::snapshot::{
    ChunkSnapshot, EntitySnapshot, InstanceSnapshot, PlayerSnapshot, ServerSnapshot,
};
use crate::{MinecraftServer, ServerConfig};

pub(super) struct ActivePlayer {
    pub(super) entity_id: i32,
    pub(super) profile: GameProfile,
    pub(super) game_mode: GameMode,
    pub(super) permission_level: u8,
    pub(super) respawn_point: Pos,
    pub(super) position: Pos,
    pub(super) last_synced_pos: Pos,
    pub(super) velocity: Vec3,
    pub(super) on_ground: bool,
    pub(super) health: f32,
    pub(super) max_health: f32,
    pub(super) dead: bool,
    pub(super) invulnerable_ticks: u32,
    pub(super) attributes: AttributeMap,
    pub(super) effects: EffectMap,
    pub(super) metadata: EntityMetadata,
    pub(super) inventory: PlayerInventory,
    pub(super) open_window: Option<(i32, Inventory)>,
    pub(super) next_window_id: i32,
    pub(super) chunk_pos: ChunkPos,
    pub(super) view_distance: u8,
    pub(super) instance: InstanceContainer,
    pub(super) loaded_chunks: FxHashSet<ChunkPos>,
    pub(super) pending_teleport_id: Option<i32>,
    pub(super) next_teleport_id: i32,
    pub(super) forced_resource_packs: FxHashSet<Uuid>,
    pub(super) outbound_tx: Sender<OutboundCommand>,
    pub(super) last_keep_alive_sent: Instant,
    pub(super) pending_keep_alive_id: Option<(i64, Instant)>,
}

impl ActivePlayer {
    pub(super) fn send_encoded(&self, payload: Bytes) -> bool {
        self.outbound_tx
            .try_send(OutboundCommand::Packet(payload))
            .is_ok()
    }

    pub(super) fn send_packet<P: minestom_protocol::PacketPayload>(&self, pkt: &P) -> bool {
        let Ok(bytes) = encode_packet_payload(pkt) else {
            return false;
        };
        self.send_encoded(bytes)
    }

    pub(super) fn command_sender(&self, pid: PlayerId) -> CommandSender {
        CommandSender::player(PlayerSenderInfo {
            player_id: pid,
            uuid: self.profile.uuid,
            username: self.profile.username.clone(),
            position: self.position,
            game_mode: self.game_mode,
            permission_level: self.permission_level,
        })
    }

    pub(super) const fn abilities_flags_for(mode: GameMode) -> u8 {
        match mode {
            GameMode::Creative => 0x01 | 0x04 | 0x08,
            GameMode::Spectator => 0x01 | 0x02 | 0x04,
            GameMode::Survival | GameMode::Adventure => 0x00,
        }
    }

    pub(super) fn stream_chunks(&mut self, targets: &[ChunkPos]) {
        // 1. Unload any previously streamed chunks that are now outside `view_distance + 1`!
        let keep_radius = i32::from(self.view_distance) + 1;
        let center = self.chunk_pos;
        let stale: Vec<ChunkPos> = self
            .loaded_chunks
            .iter()
            .copied()
            .filter(|c| (c.x - center.x).abs().max((c.z - center.z).abs()) > keep_radius)
            .collect();
        for cpos in stale {
            self.loaded_chunks.remove(&cpos);
            let _ = self.send_packet(&UnloadChunkPacket {
                chunk_x: cpos.x,
                chunk_z: cpos.z,
            });
        }

        // 2. Stream newly entered chunks inside `targets`
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

    pub(super) fn sync_window_zero(&mut self) {
        let state_id = self.inventory.bump_state_id();
        let _ = self.send_packet(&WindowItemsPacket {
            window_id: 0,
            state_id,
            items: self.inventory.window_zero_items(),
            carried_item: self.inventory.cursor_item().clone(),
        });
    }

    pub(super) fn sync_active_windows(&mut self) {
        if let Some((wid, inv)) = self.open_window.clone() {
            let state_id = inv.bump_state_id();
            let _ = self.send_packet(&WindowItemsPacket {
                window_id: wid,
                state_id,
                items: self.inventory.combined_window_items(&inv),
                carried_item: self.inventory.cursor_item().clone(),
            });
        }
        self.sync_window_zero();
    }

    pub(super) fn equipment_map(&self, include_air: bool) -> BTreeMap<EquipmentSlot, ItemStack> {
        let air = ItemStack::AIR;
        let pairs = [
            (EquipmentSlot::MainHand, self.inventory.item_in_main_hand()),
            (EquipmentSlot::OffHand, self.inventory.item_in_off_hand()),
            (
                EquipmentSlot::Helmet,
                self.inventory.get_slot(5).unwrap_or(&air),
            ),
            (
                EquipmentSlot::Chestplate,
                self.inventory.get_slot(6).unwrap_or(&air),
            ),
            (
                EquipmentSlot::Leggings,
                self.inventory.get_slot(7).unwrap_or(&air),
            ),
            (
                EquipmentSlot::Boots,
                self.inventory.get_slot(8).unwrap_or(&air),
            ),
        ];
        let mut map = BTreeMap::new();
        for (slot, stack) in pairs {
            if include_air || !stack.is_air() {
                map.insert(slot, stack.clone());
            }
        }
        map
    }

    pub(super) fn refresh_equipment_attributes(&mut self) {
        const MOD_ARMOR: &str = "minestom:equipped_armor";
        const MOD_TOUGH: &str = "minestom:equipped_armor_toughness";
        const MOD_KB: &str = "minestom:equipped_knockback_resistance";

        let mut total_armor = 0.0_f64;
        let mut total_toughness = 0.0_f64;
        let mut total_kb_res = 0.0_f64;
        for slot in 5..=8 {
            if let Ok(stack) = self.inventory.get_slot(slot)
                && !stack.is_air()
            {
                let mat = stack.material();
                total_armor += f64::from(mat.armor());
                total_toughness += f64::from(mat.armor_toughness());
                total_kb_res += f64::from(mat.knockback_resistance());
            }
        }

        if total_armor > 0.0 {
            self.attributes.add_modifier(
                Attribute::ARMOR,
                AttributeModifier::new(MOD_ARMOR, total_armor, AttributeOperation::AddValue),
            );
        } else {
            let _ = self.attributes.remove_modifier(Attribute::ARMOR, MOD_ARMOR);
        }

        if total_toughness > 0.0 {
            self.attributes.add_modifier(
                Attribute::ARMOR_TOUGHNESS,
                AttributeModifier::new(MOD_TOUGH, total_toughness, AttributeOperation::AddValue),
            );
        } else {
            let _ = self
                .attributes
                .remove_modifier(Attribute::ARMOR_TOUGHNESS, MOD_TOUGH);
        }

        if total_kb_res > 0.0 {
            self.attributes.add_modifier(
                Attribute::KNOCKBACK_RESISTANCE,
                AttributeModifier::new(MOD_KB, total_kb_res, AttributeOperation::AddValue),
            );
        } else {
            let _ = self
                .attributes
                .remove_modifier(Attribute::KNOCKBACK_RESISTANCE, MOD_KB);
        }
    }
}

pub(super) struct SpawnedWorldEntity {
    pub(super) entity: Entity,
    pub(super) instance: InstanceContainer,
    pub(super) last_synced_pos: Pos,
    pub(super) last_synced_head_yaw: f32,
    pub(super) velocity_dirty: bool,
}

pub(super) struct WorldSimState {
    pub(super) server: MinecraftServer,
    pub(super) players: SlotMap<PlayerId, ActivePlayer>,
    pub(super) player_by_entity_id: FxHashMap<i32, PlayerId>,
    pub(super) entities: FxHashMap<i32, SpawnedWorldEntity>,
    pub(super) tracker: EntityTracker,
}

impl WorldSimState {
    fn new(server: MinecraftServer) -> Self {
        Self {
            server,
            players: SlotMap::with_key(),
            player_by_entity_id: FxHashMap::default(),
            entities: FxHashMap::default(),
            tracker: EntityTracker::new(),
        }
    }

    pub(super) fn player_ref(&self, pid: PlayerId) -> PlayerRef {
        PlayerRef::new(pid, self.server.clone())
    }

    pub(super) fn send_to_storage_chunk_viewers<P: minestom_protocol::PacketPayload>(
        &self,
        source_instance: &InstanceContainer,
        chunk_pos: ChunkPos,
        pkt: &P,
    ) {
        let Ok(bytes) = encode_packet_payload(pkt) else {
            return;
        };
        for p in self.players.values() {
            if p.instance.shares_storage_with(source_instance)
                && p.loaded_chunks.contains(&chunk_pos)
            {
                let _ = p.send_encoded(bytes.clone());
            }
        }
    }

    pub(super) fn send_to_instance_players<P: minestom_protocol::PacketPayload>(
        &self,
        instance_id: InstanceId,
        pkt: &P,
    ) {
        let Ok(bytes) = encode_packet_payload(pkt) else {
            return;
        };
        for p in self.players.values() {
            if p.instance.id() == instance_id {
                let _ = p.send_encoded(bytes.clone());
            }
        }
    }

    pub(super) fn send_to_viewers<P: minestom_protocol::PacketPayload>(
        &self,
        entity_id: i32,
        pkt: &P,
    ) {
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

    pub(super) fn send_to_viewers_and_self<P: minestom_protocol::PacketPayload>(
        &self,
        entity_id: i32,
        pkt: &P,
    ) {
        let Ok(bytes) = encode_packet_payload(pkt) else {
            return;
        };
        if let Some(&self_pid) = self.player_by_entity_id.get(&entity_id)
            && let Some(p) = self.players.get(self_pid)
        {
            let _ = p.send_encoded(bytes.clone());
        }
        if let Some(viewers) = self.tracker.viewers(entity_id) {
            for &pid in viewers {
                if let Some(p) = self.players.get(pid) {
                    let _ = p.send_encoded(bytes.clone());
                }
            }
        }
    }

    pub(super) fn send_entity_spawn_bundle(&self, viewer_pid: PlayerId, target_entity_id: i32) {
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
                let equip = target_player.equipment_map(false);
                if !equip.is_empty() {
                    let _ = viewer.send_packet(&EntityEquipmentPacket {
                        entity_id: target_player.entity_id,
                        equipments: equip,
                    });
                }
                let attrs = target_player.attributes.all_synced_wire_properties();
                if !attrs.is_empty() {
                    let _ = viewer.send_packet(&EntityAttributesPacket {
                        entity_id: target_player.entity_id,
                        properties: attrs,
                    });
                }
                for potion in target_player.effects.iter() {
                    let _ = viewer.send_packet(&potion.to_add_packet(target_player.entity_id));
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
            if let Some(liv) = &we.entity.living {
                let attrs = liv.attributes.all_synced_wire_properties();
                if !attrs.is_empty() {
                    let _ = viewer.send_packet(&EntityAttributesPacket {
                        entity_id: core.network_id,
                        properties: attrs,
                    });
                }
                for potion in liv.effects.iter() {
                    let _ = viewer.send_packet(&potion.to_add_packet(core.network_id));
                }
            }
        }
    }

    pub(super) fn broadcast_player_equipment(&mut self, pid: PlayerId) {
        let mut eid_equip_attrs = None;
        if let Some(p) = self.players.get_mut(pid) {
            p.refresh_equipment_attributes();
            let dirty_attrs = p.attributes.take_dirty_wire_properties();
            eid_equip_attrs = Some((p.entity_id, p.equipment_map(true), dirty_attrs));
        }
        if let Some((eid, equip, dirty_attrs)) = eid_equip_attrs {
            if !equip.is_empty() {
                self.send_to_viewers(
                    eid,
                    &EntityEquipmentPacket {
                        entity_id: eid,
                        equipments: equip,
                    },
                );
            }
            if !dirty_attrs.is_empty() {
                self.send_to_viewers_and_self(
                    eid,
                    &EntityAttributesPacket {
                        entity_id: eid,
                        properties: dirty_attrs,
                    },
                );
            }
        }
    }

    pub(super) fn sync_shared_inventory_viewers(
        &mut self,
        inv: &Inventory,
        exclude_pid: Option<PlayerId>,
    ) {
        for (pid, p) in &mut self.players {
            if Some(pid) == exclude_pid {
                continue;
            }
            if p.open_window.as_ref().is_some_and(|(_, o)| o.ptr_eq(inv)) {
                p.sync_active_windows();
            }
        }
    }

    pub(super) fn reconcile_single_player(&mut self, pid: PlayerId) {
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

    pub(super) fn reconcile_instance_players(&mut self, instance_id: InstanceId) {
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
    pub(super) fn broadcast_entity_movement(
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

    pub(super) fn remove_and_cleanup_player(&mut self, pid: PlayerId, events: &GlobalEventHandler) {
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
            player: self.player_ref(pid),
            player_id: pid,
            username: removed.profile.username,
            instance_id: removed.instance.id(),
        };
        events.call(&mut ev);
    }
}

pub(super) fn set_player_game_mode_and_sync(
    sim: &mut WorldSimState,
    pid: PlayerId,
    mode: GameMode,
) {
    let mut uuid_opt = None;
    if let Some(p) = sim.players.get_mut(pid) {
        p.game_mode = mode;
        uuid_opt = Some(p.profile.uuid);
        let _ = p.send_packet(&ChangeGameStatePacket {
            reason: GameStateReason::ChangeGameMode,
            value: f32::from(mode as u8),
        });
        let _ = p.send_packet(&PlayerAbilitiesPacket {
            flags: ActivePlayer::abilities_flags_for(mode),
            ..PlayerAbilitiesPacket::default()
        });
    }
    if let Some(uuid) = uuid_opt
        && let Ok(encoded) = encode_packet_payload(&PlayerInfoGameModePacket {
            entries: vec![(uuid, mode)],
        })
    {
        for p in sim.players.values() {
            let _ = p.send_encoded(encoded.clone());
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_tick_thread(
    server: MinecraftServer,
    config: ServerConfig,
    events: GlobalEventHandler,
    instance_manager: InstanceManager,
    block_manager: BlockManager,
    command_manager: CommandManager,
    scheduler_manager: SchedulerManager,
    team_manager: TeamManager,
    recipe_manager: RecipeManager,
    latest_snapshot: Arc<ArcSwap<ServerSnapshot>>,
    online_count: Arc<AtomicU32>,
    next_entity_id: Arc<AtomicI32>,
    inbox_rx: Receiver<TickMessage>,
    cmd_rx: Receiver<ServerCommand>,
    stop_flag: Arc<AtomicBool>,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("Ms-TickScheduler".to_owned())
        .spawn(move || {
            run_tick_loop(
                server,
                &config,
                &events,
                &instance_manager,
                &block_manager,
                &command_manager,
                &scheduler_manager,
                &team_manager,
                &recipe_manager,
                &latest_snapshot,
                &online_count,
                &next_entity_id,
                &inbox_rx,
                &cmd_rx,
                &stop_flag,
            );
        })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_tick_loop(
    server: MinecraftServer,
    config: &ServerConfig,
    events: &GlobalEventHandler,
    instance_manager: &InstanceManager,
    block_manager: &BlockManager,
    command_manager: &CommandManager,
    scheduler_manager: &SchedulerManager,
    team_manager: &TeamManager,
    recipe_manager: &RecipeManager,
    latest_snapshot: &ArcSwap<ServerSnapshot>,
    online_count: &AtomicU32,
    next_entity_id: &AtomicI32,
    inbox_rx: &Receiver<TickMessage>,
    cmd_rx: &Receiver<ServerCommand>,
    stop_flag: &AtomicBool,
) {
    let tick_duration = Duration::from_nanos(1_000_000_000 / u64::from(config.ticks_per_second));
    let keep_alive_interval = Duration::from_millis(config.keep_alive_interval_ms);
    let keep_alive_timeout = Duration::from_millis(config.keep_alive_timeout_ms);

    let mut sim = WorldSimState::new(server);
    let mut tick_num = 0_u64;
    let mut next_tick_deadline = Instant::now() + tick_duration;

    while !stop_flag.load(Ordering::Acquire) {
        let _span = trace_span!("tick", tick_num).entered();
        let now = Instant::now();

        // 0. TickStart scheduled tasks
        scheduler_manager.process_tick_start();

        // 1. Drain unified FIFO server command channel
        drain_server_commands(&mut sim, cmd_rx, events, next_entity_id);

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
                        profile,
                        game_mode,
                        permission_level,
                        respawn_point: spawn_pos,
                        position: spawn_pos,
                        last_synced_pos: spawn_pos,
                        velocity: Vec3::ZERO,
                        on_ground: true,
                        health: 20.0,
                        max_health: 20.0,
                        dead: false,
                        invulnerable_ticks: 0,
                        attributes: AttributeMap::for_player(),
                        effects: EffectMap::new(),
                        metadata,
                        inventory: PlayerInventory::new(),
                        open_window: None,
                        next_window_id: 1,
                        chunk_pos: center,
                        view_distance: effective_vd,
                        instance,
                        loaded_chunks: FxHashSet::default(),
                        pending_teleport_id: Some(1),
                        next_teleport_id: 2,
                        forced_resource_packs: FxHashSet::default(),
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

                    initialize_joining_player(
                        pid,
                        &mut sim,
                        config,
                        events,
                        command_manager,
                        team_manager,
                        recipe_manager,
                    );
                }
                TickMessage::InboundPacket { player_id, frame } => {
                    if !sim.players.contains_key(player_id) {
                        continue;
                    }
                    if !handle_play_packet(
                        player_id,
                        &mut sim,
                        events,
                        block_manager,
                        command_manager,
                        next_entity_id,
                        frame,
                    ) {
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
                    shutdown_tick_loop(&mut sim, scheduler_manager, events, online_count);
                    return;
                }
            }
        }

        // Drain any cross-thread commands emitted inside event/command handlers this tick
        drain_server_commands(&mut sim, cmd_rx, events, next_entity_id);

        // 3. Advance world clocks & step 20 TPS Players + World Entities
        for inst in instance_manager.instances() {
            let wt = inst.tick_time();
            if tick_num.is_multiple_of(20) {
                sim.send_to_instance_players(inst.id(), &wt.to_set_time_packet());
            }
        }
        tick_players_gameplay(&mut sim);
        tick_world_entities(&mut sim, events);

        // 4. TickEnd scheduled tasks + final FIFO command drain
        scheduler_manager.process_tick_end();
        drain_server_commands(&mut sim, cmd_rx, events, next_entity_id);

        // 5. Per-tick KeepAlive check
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

        latest_snapshot.store(Arc::new(build_server_snapshot(
            &sim,
            instance_manager,
            tick_num,
        )));
        tick_num = tick_num.wrapping_add(1);

        let after = Instant::now();
        if after < next_tick_deadline {
            thread::sleep(next_tick_deadline - after);
            next_tick_deadline += tick_duration;
        } else {
            next_tick_deadline = after + tick_duration;
        }
    }

    shutdown_tick_loop(&mut sim, scheduler_manager, events, online_count);
}

fn shutdown_tick_loop(
    sim: &mut WorldSimState,
    scheduler_manager: &SchedulerManager,
    events: &GlobalEventHandler,
    online_count: &AtomicU32,
) {
    scheduler_manager.shutdown();
    let pids: Vec<PlayerId> = sim.players.keys().collect();
    for pid in pids {
        if let Some(p) = sim.players.get(pid) {
            let kick = encode_packet_payload(&PlayDisconnectPacket {
                message: Component::text("Server closed").color(NamedTextColor::Red),
            })
            .ok();
            let _ = p.outbound_tx.try_send(OutboundCommand::Disconnect(kick));
        }
        sim.remove_and_cleanup_player(pid, events);
    }
    online_count.store(0, Ordering::Relaxed);
}

fn build_server_snapshot(
    sim: &WorldSimState,
    instance_manager: &InstanceManager,
    tick_number: u64,
) -> ServerSnapshot {
    let players: Vec<PlayerSnapshot> = sim
        .players
        .iter()
        .map(|(pid, p)| PlayerSnapshot {
            player_id: pid,
            entity_id: p.entity_id,
            uuid: p.profile.uuid,
            username: p.profile.username.clone(),
            game_mode: p.game_mode,
            instance_id: p.instance.id(),
            position: p.position,
            health: p.health,
            max_health: p.max_health,
            on_ground: p.on_ground,
        })
        .collect();

    let entities: Vec<EntitySnapshot> = sim
        .entities
        .iter()
        .map(|(&eid, we)| EntitySnapshot {
            entity_id: eid,
            uuid: we.entity.core.uuid,
            entity_type: we.entity.entity_type(),
            instance_id: we.instance.id(),
            position: we.entity.core.position,
            velocity: we.entity.core.velocity,
            health: we.entity.living.as_ref().map(|l| l.health),
        })
        .collect();

    let instances: Vec<InstanceSnapshot> = instance_manager
        .instances()
        .into_iter()
        .map(|inst| {
            let inst_id = inst.id();
            let wt = inst.world_time();
            let mut chunk_counts: FxHashMap<ChunkPos, usize> = FxHashMap::default();
            let mut player_ids = Vec::new();
            for (pid, p) in &sim.players {
                if p.instance.id() == inst_id {
                    player_ids.push(pid);
                    for &cpos in &p.loaded_chunks {
                        *chunk_counts.entry(cpos).or_insert(0) += 1;
                    }
                }
            }
            let chunks = chunk_counts
                .into_iter()
                .map(|(chunk_pos, viewer_count)| ChunkSnapshot {
                    chunk_pos,
                    viewer_count,
                })
                .collect();
            let entity_ids = sim
                .entities
                .iter()
                .filter_map(|(&eid, we)| (we.instance.id() == inst_id).then_some(eid))
                .collect();
            InstanceSnapshot {
                instance_id: inst_id,
                dimension_key: inst.dimension_type().key.clone(),
                is_shared: inst.is_shared(),
                world_age: wt.world_age,
                time_of_day: wt.day_time,
                chunks,
                entity_ids,
                player_ids,
            }
        })
        .collect();

    ServerSnapshot {
        tick_number,
        players,
        entities,
        instances,
    }
}

#[allow(clippy::too_many_lines)]
fn initialize_joining_player(
    pid: PlayerId,
    sim: &mut WorldSimState,
    config: &ServerConfig,
    events: &GlobalEventHandler,
    command_manager: &CommandManager,
    team_manager: &TeamManager,
    recipe_manager: &RecipeManager,
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
        let _ = player.send_packet(&player.instance.world_border().to_initialize_packet());
        let _ = player.send_packet(&player.instance.world_time().to_set_time_packet());
        let weather = player.instance.weather();
        if weather.is_raining() || weather.thunder_level > 0.0 {
            let _ = player.send_packet(&ChangeGameStatePacket {
                reason: GameStateReason::BeginRaining,
                value: 0.0,
            });
            let _ = player.send_packet(&ChangeGameStatePacket {
                reason: GameStateReason::RainLevelChange,
                value: weather.rain_level,
            });
            let _ = player.send_packet(&ChangeGameStatePacket {
                reason: GameStateReason::ThunderLevelChange,
                value: weather.thunder_level,
            });
        }
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
        let _ = player.send_packet(&HeldItemChangePacket {
            slot: player.inventory.held_slot() as i8,
        });
        player.sync_window_zero();
        let sender_view = player.command_sender(pid);
        let declare_cmds = command_manager.build_declare_commands_packet(&sender_view);
        let _ = player.send_packet(&declare_cmds);
        if !recipe_manager.is_empty() {
            let _ = player.send_packet(&recipe_manager.to_declare_packet());
        }
        for team_pkt in team_manager.all_creation_packets() {
            let _ = player.send_packet(&team_pkt);
        }

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
        player: sim.player_ref(pid),
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

pub(super) fn teleport_player_server_side(sim: &mut WorldSimState, pid: PlayerId, target: Pos) {
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

pub(super) fn process_movement(
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
        sim.player_ref(pid),
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

pub(super) fn dispatch_player_command(
    pid: PlayerId,
    sim: &mut WorldSimState,
    events: &GlobalEventHandler,
    command_manager: &CommandManager,
    next_entity_id: &AtomicI32,
    raw_command: String,
) {
    let Some(username) = sim.players.get(pid).map(|p| p.profile.username.clone()) else {
        return;
    };
    let mut cmd_ev = PlayerCommandEvent::new(sim.player_ref(pid), pid, username, raw_command);
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
    let mut pending_give: Vec<ItemStack> = Vec::new();
    let mut pending_open_inv: Option<Inventory> = None;
    let mut pending_gm: Option<GameMode> = None;

    for effect in effects {
        match effect {
            SenderEffect::SetGameMode(mode) => pending_gm = Some(mode),
            SenderEffect::Teleport(pos) => pending_tp = Some(pos),
            SenderEffect::Broadcast(msg) => pending_broadcasts.push(msg),
            SenderEffect::Kick(reason) => kick_reason = Some(reason),
            SenderEffect::GiveItem(item) => pending_give.push(item),
            SenderEffect::OpenInventory(inv) => pending_open_inv = Some(inv),
        }
    }

    if let Some(mode) = pending_gm {
        set_player_game_mode_and_sync(sim, pid, mode);
    }

    if !pending_give.is_empty() {
        let mut overflows = Vec::new();
        if let Some(p) = sim.players.get_mut(pid) {
            for stack in pending_give {
                let rem = p.inventory.add_item(stack);
                if !rem.is_air() {
                    overflows.push(rem);
                }
            }
            p.sync_active_windows();
        }
        sim.broadcast_player_equipment(pid);
        for rem in overflows {
            let _ = drop_item_from_player(sim, pid, rem, events, next_entity_id);
        }
    }

    if let Some(inv) = pending_open_inv {
        open_player_window(sim, pid, inv, events, next_entity_id);
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
