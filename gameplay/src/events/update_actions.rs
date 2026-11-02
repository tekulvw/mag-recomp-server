//! Planned damage, healing and physical respawn for owner updates.
use super::*;

pub(super) fn plan_pending_damage<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    mut grenade_queue: std::collections::VecDeque<PendingDamage>,
) -> Result<(), &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let pending = active
        .pending_damage
        .as_ref()
        .or(grenade_queue.front())
        .unwrap();
    let registry_target = ctx.engine.bot_combat_snapshots().into_iter().find(|b| {
        ctx.engine.is_registry_bot(b.object_index)
            && (0x03800000 | u32::from(b.object_index)) == pending.target_character_id
            && pending.target_reservation_id.is_none()
            && b.generation == pending.generation
            && b.health_quarter_points > 0
    });
    let victim = if pending.target_character_id
        == (0x03800000 | u32::from(active.reservation.player))
        && pending.target_reservation_id.as_deref() == Some(active.reservation.id.as_str())
    {
        match active.life.damage_health() {
            Some((generation, health_quarter_points))
                if generation == pending.generation && active.mounted_seat.is_none() =>
            {
                Some((
                    generation,
                    health_quarter_points,
                    plan.session
                        .client_position
                        .map(|p| p.0)
                        .or(active.tracking.sample.map(|s| s.position)),
                ))
            }
            _ => None,
        }
    } else if let Some(peer) = ctx.engine.peers.values().find(|p| {
        0x03800000 | u32::from(p.reservation.player) == pending.target_character_id
            && pending.target_reservation_id.as_deref() == Some(p.reservation.id.as_str())
    }) {
        match peer.life.damage_health() {
            Some((generation, health_quarter_points))
                if generation == pending.generation && peer.mounted_seat.is_none() =>
            {
                Some((
                    generation,
                    health_quarter_points,
                    peer.tracking.sample.map(|s| s.position),
                ))
            }
            _ => None,
        }
    } else if let Some(bot) = registry_target {
        Some((
            bot.generation,
            bot.health_quarter_points,
            Some(bot.position),
        ))
    } else if pending.target_character_id == dummy::OBJECT_ID
        && pending.target_reservation_id.is_none()
        && pending.generation
            == active
                .development_target
                .as_ref()
                .and_then(|t| t.runtime.map(|r| r.generation))
                .unwrap_or(1)
    {
        active.dummy_health.filter(|hp| *hp > 0).map(|hp| {
            (
                pending.generation,
                hp,
                active
                    .development_target
                    .as_ref()
                    .map(|t| t.runtime.map_or(t.position, |r| r.position))
                    .or(ctx.engine.development_dummy_position),
            )
        })
    } else {
        None
    };
    if let Some((generation, health_quarter_points, position)) = victim {
        let mut resolved = if pending.falling {
            pending.damage.apply_falling(health_quarter_points)?
        } else {
            pending.damage.apply(health_quarter_points)?
        };
        let mut human_life = None;
        if let Some(victim) = ctx
            .engine
            .active
            .iter()
            .chain(ctx.engine.peers.values())
            .find(|p| {
                pending.target_reservation_id.as_deref() == Some(p.reservation.id.as_str())
                    && pending.target_character_id
                        == (0x0380_0000 | u32::from(p.reservation.player))
            })
        {
            let (model, life) =
                ctx.engine
                    .plan_human_damage(victim, pending.damage.amount_hp, resolved, now)?;
            resolved = model;
            human_life = Some(life);
            if matches!(victim.life,CharacterLife::Incapacitated{downed,..} if !downed.is_expired(now))
                && matches!(life, CharacterLife::AwaitingRespawn { .. })
                && let Some(killer) = ctx
                    .engine
                    .active
                    .iter()
                    .chain(ctx.engine.peers.values())
                    .find(|a| {
                        pending.damage.attacker_id
                            == (0x0380_0000 | u32::from(a.reservation.player))
                    })
            {
                statistics::finished(plan, killer, victim.team());
                if killer.team() != victim.team()
                    && let Some(recipient) = progression::RewardRecipient::from_active(killer)
                {
                    plan.combat.new_rewards.push(recipient.event(
                        format!(
                            "finish-{}-{}-{}",
                            killer.reservation.id, victim.reservation.id, pending.id
                        ),
                        progression::RewardKind::EnemyFinished,
                        1,
                    ));
                }
            }
        }
        let lethal = resolved.health_quarter_points == 0;
        if lethal
            && health_quarter_points > 0
            && pending.target_character_id == dummy::OBJECT_ID
            && pending.target_reservation_id.is_none()
            && ctx.engine.development_target_replication_enabled(active)
            && let Some(recipient) = progression::RewardRecipient::from_active(active)
        {
            plan.combat.new_rewards.push(recipient.event(
                format!("kill-{}-dummy-{:08x}", active.reservation.id, generation),
                progression::RewardKind::EnemyKilled,
                1,
            ));
        }
        let packet = replication::CharacterDamageUpdate {
            sequence: next_sequence,
            timestamp_ms: now.as_millis() as u32,
            object_index: pending.target_character_id as u16,
            damage: resolved,
            forced_state: lethal.then_some(replication::CharacterState {
                state: 7,
                stance: 0,
                user_data: [0; 3],
            }),
        }
        .encode_with_profile(ctx.engine.wire_profile)?;
        if pending.target_character_id == (0x03800000 | u32::from(active.reservation.player)) {
            plan.combat.new_damage_model = Some(resolved);
            plan.combat.new_life = Some(human_life.unwrap_or(if lethal {
                CharacterLife::AwaitingRespawn { generation }
            } else {
                CharacterLife::Alive {
                    generation,
                    health_quarter_points: resolved.health_quarter_points,
                }
            }));
        } else if let Some(peer) =
            ctx.engine.peers.values().find(|p| {
                0x03800000 | u32::from(p.reservation.player) == pending.target_character_id
            })
        {
            plan.combat
                .peer_damage
                .push((peer.reservation.id.clone(), generation, resolved));
            if let Some(life) = human_life {
                plan.combat
                    .peer_lives
                    .push((peer.reservation.id.clone(), life));
            }
        } else if let Some(bot) = registry_target {
            plan.combat
                .bot_damage
                .push((bot.object_index, bot.generation, resolved));
            // Grenade damage stays on its launcher's authenticated queue, even
            // after that launcher dies. Require its full attacker handle rather
            // than crediting whichever actor happens to dispatch an event.
            if lethal
                && pending.damage.attacker_id
                    == (0x0380_0000 | u32::from(active.reservation.player))
            {
                statistics::target_killed(plan, active, bot.team);
                if active.team() != bot.team
                    && let Some(recipient) = progression::RewardRecipient::from_active(active)
                {
                    plan.combat
                        .new_rewards
                        .push(recipient.bot_killed(bot.object_index, bot.generation));
                }
            }
        } else {
            plan.combat.dummy_health = Some(resolved.health_quarter_points);
            if lethal && health_quarter_points > 0 {
                if let Some(runtime) = active.development_target.as_ref().and_then(|t| t.runtime) {
                    plan.combat.bot_runtime = Some(development_target::BotRuntime {
                        phase: development_target::BotPhase::Dead,
                        since: now,
                        last_update: now,
                        physics_motion: None,
                        ..runtime
                    });
                }
                // This branch has resolved the server-owned target, not a peer.
                let team = active
                    .development_target
                    .as_ref()
                    .map_or(1, |t| t.squad.team());
                statistics::target_killed(plan, active, team);
            }
        }
        plan.audit.applied_damage = Some(json!({
            "target_character_id":pending.target_character_id,
            "damage_id":pending.id,"damage":pending.damage,"life_generation":generation,
            "health_before_quarter_points":health_quarter_points,"health_after_quarter_points":resolved.health_quarter_points,
            "lethal":lethal,"position":position,"replication_sequence":next_sequence,
        }));
        plan.outgoing = Some(packet.clone());
        plan.replication.damage_delta = Some(packet);
    }
    if plan.audit.applied_damage.is_none() {
        plan.gameplay_denial = Some("queued_damage_no_longer_eligible");
    }
    if active.pending_damage.is_some() {
        plan.combat.clear_pending_damage = true;
    } else {
        grenade_queue.pop_front();
        plan.combat.new_grenade_damage = Some(grenade_queue);
    }
    Ok(())
}

