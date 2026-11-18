//! Block digging, vanilla placement rules (`ReplacementContext`, `PlacementState`), sub-voxel
//! entity collision validation, 2-block door lifecycle, interactive block toggling, and bounded
//! BFS neighbor `BlockPlacementRule::block_update` propagation.

use std::collections::VecDeque;

use minestom_block::{
    Block, BlockDestroyContext, BlockGetter, BlockInteractContext, BlockManager,
    BlockPlacementContext, PlacementState, ReplacementContext, UpdateState,
};
use minestom_core::{BlockVec, ChunkPos, Direction, GameMode, Hand, PlayerId, Point, Vec3};
use minestom_entity::BoundingBox;
use minestom_event::{CancellableEvent, GlobalEventHandler};
use minestom_protocol::block::{
    AcknowledgeBlockChangePacket, BlockChangePacket, BlockEntityDataPacket,
    ClientPlayerActionPacket, ClientPlayerBlockPlacementPacket, PlayerActionStatus,
};
use minestom_world::InstanceContainer;
use rustc_hash::FxHashSet;

use super::WorldSimState;
use crate::event::{PlayerBlockBreakEvent, PlayerBlockPlaceEvent};

fn is_hand_toggleable_openable(block: Block) -> bool {
    let k = block.key();
    if k == "minecraft:iron_door" || k == "minecraft:iron_trapdoor" {
        return false;
    }
    (k.ends_with("_door") || k.ends_with("_trapdoor") || k.ends_with("_fence_gate"))
        && block.get_property("open").is_some()
}

fn try_toggle_openable(
    sim: &WorldSimState,
    instance: &InstanceContainer,
    block_manager: &BlockManager,
    pos: BlockVec,
    clicked_block: Block,
) -> bool {
    if !is_hand_toggleable_openable(clicked_block) {
        return false;
    }
    let next_open = if clicked_block.get_property("open") == Some("true") {
        "false"
    } else {
        "true"
    };
    let Ok(toggled) = clicked_block.with_property("open", next_open) else {
        return false;
    };

    let mut changed = vec![pos];
    let mut touched_chunks = FxHashSet::default();
    instance.set_block(pos, toggled);
    let cpos = pos.chunk_pos();
    touched_chunks.insert(cpos);
    sim.send_to_storage_chunk_viewers(
        instance,
        cpos,
        &BlockChangePacket {
            position: pos,
            block_state_id: i32::from(toggled.state_id()),
        },
    );

    if let Some(half) = clicked_block.get_property("half") {
        let partner_pos = if half == "lower" {
            pos.offset(0, 1, 0)
        } else {
            pos.offset(0, -1, 0)
        };
        let partner = instance.get_block(partner_pos);
        if partner.key() == clicked_block.key()
            && let Ok(partner_toggled) = partner.with_property("open", next_open)
        {
            instance.set_block(partner_pos, partner_toggled);
            let pcpos = partner_pos.chunk_pos();
            touched_chunks.insert(pcpos);
            sim.send_to_storage_chunk_viewers(
                instance,
                pcpos,
                &BlockChangePacket {
                    position: partner_pos,
                    block_state_id: i32::from(partner_toggled.state_id()),
                },
            );
            changed.push(partner_pos);
        }
    }

    propagate_neighbor_updates(sim, instance, block_manager, &changed, &mut touched_chunks);
    sync_touched_chunk_lights(sim, instance, &touched_chunks);
    true
}

pub(super) fn propagate_neighbor_updates(
    sim: &WorldSimState,
    instance: &InstanceContainer,
    block_manager: &BlockManager,
    seed_positions: &[BlockVec],
    touched_chunks: &mut FxHashSet<ChunkPos>,
) {
    let mut queue: VecDeque<(BlockVec, usize)> = seed_positions.iter().map(|&p| (p, 0)).collect();
    let mut steps = 0_usize;

    while let Some((origin, depth)) = queue.pop_front() {
        if steps >= 512 {
            break;
        }
        for dir in Direction::ALL {
            let npos = origin.relative(dir);
            let current = instance.get_block(npos);
            if current.is_air() {
                continue;
            }
            let Some(rule) = block_manager.get_placement_rule(current) else {
                continue;
            };
            let max_dist = rule.max_update_distance();
            if max_dist == 0 || depth >= max_dist {
                continue;
            }
            steps += 1;
            let updated = rule.block_update(&UpdateState {
                world: instance,
                block_position: npos,
                current_block: current,
                from_face: dir.opposite(),
            });
            if updated != current {
                let handler = instance
                    .get_block_handler(npos)
                    .or_else(|| block_manager.get_handler(updated.key()));
                let nbt = instance.get_block_nbt(npos);
                instance.set_block_with_data(npos, updated, nbt, handler);
                let ncpos = npos.chunk_pos();
                touched_chunks.insert(ncpos);
                sim.send_to_storage_chunk_viewers(
                    instance,
                    ncpos,
                    &BlockChangePacket {
                        position: npos,
                        block_state_id: i32::from(updated.state_id()),
                    },
                );
                queue.push_back((npos, depth + 1));
            }
        }
    }
}

