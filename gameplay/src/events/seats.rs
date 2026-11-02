//! Native seats event planning; authoritative state is read-only.
use super::*;

pub(super) fn mount_seat<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::MountSeat,
) -> Result<Value, &'static str> {
    if request.vehicle_id >> 24 == 1 {
        return ground_mount(ctx, plan, request);
    }
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let history_available = ctx.history_available;
    let eligible = active.entered_game
        && active.ready_for_updates
        && active.life.is_alive()
        && active.emplacement_creation.is_some()
        && ctx
            .engine
            .bunker_world
            .as_ref()
            .and_then(structures::BunkerWorld::canonical_alpha_team)
            .is_some_and(|team| team == active.team());
    let valid = request.vehicle_id == 0x0d000000
        && request.character_id == (0x03800000 | u32::from(active.reservation.player))
        && request.seat_id <= 8;
    if active.ground_seat.is_some() {
        plan.gameplay_denial = Some("must_exit_current_seat");
    } else if eligible && valid && ctx.engine.bunker_exit_policy.is_none() {
        plan.gameplay_denial = Some("bunker_exit_policy_required");
    } else if eligible && valid {
        let seat = request.seat_id as u8;
        if active.mounted_seat == Some(seat) {
            plan.outgoing = active.seat_transitions.last().cloned();
        } else if ctx
            .engine
            .peers
            .values()
            .any(|p| p.mounted_seat == Some(seat))
        {
            plan.gameplay_denial = Some(if seat == 0 {
                "alpha_turret_occupied"
            } else {
                "alpha_passenger_seat_occupied"
            });
        } else if ctx.engine.maximum_players == 1 && active.is_mounted() {
            plan.gameplay_denial = Some("must_exit_current_seat");
        } else if !history_available {
            plan.gameplay_denial = Some("replication_history_limit");
        } else {
            let delta = replication::AlphaSeatTransition {
                sequence: next_sequence,
                timestamp_ms: now.as_millis() as u32,
                action: replication::AlphaSeatAction::Mount {
                    seat,
                    client_index: active.reservation.player,
                },
            }
            .encode_for_player_with_ownership_with_profile(
                active.reservation.player,
                Some(
                    ctx.engine
                        .alpha_ownership_after(active.reservation.player, Some(seat)),
                ),
                ctx.engine.wire_profile,
            )?;
            plan.outgoing = Some(delta.clone());
            plan.replication.seat_transition = Some(delta);
            plan.session.new_mounted_seat = Some(Some(seat));
        }
    } else if !valid {
        plan.gameplay_denial = Some("invalid_alpha_mount_target");
    }
    Ok(json!({"type":"mount_seat","vehicle_id":request.vehicle_id,
        "seat_id":request.seat_id,"character_id":request.character_id,
        "eligible":eligible,"transition_sent":plan.replication.seat_transition.is_some(),
        "already_mounted":active.mounted_seat}))
}

pub(super) fn dismount_seat<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::DismountSeat,
) -> Result<Value, &'static str> {
    if request.vehicle_id >> 24 == 1 {
        return ground_dismount(ctx, plan, request);
    }
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let history_available = ctx.history_available;
    let exit = ctx
        .engine
        .bunker_exit_policy
        .as_ref()
        .map(|policy| policy.exit(request.camera_yaw))
        .transpose()?;
    let eligible = active.entered_game
        && active.ready_for_updates
        && active.life.is_alive()
        && active.emplacement_creation.is_some();
    let valid = request.vehicle_id == 0x0d000000
        && request.character_id == (0x03800000 | u32::from(active.reservation.player))
        && request.seat_id <= 8;
    if eligible && valid && exit.is_none() {
        plan.gameplay_denial = Some("bunker_exit_policy_required");
    } else if eligible && valid {
        let exit = exit.unwrap();
        if active.mounted_seat == Some(request.seat_id as u8) {
            if !history_available {
                plan.gameplay_denial = Some("replication_history_limit");
            } else {
                let delta = replication::AlphaSeatTransition {
                    sequence: next_sequence,
                    timestamp_ms: now.as_millis() as u32,
                    action: replication::AlphaSeatAction::Exit {
                        yaw: exit.packed_yaw,
                        position: exit.position,
                    },
                }
                .encode_for_player_with_ownership_with_profile(
                    active.reservation.player,
                    Some(
                        ctx.engine
                            .alpha_ownership_after(active.reservation.player, None),
                    ),
                    ctx.engine.wire_profile,
                )?;
                plan.outgoing = Some(delta.clone());
                plan.replication.seat_transition = Some(delta);
                plan.session.new_mounted_seat = Some(None);
                plan.session.last_exit = Some(request);
                plan.session.exit_yaw = Some(exit.packed_yaw);
                plan.session.server_position =
                    Some((exit.position, player_tracking::PositionOrigin::ServerExit));
            }
        } else if !active.is_mounted() && active.last_exit == Some(request) {
            plan.outgoing = active.seat_transitions.last().cloned();
        } else {
            plan.gameplay_denial = Some("alpha_seat_not_owned");
        }
    } else if !valid {
        plan.gameplay_denial = Some("invalid_alpha_exit_target");
    }
    Ok(
        json!({"type":"dismount_seat","vehicle_id":request.vehicle_id,
        "seat_id":request.seat_id,"character_id":request.character_id,
        "camera_yaw":request.camera_yaw,"eligible":eligible,
        "transition_sent":plan.replication.seat_transition.is_some(),"exit_policy":"operator_supplied_model_relative_clearance_deferred",
        "exit_candidate":exit.map(|value| value.candidate_index)}),
    )
}

