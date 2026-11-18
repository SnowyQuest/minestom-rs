//! Inbound `ConnectionState::Play` packet decoding & dispatch, container window open/close,
//! item dropping, and data-driven player melee combat.

use std::sync::atomic::{AtomicI32, Ordering};

use minestom_block::BlockManager;
use minestom_command::CommandManager;
use minestom_core::{GameMode, Hand, PlayerId, Vec3};
use minestom_entity::{
    Attribute, Damage, Entity, PlayerAttackEntityEvent, PlayerEntityInteractEvent,
    PlayerHandAnimationEvent, PotionEffect,
};
use minestom_event::{CancellableEvent, GlobalEventHandler};
use minestom_item::{
    Inventory, InventoryCloseEvent, InventoryPreClickEvent, ItemDropEvent, ItemStack,
    PlayerInventory, process_inventory_click,
};
use minestom_protocol::block::{
    ClientPlayerActionPacket, ClientPlayerBlockPlacementPacket, ClientUseItemPacket,
    PlayerActionStatus,
};
use minestom_protocol::chat_command::{
    ClientChatMessagePacket, ClientCommandChatPacket, ClientSignedCommandChatPacket,
    ClientTabCompletePacket, SystemChatPacket, TabCompletePacket,
};
use minestom_protocol::config::ClientSettingsPacket;
use minestom_protocol::entity::{
    ClientAnimationPacket, ClientAttackPacket, ClientEntityAction, ClientEntityActionPacket,
    ClientInputPacket, ClientInteractEntityPacket, EntityAnimation, EntityAnimationPacket,
    EntityMetaDataPacket, EntityPose,
};
use minestom_protocol::inventory::{
    ClientClickWindowButtonPacket, ClientClickWindowPacket, ClientCloseWindowPacket,
    ClientCreativeInventoryActionPacket, ClientHeldItemChangePacket, CloseWindowPacket,
    OpenWindowPacket,
};
use minestom_protocol::play::{
    ClientPlayKeepAlivePacket, ClientPlayerPositionAndRotationPacket, ClientPlayerPositionPacket,
    ClientPlayerRotationPacket, ClientStatusAction, ClientStatusPacket,
    ClientTeleportConfirmPacket, PlayDisconnectPacket,
};
use minestom_protocol::ui_media::{ClientResourcePackStatusPacket, ResourcePackStatus};
use minestom_protocol::{
    PacketPayload, RawPacketFrame, decode_packet_exact, encode_packet_payload,
};
use minestom_text::Component;
use tracing::debug;

use super::block_ops::{process_block_digging, process_block_placement};
use super::combat::{apply_damage_to_target, respawn_dead_player, spawn_world_entity_direct};
use super::commands::OutboundCommand;
use super::{WorldSimState, dispatch_player_command, process_movement};
use crate::event::PlayerChatEvent;
use crate::features::PlayerResourcePackStatusEvent;

