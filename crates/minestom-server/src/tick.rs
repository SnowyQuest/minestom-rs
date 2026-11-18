//! Dedicated 20 TPS game tick thread (`Ms-TickScheduler`) managing active Play sessions,
//! spatial `EntityTracker` multi-player & world-entity synchronization, 20 TPS entity physics,
//! movement chunk streaming, chat, Brigadier commands, and `KeepAlive` tracking (ADR 0002).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bytes::Bytes;
use flume::{Receiver, Sender};
use minestom_block::{
    Block, BlockDestroyContext, BlockGetter, BlockInteractContext, BlockManager,
    BlockPlacementContext, PlacementState,
};
use minestom_command::{CommandManager, CommandSender, PlayerSenderInfo, SenderEffect};
use minestom_core::{
    BlockFace, BlockVec, ChunkPos, GameMode, Hand, InstanceId, MainHand, PlayerId, Point, Pos, Vec3,
};
use minestom_entity::{
    Attribute, AttributeMap, AttributeModifier, BoundingBox, Damage, DamageType, EffectMap, Entity,
    EntityDamageEvent, EntityDeathEvent, EntityDespawnEvent, EntityMetadata, EntityRole,
    EntitySpawnEvent, EntityTracker, EntityType, PlayerAttackEntityEvent,
    PlayerEntityInteractEvent, PlayerHandAnimationEvent, PlayerRespawnEvent, Potion, PotionEffect,
    TargetCandidate, compute_knockback_velocity,
};
use minestom_event::{CancellableEvent, GlobalEventHandler};
use minestom_item::{
    EquipmentSlot, Inventory, InventoryCloseEvent, InventoryPreClickEvent, ItemDropEvent,
    ItemStack, Material, PickupItemEvent, PlayerInventory, process_inventory_click,
};
use minestom_nbt::NbtCompound;
use minestom_protocol::RawPacketFrame;
use minestom_protocol::block::{
    AcknowledgeBlockChangePacket, BlockChangePacket, BlockEntityDataPacket,
    ClientPlayerActionPacket, ClientPlayerBlockPlacementPacket, ClientUseItemPacket,
    MultiBlockChangePacket, PlayerActionStatus,
};
use minestom_protocol::chat_command::{
    ClientChatMessagePacket, ClientCommandChatPacket, ClientSignedCommandChatPacket,
    ClientTabCompletePacket, SystemChatPacket, TabCompletePacket,
};
use minestom_protocol::config::ClientSettingsPacket;
use minestom_protocol::entity::{
    ClientAnimationPacket, ClientAttackPacket, ClientEntityAction, ClientEntityActionPacket,
    ClientInputPacket, ClientInteractEntityPacket, DestroyEntitiesPacket, EntityAnimation,
    EntityAnimationPacket, EntityAttributesPacket, EntityHeadLookPacket, EntityMetaDataPacket,
    EntityPose, EntityPositionAndRotationPacket, EntityPositionPacket, EntityPositionSyncPacket,
    EntityRotationPacket, EntityVelocityPacket, HitAnimationPacket, MetadataValue,
    PlayerInfoRemovePacket, SpawnEntityPacket, delta_short_4096,
};
use minestom_protocol::inventory::{
    ClientClickWindowButtonPacket, ClientClickWindowPacket, ClientCloseWindowPacket,
    ClientCreativeInventoryActionPacket, ClientHeldItemChangePacket, CloseWindowPacket,
    CollectItemPacket, EntityEquipmentPacket, HeldItemChangePacket, OpenWindowPacket,
    WindowItemsPacket,
};
use minestom_protocol::login::GameProfile;
use minestom_protocol::play::{
    ChangeGameStatePacket, ChunkBatchFinishedPacket, ChunkBatchStartPacket,
    ClientPlayKeepAlivePacket, ClientPlayerPositionAndRotationPacket, ClientPlayerPositionPacket,
    ClientPlayerRotationPacket, ClientStatusAction, ClientStatusPacket,
    ClientTeleportConfirmPacket, GameStateReason, InitializeWorldBorderPacket, JoinGamePacket,
    PlayDisconnectPacket, PlayKeepAlivePacket, PlayerAbilitiesPacket, PlayerInfoAddEntry,
    PlayerInfoUpdatePacket, PlayerPositionAndLookPacket, PlayerSpawnInfo, RespawnPacket,
    ServerDifficultyPacket, SpawnPositionPacket, UpdateHealthPacket, UpdateViewPositionPacket,
    WorldPos,
};
use minestom_protocol::{decode_packet_exact, encode_packet_payload};
use minestom_scheduler::SchedulerManager;
use minestom_text::{Component, NamedTextColor};
use minestom_world::{InstanceContainer, InstanceManager, Weather, WorldBorder, WorldTime};
use rustc_hash::{FxHashMap, FxHashSet};
use slotmap::SlotMap;
use tracing::{debug, trace_span, warn};

use crate::ServerConfig;
use crate::event::{
    PlayerBlockBreakEvent, PlayerBlockPlaceEvent, PlayerChatEvent, PlayerCommandEvent,
    PlayerDisconnectEvent, PlayerMoveEvent, PlayerSpawnEvent,
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
    /// Apply damage to any entity or player by `network_id`.
    DamageByNetworkId { target_id: i32, damage: Damage },
    /// Apply damage to a player by `player_id`.
    DamagePlayer { player_id: PlayerId, damage: Damage },
    /// Heal a player to full `max_health`.
    HealPlayer { player_id: PlayerId },
    /// Set base attribute value on an entity/player by `network_id`.
    SetAttributeBase {
        target_id: i32,
        attribute: Attribute,
        base_value: f64,
    },
    /// Add an attribute modifier on an entity/player by `network_id`.
    AddAttributeModifier {
        target_id: i32,
        attribute: Attribute,
        modifier: AttributeModifier,
    },
    /// Set base attribute value on a player by `player_id`.
    SetPlayerAttributeBase {
        player_id: PlayerId,
        attribute: Attribute,
        base_value: f64,
    },
    /// Add an attribute modifier on a player by `player_id`.
    AddPlayerAttributeModifier {
        player_id: PlayerId,
        attribute: Attribute,
        modifier: AttributeModifier,
    },
    /// Add a potion effect to an entity/player by `network_id`.
    AddEffect { target_id: i32, potion: Potion },
    /// Remove a potion effect from an entity/player by `network_id`.
    RemoveEffect {
        target_id: i32,
        effect: PotionEffect,
    },
    /// Add a potion effect to a player by `player_id`.
    AddPlayerEffect { player_id: PlayerId, potion: Potion },
    /// Remove a potion effect from a player by `player_id`.
    RemovePlayerEffect {
        player_id: PlayerId,
        effect: PotionEffect,
    },
}

/// Cross-thread command broadcasting server-initiated single/batch block updates to chunk viewers.
pub(crate) enum WorldBlockCommand {
    BroadcastSingle {
        instance: InstanceContainer,
        position: BlockVec,
        block: Block,
        nbt: Option<NbtCompound>,
    },
    BroadcastBatch {
        instance: InstanceContainer,
        packets: Vec<(ChunkPos, MultiBlockChangePacket)>,
    },
}

/// Cross-thread command syncing world border, MC 26.2 clock time, and weather to instance players.
pub(crate) enum WorldEnvCommand {
    Border {
        instance_id: InstanceId,
        old_diameter: f64,
        border: WorldBorder,
        lerp_ms: i64,
    },
    Time {
        instance_id: InstanceId,
        time: WorldTime,
    },
    Weather {
        instance_id: InstanceId,
        weather: Weather,
    },
}

