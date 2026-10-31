//! Native combat event planning; authoritative state is read-only.
use super::*;

pub(super) fn hit<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: bullet_hit::NetMessageHit,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let history_available = ctx.history_available;
    // Shooter identity comes from the acknowledged Horizon reservation;
    // the wire has no attacker or unique shot ID. Repeated bytes can be shots.
    let mut profile_item = None;
    let mut ordinary_retail_firearm = false;
    let mut damage_kind = None;
    let mut damage_calculation = None;
    let mut applied = None;
    let peer_target = ctx.engine.peers.values().find(|p| {
        0x03800000 | u32::from(p.reservation.player) == request.defender
            && p.ready_for_updates
            && p.mounted_seat.is_none()
    });
    let registry_target = ctx.engine.bot_combat_snapshots().into_iter().find(|b| {
        ctx.engine.is_registry_bot(b.object_index)
            && (0x03800000 | u32::from(b.object_index)) == request.defender
    });
    let retail_target = active.development_target.as_ref().filter(|target| {
        active.wire_profile.application_id() == 20231
            && request.defender == dummy::OBJECT_ID
            && active.dummy_created
            && target.roster_published
            && ctx.engine.development_target_replication_enabled(active)
    });
    let target_health = registry_target
        .filter(|b| b.health_quarter_points > 0)
        .map(|b| b.health_quarter_points)
        .or_else(|| {
            peer_target
                .and_then(|p| match p.life {
                    CharacterLife::Alive {
                        health_quarter_points,
                        ..
                    } => Some(health_quarter_points),
                    _ => None,
                })
                .or_else(|| {
                    if request.defender == dummy::OBJECT_ID
                        && (ctx.engine.development_dummy_enabled() || retail_target.is_some())
                        && retail_target.is_none_or(|t| {
                            t.runtime.is_none_or(|r| {
                                !matches!(r.phase, development_target::BotPhase::Dead)
                            })
                        })
                    {
                        active.dummy_health.filter(|hp| *hp > 0)
                    } else {
                        None
                    }
                })
        });
    if peer_target.is_none()
        && registry_target.is_none()
        && retail_target.is_none()
        && (!ctx.engine.development_dummy_enabled() || request.defender != dummy::OBJECT_ID)
    {
        plan.gameplay_denial = Some("unsupported_bullet_target");
    } else if !active.entered_game
        || !active.ready_for_updates
        || !active.life.is_alive()
        || active.mounted_seat.is_some_and(|seat| seat != 0)
    {
        plan.gameplay_denial = Some("bullet_requires_living_armed_shooter");
    } else if target_health.is_none() {
        plan.gameplay_denial = Some("bullet_target_not_alive");
    } else if !history_available {
        plan.gameplay_denial = Some("replication_history_limit");
    } else {
        let resolved = (|| {
            let profile = ctx.engine.bullet_profile(active, &request)?;
            ordinary_retail_firearm =
                matches!(profile, CharacterHitProfile::Retail(_)) && active.mounted_seat.is_none();
            profile_item = Some(profile.item_id());
            damage_kind = Some(profile.kind());
            let distance = request
                .hit_origin
                .iter()
                .zip(request.hit_dest)
                .map(|(a, b)| (*a - b) * (*a - b))
                .sum::<f32>()
                .sqrt();
            let target_inventory = peer_target
                .and_then(|p| p.current_inventory_handle)
                .map_or(0, |h| h.inventory_index);
            let target_kit = if let Some(target) = registry_target {
                ctx.engine
                    .registry_bot_loadout(target.object_index)
                    .ok_or("missing_target_loadout")?
                    .clone()
            } else if let Some(target) = retail_target {
                target.loadout.clone()
            } else if let Some(peer) = peer_target {
                if peer.wire_profile.application_id() == 20231 {
                    peer.reservation.character.replication_loadout(
                        &ctx.engine.item_catalog,
                        target_inventory,
                        peer.wire_profile,
                    )
                } else {
                    peer.reservation.character.loadout(
                        &ctx.engine.item_catalog,
                        target_inventory,
                        peer.wire_profile,
                    )
                }
                .ok_or("missing_target_loadout")?
            } else {
                ctx.engine
                    .item_catalog
                    .valor_loadout(target_inventory)
                    .ok_or("missing_target_loadout")?
            };
            let armor = if active.wire_profile.application_id() == 20231 {
                armor::ArmorProfile::from_retail_loadout(
                    &target_kit,
                    ctx.engine.item_catalog.retail_combat()?,
                )?
            } else {
                armor::ArmorProfile::from_loadout_with_catalog(
                    &target_kit,
                    &ctx.engine.item_catalog,
                )?
            }
            .with_modifiers(
                &peer_target.map_or_else(skill_catalog::OwnedModifiers::default, |p| {
                    plan.leadership.modifiers(p)
                }),
            );
            let (calculation, amount, damage) = damage::resolve_character_hit(
                profile.raw_character_damage(
                    distance,
                    request.penetration_fraction(),
                    request.hit_location,
                )?,
                armor,
                profile.kind(),
                0x03800000 | u32::from(active.reservation.player),
                request.client_data,
                target_health.unwrap(),
            )?;
            damage_calculation = Some(calculation);
            let Some(damage) = damage else {
                return Ok(None);
            };
            Ok::<_, &'static str>(Some((amount, damage)))
        })();
        match resolved {
            Err(code) => plan.gameplay_denial = Some(code),
            Ok(None) => (),
            Ok(Some((amount, damage))) => {
                if ordinary_retail_firearm
                    && (peer_target.is_some() || registry_target.is_some())
                    && let Some(notification) = ctx.engine.plan_firearm_death(
                        active.reservation.world,
                        active.reservation.player,
                        request.defender as u16,
                        profile_item.unwrap(),
                        request.hit_location,
                        damage,
                    )?
                {
                    plan.combat.death_notifications.push(notification);
                }
                if damage.health_quarter_points == 0 && peer_target.is_none() {
                    statistics::target_killed(
                        plan,
                        active,
                        registry_target.map_or_else(
                            || retail_target.map_or(1, |t| t.squad.team()),
                            |b| b.team,
                        ),
                    );
                }
                if damage.health_quarter_points == 0
                    && let Some(bot) = registry_target
                    && let Some(recipient) = progression::RewardRecipient::from_active(active)
                {
                    plan.combat.new_rewards.push(recipient.event(
                        format!(
                            "kill-{}-bot-{:04x}-{:08x}",
                            active.reservation.id, bot.object_index, bot.generation
                        ),
                        progression::RewardKind::EnemyKilled,
                        1,
                    ));
                }
                if damage.health_quarter_points == 0
                    && peer_target.is_none()
                    && request.defender == dummy::OBJECT_ID
                    && (ctx.engine.development_dummy_enabled() || retail_target.is_some())
                    && let Some(recipient) = progression::RewardRecipient::from_active(active)
                {
                    plan.combat.new_rewards.push(recipient.event(
                        if let Some(target) = retail_target {
                            format!(
                                "kill-{}-target-owner-{:08x}-bot-{:08x}",
                                active.reservation.id,
                                target.generation,
                                target.runtime.map_or(1, |r| r.generation)
                            )
                        } else {
                            format!("kill-{}-dummy-00000001", active.reservation.id)
                        },
                        progression::RewardKind::EnemyKilled,
                        1,
                    ));
                }
                let packet = replication::CharacterDamageUpdate {
                    sequence: next_sequence,
                    timestamp_ms: now.as_millis() as u32,
                    object_index: request.defender as u16,
                    damage,
                    forced_state: (damage.health_quarter_points == 0).then_some(
                        replication::CharacterState {
                            state: 7,
                            stance: 0,
                            user_data: [0; 3],
                        },
                    ),
                }
                .encode_with_profile(ctx.engine.wire_profile)?;
                applied = Some(json!({"amount_hp":amount,
                    "health_before_quarter_points":target_health,
                    "health_after_quarter_points":damage.health_quarter_points,
                    "replication_sequence":next_sequence,
                    "target_owner_life_generation":retail_target.map(|t| t.generation),
                    "target_bot_life_generation":retail_target.map(|t| t.runtime.map_or(1, |r| r.generation))}));
                if let Some(peer) = peer_target {
                    if let CharacterLife::Alive { generation, .. } = peer.life {
                        plan.combat.peer_damage.push((
                            peer.reservation.id.clone(),
                            generation,
                            damage,
                        ));
                    }
                    plan.replication.damage_delta = Some(packet.clone());
                } else if let Some(bot) = registry_target {
                    plan.combat
                        .bot_damage
                        .push((bot.object_index, bot.generation, damage));
                    plan.replication.damage_delta = Some(packet.clone());
                } else {
                    plan.combat.dummy_health = Some(damage.health_quarter_points);
                    if damage.health_quarter_points == 0
                        && let Some(runtime) = retail_target.and_then(|t| t.runtime)
                    {
                        plan.combat.bot_runtime = Some(development_target::BotRuntime {
                            phase: development_target::BotPhase::Dead,
                            since: now,
                            last_update: now,
                            physics_motion: None,
                            ..runtime
                        });
                    }
                    plan.replication.dummy_delta = Some(packet.clone());
                }
                plan.outgoing = Some(packet);
            }
        }
    }
    Ok(json!({"type":"bullet_hit","target_id":request.defender,
        "gear_index":request.gear_index,"magazine":request.fire_mode,
        "hit_location":request.hit_location,"client_data":request.client_data,
        "origin":request.hit_origin,"destination":request.hit_dest,
        "penetration_milli":request.penetration_milli,
        "weapon_item":profile_item,"damage_kind":damage_kind,"damage_calculation":damage_calculation,"applied_damage":applied,
        "hit_authority":"client_reported",
        "collision_audit":collision_audit(ctx.engine.collision_world.as_ref(), ctx.engine.map_name(), request.hit_origin, request.hit_dest),
        "ammo_authority":"observed_only_no_fire_event"}))
}

