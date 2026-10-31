//! Native seats event planning; authoritative state is read-only.
use super::*;

pub(super) fn mount_seat<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::MountSeat,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let history_available = ctx.history_available;
    let eligible = active.entered_game
        && active.ready_for_updates
        && active.life.is_alive()
        && active.emplacement_creation.is_some()
        && active.reservation.character.faction == 0;
    let valid = request.vehicle_id == 0x0d000000
        && request.character_id == (0x03800000 | u32::from(active.reservation.player))
        && request.seat_id <= 8;
    if eligible && valid && ctx.engine.bunker_exit_policy.is_none() {
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
        } else if ctx.engine.maximum_players == 1 && active.mounted_seat.is_some() {
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
        } else if active.mounted_seat.is_none() && active.last_exit == Some(request) {
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
