//! Native control event planning; authoritative state is read-only.
use super::*;

pub(super) fn time_request<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::TimeRequest,
) -> Result<Value, &'static str> {
    let now = ctx.now;
    let response = protocol::TimeResponse {
        server_time_ms: now.as_millis() as u32,
        client_time_ms: request.client_time_ms,
    };
    plan.outgoing = Some(response.encode_with_profile(ctx.engine.wire_profile));
    Ok(
        json!({"type":"time_request","client_time_ms":request.client_time_ms,
        "server_time_ms":response.server_time_ms}),
    )
}

pub(super) fn claim_reservation<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    c: protocol::ClaimReservation<'_>,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let bytes = ctx.decoded.bytes;
    let info = if let Some(claimed) = &active.claimed {
        if claimed.wire.as_slice() != bytes {
            plan.rejection = Some("claim_conflict");
        }
        claimed.game_info.clone()
    } else {
        // Local single-player loading policy, based on the installed
        // Domination/Unalaska definition. Objects follow on readiness.
        let profile = ctx.engine.match_profile();
        let info = protocol::GameInfo {
            client_index: active.reservation.player,
            level_name: ctx.engine.map_name().to_owned(),
            start_time_ms: ctx.engine.world_epoch.unwrap_or(now).as_millis() as u32,
            attacking_faction: profile.attacking_faction,
            defending_faction: profile.defending_faction,
            maximum_points: profile.maximum_points,
            current_points: ctx.engine.round.scores.unwrap_or(profile.maximum_points),
            game_type: profile.game_type,
        };
        plan.session.new_claim = Some(Claimed {
            wire: bytes.to_vec(),
            game_info: info.clone(),
            np_identity: c
                .np_identity
                .try_into()
                .map_err(|_| "invalid_np_identity_size")?,
        });
        info
    };
    if plan.rejection.is_none() {
        plan.outgoing = Some(info.encode_with_profile(ctx.engine.wire_profile)?);
    }
    Ok(json!({"type":"claim_reservation",
        "np_bytes":c.np_identity.len(),"clan_bytes":c.clan.len(),"local_ip":c.local_ip,
        "public_key_bytes":c.public_key.len(),"private_key_bytes":c.private_key.len(),
        "symmetric_key_bytes":c.symmetric_key.len(),
        "map_announced":plan.rejection.is_none(),"start_time_ms":info.start_time_ms}))
}