pub(super) fn fire_grenade<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::FireGrenade,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let history_available = ctx.history_available;
    let bytes = ctx.decoded.bytes;
    let owned = request.shooter_id == (0x03800000 | u32::from(active.reservation.player))
        && request.shooter_client_index == active.reservation.player;
    let eligible =
        active.entered_game && active.ready_for_updates && active.life.is_alive() && owned;
    let mut created = false;
    let kit = active.current_inventory_handle.and_then(|h| {
        active.reservation.character.replication_loadout(
            &ctx.engine.item_catalog,
            h.inventory_index,
            active.wire_profile,
        )
    });
    let grenade_item = kit
        .as_ref()
        .and_then(|k| k.inventories.get(k.handle.inventory_index as usize))
        .and_then(|i| i.slots[4]);
    let grenade = grenade_item.and_then(|id| ctx.engine.item_catalog.grenade(id).ok());
    if !owned {
        plan.gameplay_denial = Some("unowned_projectile_shooter");
    } else if !eligible || active.mounted_seat.is_some() {
        plan.gameplay_denial = Some("projectile_requires_living_on_foot_character");
    } else if request.ammo_index != 4 || request.projectile_index >= 3 || grenade.is_none() {
        plan.gameplay_denial = Some("unsupported_projectile_item_or_slot");
    } else if ctx
        .engine
        .projectile_cleanup_pending(active, request.projectile_index as u8)
    {
        plan.gameplay_denial = Some("projectile_cleanup_pending");
    } else if active
        .projectiles
        .get(request.projectile_index as u8)
        .is_some_and(|flight| {
            !flight.completed
                && (now.as_millis() as u64).saturating_sub(flight.launched_at_ms)
                    >= flight.expiry_after_ms
        })
    {
        // A periodic owner update expires/removes this object before reuse.
        plan.gameplay_denial = Some("projectile_expiry_pending");
    } else if !history_available {
        plan.gameplay_denial = Some("replication_history_limit");
    } else if let CharacterLife::Alive { generation, .. } = active.life {
        let slot = request.projectile_index as u8;
        let mut flights = active.projectiles.clone();
        match flights.admit_item_with_catalog(
            slot,
            generation,
            now.as_millis() as u64,
            active.latest_owner_sequence,
            bytes,
            grenade_item.ok_or("missing_grenade_item")?,
            &ctx.engine.item_catalog,
        ) {
            Ok(projectile::LaunchDisposition::Duplicate { .. }) => (),
            Ok(projectile::LaunchDisposition::Created { .. }) if active.grenade_remaining == 0 => {
                plan.gameplay_denial = Some("grenade_ammo_empty")
            }
            Ok(projectile::LaunchDisposition::Created {
                generation: flight_generation,
            }) => {
                let creation = projectile::ProjectileCreation {
                    slot,
                    object_index: active.reservation.player,
                    owner_index: active.reservation.player,
                    ammo_token: grenade.ok_or("unsupported_grenade_item")?.ammo_token,
                    position: replication::CharacterPosition {
                        x: request.position[0],
                        y: request.position[1],
                        z: request.position[2],
                    },
                    rotation_xyzw: request.rotation_xyzw,
                    shooter_id: request.shooter_id,
                };
                let encoded = creation.encode_with_profile(
                    replication::UpdateHeader {
                        sequence: next_sequence,
                        nak_sequence: 0,
                        client_index: 0,
                        object_type_mask: 1 << (16 + slot),
                        timestamp_ms: now.as_millis() as u32,
                    },
                    ctx.engine.wire_profile,
                );
                match encoded {
                    Ok(packet) => {
                        plan.combat
                            .projectile_replicas
                            .remember_creation(flight_generation, creation)?;
                        plan.outgoing = Some(packet.clone());
                        plan.replication.projectile_delta = Some(packet);
                        plan.combat.new_projectiles = Some(flights);
                        plan.combat.new_grenade_remaining = Some(active.grenade_remaining - 1);
                        created = true;
                    }
                    Err(code) => plan.gameplay_denial = Some(code),
                }
            }
            Err(code) => plan.gameplay_denial = Some(code),
        }
    }
    Ok(
        json!({"type":"fire_grenade","eligible":eligible,"owned":owned,
        "position":request.position,"rotation_xyzw":request.rotation_xyzw,
        "initial_velocity":request.initial_velocity,"ammo_index":request.ammo_index,
        "throw_power":request.throw_power,"target_id":request.target_id,
        "shooter_id":request.shooter_id,"shooter_client_index":request.shooter_client_index,
        "projectile_index":request.projectile_index,"projectile_created":created,"damage_applied":false,
        "ammo_remaining":plan.combat.new_grenade_remaining.unwrap_or(active.grenade_remaining)}),
    )
}

