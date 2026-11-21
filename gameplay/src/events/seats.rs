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
    if ctx.engine.match_profile().game_type == 2 && ctx.engine.match_profile().auto_assign_teams {
        return super::bunker_seats::mount(ctx, plan, request);
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
    if request.vehicle_id >> 24 == 2 {
        return helo_dismount(ctx, plan, request);
    }
    if request.vehicle_id >> 24 == 1 {
        return ground_dismount(ctx, plan, request);
    }
    if ctx.engine.match_profile().game_type == 2 && ctx.engine.match_profile().auto_assign_teams {
        return super::bunker_seats::dismount(ctx, plan, request);
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

fn helo_dismount<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::DismountSeat,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    if !ctx.accepted_client_route || !ctx.history_available {
        plan.gameplay_denial = Some("domination_helo_exit_route_unavailable");
    } else if !active.is_mounted() && active.last_exit == Some(request) {
        plan.outgoing = active.seat_transitions.last().cloned();
    } else {
        match ctx
            .engine
            .prepare_manual_helo_exit(active, request, ctx.next_sequence, ctx.now)
        {
            Ok(exit) => {
                plan.helo_runtime = Some(exit.fleet);
                if let Some((pose, packet)) = exit.transition {
                    plan.outgoing = Some(packet.clone());
                    plan.replication.seat_transition = Some(packet);
                    plan.session.new_helo_seat = Some(None);
                    plan.session.last_exit = Some(request);
                    plan.session.exit_yaw = Some(pose.packed_yaw);
                    plan.session.server_position =
                        Some((pose.position, player_tracking::PositionOrigin::ServerExit));
                } else {
                    plan.gameplay_denial = Some("domination_helo_disembark_unresolved");
                }
            }
            Err(code) => plan.gameplay_denial = Some(code),
        }
    }
    Ok(
        json!({"type":"helo_dismount", "vehicle_id":request.vehicle_id, "seat_id":request.seat_id,
        "transition_sent":plan.replication.seat_transition.is_some()}),
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
    let driver = ctx.engine.ground_driver_seat(request.vehicle_id as u16);
    let takes_driver = driver.is_some_and(|d| u32::from(d.seat) == request.seat_id);
    let takes_gunner = request.seat_id == 1
        && ctx
            .engine
            .ground_turret_policy(request.vehicle_id as u16)
            .is_some();
    let leaves_gunner = ctx.engine.is_ground_gunner(active)
        && active
            .ground_seat
            .is_some_and(|seat| seat.vehicle_index == request.vehicle_id as u16);
    let leaves_driver = active.ground_seat.is_some_and(|seat| {
        seat.vehicle_index == request.vehicle_id as u16
            && driver.is_some_and(|d| d.seat == seat.seat)
    });
    let same_vehicle_switch = active.ground_seat.is_some_and(|seat| {
        seat.vehicle_index == request.vehicle_id as u16 && u32::from(seat.seat) != request.seat_id
    });
    let denial = if (takes_gunner || leaves_gunner)
        && (!ctx.accepted_client_route || !ctx.engine.round_combat_allowed(ctx.now))
    {
        Some("ground_turret_mount_not_allowed")
    } else if (takes_driver || leaves_driver)
        && (!ctx.accepted_client_route || ctx.engine.match_terminal.is_some())
    {
        Some("ground_driver_mount_not_allowed")
    } else if request.character_id != (0x03800000 | u32::from(active.reservation.player))
        || vehicle.is_none_or(|v| {
            v.object_id() != request.vehicle_id
                || (!takes_driver
                    && !takes_gunner
                    && !v.passenger_seats.contains(&(request.seat_id as u8)))
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
    } else if !same_vehicle_switch && ctx.engine.ground_occupancy_full(vehicle.unwrap()) {
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
        let turret = if takes_gunner || leaves_gunner {
            match stage_ground_turret_seat(ctx, plan, seat.vehicle_index, takes_gunner) {
                Ok(update) => Some(update),
                Err(reason) => {
                    plan.gameplay_denial = Some(reason);
                    return Ok(
                        json!({"type":"mount_seat","vehicle_id":request.vehicle_id,"seat_id":request.seat_id,"transition_sent":false}),
                    );
                }
            }
        } else {
            None
        };
        let authority = if takes_driver {
            // In this server the authenticated reservation slot is also the
            // assigned connection client index.
            let mut runtime = ctx
                .engine
                .ground_runtime
                .clone()
                .ok_or("ground_vehicle_runtime_required")?;
            runtime.begin_driver(seat.vehicle_index, ctx.now.as_millis() as u64)?;
            plan.ground_runtime = Some(runtime);
            Some((seat.vehicle_index, active.reservation.player))
        } else if leaves_driver {
            Some((seat.vehicle_index, ground_vehicles::SERVER_OWNER))
        } else {
            None
        };
        let packet = ground_seats::transition_with_turret(
            ctx.next_sequence,
            ctx.now.as_millis() as u32,
            active.reservation.player,
            seat.character_state(),
            None,
            (authority, turret),
            ctx.engine.wire_profile,
        )?;
        plan.outgoing = Some(packet.clone());
        plan.replication.seat_transition = Some(packet);
        plan.session.new_ground_seat = Some(Some(seat));
        if takes_gunner {
            plan.session.server_position = Some((
                ctx.engine.ground_seat_position(seat)?,
                player_tracking::PositionOrigin::ServerMount,
            ));
        }
    }
    Ok(
        json!({"type":"mount_seat","vehicle_id":request.vehicle_id,"seat_id":request.seat_id,"transition_sent":plan.replication.seat_transition.is_some()}),
    )
}
fn stage_ground_turret_seat<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    index: u16,
    entering: bool,
) -> Result<ground_vehicles::TurretUpdate, &'static str> {
    ctx.engine
        .check_hospital_turret_queue(
            ctx.engine
                .peers
                .values()
                .filter(|peer| peer.initial_objects.is_some()),
            1,
            |_| 0,
        )
        .map_err(|_| "ground_turret_publication_budget_exceeded")?;
    let (turret, definition) = ctx
        .engine
        .ground_turret_seat_change(ctx.active, index, entering)?;
    plan.ground_turret = Some((index, turret));
    Ok(ground_vehicles::TurretUpdate {
        index,
        driver_owner: None,
        turret_owner: Some(if entering {
            ctx.active.reservation.player
        } else {
            u16::MAX
        }),
        definition,
        fields: Default::default(),
    })
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
    let driver = ctx
        .engine
        .ground_driver_seat(request.vehicle_id as u16)
        .filter(|d| u32::from(d.seat) == request.seat_id);
    let gunner = request.seat_id == 1
        && ctx
            .engine
            .ground_turret_policy(request.vehicle_id as u16)
            .is_some();
    if (driver.is_some() || gunner) && !ctx.accepted_client_route {
        plan.gameplay_denial = Some("ground_driver_dismount_not_allowed");
    } else if request.character_id != (0x03800000 | u32::from(active.reservation.player)) {
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
    } else {
        let exit = ctx
            .engine
            .current_ground_vehicle(request.vehicle_id as u16)
            .ok_or("ground_exit_policy_required")
            .and_then(|vehicle| ctx.engine.ground_exit(&vehicle, active.reservation.player));
        if let Ok(mut exit) = exit {
            let turret = if gunner {
                match stage_ground_turret_seat(ctx, plan, request.vehicle_id as u16, false) {
                    Ok(update) => Some(update),
                    Err(reason) => {
                        plan.gameplay_denial = Some(reason);
                        return Ok(
                            json!({"type":"dismount_seat","vehicle_id":request.vehicle_id,"seat_id":request.seat_id,"transition_sent":false}),
                        );
                    }
                }
            } else {
                None
            };
            if request.camera_yaw <= 360 {
                exit.packed_yaw = request.packed_yaw();
            }
            let state = replication::CharacterState {
                state: 1,
                stance: 0,
                user_data: [0; 3],
            };
            let packet = ground_seats::transition_with_turret(
                ctx.next_sequence,
                ctx.now.as_millis() as u32,
                active.reservation.player,
                state,
                Some(exit),
                (
                    driver.map(|d| (d.vehicle_index, ground_vehicles::SERVER_OWNER)),
                    turret,
                ),
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
            plan.gameplay_denial = exit.err();
        }
    }
    Ok(
        json!({"type":"dismount_seat","vehicle_id":request.vehicle_id,"seat_id":request.seat_id,"transition_sent":plan.replication.seat_transition.is_some()}),
    )
}
