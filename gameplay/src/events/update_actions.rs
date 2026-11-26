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
    let human_target = ctx
        .engine
        .active
        .iter()
        .chain(ctx.engine.peers.values())
        .find(|actor| {
            actor.reservation.world == active.reservation.world
                && pending.target_reservation_id.as_deref() == Some(actor.reservation.id.as_str())
                && pending.target_character_id
                    == (0x0380_0000 | u32::from(actor.reservation.player))
        });
    let qualified_frag = matches!(
        pending.source,
        PendingDamageSource::OrdinaryRetailFrag { .. }
    );
    let scope_current = match pending.source {
        PendingDamageSource::OrdinaryRetailFrag {
            world,
            round,
            target_allocation,
            owner,
        } => {
            world == active.reservation.world
                && round == ctx.engine.round_number
                && owner == firearm_assists::FragOwner::capture(active)
                && pending.damage.attacker_id
                    == (0x0380_0000 | u32::from(active.reservation.player))
                && match target_allocation {
                    FragTargetAllocation::Bot(allocation) => {
                        registry_target.is_some_and(|target| {
                            let bot = &ctx.engine.bot_registry.bots[&target.object_index];
                            bot.allocation_generation == allocation
                                && bot.target.runtime.is_some_and(|runtime| {
                                    !matches!(runtime.phase, development_target::BotPhase::Dead)
                                })
                        })
                    }
                    FragTargetAllocation::Human(allocation) => {
                        human_target.is_some_and(|victim| victim.allocation_sequence == allocation)
                    }
                }
        }
        PendingDamageSource::Falling | PendingDamageSource::Other => true,
    };
    let victim = if !scope_current {
        None
    } else if pending.target_character_id == (0x03800000 | u32::from(active.reservation.player))
        && pending.target_reservation_id.as_deref() == Some(active.reservation.id.as_str())
    {
        match active.life.damage_health() {
            Some((generation, health_quarter_points))
                if generation == pending.generation && !active.is_mounted() =>
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
    } else if let Some(peer) = human_target {
        match peer.life.damage_health() {
            Some((generation, health_quarter_points))
                if generation == pending.generation && !peer.is_mounted() =>
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
        let mut resolved = if pending.source == PendingDamageSource::Falling {
            pending.damage.apply_falling(health_quarter_points)?
        } else {
            pending.damage.apply(health_quarter_points)?
        };
        let mut human_life = None;
        if let Some(victim) = human_target {
            let (model, life) =
                ctx.engine
                    .plan_human_damage(victim, pending.damage.amount_hp, resolved, now)?;
            resolved = model;
            human_life = Some(life);
            if ctx.accepted_owner_route
                && let PendingDamageSource::OrdinaryRetailFrag { owner, .. } = pending.source
                && let Some(assists) = firearm_assists::Plan::prepare_ordinary_frag(
                    ctx.engine,
                    now,
                    active,
                    owner,
                    victim,
                    resolved.health_quarter_points == 0,
                )
            {
                let frame = plan.frago_frame(ctx.engine);
                let victim_position = ctx.engine.frago_character_position(victim, frame);
                plan.combat
                    .new_rewards
                    .extend(assists.rewards(ctx.engine, victim_position, frame));
                statistics::firearm_assists(plan, &assists);
                plan.combat.firearm_assists = Some(assists);
            }
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
        if pending.source == PendingDamageSource::Falling
            && pending.target_character_id == (0x0380_0000 | u32::from(active.reservation.player))
            && pending.target_reservation_id.as_deref() == Some(active.reservation.id.as_str())
        {
            plan.combat.falling_death_notice = ctx.engine.plan_falling_death(active, resolved)?;
        }
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
        } else if let Some(peer) = human_target {
            plan.combat
                .peer_damage
                .push((peer.reservation.id.clone(), generation, resolved));
            if let Some(life) = human_life {
                plan.combat
                    .peer_lives
                    .push((peer.reservation.id.clone(), life));
            }
        } else if let Some(bot) = registry_target {
            if qualified_frag && lethal && ctx.engine.match_terminal.is_none() {
                plan.combat.bot_firearm_kd = Some(bot_firearm_kd::Plan::prepare(
                    ctx.engine,
                    active.reservation.world,
                    None,
                    Some(bot_firearm_kd::Life {
                        index: bot.object_index,
                        generation: bot.generation,
                        team: bot.team,
                        health: health_quarter_points,
                    }),
                )?);
            }
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
                    let reward = ctx.engine.frago_combat_reward(
                        recipient.bot_killed(bot.object_index, bot.generation),
                        active,
                        Some(crate::frago::point(bot.position)),
                        plan.frago_frame(ctx.engine),
                    );
                    plan.combat.new_rewards.push(reward);
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
            // Heal admission requires equal actual teams and squad selection
            // cannot change sides. This remains the healer's admitted team even
            // if that healer has since left; credit gating does not alter HP.
            && !ctx
                .engine
                .attacker_is_friendly(
                    active.last_damage.last_attacker,
                    active.reservation.world,
                    active.team(),
                )
        {
            for (tick, units) in &receipts {
                if *units > 0 {
                    let reward = healer.event(
                        format!("{}-{tick:02x}", effect.effect_id),
                        progression::RewardKind::TeammateHealed,
                        *units,
                    );
                    let frame = plan.frago_frame(ctx.engine);
                    let current_healer = ctx
                        .engine
                        .active
                        .iter()
                        .chain(ctx.engine.peers.values())
                        .find(|actor| healer.matches_active(actor));
                    let reward = if let Some(actor) = current_healer {
                        ctx.engine.frago_action_reward(
                            reward,
                            actor,
                            ctx.engine.frago_character_position(active, frame),
                            frame,
                        )
                    } else {
                        reward
                    };
                    plan.combat.new_rewards.push(reward);
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

/// A point the squad may use whose hospital or vehicle is destroyed. Retail
/// treats such a point as blocked when a player selects it; free seats are
/// never considered there.
fn spawn_point_blocked<W: Write>(engine: &Engine<W>, index: u16) -> bool {
    match engine
        .spawn_catalog
        .as_ref()
        .and_then(|c| c.point(index))
        .map(|p| p.kind)
    {
        Some(spawn_catalog::SpawnKind::BunkerPassenger) => engine
            .bunker_world
            .as_ref()
            .and_then(|world| world.hospital_at_spawn(index))
            .is_some_and(|hospital| !hospital.operational),
        Some(spawn_catalog::SpawnKind::VehiclePassenger) => engine
            .ground_vehicles
            .as_ref()
            .and_then(|world| world.passenger_vehicle(index))
            .and_then(|binding| engine.current_ground_vehicle(binding.index))
            .is_some_and(|vehicle| !engine.ground_vehicle_is_operational(vehicle.index)),
        _ => false,
    }
}

/// The point a due life starts at: the player's selection (else the squad's
/// shared spawn) while the squad may use it. A selection the squad lost is
/// replaced by the squad's first point, in authored order, that it may use
/// and that is not blocked, as retail does when a player selects a blocked
/// point. Seating into a vehicle or hospital is decided when the life
/// starts (see `plan_respawn`).
pub(super) fn respawn_point<W: Write>(engine: &Engine<W>, active: &Active) -> Option<u16> {
    let catalog = active.squad_catalog.as_ref();
    let chosen = active
        .selected_spawn
        .map_or(active.squad_spawn, |s| s.spawn_point_index);
    if catalog.alpha(chosen).is_some() {
        return Some(chosen);
    }
    catalog
        .available_for_squad(catalog.squad_index())
        .unwrap_or_default()
        .iter()
        .copied()
        .find(|&index| catalog.alpha(index).is_some() && !spawn_point_blocked(engine, index))
}

/// The squad's alternate insertion (retail spawn category 1) when the squad
/// may use it and it is not `original`.
fn alternate_insertion(
    catalog: &spawn_catalog::SpawnCatalog,
    original: u16,
) -> Option<&spawn_catalog::SpawnLocation> {
    catalog
        .available_for_squad(catalog.squad_index())?
        .iter()
        .filter(|&&index| index != original)
        .find_map(|&index| {
            catalog
                .alpha(index)
                .filter(|point| matches!(point.kind, spawn_catalog::SpawnKind::Alternate))
        })
}

/// The passenger seat an APC insertion seats a new life in: the bound vehicle
/// must exist, stand, belong to the player's side and let
/// passengers out, and occupancy (crew included) must be below its maximum;
/// the highest free passenger seat is used.
fn ground_spawn_seat<W: Write>(
    ctx: &PlanningContext<'_, W>,
    index: u16,
) -> Result<crate::ground_seats::PassengerSeat, &'static str> {
    let active = ctx.active;
    let world = ctx
        .engine
        .ground_vehicles
        .as_ref()
        .ok_or("ground_vehicle_catalog_required")?;
    let vehicle = world
        .passenger_vehicle(index)
        .and_then(|binding| ctx.engine.current_ground_vehicle(binding.index))
        .ok_or("passenger_spawn_binding_required")?;
    if !ctx.engine.ground_vehicle_is_operational(vehicle.index) {
        return Err("ground_vehicle_not_operational");
    }
    if vehicle.team != active.team() || vehicle.faction != active.reservation.character.faction {
        return Err("passenger_vehicle_side_mismatch");
    }
    if vehicle.exit_policy().is_none() {
        return Err("ground_exit_policy_required");
    }
    ctx.engine
        .ground_passenger_seat(&vehicle)
        .ok_or("ground_passenger_seats_full")
}

/// The seat a Domination hospital point seats a new life in, under the same
/// rules: the hospital stands on the player's side and has room.
fn hospital_spawn_seat<'a, W: Write>(
    ctx: &PlanningContext<'a, W>,
    index: u16,
) -> Result<(crate::structures::IndexedHospital<'a>, u8), &'static str> {
    let active = ctx.active;
    let hospital = ctx
        .engine
        .bunker_world
        .as_ref()
        .and_then(|world| world.hospital_at_spawn(index))
        .ok_or("bunker_spawn_binding_required")?;
    if !hospital.operational
        || hospital.team != active.team()
        || hospital.faction != active.reservation.character.faction
    {
        return Err("bunker_spawn_not_operational_or_wrong_side");
    }
    let seat = ctx
        .engine
        .bunker_passenger_seat(&hospital, active.reservation.player)?;
    Ok((hospital, seat))
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
    let mut point = active
        .squad_catalog
        .as_ref()
        .alpha(respawn_point(ctx.engine, active).ok_or("respawn_point_unavailable")?)
        .unwrap();
    // A stored selection the squad can no longer use is dropped once a life
    // starts elsewhere; the player then follows the squad's shared spawn.
    plan.session.clear_selected_spawn = active
        .selected_spawn
        .is_some_and(|s| active.squad_catalog.alpha(s.spawn_point_index).is_none());
    // Retail never refuses or holds a life whose APC or hospital cannot seat
    // it: the life starts on foot at the squad's alternate insertion in the
    // same wave, else on foot at the vehicle point itself. (Helicopter
    // boarding keeps its own refusals for now.)
    let mut mount_failure = None;
    let mut helo_passenger = None;
    if matches!(point.kind, spawn_catalog::SpawnKind::HeloPassenger) {
        let candidate = ctx
            .engine
            .domination_gameplay
            .as_ref()
            .and_then(|world| world.state.helo.as_ref())
            .filter(|_| !active.is_mounted() && active.helo_creation_sent && active.team() == 0)
            .cloned();
        let result = candidate
            .ok_or("domination_helo_boarding_unavailable")
            .and_then(|mut fleet| {
                let carrier = fleet
                    .by_spawn_mut(point.index)
                    .filter(|carrier| {
                        carrier.source.faction == active.reservation.character.faction
                            && carrier.damage.state == replication::VehicleState::Operational
                            && carrier.damage.wire.conditions & 8 == 0
                            && carrier
                                .source
                                .squads
                                .contains(&(active.squad_catalog.squad_index() as u8))
                    })
                    .ok_or("domination_helo_passenger_source_mismatch")?;
                let position = carrier
                    .current_pose
                    .ok_or("domination_helo_position_pending")?
                    .position;
                let object_index = carrier.source.object_index;
                let seat = carrier.motion.board_passenger(active.reservation.player)?;
                Ok((
                    fleet,
                    domination_helo::PassengerSeat { object_index, seat },
                    position,
                ))
            });
        match result {
            Ok((fleet, seat, position)) => {
                helo_passenger = Some((seat, position));
                plan.helo_runtime = Some(fleet);
            }
            Err(reason) => {
                plan.gameplay_denial = Some(reason);
                return Ok(());
            }
        }
    }
    let ground_passenger = matches!(point.kind, spawn_catalog::SpawnKind::VehiclePassenger);
    if ground_passenger && active.is_mounted() {
        plan.gameplay_denial = Some("must_exit_current_seat");
        return Ok(());
    }
    // A recipient still receiving the vehicle creation batches waits for them
    // (a replication-order guard, not a seating failure).
    if ground_passenger
        && ctx
            .engine
            .ground_vehicles
            .as_ref()
            .is_some_and(|world| active.ground_vehicle_batch < world.batch_count())
    {
        plan.gameplay_denial = Some("ground_vehicle_creation_required");
        return Ok(());
    }
    let ground_seat = if ground_passenger {
        match ground_spawn_seat(ctx, point.index) {
            Ok(seat) => Some(seat),
            Err(reason) => {
                mount_failure = Some(reason);
                None
            }
        }
    } else {
        None
    };
    let bunker = matches!(point.kind, spawn_catalog::SpawnKind::BunkerPassenger);
    if matches!(point.kind, spawn_catalog::SpawnKind::Parachute) && loadout.is_none() {
        plan.gameplay_denial = Some("parachute_spawn_requires_inventory");
        return Ok(());
    }
    let indexed = bunker && ctx.engine.indexed_hospital_seats();
    let mut hospital_seat = None;
    if indexed {
        match hospital_spawn_seat(ctx, point.index) {
            Ok(seated) => hospital_seat = Some(seated),
            Err(reason) => mount_failure = Some(reason),
        }
    }
    if let Some(reason) = mount_failure {
        let original = point.index;
        if let Some(alternate) = alternate_insertion(active.squad_catalog.as_ref(), original) {
            point = alternate;
        }
        plan.audit.spawn_fallback = Some((original, point.index, reason));
    }
    let bunker = bunker && mount_failure.is_none();
    let (indexed_bunker, hospital_passenger) = match hospital_seat {
        Some((hospital, seat)) => (Some(hospital), Some(seat)),
        None => (None, None),
    };
    if bunker && !indexed && ctx.engine.bunker_exit_policy.is_none() {
        plan.gameplay_denial = Some("bunker_exit_policy_required");
        return Ok(());
    }
    let passenger_seat = if bunker {
        Some(if let Some(seat) = hospital_passenger {
            seat
        } else {
            ctx.engine
                .alpha_passenger_seat(active.reservation.player)
                .ok_or("alpha_passenger_seats_full")?
        })
    } else {
        None
    };
    let generation = active.life.next_generation();
    // The authored facing applies only when the character stands at the point itself.
    let on_foot_at_point =
        indexed_bunker.is_none() && helo_passenger.is_none() && ground_seat.is_none();
    let mut spawn_position = if let Some(hospital) = &indexed_bunker {
        hospital
            .seat_position(passenger_seat.ok_or("bunker_passenger_seat_required")?)
            .ok_or("bunker_passenger_pose_unqualified")?
    } else if let Some((_, position)) = helo_passenger {
        position
    } else if let Some(seat) = ground_seat {
        ctx.engine
            .current_ground_vehicle(seat.vehicle_index)
            .ok_or("ground_vehicle_runtime_required")?
            .position
    } else {
        point.position
    };
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
    let wire_position = if helo_passenger.is_some() {
        domination_helo::mounted_wire_position(spawn_position)
    } else {
        spawn_position
    };
    let mut respawn =
        replication::CharacterRespawn::at_position(now.as_millis() as u32, wire_position);
    if on_foot_at_point && let Some(yaw) = point.yaw {
        respawn.yaw = yaw;
    }
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
    let position_origin = if helo_passenger.is_some() {
        player_tracking::PositionOrigin::ServerMount
    } else {
        player_tracking::PositionOrigin::ServerSpawn
    };
    plan.session.server_position = Some((spawn_position, position_origin));
    plan.audit.spawned_point = Some(point.index);
    if let Some(seat) = passenger_seat {
        respawn.state = replication::CharacterState {
            state: 14,
            stance: 0,
            user_data: [
                13,
                indexed_bunker
                    .as_ref()
                    .map_or(0, |bunker| bunker.index as u8),
                seat,
            ],
        };
    }
    if let Some(seat) = ground_seat {
        respawn.state = seat.character_state();
    }
    if let Some((seat, _)) = helo_passenger {
        respawn.state = seat.character_state();
    }
    plan.session.new_helo_seat = Some(helo_passenger.map(|(seat, _)| seat));
    plan.session.new_ground_seat = Some(ground_seat);
    plan.session.new_mounted_seat = Some(passenger_seat);
    plan.session.new_mounted_bunker = indexed_bunker.as_ref().map(|bunker| bunker.index);
    plan.session.spawn_pose = Some((respawn.state, respawn.yaw));
    plan.combat.new_life = Some(CharacterLife::Alive {
        generation: generation.unwrap(),
        health_quarter_points: respawn.damage.health_quarter_points,
    });
    plan.session.new_inventory_handle = loadout.as_ref().map(|l| l.handle);
    plan.session.new_spawned_loadout = loadout.clone();
    plan.combat.observed_primary = loadout.as_ref().map(|l| l.primary);
    plan.combat.primary_equipped = loadout.as_ref().map(|l| l.primary_equipped);
    // Native invalid/empty inventory selection does not prevent
    // base physical spawn. Never silently replace the desired slot.
    let delta = if let Some(hospital) = &indexed_bunker {
        respawn.encode_in_bunker_with_profile(
            loadout.as_ref(),
            active.reservation.player,
            ctx.engine.bunker_ownership_after(
                hospital.index,
                active.reservation.player,
                passenger_seat,
            ),
            ctx.engine.wire_profile,
        )?
    } else if bunker {
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
