//! 20 TPS entity simulation, AI stepping, item pickup, attributes, potion effects (including
//! instantaneous effects), damage, knockback, death, and player respawning.

use minestom_core::{GameMode, InstanceId, PlayerId, Point, Pos, Vec3};
use minestom_entity::{
    Attribute, AttributeModifier, Damage, DamageType, Entity, EntityDamageEvent, EntityDeathEvent,
    EntityDespawnEvent, EntityRole, EntitySpawnEvent, EntityType, PlayerRespawnEvent, Potion,
    PotionEffect, TargetCandidate, compute_knockback_velocity,
};
use minestom_event::{CancellableEvent, GlobalEventHandler};
use minestom_item::{ItemStack, PickupItemEvent};
use minestom_protocol::encode_packet_payload;
use minestom_protocol::entity::{
    DestroyEntitiesPacket, EntityAnimation, EntityAnimationPacket, EntityAttributesPacket,
    EntityMetaDataPacket, EntityPose, EntityVelocityPacket, HitAnimationPacket, MetadataValue,
};
use minestom_protocol::inventory::CollectItemPacket;
use minestom_protocol::play::{
    PlayerAbilitiesPacket, PlayerSpawnInfo, RespawnPacket, UpdateHealthPacket,
};
use minestom_world::InstanceContainer;
use rustc_hash::{FxHashMap, FxHashSet};

use super::play_packets::sync_player_dirty_metadata;
use super::{ActivePlayer, SpawnedWorldEntity, WorldSimState, teleport_player_server_side};
use crate::event::PlayerSpawnEvent;

pub(super) fn spawn_world_entity_direct(
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

pub(super) fn despawn_world_entity(
    sim: &mut WorldSimState,
    network_id: i32,
    events: &GlobalEventHandler,
) {
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

pub(super) fn set_attribute_on_target(
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

pub(super) fn add_effect_on_target(
    sim: &mut WorldSimState,
    events: &GlobalEventHandler,
    target_id: i32,
    potion: Potion,
) {
    // Handle instantaneous effects (`INSTANT_HEALTH`, `INSTANT_DAMAGE`) immediately without storing!
    if potion.effect.is_instantaneous() {
        let amp_scale = 2.0_f32.powi(potion.amplifier.clamp(0, 10));
        if potion.effect == PotionEffect::INSTANT_HEALTH {
            let heal = 4.0 * amp_scale;
            if let Some(&pid) = sim.player_by_entity_id.get(&target_id) {
                let mut hp_out = None;
                if let Some(p) = sim.players.get_mut(pid)
                    && !p.dead
                {
                    p.health = (p.health + heal).min(p.max_health);
                    p.metadata.set_health(p.health);
                    hp_out = Some(p.health);
                }
                if let Some(hp) = hp_out {
                    if let Some(p) = sim.players.get(pid) {
                        let _ = p.send_packet(&UpdateHealthPacket {
                            health: hp,
                            food: 20,
                            food_saturation: 5.0,
                        });
                    }
                    sync_player_dirty_metadata(sim, pid);
                }
            } else if let Some(we) = sim.entities.get_mut(&target_id)
                && let Some(liv) = &mut we.entity.living
                && !liv.dead
            {
                liv.health = (liv.health + heal).min(liv.max_health);
                we.entity.core.metadata.set_health(liv.health);
                let entries = we.entity.core.metadata.take_dirty();
                if !entries.is_empty() {
                    sim.send_to_viewers(
                        target_id,
                        &EntityMetaDataPacket {
                            entity_id: target_id,
                            entries,
                        },
                    );
                }
            }
        } else if potion.effect == PotionEffect::INSTANT_DAMAGE {
            apply_damage_to_target(
                sim,
                events,
                target_id,
                Damage::of(DamageType::MAGIC, 6.0 * amp_scale),
            );
        }
        return;
    }

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

pub(super) fn remove_effect_on_target(
    sim: &mut WorldSimState,
    target_id: i32,
    effect: PotionEffect,
) {
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

pub(super) fn tick_players_gameplay(sim: &mut WorldSimState) {
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

#[allow(clippy::too_many_lines)]
pub(super) fn apply_damage_to_target(
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

pub(super) fn respawn_dead_player(
    sim: &mut WorldSimState,
    pid: PlayerId,
    events: &GlobalEventHandler,
) {
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
            player: sim.player_ref(pid),
            player_id: pid,
            username,
            position: dest,
            instance,
            first_spawn: false,
        };
        events.call(&mut spawn_ev);
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn tick_world_entities(sim: &mut WorldSimState, events: &GlobalEventHandler) {
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