/// Cross-thread command mutating player inventories and open container windows.
pub(crate) enum PlayerInvCommand {
    GiveItem {
        player_id: PlayerId,
        item: ItemStack,
    },
    SetSlot {
        player_id: PlayerId,
        slot: usize,
        item: ItemStack,
    },
    OpenWindow {
        player_id: PlayerId,
        inventory: Inventory,
    },
    CloseWindow {
        player_id: PlayerId,
    },
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
    respawn_point: Pos,
    position: Pos,
    last_synced_pos: Pos,
    velocity: Vec3,
    on_ground: bool,
    health: f32,
    max_health: f32,
    dead: bool,
    invulnerable_ticks: u32,
    attributes: AttributeMap,
    effects: EffectMap,
    metadata: EntityMetadata,
    inventory: PlayerInventory,
    open_window: Option<(i32, Inventory)>,
    next_window_id: i32,
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

    fn sync_window_zero(&mut self) {
        let state_id = self.inventory.bump_state_id();
        let _ = self.send_packet(&WindowItemsPacket {
            window_id: 0,
            state_id,
            items: self.inventory.window_zero_items(),
            carried_item: self.inventory.cursor_item().clone(),
        });
    }

    fn sync_active_windows(&mut self) {
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

    fn equipment_map(&self, include_air: bool) -> BTreeMap<EquipmentSlot, ItemStack> {
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

    fn send_to_storage_chunk_viewers<P: minestom_protocol::PacketPayload>(
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

    fn send_to_instance_players<P: minestom_protocol::PacketPayload>(
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

    fn send_to_viewers_and_self<P: minestom_protocol::PacketPayload>(
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

    fn broadcast_player_equipment(&self, pid: PlayerId) {
        let Some((eid, equip)) = sim_player_equip(self, pid) else {
            return;
        };
        if !equip.is_empty() {
            self.send_to_viewers(
                eid,
                &EntityEquipmentPacket {
                    entity_id: eid,
                    equipments: equip,
                },
            );
        }
    }

    fn sync_shared_inventory_viewers(&mut self, inv: &Inventory, exclude_pid: Option<PlayerId>) {
        for (pid, p) in &mut self.players {
            if Some(pid) == exclude_pid {
                continue;
            }
            if p.open_window.as_ref().is_some_and(|(_, o)| o.ptr_eq(inv)) {
                p.sync_active_windows();
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

fn sim_player_equip(
    sim: &WorldSimState,
    pid: PlayerId,
) -> Option<(i32, BTreeMap<EquipmentSlot, ItemStack>)> {
    sim.players
        .get(pid)
        .map(|p| (p.entity_id, p.equipment_map(true)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_tick_thread(
    config: ServerConfig,
    events: GlobalEventHandler,
    instance_manager: InstanceManager,
    block_manager: BlockManager,
    command_manager: CommandManager,
    scheduler_manager: SchedulerManager,
    online_count: Arc<AtomicU32>,
    next_entity_id: Arc<AtomicI32>,
    inbox_rx: Receiver<TickMessage>,
    entity_cmd_rx: Receiver<EntityCommand>,
    block_cmd_rx: Receiver<WorldBlockCommand>,
    inv_cmd_rx: Receiver<PlayerInvCommand>,
    env_cmd_rx: Receiver<WorldEnvCommand>,
    stop_flag: Arc<AtomicBool>,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("Ms-TickScheduler".to_owned())
        .spawn(move || {
            run_tick_loop(
                &config,
                &events,
                &instance_manager,
                &block_manager,
                &command_manager,
                &scheduler_manager,
                &online_count,
                &next_entity_id,
                &inbox_rx,
                &entity_cmd_rx,
                &block_cmd_rx,
                &inv_cmd_rx,
                &env_cmd_rx,
                &stop_flag,
            );
        })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_tick_loop(
    config: &ServerConfig,
    events: &GlobalEventHandler,
    instance_manager: &InstanceManager,
    block_manager: &BlockManager,
    command_manager: &CommandManager,
    scheduler_manager: &SchedulerManager,
    online_count: &AtomicU32,
    next_entity_id: &AtomicI32,
    inbox_rx: &Receiver<TickMessage>,
    entity_cmd_rx: &Receiver<EntityCommand>,
    block_cmd_rx: &Receiver<WorldBlockCommand>,
    inv_cmd_rx: &Receiver<PlayerInvCommand>,
    env_cmd_rx: &Receiver<WorldEnvCommand>,
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

        // 0. TickStart scheduled tasks
        scheduler_manager.process_tick_start();

        // 1. Drain queued world EntityCommands, WorldBlockCommands, PlayerInvCommands & WorldEnvCommands
        drain_entity_commands(&mut sim, entity_cmd_rx, events);
        drain_block_commands(&sim, block_cmd_rx);
        drain_inv_commands(&mut sim, inv_cmd_rx, events, next_entity_id);
        drain_env_commands(&sim, env_cmd_rx);

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
        drain_entity_commands(&mut sim, entity_cmd_rx, events);
        drain_block_commands(&sim, block_cmd_rx);
        drain_inv_commands(&mut sim, inv_cmd_rx, events, next_entity_id);
        drain_env_commands(&sim, env_cmd_rx);

        // 3. Advance world clocks & step 20 TPS Players + World Entities
        for inst in instance_manager.instances() {
            let wt = inst.tick_time();
            if tick_num.is_multiple_of(20) {
                sim.send_to_instance_players(inst.id(), &wt.to_set_time_packet());
            }
        }
        tick_players_gameplay(&mut sim);
        tick_world_entities(&mut sim, events);

        // 4. TickEnd scheduled tasks + final command drain
        scheduler_manager.process_tick_end();
        drain_entity_commands(&mut sim, entity_cmd_rx, events);
        drain_block_commands(&sim, block_cmd_rx);
        drain_inv_commands(&mut sim, inv_cmd_rx, events, next_entity_id);
        drain_env_commands(&sim, env_cmd_rx);

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

fn spawn_world_entity_direct(
    sim: &mut WorldSimState,
    network_id: i32,
    instance: InstanceContainer,
    mut entity: Entity,
    position: Pos,
    events: &GlobalEventHandler,
) {
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
            entity,
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

fn drop_item_from_player(
    sim: &mut WorldSimState,
    pid: PlayerId,
    stack: ItemStack,
    events: &GlobalEventHandler,
    next_entity_id: &AtomicI32,
) -> bool {
    if stack.is_air() {
        return true;
    }
    let Some((instance, pos)) = sim
        .players
        .get(pid)
        .map(|p| (p.instance.clone(), p.position))
    else {
        return true;
    };
    let mut ev = ItemDropEvent::new(pid, instance.id(), stack);
    events.call(&mut ev);
    if ev.is_cancelled() || ev.item_stack.is_air() {
        return false;
    }

    let yaw_rad = pos.yaw.to_radians();
    let pitch_rad = pos.pitch.to_radians();
    let throw_vel = Vec3::new(
        f64::from(-yaw_rad.sin() * pitch_rad.cos()) * 0.25,
        f64::from(-pitch_rad.sin()) * 0.25 + 0.1,
        f64::from(yaw_rad.cos() * pitch_rad.cos()) * 0.25,
    );
    let drop_pos = pos.with_y(pos.y + 1.32);
    let eid = next_entity_id.fetch_add(1, Ordering::Relaxed);
    let entity = Entity::item(ev.item_stack, 10).with_velocity(throw_vel);
    spawn_world_entity_direct(sim, eid, instance, entity, drop_pos, events);
    true
}

fn open_player_window(
    sim: &mut WorldSimState,
    pid: PlayerId,
    inventory: Inventory,
    events: &GlobalEventHandler,
    next_entity_id: &AtomicI32,
) {
    if sim
        .players
        .get(pid)
        .is_some_and(|p| p.open_window.is_some())
    {
        close_player_window(sim, pid, false, events, next_entity_id);
    }
    if let Some(p) = sim.players.get_mut(pid) {
        let wid = p.next_window_id;
        p.next_window_id = (wid % 100) + 1;
        let _ = p.send_packet(&OpenWindowPacket {
            window_id: wid,
            window_type: inventory.inventory_type().window_type_id(),
            title: inventory.title(),
        });
        p.open_window = Some((wid, inventory));
        p.sync_active_windows();
    }
}

fn close_player_window(
    sim: &mut WorldSimState,
    pid: PlayerId,
    send_packet_to_client: bool,
    events: &GlobalEventHandler,
    next_entity_id: &AtomicI32,
) {
    let mut overflow = ItemStack::AIR;
    let mut closed_wid = None;
    if let Some(p) = sim.players.get_mut(pid) {
        p.inventory.cancel_drag();
        if !p.inventory.cursor_item().is_air() {
            let carried = p.inventory.cursor_item().clone();
            p.inventory.set_cursor_item(ItemStack::AIR);
            overflow = p.inventory.add_item(carried);
        }
        if let Some((wid, _)) = p.open_window.take() {
            closed_wid = Some(wid);
            if send_packet_to_client {
                let _ = p.send_packet(&CloseWindowPacket { window_id: wid });
            }
        }
        p.sync_window_zero();
    }
    sim.broadcast_player_equipment(pid);
    if !overflow.is_air() {
        let _ = drop_item_from_player(sim, pid, overflow, events, next_entity_id);
    }
    if let Some(wid) = closed_wid {
        let mut ev = InventoryCloseEvent {
            player_id: pid,
            window_id: wid,
        };
        events.call(&mut ev);
    }
}

fn drain_inv_commands(
    sim: &mut WorldSimState,
    inv_cmd_rx: &Receiver<PlayerInvCommand>,
    events: &GlobalEventHandler,
    next_entity_id: &AtomicI32,
) {
    while let Ok(cmd) = inv_cmd_rx.try_recv() {
        match cmd {
            PlayerInvCommand::GiveItem { player_id, item } => {
                let mut overflow = ItemStack::AIR;
                if let Some(p) = sim.players.get_mut(player_id) {
                    overflow = p.inventory.add_item(item);
                    p.sync_active_windows();
                }
                sim.broadcast_player_equipment(player_id);
                if !overflow.is_air() {
                    let _ = drop_item_from_player(sim, player_id, overflow, events, next_entity_id);
                }
            }
            PlayerInvCommand::SetSlot {
                player_id,
                slot,
                item,
            } => {
                if let Some(p) = sim.players.get_mut(player_id) {
                    let _ = p.inventory.set_slot(slot, item);
                    p.sync_active_windows();
                }
                sim.broadcast_player_equipment(player_id);
            }
            PlayerInvCommand::OpenWindow {
                player_id,
                inventory,
            } => {
                open_player_window(sim, player_id, inventory, events, next_entity_id);
            }
            PlayerInvCommand::CloseWindow { player_id } => {
                close_player_window(sim, player_id, true, events, next_entity_id);
            }
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
                entity,
                position,
            } => {
                spawn_world_entity_direct(sim, network_id, instance, *entity, position, events);
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
                } else if let Some(&pid) = sim.player_by_entity_id.get(&network_id) {
                    if let Some(p) = sim.players.get_mut(pid) {
                        p.velocity = velocity;
                    }
                    sim.send_to_viewers_and_self(
                        network_id,
                        &EntityVelocityPacket {
                            entity_id: network_id,
                            velocity,
                        },
                    );
                }
            }
            EntityCommand::DamageByNetworkId { target_id, damage } => {
                apply_damage_to_target(sim, events, target_id, damage);
            }
            EntityCommand::DamagePlayer { player_id, damage } => {
                if let Some(eid) = sim.players.get(player_id).map(|p| p.entity_id) {
                    apply_damage_to_target(sim, events, eid, damage);
                }
            }
            EntityCommand::HealPlayer { player_id } => {
                if let Some(p) = sim.players.get_mut(player_id) {
                    p.dead = false;
                    p.health = p.max_health;
                    p.metadata.set_health(p.health);
                    p.metadata.set_pose(EntityPose::Standing);
                    let _ = p.send_packet(&UpdateHealthPacket {
                        health: p.health,
                        food: 20,
                        food_saturation: 5.0,
                    });
                }
                sync_player_dirty_metadata(sim, player_id);
            }
            EntityCommand::SetAttributeBase {
                target_id,
                attribute,
                base_value,
            } => {
                set_attribute_on_target(sim, target_id, attribute, base_value, None);
            }
            EntityCommand::AddAttributeModifier {
                target_id,
                attribute,
                modifier,
            } => {
                set_attribute_on_target(sim, target_id, attribute, 0.0, Some(modifier));
            }
            EntityCommand::SetPlayerAttributeBase {
                player_id,
                attribute,
                base_value,
            } => {
                if let Some(eid) = sim.players.get(player_id).map(|p| p.entity_id) {
                    set_attribute_on_target(sim, eid, attribute, base_value, None);
                }
            }
            EntityCommand::AddPlayerAttributeModifier {
                player_id,
                attribute,
                modifier,
            } => {
                if let Some(eid) = sim.players.get(player_id).map(|p| p.entity_id) {
                    set_attribute_on_target(sim, eid, attribute, 0.0, Some(modifier));
                }
            }
            EntityCommand::AddEffect { target_id, potion } => {
                add_effect_on_target(sim, target_id, potion);
            }
            EntityCommand::RemoveEffect { target_id, effect } => {
                remove_effect_on_target(sim, target_id, effect);
            }
            EntityCommand::AddPlayerEffect { player_id, potion } => {
                if let Some(eid) = sim.players.get(player_id).map(|p| p.entity_id) {
                    add_effect_on_target(sim, eid, potion);
                }
            }
            EntityCommand::RemovePlayerEffect { player_id, effect } => {
                if let Some(eid) = sim.players.get(player_id).map(|p| p.entity_id) {
                    remove_effect_on_target(sim, eid, effect);
                }
            }
        }
    }
}

fn set_attribute_on_target(
    sim: &mut WorldSimState,
    target_id: i32,
    attribute: Attribute,
    base_value: f64,
    modifier: Option<AttributeModifier>,
) {
    let mut sync_props = Vec::new();
    let mut new_player_health = None;
    if let Some(&pid) = sim.player_by_entity_id.get(&target_id) {
        if let Some(p) = sim.players.get_mut(pid) {
            if let Some(m) = modifier {
                p.attributes.add_modifier(attribute, m);
            } else {
                p.attributes.set_base_value(attribute, base_value);
            }
            if attribute == Attribute::MAX_HEALTH {
                #[allow(clippy::cast_possible_truncation)]
                let mh = p.attributes.get_value(Attribute::MAX_HEALTH) as f32;
                p.max_health = mh;
                if p.health > mh {
                    p.health = mh;
                    p.metadata.set_health(p.health);
                }
                new_player_health = Some((pid, p.health));
            }
            sync_props = p.attributes.take_dirty_wire_properties();
        }
    } else if let Some(we) = sim.entities.get_mut(&target_id)
        && let Some(liv) = &mut we.entity.living
    {
        if let Some(m) = modifier {
            liv.attributes.add_modifier(attribute, m);
        } else {
            liv.attributes.set_base_value(attribute, base_value);
        }
        if attribute == Attribute::MAX_HEALTH {
            #[allow(clippy::cast_possible_truncation)]
            let mh = liv.attributes.get_value(Attribute::MAX_HEALTH) as f32;
            liv.max_health = mh;
            if liv.health > mh {
                liv.health = mh;
                we.entity.core.metadata.set_health(liv.health);
            }
        }
        sync_props = liv.attributes.take_dirty_wire_properties();
    }
    if !sync_props.is_empty() {
        sim.send_to_viewers_and_self(
            target_id,
            &EntityAttributesPacket {
                entity_id: target_id,
                properties: sync_props,
            },
        );
    }
    if let Some((pid, hp)) = new_player_health {
        if let Some(p) = sim.players.get(pid) {
            let _ = p.send_packet(&UpdateHealthPacket {
                health: hp,
                food: 20,
                food_saturation: 5.0,
            });
        }
        sync_player_dirty_metadata(sim, pid);
    }
}

fn add_effect_on_target(sim: &mut WorldSimState, target_id: i32, potion: Potion) {
    let mut applied = false;
    if let Some(&pid) = sim.player_by_entity_id.get(&target_id) {
        if let Some(p) = sim.players.get_mut(pid) {
            p.effects.add(potion);
            applied = true;
        }
    } else if let Some(we) = sim.entities.get_mut(&target_id)
        && let Some(liv) = &mut we.entity.living
    {
        liv.effects.add(potion);
        applied = true;
    }
    if applied {
        sim.send_to_viewers_and_self(target_id, &potion.to_add_packet(target_id));
    }
}

fn remove_effect_on_target(sim: &mut WorldSimState, target_id: i32, effect: PotionEffect) {
    let mut removed = false;
    if let Some(&pid) = sim.player_by_entity_id.get(&target_id) {
        if let Some(p) = sim.players.get_mut(pid) {
            removed = p.effects.remove(effect);
        }
    } else if let Some(we) = sim.entities.get_mut(&target_id)
        && let Some(liv) = &mut we.entity.living
    {
        removed = liv.effects.remove(effect);
    }
    if removed {
        sim.send_to_viewers_and_self(target_id, &Potion::to_remove_packet(effect, target_id));
    }
}

fn tick_players_gameplay(sim: &mut WorldSimState) {
    let mut expired_packets = Vec::new();
    let mut attr_packets = Vec::new();
    for p in sim.players.values_mut() {
        p.invulnerable_ticks = p.invulnerable_ticks.saturating_sub(1);
        for eff in p.effects.tick() {
            expired_packets.push((p.entity_id, Potion::to_remove_packet(eff, p.entity_id)));
        }
        let dirty = p.attributes.take_dirty_wire_properties();
        if !dirty.is_empty() {
            attr_packets.push((
                p.entity_id,
                EntityAttributesPacket {
                    entity_id: p.entity_id,
                    properties: dirty,
                },
            ));
        }
    }
    for (eid, pkt) in expired_packets {
        sim.send_to_viewers_and_self(eid, &pkt);
    }
    for (eid, pkt) in attr_packets {
        sim.send_to_viewers_and_self(eid, &pkt);
    }
}

fn melee_weapon_bonus(material: Material) -> f32 {
    match material.key() {
        "minecraft:wooden_sword" | "minecraft:golden_sword" => 3.0,
        "minecraft:stone_sword" | "minecraft:copper_sword" => 4.0,
        "minecraft:iron_sword" => 5.0,
        "minecraft:diamond_sword" | "minecraft:wooden_axe" | "minecraft:golden_axe" => 6.0,
        "minecraft:netherite_sword" => 7.0,
        "minecraft:stone_axe"
        | "minecraft:copper_axe"
        | "minecraft:iron_axe"
        | "minecraft:diamond_axe" => 8.0,
        "minecraft:netherite_axe" => 9.0,
        _ => 0.0,
    }
}

#[allow(clippy::too_many_lines)]
fn apply_damage_to_target(
    sim: &mut WorldSimState,
    events: &GlobalEventHandler,
    target_id: i32,
    damage: Damage,
) {
    let bypass_gm = matches!(
        damage.damage_type,
        DamageType::OUT_OF_WORLD | DamageType::GENERIC_KILL
    );
    let target_info = if let Some(&pid) = sim.player_by_entity_id.get(&target_id) {
        let Some(p) = sim.players.get(pid) else {
            return;
        };
        if p.dead || (!bypass_gm && matches!(p.game_mode, GameMode::Creative | GameMode::Spectator))
        {
            return;
        }
        Some((Some(pid), p.instance.id(), EntityType::PLAYER, p.position))
    } else if let Some(we) = sim.entities.get(&target_id) {
        let Some(liv) = &we.entity.living else {
            return;
        };
        if liv.dead || we.entity.core.should_remove {
            return;
        }
        Some((
            None,
            we.instance.id(),
            we.entity.entity_type(),
            we.entity.core.position,
        ))
    } else {
        None
    };

    let Some((target_pid, inst_id, etype, target_pos)) = target_info else {
        return;
    };

    let mut dmg_ev = EntityDamageEvent::new(target_id, target_pid, inst_id, damage);
    events.call(&mut dmg_ev);
    if dmg_ev.is_cancelled() || dmg_ev.damage.amount <= 0.0 {
        return;
    }

    let final_dmg = dmg_ev.damage;
    let mut died = false;
    let mut kb_packet_for_player = None;
    let mut player_hp_update = None;

    if let Some(pid) = target_pid {
        if let Some(p) = sim.players.get_mut(pid) {
            p.health = (p.health - final_dmg.amount).max(0.0);
            p.invulnerable_ticks = 10;
            p.metadata.set_health(p.health);
            if let Some(src_pos) = final_dmg.source_pos {
                let kb_res = p.attributes.get_value(Attribute::KNOCKBACK_RESISTANCE);
                p.velocity = compute_knockback_velocity(
                    p.velocity,
                    src_pos,
                    p.position,
                    0.4,
                    kb_res,
                    p.on_ground,
                );
                kb_packet_for_player = Some(p.velocity);
            }
            if p.health <= 0.0 {
                p.dead = true;
                p.metadata.set_pose(EntityPose::Dying);
                died = true;
            }
            player_hp_update = Some((pid, p.health));
        }
        sync_player_dirty_metadata(sim, pid);
    } else if let Some(we) = sim.entities.get_mut(&target_id)
        && let Some(liv) = &mut we.entity.living
    {
        liv.health = (liv.health - final_dmg.amount).max(0.0);
        liv.invulnerable_ticks = 10;
        liv.last_damager_network_id = final_dmg.source_entity_id;
        we.entity.core.metadata.set_health(liv.health);
        if let Some(src_pos) = final_dmg.source_pos {
            let kb_res = liv.attributes.get_value(Attribute::KNOCKBACK_RESISTANCE);
            we.entity.core.velocity = compute_knockback_velocity(
                we.entity.core.velocity,
                src_pos,
                we.entity.core.position,
                0.4,
                kb_res,
                we.entity.core.on_ground,
            );
            we.velocity_dirty = true;
        }
        if liv.health <= 0.0 {
            liv.dead = true;
            we.entity.core.should_remove = true;
            died = true;
        }
    }

    if dmg_ev.animation {
        sim.send_to_viewers_and_self(target_id, &final_dmg.to_packet(target_id));
        let hit_yaw = final_dmg.source_pos.map_or(0.0, |sp| {
            let dx = sp.x - target_pos.x;
            let dz = sp.z - target_pos.z;
            #[allow(clippy::cast_possible_truncation)]
            let angle = dz.atan2(dx).to_degrees() as f32 - target_pos.yaw;
            angle
        });
        sim.send_to_viewers_and_self(
            target_id,
            &HitAnimationPacket {
                entity_id: target_id,
                yaw: hit_yaw,
            },
        );
    }

    if let Some(vel) = kb_packet_for_player {
        sim.send_to_viewers_and_self(
            target_id,
            &EntityVelocityPacket {
                entity_id: target_id,
                velocity: vel,
            },
        );
    }

    if let Some((pid, hp)) = player_hp_update
        && let Some(p) = sim.players.get(pid)
    {
        let _ = p.send_packet(&UpdateHealthPacket {
            health: hp,
            food: 20,
            food_saturation: 5.0,
        });
    }

    if died {
        let mut death_ev = EntityDeathEvent {
            network_id: target_id,
            player_id: target_pid,
            entity_type: etype,
            instance_id: inst_id,
            position: target_pos,
        };
        events.call(&mut death_ev);
        if target_pid.is_none() {
            despawn_world_entity(sim, target_id, events);
        }
    }
}

fn respawn_dead_player(sim: &mut WorldSimState, pid: PlayerId, events: &GlobalEventHandler) {
    let Some((inst_id, default_respawn, is_dead)) = sim
        .players
        .get(pid)
        .map(|p| (p.instance.id(), p.respawn_point, p.dead))
    else {
        return;
    };
    if !is_dead {
        return;
    }

    let mut resp_ev = PlayerRespawnEvent {
        player_id: pid,
        instance_id: inst_id,
        respawn_position: default_respawn,
    };
    events.call(&mut resp_ev);
    let dest = resp_ev.respawn_position;

    let mut spawn_ev_info = None;
    if let Some(p) = sim.players.get_mut(pid) {
        p.dead = false;
        p.health = p.max_health;
        p.velocity = Vec3::ZERO;
        p.invulnerable_ticks = 20;
        let _ = p.effects.clear();
        p.metadata.set_health(p.health);
        p.metadata.set_pose(EntityPose::Standing);

        let dim = p.instance.dimension_type().clone();
        let _ = p.send_packet(&RespawnPacket {
            spawn_info: PlayerSpawnInfo {
                dimension_type: dim.protocol_id as i32,
                world: dim.key,
                hashed_seed: 0,
                game_mode: p.game_mode,
                previous_game_mode: None,
                is_debug: false,
                is_flat: true,
                death_location: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            copy_data: RespawnPacket::COPY_NONE,
        });
        let _ = p.send_packet(&PlayerAbilitiesPacket {
            flags: ActivePlayer::abilities_flags_for(p.game_mode),
            ..PlayerAbilitiesPacket::default()
        });
        let _ = p.send_packet(&UpdateHealthPacket {
            health: p.health,
            food: 20,
            food_saturation: 5.0,
        });
        let attrs = p.attributes.all_synced_wire_properties();
        if !attrs.is_empty() {
            let _ = p.send_packet(&EntityAttributesPacket {
                entity_id: p.entity_id,
                properties: attrs,
            });
        }
        p.sync_window_zero();
        spawn_ev_info = Some((p.profile.username.clone(), p.instance.clone()));
    }

    teleport_player_server_side(sim, pid, dest);
    sync_player_dirty_metadata(sim, pid);

    if let Some((username, instance)) = spawn_ev_info {
        let mut spawn_ev = PlayerSpawnEvent {
            player_id: pid,
            username,
            position: dest,
            instance,
            first_spawn: false,
        };
        events.call(&mut spawn_ev);
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

#[allow(clippy::too_many_lines)]
fn tick_world_entities(sim: &mut WorldSimState, events: &GlobalEventHandler) {
    // Build per-instance target snapshots for creature AI selectors
    let mut candidates_by_instance: FxHashMap<InstanceId, Vec<TargetCandidate>> =
        FxHashMap::default();
    for (pid, p) in &sim.players {
        if matches!(p.game_mode, GameMode::Creative | GameMode::Spectator) {
            continue;
        }
        candidates_by_instance
            .entry(p.instance.id())
            .or_default()
            .push(TargetCandidate {
                network_id: p.entity_id,
                player_id: Some(pid),
                entity_type: EntityType::PLAYER,
                position: p.position,
                dead: p.dead,
            });
    }
    for (&eid, we) in &sim.entities {
        if let Some(liv) = &we.entity.living
            && !we.entity.core.should_remove
        {
            candidates_by_instance
                .entry(we.instance.id())
                .or_default()
                .push(TargetCandidate {
                    network_id: eid,
                    player_id: None,
                    entity_type: we.entity.entity_type(),
                    position: we.entity.core.position,
                    dead: liv.dead,
                });
        }
    }

    let eids: Vec<i32> = sim.entities.keys().copied().collect();
    let mut to_remove = Vec::new();
    let mut instances_needing_reconcile = FxHashSet::default();
    let mut ai_melee_attacks: Vec<(i32, Pos, i32, f32)> = Vec::new();

    for eid in eids {
        let mut sync_step = None;
        let mut dirty_meta = None;
        let mut send_vel = None;
        let mut expired_effects = Vec::new();
        let mut dirty_attrs = Vec::new();

        if let Some(we) = sim.entities.get_mut(&eid) {
            let inst_id = we.instance.id();
            if we.entity.role == EntityRole::Creature
                && let Some(liv) = &mut we.entity.living
            {
                liv.nearby_targets = candidates_by_instance
                    .get(&inst_id)
                    .cloned()
                    .unwrap_or_default();
            }

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

            if let Some(liv) = &mut we.entity.living {
                expired_effects = std::mem::take(&mut liv.expired_effects);
                dirty_attrs = liv.attributes.take_dirty_wire_properties();
                for hit in liv.pending_melee_hits.drain(..) {
                    ai_melee_attacks.push((eid, new_pos, hit.target_network_id, hit.damage));
                }
            }

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

        if !dirty_attrs.is_empty() {
            sim.send_to_viewers(
                eid,
                &EntityAttributesPacket {
                    entity_id: eid,
                    properties: dirty_attrs,
                },
            );
        }

        for eff in expired_effects {
            sim.send_to_viewers(eid, &Potion::to_remove_packet(eff, eid));
        }
    }

    for inst_id in instances_needing_reconcile {
        sim.reconcile_instance_players(inst_id);
    }

    // Resolve queued AI melee strikes
    for (attacker_eid, attacker_pos, victim_eid, dmg_amount) in ai_melee_attacks {
        sim.send_to_viewers(
            attacker_eid,
            &EntityAnimationPacket {
                entity_id: attacker_eid,
                animation: EntityAnimation::SwingMainArm,
            },
        );
        apply_damage_to_target(
            sim,
            events,
            victim_eid,
            Damage::from_mob(attacker_eid, attacker_pos, dmg_amount),
        );
    }

    // Check dropped ItemEntities ready for pickup (`pickup_delay_ticks == 0`)
    let item_candidates: Vec<(i32, InstanceId, Pos, ItemStack)> = sim
        .entities
        .iter()
        .filter_map(|(&eid, we)| {
            if we.entity.core.should_remove || to_remove.contains(&eid) {
                return None;
            }
            match &we.entity.role {
                EntityRole::DroppedItem {
                    item_stack,
                    pickup_delay_ticks: 0,
                } if !item_stack.is_air() => Some((
                    eid,
                    we.instance.id(),
                    we.entity.core.position,
                    item_stack.clone(),
                )),
                _ => None,
            }
        })
        .collect();

    for (eid, inst_id, item_pos, item_stack) in item_candidates {
        let mut best_collector: Option<(PlayerId, i32, f64)> = None;
        for (pid, p) in &sim.players {
            if p.instance.id() != inst_id || p.game_mode == GameMode::Spectator {
                continue;
            }
            let dx = item_pos.x - p.position.x;
            let dz = item_pos.z - p.position.z;
            let dy = item_pos.y - p.position.y;
            let horiz_sq = dx * dx + dz * dz;
            if horiz_sq <= 2.25
                && (-0.5..=2.0).contains(&dy)
                && best_collector.is_none_or(|(_, _, prev)| horiz_sq < prev)
            {
                best_collector = Some((pid, p.entity_id, horiz_sq));
            }
        }

        let Some((collector_pid, collector_eid, _)) = best_collector else {
            continue;
        };

        let mut ev = PickupItemEvent::new(collector_pid, inst_id, eid, item_stack);
        events.call(&mut ev);
        if ev.is_cancelled() || ev.item_stack.is_air() {
            continue;
        }

        let orig_amount = ev.item_stack.amount();
        let mut leftover = ev.item_stack.clone();
        if let Some(collector) = sim.players.get_mut(collector_pid) {
            leftover = collector.inventory.add_item(ev.item_stack);
            if leftover.amount() < orig_amount {
                collector.sync_active_windows();
            }
        }
        let picked = orig_amount.saturating_sub(leftover.amount());
        if picked == 0 {
            continue;
        }

        sim.broadcast_player_equipment(collector_pid);
        let collect_pkt = CollectItemPacket {
            collected_entity_id: eid,
            collector_entity_id: collector_eid,
            pickup_item_count: picked as i32,
        };
        sim.send_to_viewers(eid, &collect_pkt);
        if !sim
            .tracker
            .viewers(eid)
            .is_some_and(|v| v.contains(&collector_pid))
            && let Some(collector) = sim.players.get(collector_pid)
        {
            let _ = collector.send_packet(&collect_pkt);
        }

        if leftover.is_air() {
            to_remove.push(eid);
        } else if let Some(we) = sim.entities.get_mut(&eid) {
            we.entity.role = EntityRole::DroppedItem {
                item_stack: leftover.clone(),
                pickup_delay_ticks: 0,
            };
            we.entity
                .core
                .metadata
                .set(8, MetadataValue::ItemStack(leftover));
            let entries = we.entity.core.metadata.take_dirty();
            if !entries.is_empty() {
                sim.send_to_viewers(
                    eid,
                    &EntityMetaDataPacket {
                        entity_id: eid,
                        entries,
                    },
                );
            }
        }
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
    next_entity_id: &AtomicI32,
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
    let mut pending_give: Vec<ItemStack> = Vec::new();
    let mut pending_open_inv: Option<Inventory> = None;

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
                SenderEffect::GiveItem(item) => pending_give.push(item),
                SenderEffect::OpenInventory(inv) => pending_open_inv = Some(inv),
            }
        }
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

fn drain_env_commands(sim: &WorldSimState, env_cmd_rx: &Receiver<WorldEnvCommand>) {
    while let Ok(cmd) = env_cmd_rx.try_recv() {
        match cmd {
            WorldEnvCommand::Border {
                instance_id,
                old_diameter,
                border,
                lerp_ms,
            } => {
                sim.send_to_instance_players(
                    instance_id,
                    &InitializeWorldBorderPacket {
                        x: border.center_x,
                        z: border.center_z,
                        old_diameter,
                        new_diameter: border.diameter,
                        speed: lerp_ms.max(0),
                        portal_teleport_boundary: border.portal_teleport_boundary,
                        warning_time: border.warning_time,
                        warning_blocks: border.warning_blocks,
                    },
                );
            }
            WorldEnvCommand::Time { instance_id, time } => {
                sim.send_to_instance_players(instance_id, &time.to_set_time_packet());
            }
            WorldEnvCommand::Weather {
                instance_id,
                weather,
            } => {
                sim.send_to_instance_players(
                    instance_id,
                    &ChangeGameStatePacket {
                        reason: if weather.is_raining() {
                            GameStateReason::BeginRaining
                        } else {
                            GameStateReason::EndRaining
                        },
                        value: 0.0,
                    },
                );
                sim.send_to_instance_players(
                    instance_id,
                    &ChangeGameStatePacket {
                        reason: GameStateReason::RainLevelChange,
                        value: weather.rain_level,
                    },
                );
                sim.send_to_instance_players(
                    instance_id,
                    &ChangeGameStatePacket {
                        reason: GameStateReason::ThunderLevelChange,
                        value: weather.thunder_level,
                    },
                );
            }
        }
    }
}

fn drain_block_commands(sim: &WorldSimState, block_cmd_rx: &Receiver<WorldBlockCommand>) {
    while let Ok(cmd) = block_cmd_rx.try_recv() {
        match cmd {
            WorldBlockCommand::BroadcastSingle {
                instance,
                position,
                block,
                nbt,
            } => {
                let cpos = position.chunk_pos();
                sim.send_to_storage_chunk_viewers(
                    &instance,
                    cpos,
                    &BlockChangePacket {
                        position,
                        block_state_id: i32::from(block.state_id()),
                    },
                );
                if let Some(be_id) = block.block_entity_id() {
                    sim.send_to_storage_chunk_viewers(
                        &instance,
                        cpos,
                        &BlockEntityDataPacket {
                            position,
                            block_entity_type: i32::from(be_id),
                            data: nbt,
                        },
                    );
                }
                if let Some(light_pkt) = instance.build_light_update_packet(cpos) {
                    sim.send_to_storage_chunk_viewers(&instance, cpos, &light_pkt);
                }
            }
            WorldBlockCommand::BroadcastBatch { instance, packets } => {
                let mut touched_chunks = FxHashSet::default();
                for (cpos, pkt) in packets {
                    sim.send_to_storage_chunk_viewers(&instance, cpos, &pkt);
                    touched_chunks.insert(cpos);
                }
                if instance.is_lighting_enabled() {
                    for cpos in touched_chunks {
                        if let Some(light_pkt) = instance.build_light_update_packet(cpos) {
                            sim.send_to_storage_chunk_viewers(&instance, cpos, &light_pkt);
                        }
                    }
                }
            }
        }
    }
}

fn face_offset(face: BlockFace) -> (i32, i32, i32) {
    match face {
        BlockFace::Down => (0, -1, 0),
        BlockFace::Up => (0, 1, 0),
        BlockFace::North => (0, 0, -1),
        BlockFace::South => (0, 0, 1),
        BlockFace::West => (-1, 0, 0),
        BlockFace::East => (1, 0, 0),
    }
}

fn process_block_digging(
    pid: PlayerId,
    sim: &WorldSimState,
    events: &GlobalEventHandler,
    block_manager: &BlockManager,
    pkt: ClientPlayerActionPacket,
) {
    let Some(player) = sim.players.get(pid) else {
        return;
    };
    let _ = player.send_packet(&AcknowledgeBlockChangePacket {
        sequence: pkt.sequence,
    });

    let should_break = match pkt.status {
        PlayerActionStatus::StartedDigging => player.game_mode == GameMode::Creative,
        PlayerActionStatus::FinishedDigging => {
            matches!(player.game_mode, GameMode::Survival | GameMode::Creative)
        }
        _ => false,
    };
    if !should_break {
        return;
    }

    let instance = player.instance.clone();
    let inst_id = instance.id();
    let current = instance.get_block(pkt.position);
    if current.is_air() {
        return;
    }

    let mut ev = PlayerBlockBreakEvent::new(
        pid,
        player.profile.username.clone(),
        inst_id,
        pkt.position,
        pkt.block_face,
        current,
    );
    events.call(&mut ev);

    if ev.is_cancelled() {
        let _ = player.send_packet(&BlockChangePacket {
            position: pkt.position,
            block_state_id: i32::from(current.state_id()),
        });
        if let Some(be_id) = current.block_entity_id() {
            let _ = player.send_packet(&BlockEntityDataPacket {
                position: pkt.position,
                block_entity_type: i32::from(be_id),
                data: instance.get_block_nbt(pkt.position),
            });
        }
        return;
    }

    let handler = instance
        .get_block_handler(pkt.position)
        .or_else(|| block_manager.get_handler(current.key()));
    if let Some(h) = handler {
        h.on_destroy(&BlockDestroyContext {
            block: current,
            new_block: ev.result_block,
            instance_id: inst_id,
            position: pkt.position,
            player_id: Some(pid),
        });
    }

    let cpos = pkt.position.chunk_pos();
    instance.set_block(pkt.position, ev.result_block);
    sim.send_to_storage_chunk_viewers(
        &instance,
        cpos,
        &BlockChangePacket {
            position: pkt.position,
            block_state_id: i32::from(ev.result_block.state_id()),
        },
    );
    if let Some(light_pkt) = instance.build_light_update_packet(cpos) {
        sim.send_to_storage_chunk_viewers(&instance, cpos, &light_pkt);
    }
}

fn process_block_placement(
    pid: PlayerId,
    sim: &mut WorldSimState,
    events: &GlobalEventHandler,
    block_manager: &BlockManager,
    pkt: ClientPlayerBlockPlacementPacket,
) {
    let Some((game_mode, instance, sneaking, player_pos, player_name, held_item)) =
        sim.players.get(pid).map(|player| {
            let _ = player.send_packet(&AcknowledgeBlockChangePacket {
                sequence: pkt.sequence,
            });
            let held = match pkt.hand {
                Hand::Main => player.inventory.item_in_main_hand().clone(),
                Hand::Off => player.inventory.item_in_off_hand().clone(),
            };
            (
                player.game_mode,
                player.instance.clone(),
                player.metadata.is_sneaking(),
                player.position,
                player.profile.username.clone(),
                held,
            )
        })
    else {
        return;
    };
    if game_mode == GameMode::Spectator || game_mode == GameMode::Adventure {
        return;
    }

    let inst_id = instance.id();
    let clicked_block = instance.get_block(pkt.position);
    let cursor = Vec3::new(
        f64::from(pkt.cursor_x),
        f64::from(pkt.cursor_y),
        f64::from(pkt.cursor_z),
    );

    if !sneaking {
        let handler = instance
            .get_block_handler(pkt.position)
            .or_else(|| block_manager.get_handler(clicked_block.key()));
        if let Some(h) = handler
            && !h.on_interact(&BlockInteractContext {
                block: clicked_block,
                instance_id: inst_id,
                position: pkt.position,
                block_face: pkt.block_face,
                cursor_position: cursor,
                player_id: pid,
                hand: pkt.hand,
            })
        {
            return;
        }
    }

    let default_candidate = if !held_item.is_air() {
        let Some(b) = held_item.material().corresponding_block() else {
            return;
        };
        b
    } else if game_mode == GameMode::Creative {
        Block::STONE
    } else {
        return;
    };

    let place_pos = if clicked_block.is_air() || clicked_block.is_liquid() {
        pkt.position
    } else {
        let (dx, dy, dz) = face_offset(pkt.block_face);
        pkt.position.offset(dx, dy, dz)
    };
    let previous = instance.get_block(place_pos);
    if !previous.is_air() && !previous.is_liquid() {
        if let Some(player) = sim.players.get(pid) {
            let _ = player.send_packet(&BlockChangePacket {
                position: place_pos,
                block_state_id: i32::from(previous.state_id()),
            });
        }
        return;
    }

    let resolved_block = if let Some(rule) = block_manager.get_placement_rule(default_candidate) {
        let Some(b) = rule.block_place(&PlacementState {
            world: &instance,
            block: default_candidate,
            block_face: pkt.block_face,
            place_position: place_pos,
            cursor_position: cursor,
            player_position: player_pos,
            is_player_shifting: sneaking,
        }) else {
            if let Some(player) = sim.players.get(pid) {
                let _ = player.send_packet(&BlockChangePacket {
                    position: place_pos,
                    block_state_id: i32::from(previous.state_id()),
                });
            }
            return;
        };
        b
    } else {
        default_candidate
    };

    // Prevent placing a solid block inside the placing player's own AABB
    let player_bbox = if sneaking {
        BoundingBox::PLAYER_SNEAKING
    } else {
        BoundingBox::PLAYER
    };
    let block_bbox = BoundingBox::symmetric(1.0, 1.0);
    let block_anchor = Pos::new(
        f64::from(place_pos.x) + 0.5,
        f64::from(place_pos.y),
        f64::from(place_pos.z) + 0.5,
    );
    if resolved_block.is_solid()
        && player_bbox.intersects_at(&player_pos, &block_bbox, &block_anchor)
    {
        if let Some(player) = sim.players.get(pid) {
            let _ = player.send_packet(&BlockChangePacket {
                position: place_pos,
                block_state_id: i32::from(previous.state_id()),
            });
        }
        return;
    }

    let mut ev = PlayerBlockPlaceEvent::new(
        pid,
        player_name,
        inst_id,
        place_pos,
        pkt.block_face,
        pkt.hand,
        resolved_block,
    );
    events.call(&mut ev);

    if ev.is_cancelled() {
        if let Some(player) = sim.players.get_mut(pid) {
            let _ = player.send_packet(&BlockChangePacket {
                position: place_pos,
                block_state_id: i32::from(previous.state_id()),
            });
            player.sync_window_zero();
        }
        return;
    }

    if !held_item.is_air() && game_mode == GameMode::Survival {
        if let Some(player) = sim.players.get_mut(pid) {
            let next_amt = held_item.amount().saturating_sub(1);
            let updated = held_item.with_amount(next_amt);
            match pkt.hand {
                Hand::Main => player.inventory.set_item_in_main_hand(updated),
                Hand::Off => player.inventory.set_item_in_off_hand(updated),
            }
            player.sync_window_zero();
        }
        sim.broadcast_player_equipment(pid);
    }

    let handler = block_manager.get_handler(ev.block.key());
    instance.set_block_with_data(place_pos, ev.block, ev.block_nbt.clone(), handler.clone());
    if let Some(h) = handler {
        h.on_place(&BlockPlacementContext {
            block: ev.block,
            previous_block: previous,
            instance_id: inst_id,
            position: place_pos,
            player_id: Some(pid),
        });
    }

    let cpos = place_pos.chunk_pos();
    sim.send_to_storage_chunk_viewers(
        &instance,
        cpos,
        &BlockChangePacket {
            position: place_pos,
            block_state_id: i32::from(ev.block.state_id()),
        },
    );
    if let Some(be_id) = ev.block.block_entity_id() {
        sim.send_to_storage_chunk_viewers(
            &instance,
            cpos,
            &BlockEntityDataPacket {
                position: place_pos,
                block_entity_type: i32::from(be_id),
                data: ev.block_nbt,
            },
        );
    }
    if let Some(light_pkt) = instance.build_light_update_packet(cpos) {
        sim.send_to_storage_chunk_viewers(&instance, cpos, &light_pkt);
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

#[allow(clippy::too_many_lines)]
fn handle_play_packet(
    pid: PlayerId,
    sim: &mut WorldSimState,
    events: &GlobalEventHandler,
    block_manager: &BlockManager,
    command_manager: &CommandManager,
    next_entity_id: &AtomicI32,
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
            let Some((attacker_eid, inst_id, attacker_pos, is_dead, gm, attack_dmg)) =
                sim.players.get(pid).map(|p| {
                    #[allow(clippy::cast_possible_truncation)]
                    let base = p.attributes.get_value(Attribute::ATTACK_DAMAGE) as f32;
                    let weapon = melee_weapon_bonus(p.inventory.item_in_main_hand().material());
                    let strength = p
                        .effects
                        .get(PotionEffect::STRENGTH)
                        .map_or(0.0, |s| (s.amplifier + 1).max(0) as f32 * 3.0);
                    (
                        p.entity_id,
                        p.instance.id(),
                        p.position,
                        p.dead,
                        p.game_mode,
                        (base + weapon + strength).max(0.5),
                    )
                })
            else {
                return true;
            };
            if is_dead || gm == GameMode::Spectator {
                return true;
            }
            let mut ev = PlayerAttackEntityEvent::new(pid, attacker_eid, pkt.target_id, inst_id);
            events.call(&mut ev);
            if !ev.is_cancelled() {
                apply_damage_to_target(
                    sim,
                    events,
                    pkt.target_id,
                    Damage::from_player(attacker_eid, attacker_pos, attack_dmg),
                );
            }
            true
        }
        0x07 => {
            let Ok(pkt) = decode_packet_exact::<ClientCommandChatPacket>(frame.payload) else {
                return false;
            };
            dispatch_player_command(
                pid,
                sim,
                events,
                command_manager,
                next_entity_id,
                pkt.message,
            );
            true
        }
        0x08 => {
            let Ok(pkt) = decode_packet_exact::<ClientSignedCommandChatPacket>(frame.payload)
            else {
                return false;
            };
            dispatch_player_command(
                pid,
                sim,
                events,
                command_manager,
                next_entity_id,
                pkt.message,
            );
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
        0x0C => {
            let Ok(pkt) = decode_packet_exact::<ClientStatusPacket>(frame.payload) else {
                return false;
            };
            if pkt.action == ClientStatusAction::PerformRespawn {
                respawn_dead_player(sim, pid, events);
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
        0x11 => decode_packet_exact::<ClientClickWindowButtonPacket>(frame.payload).is_ok(),
        0x12 => {
            let Ok(pkt) = decode_packet_exact::<ClientClickWindowPacket>(frame.payload) else {
                return false;
            };
            let mut pre_ev = InventoryPreClickEvent::new(
                pid,
                pkt.window_id,
                pkt.slot,
                pkt.button,
                pkt.click_type,
            );
            events.call(&mut pre_ev);

            if pre_ev.is_cancelled() {
                if let Some(p) = sim.players.get_mut(pid) {
                    p.sync_active_windows();
                }
                return true;
            }

            let mut dropped_items = Vec::new();
            let mut shared_inv_to_sync = None;
            if let Some(p) = sim.players.get_mut(pid) {
                let creative = p.game_mode == GameMode::Creative;
                let valid_target = if pkt.window_id == 0 {
                    Some(None)
                } else if let Some((wid, inv)) = &p.open_window
                    && *wid == pkt.window_id
                {
                    Some(Some(inv.clone()))
                } else {
                    None
                };

                if let Some(open_inv) = valid_target {
                    let outcome = process_inventory_click(
                        &mut p.inventory,
                        open_inv.as_ref(),
                        pkt.slot,
                        pkt.button,
                        pkt.click_type,
                        creative,
                    );
                    dropped_items = outcome.dropped_items;
                    shared_inv_to_sync = open_inv;
                }
                p.sync_active_windows();
            }

            sim.broadcast_player_equipment(pid);
            if let Some(inv) = shared_inv_to_sync {
                sim.sync_shared_inventory_viewers(&inv, Some(pid));
            }
            for stack in dropped_items {
                if !drop_item_from_player(sim, pid, stack.clone(), events, next_entity_id)
                    && let Some(p) = sim.players.get_mut(pid)
                {
                    let _ = p.inventory.add_item(stack);
                    p.sync_active_windows();
                }
            }
            true
        }
        0x13 => {
            let Ok(_pkt) = decode_packet_exact::<ClientCloseWindowPacket>(frame.payload) else {
                return false;
            };
            close_player_window(sim, pid, false, events, next_entity_id);
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
        0x29 => {
            let Ok(pkt) = decode_packet_exact::<ClientPlayerActionPacket>(frame.payload) else {
                return false;
            };
            match pkt.status {
                PlayerActionStatus::DropItem | PlayerActionStatus::DropItemStack => {
                    let drop_all = pkt.status == PlayerActionStatus::DropItemStack;
                    let mut to_drop = ItemStack::AIR;
                    if let Some(p) = sim.players.get_mut(pid) {
                        let held = p.inventory.item_in_main_hand().clone();
                        if !held.is_air() {
                            if drop_all {
                                to_drop = held;
                                p.inventory.set_item_in_main_hand(ItemStack::AIR);
                            } else {
                                let rem = held.amount() - 1;
                                to_drop = held.clone().with_amount(1);
                                p.inventory.set_item_in_main_hand(held.with_amount(rem));
                            }
                        }
                    }
                    if !to_drop.is_air() {
                        if !drop_item_from_player(sim, pid, to_drop.clone(), events, next_entity_id)
                            && let Some(p) = sim.players.get_mut(pid)
                        {
                            let _ = p.inventory.add_item(to_drop);
                        }
                        if let Some(p) = sim.players.get_mut(pid) {
                            p.sync_active_windows();
                        }
                        sim.broadcast_player_equipment(pid);
                    }
                }
                PlayerActionStatus::SwapItemHand => {
                    if let Some(p) = sim.players.get_mut(pid) {
                        let main = p.inventory.item_in_main_hand().clone();
                        let off = p.inventory.item_in_off_hand().clone();
                        p.inventory.set_item_in_main_hand(off);
                        p.inventory.set_item_in_off_hand(main);
                        p.sync_active_windows();
                    }
                    sim.broadcast_player_equipment(pid);
                }
                _ => process_block_digging(pid, sim, events, block_manager, pkt),
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
        0x35 => {
            let Ok(pkt) = decode_packet_exact::<ClientHeldItemChangePacket>(frame.payload) else {
                return false;
            };
            if let Ok(slot_u8) = u8::try_from(pkt.slot)
                && slot_u8 <= 8
            {
                if let Some(p) = sim.players.get_mut(pid) {
                    let _ = p.inventory.set_held_slot(slot_u8);
                }
                sim.broadcast_player_equipment(pid);
            }
            true
        }
        0x38 => {
            let Ok(pkt) = decode_packet_exact::<ClientCreativeInventoryActionPacket>(frame.payload)
            else {
                return false;
            };
            let is_creative = sim
                .players
                .get(pid)
                .is_some_and(|p| p.game_mode == GameMode::Creative);
            if is_creative {
                if pkt.slot < 0 {
                    let _ = drop_item_from_player(sim, pid, pkt.item, events, next_entity_id);
                } else if let Ok(slot_idx) = usize::try_from(pkt.slot)
                    && slot_idx < PlayerInventory::SIZE
                {
                    if let Some(p) = sim.players.get_mut(pid) {
                        let _ = p.inventory.set_slot(slot_idx, pkt.item);
                        p.sync_window_zero();
                    }
                    sim.broadcast_player_equipment(pid);
                }
            }
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
        0x42 => {
            let Ok(pkt) = decode_packet_exact::<ClientPlayerBlockPlacementPacket>(frame.payload)
            else {
                return false;
            };
            process_block_placement(pid, sim, events, block_manager, pkt);
            true
        }
        0x43 => {
            let Ok(_pkt) = decode_packet_exact::<ClientUseItemPacket>(frame.payload) else {
                return false;
            };
            true
        }
        _ => true,
    }
}