pub(super) fn plan_healing_tick<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
) -> Result<(), &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let effect = active.healing.as_ref().unwrap();
    if let CharacterLife::Alive {
        generation,
        health_quarter_points,
    } = active.life
        && effect.generation == generation
        && let Some((resolved, remaining, receipts)) =
            effect.effect.advance_with_receipts(now, active.last_damage)
    {
        let packet = replication::CharacterDamageUpdate {
            sequence: next_sequence,
            timestamp_ms: now.as_millis() as u32,
            object_index: active.reservation.player,
            damage: resolved,
            forced_state: None,
        }
        .encode_with_profile(ctx.engine.wire_profile)?;
        if let Some(healer) = &effect.healer
            && healer.reservation_id != active.reservation.id
            && !ctx
                .engine
                .attacker_is_friendly(active.last_damage.last_attacker)
        {
            for (tick, units) in &receipts {
                if *units > 0 {
                    plan.combat.new_rewards.push(healer.event(
                        format!("{}-{tick:02x}", effect.effect_id),
                        progression::RewardKind::TeammateHealed,
                        *units,
                    ));
                }
            }
        }
        plan.combat.new_healing = Some(remaining.map(|remaining| progression::HealingSession {
            effect: remaining,
            healer: effect.healer.clone(),
            effect_id: effect.effect_id.clone(),
        }));
        plan.combat.new_damage_model = Some(resolved);
        plan.combat.new_life = Some(CharacterLife::Alive {
            generation,
            health_quarter_points: resolved.health_quarter_points,
        });
        plan.audit.applied_healing = Some(json!({"life_generation":generation,
                    "healer_max_health_hp":effect.healer_max_health_hp(),"target_max_health_hp":effect.recipient_max_health_hp(),
                    "heal_amount_modifier":effect.heal_amount_modifier(),"restored_hp_per_tick":receipts,"tick_amount_hp":effect.tick_amount_hp(),
                    "health_before_quarter_points":health_quarter_points,
                    "health_after_quarter_points":resolved.health_quarter_points,
                    "replication_sequence":next_sequence}));
        plan.outgoing = Some(packet.clone());
        plan.replication.damage_delta = Some(packet);
    }
    Ok(())
}