pub(super) fn heal<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::Heal,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let kit = active.current_inventory_handle.and_then(|h| {
        active.reservation.character.replication_loadout(
            &ctx.engine.item_catalog,
            h.inventory_index,
            active.wire_profile,
        )
    });
    let item = kit
        .as_ref()
        .ok_or("healing_inventory_unavailable")
        .and_then(|kit| {
            healing::HealItem::from_loadout_with_catalog(
                kit,
                request.gear_slot,
                &ctx.engine.item_catalog,
            )
        });
    let mut accepted = false;
    if !active.entered_game || !active.ready_for_updates || !active.life.is_alive() {
        plan.gameplay_denial = Some("healing_requires_living_character");
    } else if active.mounted_seat.is_some() {
        plan.gameplay_denial = Some("mounted_healing_not_supported");
    } else if now < active.next_heal_allowed {
        plan.gameplay_denial = Some("healing_cooldown");
    } else if let Err(code) = item {
        plan.gameplay_denial = Some(code);
    } else {
        let item = item.unwrap();
        let sequence = active
            .healing_sequence
            .checked_add(1)
            .ok_or("healing_sequence_exhausted")?;
        let mut targets = std::collections::BTreeSet::new();
        for target_id in &request.target_ids {
            if !targets.insert(*target_id) {
                continue;
            }
            let own_id = 0x03800000 | u32::from(active.reservation.player);
            let target = if *target_id == own_id {
                Some(active)
            } else {
                ctx.engine
                    .peers
                    .values()
                    .find(|peer| (0x03800000 | u32::from(peer.reservation.player)) == *target_id)
            };
            let Some(target) = target else {
                continue;
            };
            if target.team() != active.team()
                || !target.entered_game
                || !target.ready_for_updates
                || target.mounted_seat.is_some()
            {
                continue;
            }
            let CharacterLife::Alive {
                generation,
                health_quarter_points,
            } = target.life
            else {
                continue;
            };
            if target.healing.is_some()
                || health_quarter_points
                    >= plan
                        .leadership
                        .modifiers(target)
                        .max_health_quarter_points()?
            {
                continue;
            }
            let effect = healing::HealingEffect::new_for_recipient(
                item,
                generation,
                now,
                &ctx.engine.item_catalog,
                &plan.leadership.modifiers(active),
                &plan.leadership.modifiers(target),
            )?;
            let session = progression::HealingSession {
                effect,
                healer: progression::RewardRecipient::from_active(active),
                effect_id: format!(
                    "heal-{}-{sequence:016x}-{:04x}-{generation:08x}",
                    active.reservation.id, target.reservation.player
                ),
            };
            if *target_id == own_id {
                plan.combat.new_healing = Some(Some(session));
            } else {
                plan.combat
                    .peer_healing
                    .push((target.reservation.id.clone(), generation, session));
            }
            accepted = true;
        }
        if accepted {
            plan.combat.new_healing_sequence = Some(sequence);
            plan.combat.new_heal_allowed = Some(
                now.checked_add(item.cooldown_with_catalog(&ctx.engine.item_catalog)?)
                    .ok_or("healing_clock_overflow")?,
            );
        } else if active.healing.is_some()
            && request
                .target_ids
                .contains(&(0x03800000 | u32::from(active.reservation.player)))
        {
            plan.gameplay_denial = Some("healing_cooldown");
        } else if request.target_ids.as_slice()
            == [0x03800000 | u32::from(active.reservation.player)]
        {
            plan.gameplay_denial = Some("healing_full_health");
        } else {
            plan.gameplay_denial = Some("ineligible_healing_targets");
        }
    }
    Ok(
        json!({"type":"heal","target_ids":request.target_ids,"gear_slot":request.gear_slot,
        "accepted":accepted,"policy":"same_team_authored_ticks_healer_and_recipient_modifiers"}),
    )
}

