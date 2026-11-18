//! Unified FIFO cross-thread command channel (`ServerCommand`) and async-to-tick `TickMessage`
//! definitions (`ADR 0002`).
//!
//! All server-handle, `PlayerRef`, and `EntityRef` mutations travel over a single bounded FIFO
//! `flume` channel so cross-subsystem operations (e.g. `player.set_slot(...)` followed by
//! `player.open_inventory(...)`, or `player.send_message(...)` followed by `player.kick(...)`)
//! execute in exact causal order on the 20 TPS tick loop.

use std::sync::atomic::AtomicI32;

use bytes::Bytes;
use flume::{Receiver, Sender};
use minestom_block::Block;
use minestom_core::{BlockVec, ChunkPos, GameMode, InstanceId, PlayerId, Point, Pos, Vec3};
use minestom_entity::{Attribute, AttributeModifier, Damage, Entity, Potion, PotionEffect};
use minestom_event::GlobalEventHandler;
use minestom_item::{Inventory, ItemStack};
use minestom_nbt::NbtCompound;
use minestom_protocol::RawPacketFrame;
use minestom_protocol::block::{BlockChangePacket, BlockEntityDataPacket, MultiBlockChangePacket};
use minestom_protocol::chat_command::SystemChatPacket;
use minestom_protocol::encode_packet_payload;
use minestom_protocol::entity::{
    EntityHeadLookPacket, EntityMetaDataPacket, EntityPose, EntityPositionSyncPacket,
    EntityVelocityPacket, MetadataValue,
};
use minestom_protocol::login::GameProfile;
use minestom_protocol::play::{
    ChangeGameStatePacket, GameStateReason, InitializeWorldBorderPacket, PlayDisconnectPacket,
    UpdateHealthPacket,
};
use minestom_text::Component;
use minestom_world::{InstanceContainer, Weather, WorldBorder, WorldTime};
use rustc_hash::FxHashSet;
use uuid::Uuid;

use super::WorldSimState;
use super::combat::{
    add_effect_on_target, apply_damage_to_target, despawn_world_entity, remove_effect_on_target,
    set_attribute_on_target, spawn_world_entity_direct,
};
use super::play_packets::{
    close_player_window, drop_item_from_player, open_player_window, sync_player_dirty_metadata,
};
use super::{set_player_game_mode_and_sync, teleport_player_server_side};

/// Outbound command sent from the tick thread to a player's Tokio writer task.
pub(crate) enum OutboundCommand {
    /// Send a pre-encoded `[VarInt packet_id][payload]` body.
    Packet(Bytes),
    /// Send a final packet (if any) and close the TCP connection.
    Disconnect(Option<Bytes>),
}

/// Unified FIFO command sent from [`crate::MinecraftServer`], [`crate::PlayerRef`], or
/// [`crate::EntityRef`] to the dedicated 20 TPS tick thread.
pub(crate) enum ServerCommand {
    Entity(EntityCommand),
    Player(PlayerControlCommand),
    Block(WorldBlockCommand),
    Env(WorldEnvCommand),
    Inv(PlayerInvCommand),
    Ui(UiCommand),
}

/// Cross-thread command mutating non-player world entities (or targeting by `network_id`).
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
    /// Set velocity on an existing world entity or player.
    SetVelocity { network_id: i32, velocity: Vec3 },
    /// Update a metadata entry on a spawned world entity and broadcast to viewers.
    UpdateMetadata {
        network_id: i32,
        index: u8,
        value: MetadataValue,
    },
    /// Update a bitmask flag on a spawned world entity and broadcast to viewers.
    UpdateBitMask {
        network_id: i32,
        index: u8,
        bit: u8,
        enabled: bool,
    },
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