pub(super) fn client_ready<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::ClientReady,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    if let Some(claimed) = &active.claimed {
        plan.session.new_ready = Some(request.ready);
        if request.ready && active.initial_objects.is_none() {
            let character = &active.reservation.character;
            let team = active.team();
            if !ctx.engine.actor_matches_team(active) {
                return Err("character_faction_not_in_match");
            }
            // Membership comes from the participant's authored squad binding.
            // Native squad indices include the match-side offset.
            let squad_address =
                replication::SquadAddress::from_index(active.squad_catalog.squad_index())?;
            if squad_address.team() != team {
                return Err("squad_side_mismatch");
            }
            let squads = ctx.engine.squad_creations(Some(active.reservation.player));
            let initial_inventory = character.replication_loadout(
                &ctx.engine.item_catalog,
                character
                    .replication_default_inventory_index(
                        &ctx.engine.item_catalog,
                        active.wire_profile,
                    )
                    .unwrap_or_else(|| character.initial_inventory_index()),
                active.wire_profile,
            );
            let snapshot = replication::PlayerAndSquad {
                sequence: 1,
                timestamp_ms: now.as_millis() as u32,
                player: replication::CharacterCreation {
                    object_index: active.reservation.player,
                    owner_index: active.reservation.player,
                    info: replication::CharacterInfo {
                        team,
                        faction: character.faction,
                        character_class: 21,
                        name: character.name.clone(),
                        client_index: active.reservation.player,
                        squad_index: squad_address.squad_index(),
                        account_id: active.reservation.account,
                        np_identity: claimed.np_identity,
                    },
                    mag_info: active.mag_info(
                        active
                            .reservation
                            .character
                            .progression
                            .unwrap_or_default()
                            .level,
                    ),
                },
                squad: replication::SquadCreation {
                    object_index: squad_address.squad_index(),
                    owner_index: 0x105,
                    members: ctx.engine.squad_members(
                        squad_address.squad_index(),
                        Some(active.reservation.player),
                    ),
                    leader_index: ctx.engine.squad_leader_index(
                        squad_address.squad_index(),
                        &ctx.engine.squad_members(
                            squad_address.squad_index(),
                            Some(active.reservation.player),
                        ),
                    ),
                },
            }
            .encode_with_match_state_inventory_and_squads_with_profile(
                &replication::MatchStateCreation {
                    object_index: 0,
                    owner_index: 0x105,
                    phase: if let Some(outcome) = &ctx.engine.round.outcome {
                        if outcome.finalized {
                            replication::MatchPhase::PostGame
                        } else {
                            replication::MatchPhase::EndCountdown
                        }
                    } else if ctx.engine.match_started
                        || ctx.engine.ready_count() + 1 >= ctx.engine.minimum_players
                    {
                        replication::MatchPhase::Playing
                    } else {
                        replication::MatchPhase::PreGame
                    },
                },
                initial_inventory.as_ref(),
                &squads,
                ctx.engine.wire_profile,
            )?;
            plan.session.new_inventory_handle = initial_inventory.as_ref().map(|l| l.handle);
            plan.outgoing = Some(snapshot.clone());
            plan.replication.initial_objects = Some(snapshot);
        }
    }
    Ok(
        json!({"type":"client_ready","ready":request.ready,"map_announced":active.claimed.is_some(),
        "initial_player_squad":plan.replication.initial_objects.is_some(),
        "initial_match_state":plan.replication.initial_objects.is_some()}),
    )
}

pub(super) fn enter_game<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    // The initial snapshot includes Playing state and a character.
    // Native EnterGame marks entry; it has no direct spawn or ack.
    plan.session.entered_game =
        active.claimed.is_some() && active.initial_objects.is_some() && active.ready_for_updates;
    Ok(
        json!({"type":"enter_game","accepted":plan.session.entered_game,
        "already_entered":active.entered_game,
        "desired_inventory_index":active.selected_loadout.map(|s| s.desired_inventory_index())}),
    )
}

pub(super) fn select_loadout<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::SelectLoadout,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let accepted = active.claimed.is_some() && active.initial_objects.is_some();
    if accepted {
        plan.session.selected_loadout = Some(request);
    }
    // This stores a desired slot, not equipped inventory. Native
    // inventory validity and application belong to respawn.
    Ok(
        json!({"type":"select_loadout","index":request.index,"accepted":accepted,
        "desired_inventory_index":request.desired_inventory_index()}),
    )
}

pub(super) fn squad_spawn_selection<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::SquadSpawnSelection,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let history_available = ctx.history_available;
    // The current squad leader may choose its authored shared point.
    let accepted = active.entered_game
        && active.ready_for_updates
        && active.emplacement_creation.is_some()
        && active
            .squad_catalog
            .as_ref()
            .alpha(request.spawn_point_index)
            .is_some()
        && ctx
            .engine
            .squad_members(active.squad_catalog.squad_index(), None)[0]
            == active.reservation.player
        && history_available;
    if accepted && request.spawn_point_index != active.squad_spawn {
        let follows = active
            .selected_spawn
            .is_none_or(|s| s.spawn_point_index == active.squad_spawn);
        let packet = replication::SquadSpawnAssignment {
            sequence: next_sequence,
            timestamp_ms: now.as_millis() as u32,
            spawn_point_index: request.spawn_point_index,
        }
        .encode_with_character_for_player_and_catalog_with_profile(
            follows,
            active.reservation.player,
            active.squad_catalog.as_ref(),
            ctx.engine.wire_profile,
        )?;
        plan.outgoing = Some(packet.clone());
        plan.replication.squad_spawn_delta = Some(packet);
        plan.session.squad_spawn = Some(request.spawn_point_index);
        // Native SetSpawnPointID changes members following the old
        // squad point, preserving their individual overrides.
        if follows {
            plan.session.selected_spawn = Some(protocol::CharacterSpawnSelection {
                spawn_point_index: request.spawn_point_index,
            });
        }
    }
    if !accepted {
        plan.gameplay_denial = Some("squad_spawn_unavailable");
    }
    Ok(
        json!({"type":"squad_spawn_selection","spawn_point_index":request.spawn_point_index,
        "alt_insertion_index":request.alt_insertion_index,"accepted":accepted,
        "previous_spawn_point_index":active.squad_spawn}),
    )
}