fn sync_touched_chunk_lights(
    sim: &WorldSimState,
    instance: &InstanceContainer,
    touched_chunks: &FxHashSet<ChunkPos>,
) {
    if !instance.is_lighting_enabled() {
        return;
    }
    for &cpos in touched_chunks {
        if let Some(light_pkt) = instance.build_light_update_packet(cpos) {
            sim.send_to_storage_chunk_viewers(instance, cpos, &light_pkt);
        }
    }
}

fn collides_with_any_entity_in_instance(
    sim: &WorldSimState,
    instance_id: minestom_core::InstanceId,
    pos: BlockVec,
    block: Block,
) -> bool {
    if !block.blocks_motion() {
        return false;
    }
    let eps = 1e-6;
    for p in sim.players.values() {
        if p.instance.id() != instance_id || p.game_mode == GameMode::Spectator || p.dead {
            continue;
        }
        let bbox = if p.metadata.is_sneaking() {
            BoundingBox::PLAYER_SNEAKING
        } else {
            BoundingBox::PLAYER
        };
        let hw = bbox.width() * 0.5;
        let min = Vec3::new(
            p.position.x - hw + eps,
            p.position.y + eps,
            p.position.z - hw + eps,
        );
        let max = Vec3::new(
            p.position.x + hw - eps,
            p.position.y + bbox.height() - eps,
            p.position.z + hw - eps,
        );
        if block.collides_world_aabb(pos, min, max) {
            return true;
        }
    }
    for we in sim.entities.values() {
        if we.instance.id() != instance_id || we.entity.core.should_remove {
            continue;
        }
        if we.entity.living.is_none() {
            continue;
        }
        let bbox = we.entity.core.bounding_box;
        let epos = we.entity.core.position;
        let hw = bbox.width() * 0.5;
        let min = Vec3::new(epos.x - hw + eps, epos.y + eps, epos.z - hw + eps);
        let max = Vec3::new(
            epos.x + hw - eps,
            epos.y + bbox.height() - eps,
            epos.z + hw - eps,
        );
        if block.collides_world_aabb(pos, min, max) {
            return true;
        }
    }
    false
}

pub(super) fn process_block_digging(
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
        sim.player_ref(pid),
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

    let mut changed = vec![pkt.position];
    let mut touched_chunks = FxHashSet::default();
    let cpos = pkt.position.chunk_pos();
    touched_chunks.insert(cpos);
    instance.set_block(pkt.position, ev.result_block);
    sim.send_to_storage_chunk_viewers(
        &instance,
        cpos,
        &BlockChangePacket {
            position: pkt.position,
            block_state_id: i32::from(ev.result_block.state_id()),
        },
    );

    // If breaking either half of a 2-block door/tall block, break its partner half too!
    if ev.result_block.is_air()
        && let Some(half) = current.get_property("half")
    {
        let partner_pos = if half == "lower" {
            pkt.position.offset(0, 1, 0)
        } else if half == "upper" {
            pkt.position.offset(0, -1, 0)
        } else {
            pkt.position
        };
        if partner_pos != pkt.position {
            let partner = instance.get_block(partner_pos);
            if partner.key() == current.key() {
                instance.set_block(partner_pos, Block::AIR);
                let pcpos = partner_pos.chunk_pos();
                touched_chunks.insert(pcpos);
                sim.send_to_storage_chunk_viewers(
                    &instance,
                    pcpos,
                    &BlockChangePacket {
                        position: partner_pos,
                        block_state_id: i32::from(Block::AIR.state_id()),
                    },
                );
                changed.push(partner_pos);
            }
        }
    }

    propagate_neighbor_updates(sim, &instance, block_manager, &changed, &mut touched_chunks);
    sync_touched_chunk_lights(sim, &instance, &touched_chunks);
}