fn ground_mount<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::MountSeat,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let current_vehicle = ctx.engine.current_ground_vehicle(request.vehicle_id as u16);
    let vehicle = current_vehicle.as_ref();
    let same_vehicle_switch = active.ground_seat.is_some_and(|seat| {
        seat.vehicle_index == request.vehicle_id as u16 && u32::from(seat.seat) != request.seat_id
    });
    let denial = if request.character_id != (0x03800000 | u32::from(active.reservation.player))
        || vehicle.is_none_or(|v| {
            v.object_id() != request.vehicle_id
                || !v.passenger_seats.contains(&(request.seat_id as u8))
        })
        || request.seat_id >= 16
    {
        Some("invalid_ground_mount_target")
    } else if !ctx
        .engine
        .ground_vehicle_is_operational(request.vehicle_id as u16)
    {
        Some("ground_vehicle_not_operational")
    } else if !active.entered_game || !active.ready_for_updates || !active.life.is_alive() {
        Some("ground_mount_requires_living_actor")
    } else if active.is_mounted() && !same_vehicle_switch {
        Some("must_exit_current_seat")
    } else if active.ground_vehicle_batch
        < ctx.engine.ground_vehicles.as_ref().unwrap().batch_count()
    {
        Some("ground_vehicle_creation_required")
    } else if vehicle.unwrap().team != active.team()
        || vehicle.unwrap().faction != active.reservation.character.faction
    {
        Some("passenger_vehicle_side_mismatch")
    } else if vehicle.unwrap().exit_policy().is_none() {
        Some("ground_exit_policy_required")
    } else if ctx
        .engine
        .peers
        .values()
        .chain(ctx.engine.active.iter())
        .any(|p| {
            p.ground_seat
                == Some(ground_seats::PassengerSeat {
                    vehicle_index: request.vehicle_id as u16,
                    seat: request.seat_id as u8,
                })
        })
    {
        Some("ground_passenger_seat_occupied")
    } else if !same_vehicle_switch && ctx.engine.ground_passenger_seat(vehicle.unwrap()).is_none() {
        Some("ground_passenger_seats_full")
    } else if !same_vehicle_switch
        && let Err(code) = ground_seats::authorize_mount_position(active, vehicle.unwrap(), ctx.now)
    {
        Some(code)
    } else if !ctx.history_available {
        Some("replication_history_limit")
    } else {
        None
    };
    if let Some(denial) = denial {
        plan.gameplay_denial = Some(denial);
    } else {
        let seat = ground_seats::PassengerSeat {
            vehicle_index: vehicle.unwrap().index,
            seat: request.seat_id as u8,
        };
        let packet = ground_seats::transition(
            ctx.next_sequence,
            ctx.now.as_millis() as u32,
            active.reservation.player,
            seat.character_state(),
            None,
            ctx.engine.wire_profile,
        )?;
        plan.outgoing = Some(packet.clone());
        plan.replication.seat_transition = Some(packet);
        plan.session.new_ground_seat = Some(Some(seat));
    }
    Ok(
        json!({"type":"mount_seat","vehicle_id":request.vehicle_id,"seat_id":request.seat_id,"transition_sent":plan.replication.seat_transition.is_some()}),
    )
}
fn ground_dismount<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::DismountSeat,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let owned = active.ground_seat.is_some_and(|seat| {
        (0x01000000 | u32::from(seat.vehicle_index)) == request.vehicle_id
            && u32::from(seat.seat) == request.seat_id
    });
    if request.character_id != (0x03800000 | u32::from(active.reservation.player)) {
        plan.gameplay_denial = Some("invalid_ground_exit_target");
    } else if !active.entered_game || !active.ready_for_updates || !active.life.is_alive() {
        plan.gameplay_denial = Some("ground_exit_requires_living_actor");
    } else if !owned {
        if !active.is_mounted() && active.last_exit == Some(request) {
            plan.outgoing = active.seat_transitions.last().cloned();
        } else {
            plan.gameplay_denial = Some("ground_passenger_seat_not_owned");
        }
    } else if !ctx.history_available {
        plan.gameplay_denial = Some("replication_history_limit");
    } else if let Some(policy) = ctx
        .engine
        .current_ground_vehicle(request.vehicle_id as u16)
        .and_then(|vehicle| vehicle.exit_policy())
    {
        // Ground candidate generation follows the externally qualified native
        // order. Camera facing determines final yaw, never candidate position.
        let mut exit = policy.exit(u32::MAX)?;
        if request.camera_yaw <= 360 {
            exit.packed_yaw = request.packed_yaw();
        }
        let state = replication::CharacterState {
            state: 1,
            stance: 0,
            user_data: [0; 3],
        };
        let packet = ground_seats::transition(
            ctx.next_sequence,
            ctx.now.as_millis() as u32,
            active.reservation.player,
            state,
            Some(exit),
            ctx.engine.wire_profile,
        )?;
        plan.outgoing = Some(packet.clone());
        plan.replication.seat_transition = Some(packet);
        plan.session.new_ground_seat = Some(None);
        plan.session.last_exit = Some(request);
        plan.session.exit_yaw = Some(exit.packed_yaw);
        plan.session.server_position =
            Some((exit.position, player_tracking::PositionOrigin::ServerExit));
    } else {
        plan.gameplay_denial = Some("ground_exit_policy_required");
    }
    Ok(
        json!({"type":"dismount_seat","vehicle_id":request.vehicle_id,"seat_id":request.seat_id,"transition_sent":plan.replication.seat_transition.is_some()}),
    )
}
