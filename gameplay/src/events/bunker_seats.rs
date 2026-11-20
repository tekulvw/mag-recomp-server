//! Source-bound family13 hospital mount/dismount planning.
use super::*;

pub(super) fn mount<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::MountSeat,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let index = request.vehicle_id as u16;
    let world = ctx.engine.bunker_world.as_ref();
    let bunker = world.and_then(|world| world.domination_bunker(index));
    let seat = request.seat_id as u8;
    let duplicate = active.mounted_bunker == Some(index) && active.mounted_seat == Some(seat);
    let denial = if !ctx.accepted_client_route || !ctx.engine.round_combat_allowed(ctx.now) {
        Some("bunker_mount_not_allowed")
    } else if request.vehicle_id != 0x0d00_0000 | u32::from(index)
        || request.character_id != 0x0380_0000 | u32::from(active.reservation.player)
        || bunker.is_none()
        || request.seat_id > 16
    {
        Some("invalid_bunker_mount_target")
    } else if !active.entered_game
        || !active.ready_for_updates
        || !active.life.is_alive()
        || active.emplacement_creation.is_none()
    {
        Some("bunker_mount_requires_ready_living_actor")
    } else if !bunker.as_ref().unwrap().operational {
        Some("bunker_not_operational")
    } else if bunker.as_ref().unwrap().team != active.team()
        || bunker.as_ref().unwrap().faction != active.reservation.character.faction
    {
        Some("bunker_mount_side_mismatch")
    } else if seat == 0 {
        Some("bunker_turret_authority_unavailable")
    } else if seat > bunker.as_ref().unwrap().passenger_seats {
        Some("invalid_bunker_seat")
    } else if duplicate {
        plan.outgoing = active.seat_transitions.last().cloned();
        None
    } else if active.is_mounted() {
        Some("must_exit_current_seat")
    } else if ctx
        .engine
        .peers
        .values()
        .chain(ctx.engine.active.iter())
        .any(|actor| actor.mounted_bunker == Some(index) && actor.mounted_seat == Some(seat))
    {
        Some("bunker_seat_occupied")
    } else if ctx
        .engine
        .peers
        .values()
        .chain(ctx.engine.active.iter())
        .filter(|actor| actor.mounted_bunker == Some(index) && actor.mounted_seat.is_some())
        .count()
        >= usize::from(bunker.as_ref().unwrap().max_occupancy)
    {
        Some("bunker_seats_full")
    } else if !active.tracking.sample.is_some_and(|sample| {
        sample.received_at_ms <= ctx.now.as_millis() as u64
            && bunker.as_ref().unwrap().within_mount_range(sample.position)
            && (!matches!(
                sample.origin,
                player_tracking::PositionOrigin::ClientReported
            ) || sample.client_sequence == Some(active.tracking.accepted_position_sequence)
                && sample
                    .client_sequence
                    .is_some_and(|s| s <= active.latest_owner_sequence))
    }) {
        Some("bunker_mount_position_unavailable_or_out_of_range")
    } else if !ctx.history_available {
        Some("replication_history_limit")
    } else {
        None
    };
    if let Some(denial) = denial {
        plan.gameplay_denial = Some(denial);
    } else if !duplicate {
        let packet = replication::AlphaSeatTransition {
            sequence: ctx.next_sequence,
            timestamp_ms: ctx.now.as_millis() as u32,
            action: replication::AlphaSeatAction::Mount {
                seat,
                client_index: active.reservation.player,
            },
        }
        .encode_for_bunker_with_profile(
            index,
            active.reservation.player,
            Some(
                ctx.engine
                    .bunker_ownership_after(index, active.reservation.player, Some(seat)),
            ),
            ctx.engine.wire_profile,
        )?;
        plan.outgoing = Some(packet.clone());
        plan.replication.seat_transition = Some(packet);
        plan.session.new_mounted_seat = Some(Some(seat));
        plan.session.new_mounted_bunker = Some(index);
        let position = bunker
            .as_ref()
            .unwrap()
            .seat_position(seat)
            .ok_or("bunker_passenger_pose_unqualified")?;
        plan.session.server_position =
            Some((position, player_tracking::PositionOrigin::ServerMount));
    }
    Ok(
        json!({"type":"mount_seat","vehicle_id":request.vehicle_id,"seat_id":request.seat_id,
        "character_id":request.character_id,"indexed_hospital":true,
        "transition_sent":plan.replication.seat_transition.is_some()}),
    )
}

pub(super) fn dismount<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::DismountSeat,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let index = request.vehicle_id as u16;
    if !ctx.accepted_client_route
        || request.vehicle_id != 0x0d00_0000 | u32::from(index)
        || request.character_id != 0x0380_0000 | u32::from(active.reservation.player)
        || request.seat_id > 16
    {
        plan.gameplay_denial = Some("invalid_bunker_exit_target");
    } else if !active.is_mounted() && active.last_exit == Some(request) {
        plan.outgoing = active.seat_transitions.last().cloned();
    } else if active.mounted_bunker != Some(index)
        || active.mounted_seat != Some(request.seat_id as u8)
    {
        plan.gameplay_denial = Some("bunker_seat_not_owned");
    } else if !active.entered_game || !active.ready_for_updates || !active.life.is_alive() {
        plan.gameplay_denial = Some("bunker_exit_requires_ready_living_actor");
    } else if !ctx.history_available {
        plan.gameplay_denial = Some("replication_history_limit");
    } else {
        match ctx
            .engine
            .current_bunker_exit(index, active.reservation.player, request.camera_yaw)
        {
            Err(reason) => plan.gameplay_denial = Some(reason),
            Ok(exit) => {
                let packet = replication::AlphaSeatTransition {
                    sequence: ctx.next_sequence,
                    timestamp_ms: ctx.now.as_millis() as u32,
                    action: replication::AlphaSeatAction::Exit {
                        yaw: exit.packed_yaw,
                        position: exit.position,
                    },
                }
                .encode_for_bunker_with_profile(
                    index,
                    active.reservation.player,
                    Some(
                        ctx.engine
                            .bunker_ownership_after(index, active.reservation.player, None),
                    ),
                    ctx.engine.wire_profile,
                )?;
                plan.outgoing = Some(packet.clone());
                plan.replication.seat_transition = Some(packet);
                plan.session.new_mounted_seat = Some(None);
                plan.session.last_exit = Some(request);
                plan.session.exit_yaw = Some(exit.packed_yaw);
                plan.session.server_position =
                    Some((exit.position, player_tracking::PositionOrigin::ServerExit));
            }
        }
    }
    Ok(
        json!({"type":"dismount_seat","vehicle_id":request.vehicle_id,
        "seat_id":request.seat_id,"indexed_hospital":true,
        "transition_sent":plan.replication.seat_transition.is_some()}),
    )
}