/// Cross-thread player lifecycle, movement, and gamemode control command.
pub(crate) enum PlayerControlCommand {
    Teleport {
        player_id: PlayerId,
        position: Pos,
    },
    SetVelocity {
        player_id: PlayerId,
        velocity: Vec3,
    },
    SetGameMode {
        player_id: PlayerId,
        mode: GameMode,
    },
    SetRespawnPoint {
        player_id: PlayerId,
        position: Pos,
    },
    SendMessage {
        player_id: PlayerId,
        message: Component,
    },
    Kick {
        player_id: PlayerId,
        reason: Component,
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
#[derive(Clone, Copy)]
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

/// Cross-thread UI/media/scoreboard/resource-pack/dialog packet routing command.
pub(crate) enum UiCommand {
    ToPlayer {
        player_id: PlayerId,
        payload: Bytes,
    },
    ToPlayers {
        player_ids: Vec<PlayerId>,
        payload: Bytes,
    },
    ToInstance {
        instance_id: InstanceId,
        payload: Bytes,
    },
    ToAll {
        payload: Bytes,
    },
    TrackResourcePack {
        player_id: PlayerId,
        pack_id: Uuid,
        forced: bool,
        push_payload: Bytes,
    },
}

/// Message sent from async connection tasks into the dedicated 20 TPS tick loop.
pub(crate) enum TickMessage {
    /// A player completed Configuration and is ready to join Play.
    RegisterPlayer {
        entity_id: i32,
        profile: GameProfile,
        game_mode: minestom_core::GameMode,
        permission_level: u8,
        main_hand: minestom_core::MainHand,
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

#[allow(clippy::too_many_lines)]
pub(super) fn drain_server_commands(
    sim: &mut WorldSimState,
    cmd_rx: &Receiver<ServerCommand>,
    events: &GlobalEventHandler,
    next_entity_id: &AtomicI32,
) {
    while let Ok(cmd) = cmd_rx.try_recv() {
        match cmd {
            ServerCommand::Entity(ec) => handle_entity_command(sim, ec, events),
            ServerCommand::Player(pc) => handle_player_command(sim, pc, events),
            ServerCommand::Block(bc) => handle_block_command(sim, bc),
            ServerCommand::Env(ec) => handle_env_command(sim, ec),
            ServerCommand::Inv(ic) => handle_inv_command(sim, ic, events, next_entity_id),
            ServerCommand::Ui(uc) => handle_ui_command(sim, uc),
        }
    }
}

#[allow(clippy::too_many_lines)]
fn handle_entity_command(sim: &mut WorldSimState, cmd: EntityCommand, events: &GlobalEventHandler) {
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
            } else if let Some(&pid) = sim.player_by_entity_id.get(&network_id) {
                teleport_player_server_side(sim, pid, position);
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
        EntityCommand::UpdateMetadata {
            network_id,
            index,
            value,
        } => {
            let mut dirty_entries = None;
            if let Some(we) = sim.entities.get_mut(&network_id) {
                we.entity.core.metadata.set(index, value);
                if we.entity.core.metadata.has_dirty() {
                    dirty_entries = Some(we.entity.core.metadata.take_dirty());
                }
            }
            if let Some(entries) = dirty_entries {
                sim.send_to_viewers(
                    network_id,
                    &EntityMetaDataPacket {
                        entity_id: network_id,
                        entries,
                    },
                );
            }
        }
        EntityCommand::UpdateBitMask {
            network_id,
            index,
            bit,
            enabled,
        } => {
            let mut dirty_entries = None;
            if let Some(we) = sim.entities.get_mut(&network_id) {
                let mut cur = match we.entity.core.metadata.get(index) {
                    Some(MetadataValue::Byte(b)) => *b,
                    _ => 0,
                };
                if enabled {
                    cur |= bit as i8;
                } else {
                    cur &= !(bit as i8);
                }
                we.entity.core.metadata.set(index, MetadataValue::Byte(cur));
                if we.entity.core.metadata.has_dirty() {
                    dirty_entries = Some(we.entity.core.metadata.take_dirty());
                }
            }
            if let Some(entries) = dirty_entries {
                sim.send_to_viewers(
                    network_id,
                    &EntityMetaDataPacket {
                        entity_id: network_id,
                        entries,
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
            add_effect_on_target(sim, events, target_id, potion);
        }
        EntityCommand::RemoveEffect { target_id, effect } => {
            remove_effect_on_target(sim, target_id, effect);
        }
        EntityCommand::AddPlayerEffect { player_id, potion } => {
            if let Some(eid) = sim.players.get(player_id).map(|p| p.entity_id) {
                add_effect_on_target(sim, events, eid, potion);
            }
        }
        EntityCommand::RemovePlayerEffect { player_id, effect } => {
            if let Some(eid) = sim.players.get(player_id).map(|p| p.entity_id) {
                remove_effect_on_target(sim, eid, effect);
            }
        }
    }
}

fn handle_player_command(
    sim: &mut WorldSimState,
    cmd: PlayerControlCommand,
    events: &GlobalEventHandler,
) {
    match cmd {
        PlayerControlCommand::Teleport {
            player_id,
            position,
        } => {
            teleport_player_server_side(sim, player_id, position);
        }
        PlayerControlCommand::SetVelocity {
            player_id,
            velocity,
        } => {
            let mut eid_opt = None;
            if let Some(p) = sim.players.get_mut(player_id) {
                p.velocity = velocity;
                eid_opt = Some(p.entity_id);
            }
            if let Some(eid) = eid_opt {
                sim.send_to_viewers_and_self(
                    eid,
                    &EntityVelocityPacket {
                        entity_id: eid,
                        velocity,
                    },
                );
            }
        }
        PlayerControlCommand::SetGameMode { player_id, mode } => {
            set_player_game_mode_and_sync(sim, player_id, mode);
        }
        PlayerControlCommand::SetRespawnPoint {
            player_id,
            position,
        } => {
            if let Some(p) = sim.players.get_mut(player_id) {
                p.respawn_point = position;
            }
        }
        PlayerControlCommand::SendMessage { player_id, message } => {
            if let Some(p) = sim.players.get(player_id) {
                let _ = p.send_packet(&SystemChatPacket {
                    message,
                    overlay: false,
                });
            }
        }
        PlayerControlCommand::Kick { player_id, reason } => {
            if let Some(p) = sim.players.get(player_id) {
                let kick = encode_packet_payload(&PlayDisconnectPacket { message: reason }).ok();
                let _ = p.outbound_tx.try_send(OutboundCommand::Disconnect(kick));
            }
            sim.remove_and_cleanup_player(player_id, events);
        }
    }
}

fn handle_inv_command(
    sim: &mut WorldSimState,
    cmd: PlayerInvCommand,
    events: &GlobalEventHandler,
    next_entity_id: &AtomicI32,
) {
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

fn handle_env_command(sim: &WorldSimState, cmd: WorldEnvCommand) {
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

fn handle_block_command(sim: &WorldSimState, cmd: WorldBlockCommand) {
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

fn handle_ui_command(sim: &mut WorldSimState, cmd: UiCommand) {
    match cmd {
        UiCommand::ToPlayer { player_id, payload } => {
            if let Some(p) = sim.players.get(player_id) {
                let _ = p.send_encoded(payload);
            }
        }
        UiCommand::ToPlayers {
            player_ids,
            payload,
        } => {
            for pid in player_ids {
                if let Some(p) = sim.players.get(pid) {
                    let _ = p.send_encoded(payload.clone());
                }
            }
        }
        UiCommand::ToInstance {
            instance_id,
            payload,
        } => {
            for p in sim.players.values() {
                if p.instance.id() == instance_id {
                    let _ = p.send_encoded(payload.clone());
                }
            }
        }
        UiCommand::ToAll { payload } => {
            for p in sim.players.values() {
                let _ = p.send_encoded(payload.clone());
            }
        }
        UiCommand::TrackResourcePack {
            player_id,
            pack_id,
            forced,
            push_payload,
        } => {
            if let Some(p) = sim.players.get_mut(player_id) {
                if forced {
                    p.forced_resource_packs.insert(pack_id);
                }
                let _ = p.send_encoded(push_payload);
            }
        }
    }
}