pub(super) fn drop_item_from_player(
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

pub(super) fn open_player_window(
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

pub(super) fn close_player_window(
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

pub(super) fn sync_player_dirty_metadata(sim: &mut WorldSimState, pid: PlayerId) {
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
pub(super) fn handle_play_packet(
    pid: PlayerId,
    sim: &mut WorldSimState,
    events: &GlobalEventHandler,
    block_manager: &BlockManager,
    command_manager: &CommandManager,
    next_entity_id: &AtomicI32,
    frame: RawPacketFrame,
) -> bool {
    const PLAY_CLIENT_SETTINGS_ID: i32 = 0x0E;

    match frame.packet_id {
        ClientTeleportConfirmPacket::PACKET_ID => {
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
        ClientAttackPacket::PACKET_ID => {
            let Ok(pkt) = decode_packet_exact::<ClientAttackPacket>(frame.payload) else {
                return false;
            };
            let Some((attacker_eid, inst_id, attacker_pos, is_dead, gm, attack_dmg)) =
                sim.players.get(pid).map(|p| {
                    #[allow(clippy::cast_possible_truncation)]
                    let base = p.attributes.get_value(Attribute::ATTACK_DAMAGE) as f32;
                    let weapon = p
                        .inventory
                        .item_in_main_hand()
                        .material()
                        .attack_damage_bonus();
                    #[allow(clippy::cast_precision_loss)]
                    let strength = p
                        .effects
                        .get(PotionEffect::STRENGTH)
                        .map_or(0.0, |s| (s.amplifier + 1).max(0) as f32 * 3.0);
                    #[allow(clippy::cast_precision_loss)]
                    let weakness = p
                        .effects
                        .get(PotionEffect::WEAKNESS)
                        .map_or(0.0, |w| (w.amplifier + 1).max(0) as f32 * 4.0);
                    (
                        p.entity_id,
                        p.instance.id(),
                        p.position,
                        p.dead,
                        p.game_mode,
                        (base + weapon + strength - weakness).max(0.5),
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
        ClientCommandChatPacket::PACKET_ID => {
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
        ClientSignedCommandChatPacket::PACKET_ID => {
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
        ClientChatMessagePacket::PACKET_ID => {
            let Ok(pkt) = decode_packet_exact::<ClientChatMessagePacket>(frame.payload) else {
                return false;
            };
            let Some(username) = sim.players.get(pid).map(|p| p.profile.username.clone()) else {
                return true;
            };
            let mut chat_ev = PlayerChatEvent::new(sim.player_ref(pid), pid, username, pkt.message);
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
        ClientStatusPacket::PACKET_ID => {
            let Ok(pkt) = decode_packet_exact::<ClientStatusPacket>(frame.payload) else {
                return false;
            };
            if pkt.action == ClientStatusAction::PerformRespawn {
                respawn_dead_player(sim, pid, events);
            }
            true
        }
        PLAY_CLIENT_SETTINGS_ID => {
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
        ClientTabCompletePacket::PACKET_ID => {
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
        ClientClickWindowButtonPacket::PACKET_ID => {
            decode_packet_exact::<ClientClickWindowButtonPacket>(frame.payload).is_ok()
        }
        ClientClickWindowPacket::PACKET_ID => {
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
        ClientCloseWindowPacket::PACKET_ID => {
            let Ok(_pkt) = decode_packet_exact::<ClientCloseWindowPacket>(frame.payload) else {
                return false;
            };
            close_player_window(sim, pid, false, events, next_entity_id);
            true
        }
        ClientInteractEntityPacket::PACKET_ID => {
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
        ClientPlayKeepAlivePacket::PACKET_ID => {
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
        ClientPlayerPositionPacket::PACKET_ID => {
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
        ClientPlayerPositionAndRotationPacket::PACKET_ID => {
            let Ok(pkt) =
                decode_packet_exact::<ClientPlayerPositionAndRotationPacket>(frame.payload)
            else {
                return false;
            };
            process_movement(pid, sim, events, pkt.position, (pkt.flags & 0x01) != 0);
            true
        }
        ClientPlayerRotationPacket::PACKET_ID => {
            let Ok(pkt) = decode_packet_exact::<ClientPlayerRotationPacket>(frame.payload) else {
                return false;
            };
            if let Some(cur) = sim.players.get(pid).map(|p| p.position) {
                let proposed = cur.with_yaw(pkt.yaw).with_pitch(pkt.pitch);
                process_movement(pid, sim, events, proposed, (pkt.flags & 0x01) != 0);
            }
            true
        }
        ClientPlayerActionPacket::PACKET_ID => {
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
        ClientEntityActionPacket::PACKET_ID => {
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
        ClientInputPacket::PACKET_ID => {
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
        ClientResourcePackStatusPacket::PACKET_ID => {
            let Ok(pkt) = decode_packet_exact::<ClientResourcePackStatusPacket>(frame.payload)
            else {
                return false;
            };
            let mut ev = PlayerResourcePackStatusEvent {
                player_id: pid,
                pack_id: pkt.id,
                status: pkt.status,
            };
            events.call(&mut ev);
            if let Some(p) = sim.players.get_mut(pid) {
                let is_forced = p.forced_resource_packs.contains(&pkt.id);
                if matches!(
                    pkt.status,
                    ResourcePackStatus::SuccessfullyLoaded | ResourcePackStatus::Discarded
                ) {
                    p.forced_resource_packs.remove(&pkt.id);
                } else if is_forced
                    && matches!(
                        pkt.status,
                        ResourcePackStatus::Declined
                            | ResourcePackStatus::FailedDownload
                            | ResourcePackStatus::InvalidUrl
                            | ResourcePackStatus::FailedReload
                    )
                {
                    let kick = encode_packet_payload(&PlayDisconnectPacket {
                        message: Component::translatable(
                            "multiplayer.requiredTexturePrompt.disconnect",
                        ),
                    })
                    .ok();
                    let _ = p.outbound_tx.try_send(OutboundCommand::Disconnect(kick));
                    sim.remove_and_cleanup_player(pid, events);
                }
            }
            true
        }
        ClientHeldItemChangePacket::PACKET_ID => {
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
        ClientCreativeInventoryActionPacket::PACKET_ID => {
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
        ClientAnimationPacket::PACKET_ID => {
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
        ClientPlayerBlockPlacementPacket::PACKET_ID => {
            let Ok(pkt) = decode_packet_exact::<ClientPlayerBlockPlacementPacket>(frame.payload)
            else {
                return false;
            };
            process_block_placement(pid, sim, events, block_manager, pkt);
            true
        }
        ClientUseItemPacket::PACKET_ID => {
            decode_packet_exact::<ClientUseItemPacket>(frame.payload).is_ok()
        }
        _ => true,
    }
}