pub(super) fn kill_self<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::KillSelf,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let history_available = ctx.history_available;
    let eligible = active.entered_game && active.ready_for_updates;
    let owned = request.target_character_id == (0x03800000 | u32::from(active.reservation.player));
    let health_before = match active.life {
        CharacterLife::Alive {
            health_quarter_points,
            ..
        } => Some(health_quarter_points),
        CharacterLife::AwaitingRespawn { .. } => Some(0),
        CharacterLife::AwaitingFirstSpawn => None,
    };
    if eligible && !owned {
        plan.gameplay_denial = Some("unowned_damage_target");
    } else if eligible
        && owned
        && let CharacterLife::Alive { generation, .. } = active.life
    {
        // Mounted death also needs vehicle-occupancy cleanup. Keep
        // that untraced transition nonfatal and explicitly bounded.
        if active.mounted_seat.is_some() {
            plan.gameplay_denial = Some("mounted_death_not_supported");
        } else if !history_available {
            plan.gameplay_denial = Some("replication_history_limit");
        } else {
            let damage = damage::ResolvedDamage::kill_self(
                request.target_character_id,
                health_before.unwrap(),
            )?;
            plan.combat.new_damage_model = Some(damage);
            let update = replication::CharacterDamageUpdate {
                sequence: next_sequence,
                timestamp_ms: now.as_millis() as u32,
                object_index: active.reservation.player,
                forced_state: Some(replication::CharacterState {
                    state: 7,
                    stance: 0,
                    user_data: [0; 3],
                }),
                damage,
            }
            .encode_with_profile(ctx.engine.wire_profile)?;
            plan.combat.clear_pending_damage = true;
            plan.outgoing = Some(update.clone());
            plan.replication.damage_delta = Some(update);
            plan.combat.new_life = Some(CharacterLife::AwaitingRespawn { generation });
        }
    }
    Ok(
        json!({"type":"kill_self","target_character_id":request.target_character_id,
        "eligible":eligible,"owned":owned,"health_before_quarter_points":health_before,
        "damage_sent":plan.replication.damage_delta.is_some(),
        "health_after_quarter_points":if plan.replication.damage_delta.is_some() {Some(0)} else {health_before},
        "replication_sequence":if plan.replication.damage_delta.is_some() {Some(next_sequence)} else {None}}),
    )
}