#[allow(clippy::too_many_lines)]
pub(super) fn process_block_placement(
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
    if matches!(game_mode, GameMode::Spectator | GameMode::Adventure) {
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
        if try_toggle_openable(sim, &instance, block_manager, pkt.position, clicked_block) {
            return;
        }
    }

    if held_item.is_air() {
        return;
    }
    let Some(default_candidate) = held_item.material().corresponding_block() else {
        return;
    };

    let clicked_self_replaceable =
        block_manager
            .get_placement_rule(clicked_block)
            .is_some_and(|r| {
                r.is_self_replaceable(&ReplacementContext {
                    world: &instance,
                    current_block: clicked_block,
                    placing_block: default_candidate,
                    block_face: pkt.block_face,
                    cursor_position: cursor,
                })
            });

    let place_pos = if clicked_self_replaceable || clicked_block.is_replaceable() {
        pkt.position
    } else {
        pkt.position.relative(pkt.block_face)
    };

    let dim = instance.dimension_type();
    if place_pos.y < dim.min_y || place_pos.y >= dim.max_y() {
        return;
    }

    let previous = instance.get_block(place_pos);
    let target_self_replaceable = if place_pos == pkt.position {
        clicked_self_replaceable
    } else {
        block_manager.get_placement_rule(previous).is_some_and(|r| {
            r.is_self_replaceable(&ReplacementContext {
                world: &instance,
                current_block: previous,
                placing_block: default_candidate,
                block_face: pkt.block_face.opposite(),
                cursor_position: cursor,
            })
        })
    };

    if !target_self_replaceable && !previous.is_replaceable() {
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

    // Sub-voxel entity collision check across ALL players & living entities in this instance!
    if collides_with_any_entity_in_instance(sim, inst_id, place_pos, resolved_block) {
        if let Some(player) = sim.players.get(pid) {
            let _ = player.send_packet(&BlockChangePacket {
                position: place_pos,
                block_state_id: i32::from(previous.state_id()),
            });
        }
        return;
    }

    // Check upper-half clearance & collision for 2-block doors (`half=lower`)
    if resolved_block.get_property("half") == Some("lower") {
        let upper_pos = place_pos.offset(0, 1, 0);
        if upper_pos.y >= dim.max_y() || !instance.get_block(upper_pos).is_replaceable() {
            if let Some(player) = sim.players.get(pid) {
                let _ = player.send_packet(&BlockChangePacket {
                    position: place_pos,
                    block_state_id: i32::from(previous.state_id()),
                });
            }
            return;
        }
        if let Ok(upper_block) = resolved_block.with_property("half", "upper")
            && collides_with_any_entity_in_instance(sim, inst_id, upper_pos, upper_block)
        {
            if let Some(player) = sim.players.get(pid) {
                let _ = player.send_packet(&BlockChangePacket {
                    position: place_pos,
                    block_state_id: i32::from(previous.state_id()),
                });
            }
            return;
        }
    }

    let mut ev = PlayerBlockPlaceEvent::new(
        sim.player_ref(pid),
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

    if game_mode == GameMode::Survival {
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

    let mut changed = vec![place_pos];
    let mut touched_chunks = FxHashSet::default();
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
    touched_chunks.insert(cpos);
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

    // Automatically place upper half of 2-block doors (`half=lower` -> `half=upper` at y + 1)!
    if ev.block.get_property("half") == Some("lower")
        && let Ok(upper_block) = ev.block.with_property("half", "upper")
    {
        let upper_pos = place_pos.offset(0, 1, 0);
        instance.set_block(upper_pos, upper_block);
        let ucpos = upper_pos.chunk_pos();
        touched_chunks.insert(ucpos);
        sim.send_to_storage_chunk_viewers(
            &instance,
            ucpos,
            &BlockChangePacket {
                position: upper_pos,
                block_state_id: i32::from(upper_block.state_id()),
            },
        );
        changed.push(upper_pos);
    }

    propagate_neighbor_updates(sim, &instance, block_manager, &changed, &mut touched_chunks);
    sync_touched_chunk_lights(sim, &instance, &touched_chunks);
}