pub(super) fn plan_respawn<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
) -> Result<(), &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let desired_inventory = active
        .selected_loadout
        .map(|s| s.desired_inventory_index())
        .or_else(|| {
            active
                .reservation
                .character
                .replication_default_inventory_index(&ctx.engine.item_catalog, active.wire_profile)
        });
    let mut loadout = desired_inventory.and_then(|index| {
        active.reservation.character.replication_loadout(
            &ctx.engine.item_catalog,
            index,
            active.wire_profile,
        )
    });
    if active.wire_profile.application_id() == 20231
        && ctx
            .engine
            .item_catalog
            .bootstrap_default_inventory_index(active.reservation.character.faction)
            .is_some()
        && loadout.is_none()
    {
        // Preserve the pending wave and connection; never spawn another null kit.
        plan.gameplay_denial = Some("unsupported_bootstrap_inventory_selection");
        return Ok(());
    }
    if let (Some(kit), Some(current)) = (&mut loadout, active.current_inventory_handle) {
        // Native SetInventoryIndex retains version for the same index,
        // otherwise increments while skipping the invalid zero version.
        let version = if kit.handle.inventory_index == current.inventory_index {
            current.update_id
        } else {
            current.update_id.wrapping_add(1).max(1)
        };
        kit.handle.update_id = version;
        kit.primary.inventory_update_id = version;
    }
    let point = active
        .squad_catalog
        .as_ref()
        .alpha(
            active
                .selected_spawn
                .map_or(active.squad_spawn, |s| s.spawn_point_index),
        )
        .unwrap();
    let bunker = matches!(point.kind, spawn_catalog::SpawnKind::BunkerPassenger);
    if matches!(point.kind, spawn_catalog::SpawnKind::Parachute) && loadout.is_none() {
        plan.gameplay_denial = Some("parachute_spawn_requires_inventory");
        return Ok(());
    }
    if bunker && ctx.engine.bunker_exit_policy.is_none() {
        plan.gameplay_denial = Some("bunker_exit_policy_required");
        return Ok(());
    }
    let generation = active.life.next_generation();
    let mut spawn_position = point.position;
    if matches!(point.kind, spawn_catalog::SpawnKind::Parachute) {
        let deadline = active
            .pending_spawn_deadline
            .ok_or("parachute_spawn_requires_pending_wave")?;
        let ordinal = match ctx.engine.parachute_ordinal(point.index, deadline) {
            Ok(ordinal) => ordinal,
            Err(code) => {
                plan.gameplay_denial = Some(code);
                return Ok(());
            }
        };
        spawn_position = point.parachute_position(ordinal)?;
        plan.parachute_spawn = Some((point.index, deadline, ordinal));
    }
    let mut respawn =
        replication::CharacterRespawn::at_position(now.as_millis() as u32, spawn_position);
    if let Some(pose) = point.parachute {
        respawn.state = replication::CharacterState {
            state: 17,
            stance: 0,
            user_data: [0; 3],
        };
        respawn.yaw = pose.yaw;
    }
    respawn.damage.health_quarter_points = active.modifiers.max_health_quarter_points()?;
    respawn.sequence = next_sequence;
    respawn.object_index = active.reservation.player;
    plan.session.server_position =
        Some((spawn_position, player_tracking::PositionOrigin::ServerSpawn));
    plan.audit.spawned_point = Some(point.index);
    let passenger_seat = if bunker {
        Some(
            ctx.engine
                .alpha_passenger_seat(active.reservation.player)
                .ok_or("alpha_passenger_seats_full")?,
        )
    } else {
        None
    };
    if let Some(seat) = passenger_seat {
        respawn.state = replication::CharacterState {
            state: 14,
            stance: 0,
            user_data: [13, 0, seat],
        };
    }
    plan.session.new_mounted_seat = Some(passenger_seat);
    plan.session.spawn_pose = Some((respawn.state, respawn.yaw));
    plan.combat.new_life = Some(CharacterLife::Alive {
        generation: generation.unwrap(),
        health_quarter_points: respawn.damage.health_quarter_points,
    });
    plan.session.new_inventory_handle = loadout.as_ref().map(|l| l.handle);
    plan.combat.observed_primary = loadout.as_ref().map(|l| l.primary);
    plan.combat.primary_equipped = loadout.as_ref().map(|l| l.primary_equipped);
    // Native invalid/empty inventory selection does not prevent
    // base physical spawn. Never silently replace the desired slot.
    let delta = if bunker {
        respawn.encode_in_alpha_with_ownership_with_profile(
            loadout.as_ref(),
            active.reservation.player,
            ctx.engine
                .alpha_ownership_after(active.reservation.player, passenger_seat),
            ctx.engine.wire_profile,
        )?
    } else if let Some(loadout) = &loadout {
        respawn.encode_with_loadout_with_profile(loadout, ctx.engine.wire_profile)?
    } else {
        respawn.encode_with_profile(ctx.engine.wire_profile)?
    };
    plan.outgoing = Some(delta.clone());
    plan.replication.spawn_delta = Some(delta);
    Ok(())
}