pub(super) fn character_spawn_selection<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::CharacterSpawnSelection,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let history_available = ctx.history_available;
    let accepted = active.entered_game
        && active.ready_for_updates
        && !active.life.is_alive()
        && history_available
        && active
            .squad_catalog
            .as_ref()
            .alpha(request.spawn_point_index)
            .is_some();
    if accepted {
        plan.session.selected_spawn = Some(request);
        if request.spawn_point_index
            != active
                .selected_spawn
                .map_or(active.squad_spawn, |s| s.spawn_point_index)
        {
            let packet = replication::CharacterSpawnAssignment {
                sequence: next_sequence,
                timestamp_ms: now.as_millis() as u32,
                spawn_point_index: request.spawn_point_index,
            }
            .encode_for_player_with_catalog_with_profile(
                active.reservation.player,
                active.squad_catalog.as_ref(),
                ctx.engine.wire_profile,
            )?;
            plan.outgoing = Some(packet.clone());
            plan.replication.squad_spawn_delta = Some(packet);
        }
    } else {
        plan.gameplay_denial = Some("character_spawn_unavailable");
    }
    Ok(
        json!({"type":"spawn_selection","spawn_point_index":request.spawn_point_index,"accepted":accepted}),
    )
}

pub(super) fn cancel_respawn<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    // Native connection handler always returns 0x0214; an existing
    // unspawned character can leave the queue idempotently. Neither
    // readiness nor an expired deadline means it already spawned.
    let accepted = active.initial_objects.is_some() && !active.life.is_alive();
    plan.session.cancel_pending_spawn = accepted;
    plan.outgoing = Some(protocol::CancelRespawnResult { success: accepted }.encode());
    Ok(json!({"type":"cancel_respawn","accepted":accepted,
        "was_queued":active.pending_spawn_deadline.is_some(),
        "alive":active.life.is_alive(),"ready":active.ready_for_updates}))
}

pub(super) fn confirm_respawn<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let eligible = active.entered_game
        && active.ready_for_updates
        && active
            .squad_catalog
            .as_ref()
            .alpha(
                active
                    .selected_spawn
                    .map_or(active.squad_spawn, |s| s.spawn_point_index),
            )
            .is_some()
        && active.emplacement_creation.is_some()
        && ctx.engine.actor_matches_team(active)
        && active.life.next_generation().is_some();
    if eligible && active.pending_spawn_deadline.is_none() {
        // Reserve room for both the deadline and its physical spawn.
        if ctx.engine.maximum_players == 1
            && active.replication_history.len() + 2 > MAX_REPLICATION_HISTORY
        {
            plan.gameplay_denial = Some("replication_history_limit");
        } else {
            let team = active.team();
            let policy = ctx.engine.match_profile();
            let period = Duration::from_millis(u64::from(policy.spawn_wave_ms[usize::from(team)]));
            let clock = match ctx.engine.team_waves[usize::from(team)] {
                Some(clock) => clock,
                None => TeamWaveClock {
                    first_deadline: now
                        .checked_add(Duration::from_millis(u64::from(
                            policy.initial_spawn_delay_ms[usize::from(team)],
                        )))
                        .ok_or("wave_clock_overflow")?,
                    period,
                },
            };
            let deadline = clock.next_after(now)?;
            let packet = replication::TeamSpawnDeadline {
                sequence: next_sequence,
                timestamp_ms: now.as_millis() as u32,
                object_index: 0,
                team,
                deadline_ms: deadline.as_millis() as u32,
            }
            .encode_with_profile(ctx.engine.wire_profile)?;
            plan.outgoing = Some(packet.clone());
            plan.replication.wave_delta = Some(packet);
            plan.session.new_spawn_deadline = Some(deadline);
            plan.session.published_wave_deadline = Some(deadline);
            plan.session.new_team_wave = Some(clock);
        }
    }
    Ok(
        json!({"type":"confirm_respawn","eligible":eligible,"spawn_sent":false,
        "already_spawned":active.life.is_alive(),
        "queued":plan.session.new_spawn_deadline.or(active.pending_spawn_deadline).is_some(),
        "deadline_ms":plan.session.new_spawn_deadline.or(active.pending_spawn_deadline).map(|d|d.as_millis() as u32),
        "replication_sequence":if plan.replication.wave_delta.is_some(){Some(next_sequence)}else{None},
        "spawn_point_index":active.selected_spawn.map_or(active.squad_spawn,|s|s.spawn_point_index),
        "policy":"authored_match_side_spawn_wave"}),
    )
}

pub(super) fn ping_request<W: Write>(
    _ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::PingRequest,
) -> Result<Value, &'static str> {
    plan.outgoing = Some(
        protocol::PingResponse {
            client_time_ms: request.client_time_ms,
        }
        .encode(),
    );
    Ok(json!({"type":"ping_request","client_time_ms":request.client_time_ms}))
}

/// Retail `MagSimCharacter::HandleChangeNominations` (0x0093d0c0): an unchanged
/// value is ignored; otherwise SetNominations masks it and, in game, removes a
/// leader whose bit cleared or promotes into a vacancy. No reply is sent.
pub(super) fn change_player_nominations<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::ChangePlayerNominations,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let masked = ctx
        .engine
        .requested_nominations(active, request.nominations)
        .filter(|_| active.initial_objects.is_some());
    let changed = masked.filter(|n| *n != active.nominations);
    plan.session.new_nominations = changed;
    Ok(
        json!({"type":"change_player_nominations","requested":request.nominations,
        "masked":masked,"previous":active.nominations,"changed":changed.is_some(),
        "supported":masked.is_some()}),
    )
}

pub(super) fn change_player_squad<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::ChangePlayerSquad,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let current = active.squad_catalog.squad_index();
    let target = u16::from(request.squad_index);
    let count = ctx.engine.squad_reservation_count(target);
    let candidate = active
        .squad_catalog
        .for_squad(target)
        .ok()
        .and_then(|mut catalog| {
            let inherited = ctx.engine.squad_spawn_for(target)?;
            catalog.set_runtime_default(inherited).ok()?;
            Some(catalog)
        });
    let bunker_supported = candidate.as_ref().is_some_and(|c| {
        !c.match_profile().has_bunkers
            || target == ctx.engine.spawn_catalog.as_ref().unwrap().squad_index()
    });
    let accepted = active.initial_objects.is_some()
        && active.ready_for_updates
        && current != target
        && current & !3 == target & !3
        && count < ctx.engine.match_profile().squad_capacity
        && candidate.is_some()
        && bunker_supported
        && !active.replication_backlogged;
    if accepted {
        plan.session.squad_catalog = candidate.map(std::sync::Arc::new);
    }
    plan.outgoing = Some(protocol::ChangePlayerSquadResult { success: accepted }.encode());
    Ok(
        json!({"type":"change_player_squad","previous_squad_index":current,
        "squad_index":target,"accepted":accepted,"target_member_count":count,
        "bunker_vehicle_support":bunker_supported}),
    )
}
