//! Native owner event planning; authoritative state is read-only.
use super::*;

pub(super) fn object_update<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    update: replication::ObjectUpdate<'_>,
) -> Result<Value, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let h = update.header;
    let OwnerReport {
        reported_position,
        position_status,
        owner_fields,
        body_supported,
        accepted_stance,
    } = plan_owner_report(ctx, plan, &update)?;
    let mut flash_recipients = std::collections::BTreeMap::new();
    for terminal in std::mem::take(&mut plan.combat.flash_pending) {
        match ctx.engine.plan_flash(
            active,
            &terminal,
            crate::domination_structural_world::OnFootBlastPose {
                position: plan.session.client_position.map(|(p, _)| p),
                stance: accepted_stance,
            },
            plan.ground_runtime.as_ref(),
            if plan.session.clear_flash_yaw {
                None
            } else {
                plan.session.flash_yaw.or(active.flash_yaw)
            },
        ) {
            Ok((messages, audit)) => {
                for delivery in messages {
                    let message = &delivery.message;
                    let recipient = (
                        message.reservation_id.clone(),
                        message.world_id,
                        message.player_id,
                    );
                    // Bounded replacement policy: the native visual retains the
                    // stronger duration. Merge only this same recorded event;
                    // equal-pulse retrigger timing is not a parity claim.
                    flash_recipients
                        .entry(recipient)
                        .and_modify(|current: &mut crate::flash_grenades::BlindDelivery| {
                            if delivery.duration > current.duration {
                                current
                                    .message
                                    .payload_hex
                                    .clone_from(&delivery.message.payload_hex);
                                current.duration = delivery.duration;
                            }
                        })
                        .or_insert(delivery);
                }
                plan.audit.flash_terminals.extend(audit);
            }
            Err(code) => {
                plan.audit
                    .flash_terminals
                    .push(json!({"terminal":terminal,"supported":false,"reason":code}));
            }
        }
    }
    plan.messages.extend(
        flash_recipients
            .into_values()
            .map(|delivery| delivery.message),
    );
    let mut grenade_queue = plan_grenade_blasts(ctx, plan, h, body_supported, accepted_stance)?;
    if let Some(fall) = plan.combat.queued_fall.take() {
        if grenade_queue.len() < 64 {
            grenade_queue.push_back(fall);
            plan.combat.new_grenade_damage = Some(grenade_queue.clone());
        } else {
            plan.gameplay_denial = Some("fall_damage_queue_full");
        }
    }
    plan_update_reply(ctx, plan, h, body_supported, grenade_queue)?;
    // Owner planning selects one direct reply; shared delivery is separate.
    // Replication/replay takes priority; leave clock state untouched
    // so the next valid update catches up to absolute elapsed time.
    if plan.rejection.is_none()
        && body_supported
        && active.ready_for_updates
        && plan.outgoing.is_none()
        && !ctx.engine.bunker_update_due(now)
        && ctx.engine.match_profile().game_type == 2
        && !ctx.engine.match_profile().auto_assign_teams
        && let Some(started) = ctx.engine.match_epoch.or(active.match_clock_started)
    {
        // Local Domination's attacker points encode its countdown. Other
        // modes require their own score/lifecycle rules; time is not a kill.
        // Coalesce absolute elapsed time against the configured initial value.
        let elapsed = now.saturating_sub(started).as_secs();
        let initial = ctx.engine.match_profile().maximum_points[0] as u64;
        let remaining = initial.saturating_sub(elapsed) as i16;
        if remaining < active.last_clock_points {
            plan.session.clock_points = Some(remaining);
        }
    }
    let mut audit = json!({"type":"object_update","sequence":h.sequence,"nak_sequence":h.nak_sequence,
        "client_index":h.client_index,"object_type_mask":h.object_type_mask,"timestamp_ms":h.timestamp_ms,
        "body_supported":body_supported,
        "ground_dynamics_accepted":plan.ground_dynamics_relay.is_some(),
        "reported_position":reported_position,"position_status":position_status,"owner_fields":owner_fields,
        "fall_damage":plan.audit.falling_damage,
        "applied_damage":plan.audit.applied_damage,"applied_healing":plan.audit.applied_healing,
        "projectile_reports":plan.audit.projectile_reports,"projectile_terminals":plan.audit.projectile_terminals,
        "smoke_terminals":plan.audit.smoke_terminals,
        "flash_terminals":plan.audit.flash_terminals,
        "grenade_blast_checks":plan.audit.grenade_blast_checks,
        "grenade_visibility_policy":if ctx.engine.frag_character_blast {"current_body_stance_material_ray"} else if ctx.engine.static_frag_world().is_some() {"static_world_stance_anchor_ray"} else if ctx.engine.development_grenade_radius {"disk_triangles_three_body_samples"} else {"unknown_no_damage"},
        "initial_objectives":plan.replication.objective_delta.is_some(),"objective_count":plan.replication.objective_count,
        "initial_emplacement":plan.replication.emplacement_creation.is_some(),
        "assigned_objective":plan.replication.objective_target,
        "spawn_sent":plan.replication.spawn_delta.is_some(),
        "development_target_created":plan.combat.dummy_created,
        "bot_script":plan.audit.bot_script,
        "development_target_spawned":plan.combat.dummy_health.is_some() && plan.audit.bot_script.as_ref().is_none_or(|r|r["phase"]=="spawn" || r["phase"]=="respawn"),
        "team_wave_deadline_ms":plan.session.published_wave_deadline.map(|d|d.as_millis() as u32),
        "spawn_point_index":plan.audit.spawned_point,"spawn_mounted_seat":plan.session.new_mounted_seat.flatten(),"spawn_mounted_bunker":plan.session.new_mounted_bunker,
        "spawn_ground_vehicle_index":plan.session.new_ground_seat.flatten().map(|s|s.vehicle_index),
        "spawn_ground_passenger_seat":plan.session.new_ground_seat.flatten().map(|s|s.seat),
        "life_generation":if plan.replication.spawn_delta.is_some(){active.life.next_generation()}else{None},
        "equipped_inventory_index":plan.session.new_inventory_handle.map(|h|h.inventory_index),
        "replication_sequence":if plan.replication.spawn_delta.is_some() || plan.replication.wave_delta.is_some(){Some(next_sequence)}else{None}});
    if !plan.audit.ground_grenade_checks.is_empty() {
        audit["ground_grenade_checks"] = json!(plan.audit.ground_grenade_checks);
    }
    if !plan.audit.helo_grenade_checks.is_empty() {
        audit["helo_grenade_checks"] = json!(plan.audit.helo_grenade_checks);
    }
    if !plan.audit.hospital_grenade_checks.is_empty() {
        audit["hospital_grenade_checks"] = json!(plan.audit.hospital_grenade_checks);
    }
    if !plan.audit.native_grenade_character_checks.is_empty() {
        audit["native_grenade_character_checks"] =
            json!(plan.audit.native_grenade_character_checks);
    }
    Ok(audit)
}

fn stage_ground_dynamics<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    reports: &[ground_vehicles::OwnerDynamics],
) -> Result<(), &'static str> {
    let mut changes = reports
        .iter()
        .copied()
        .filter(|report| report.has_changes());
    let Some(report) = changes.next() else {
        return Ok(());
    };
    if changes.next().is_some()
        || !ctx.accepted_owner_route
        || ctx.engine.match_terminal.is_some()
        || !ctx.engine.ground_driver_authority(ctx.active, report.index)
        || ctx.engine.ground_driver_owner(report.index) != Some(ctx.active.reservation.player)
    {
        return Err("unowned_ground_vehicle_update");
    }
    let driver = ctx
        .engine
        .ground_driver_seat(report.index)
        .ok_or("ground_driver_policy_required")?;
    let mut runtime = ctx
        .engine
        .ground_runtime
        .clone()
        .ok_or("ground_vehicle_runtime_required")?;
    let now_ms = ctx.now.as_millis() as u64;
    runtime.apply_driver_dynamics(report, driver, now_ms)?;
    let relay = report.encode_relay(ctx.engine.wire_profile, now_ms as u32)?;
    plan.ground_runtime = Some(runtime);
    plan.ground_dynamics_relay = Some((report.index, relay));
    Ok(())
}

struct OwnerReport {
    reported_position: Option<replication::CharacterPosition>,
    position_status: &'static str,
    owner_fields: Option<Value>,
    body_supported: bool,
    accepted_stance: Option<u8>,
}

fn plan_owner_report<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    update: &replication::ObjectUpdate<'_>,
) -> Result<OwnerReport, &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let h = update.header;
    let fields = ctx
        .decoded
        .owner_fields
        .as_ref()
        .expect("ObjectUpdate has a complete cached owner decode")
        .clone()
        .map(|report| {
            if let Some(fields) = report.turret.filter(|f| f.has_changes()) {
                // Parsing proves the complete typed body, never gunner authority.
                // A denied turret component does not discard valid character fields.
                match ctx.engine.alpha_turret_authority(active) {
                    Ok(_)
                        if h.client_index == 0
                            && h.nak_sequence == 0
                            && h.sequence > active.latest_owner_sequence =>
                    {
                        plan.session.new_turret_fields = Some(fields);
                    }
                    Ok(_) => (),
                    Err(code) => plan.gameplay_denial = Some(code),
                }
            }
            let mut hospital_rows = report.hospital_turrets.into_iter().filter(|row| {
                let fields = row.fields;
                fields.yaw.is_some()
                    || fields.pitch.is_some()
                    || fields.primary.is_some()
                    || fields.secondary.is_some()
                    || fields.overheat_info.is_some()
            });
            if let Some(row) = hospital_rows.next() {
                if hospital_rows.next().is_some()
                    || !ctx.accepted_owner_route
                    || !ctx.engine.round_combat_allowed(ctx.now)
                    || h.client_index != 0
                    || h.nak_sequence != 0
                    || h.sequence <= active.latest_owner_sequence
                {
                    plan.gameplay_denial = Some("hospital_turret_owner_report_not_allowed");
                } else {
                    match ctx.engine.prepare_hospital_turret_report(active, row, now) {
                        Ok((turret, packets)) => {
                            plan.hospital_turret = Some((row.index, turret));
                            plan.hospital_aim_packets = packets;
                        }
                        Err(reason) => plan.gameplay_denial = Some(reason),
                    }
                }
            }
            match report.ground_turrets.as_slice() {
                [] => (),
                [row]
                    if ctx.accepted_owner_route
                        && ctx.engine.round_combat_allowed(ctx.now)
                        && h.client_index == 0
                        && h.nak_sequence == 0
                        && h.sequence > active.latest_owner_sequence =>
                {
                    match ctx.engine.prepare_ground_turret_report(active, *row, now) {
                        Ok((turret, packets)) => {
                            plan.ground_turret = Some((row.dynamics.index, turret));
                            plan.ground_turret_packets = packets;
                        }
                        Err(reason) => plan.gameplay_denial = Some(reason),
                    }
                }
                _ => plan.gameplay_denial = Some("ground_turret_owner_report_not_allowed"),
            }
            if !report.ground_vehicles.is_empty()
                && h.client_index == 0
                && h.nak_sequence == 0
                && h.sequence > active.latest_owner_sequence
                && let Err(code) = stage_ground_dynamics(ctx, plan, &report.ground_vehicles)
            {
                plan.gameplay_denial = Some(code);
            }
            let mut fields = report.character.unwrap_or_default();
            fields.projectiles = report.projectiles;
            fields
        });
    let mut reported_position = None;
    let mut position_status = "absent";
    let mut owner_fields = None;
    let mut body_supported = true;
    let mut accepted_stance = None;
    match fields {
        Err(
            error @ (owner_update::OwnerUpdateError::UnsupportedCharacterField
            | owner_update::OwnerUpdateError::UnsupportedObjectType
            | owner_update::OwnerUpdateError::UnsupportedProjectileField
            | owner_update::OwnerUpdateError::UnsupportedTurretField
            | owner_update::OwnerUpdateError::UnsupportedGroundVehicleField),
        ) => {
            // Coverage gaps discard the whole body without disconnecting.
            plan.gameplay_denial.get_or_insert(error.code());
            body_supported = false;
            position_status = "unsupported";
        }
        Err(owner_update::OwnerUpdateError::Invalid(code)) => plan.rejection = Some(code),
        Ok(fields) => {
            // Apply the route constraint to both the report that seeds a
            // terminal and the update that consumes it.
            let qualified_owner_route = ctx.accepted_owner_route
                || (ctx.engine.acquisition_gameplay.is_none()
                    && ctx.engine.domination_gameplay.is_none());
            if h.client_index == 0 && h.nak_sequence == 0 && qualified_owner_route {
                plan.session.new_owner_sequence =
                    Some(active.latest_owner_sequence.max(h.sequence));
                if ctx.accepted_owner_route
                    && h.sequence > active.latest_owner_sequence
                    && fields.state.is_some_and(|s| matches!(s.state, 7 | 14))
                {
                    plan.session.clear_flash_yaw = true;
                }
                if ctx.accepted_owner_route
                    && fields
                        .state
                        .or(active.owner_fields.state)
                        .is_none_or(|s| !matches!(s.state, 7 | 14))
                    && h.sequence > active.latest_owner_sequence
                    && !active.is_mounted()
                    && let CharacterLife::Alive { generation, .. } = active.life
                    && let Some(yaw) = fields.yaw
                {
                    plan.session.flash_yaw = Some(crate::flash_grenades::YawWitness {
                        allocation: active.allocation_sequence,
                        generation,
                        yaw,
                    });
                }
                if h.sequence > active.latest_owner_sequence && active.life.is_alive() {
                    plan.combat.observed_primary = fields.primary;
                    plan.combat.observed_secondary = fields.secondary;
                    plan.combat.primary_equipped = fields.primary_equipped;
                }
                if ctx.engine.source_frag_owner_updates()
                    && ctx.accepted_owner_route
                    && h.sequence > active.latest_owner_sequence
                    && !active.is_mounted()
                {
                    accepted_stance = active.owner_presentation(&fields).state.map(|s| s.stance);
                }
                if let CharacterLife::Alive { generation, .. } = active.life
                    && (!ctx.engine.source_frag_owner_updates()
                        || h.sequence > active.latest_owner_sequence)
                {
                    let mut flights = active.projectiles.clone();
                    for report in fields.projectiles.iter().cloned() {
                        if active.hospital_rocket_flights.occupies(report.slot) {
                            plan.audit.projectile_reports.push(json!({
                                "report":report,"accepted":false,"reason":"server_owned_hospital_rocket"
                            }));
                            continue;
                        }
                        let accepted = flights.accept_owner(
                            report.slot,
                            generation,
                            h.sequence,
                            now.as_millis() as u64,
                            report.fields,
                        );
                        if accepted.is_ok_and(|value| value) {
                            let flight_generation = flights.get(report.slot).unwrap().generation;
                            let mut published = report;
                            // Detonation is published only at the authoritative fuse gate.
                            if matches!(published.fields.state, Some(2 | 4)) {
                                published.fields.state = None;
                            }
                            if plan
                                .combat
                                .projectile_replicas
                                .accept_owner(flight_generation, published)?
                                && (published.fields.state.is_some()
                                    || published.fields.position.is_some()
                                    || published.fields.rotation_xyzw.is_some())
                            {
                                let wire = projectile_replication::ProjectileOwnerDelta {
                                    header: replication::UpdateHeader {
                                        sequence: 1,
                                        nak_sequence: 0,
                                        client_index: 0,
                                        object_type_mask: 1 << (16 + published.slot),
                                        timestamp_ms: now.as_millis() as u32,
                                    },
                                    update: &published,
                                }
                                .encode_with_profile(ctx.engine.wire_profile)?;
                                plan.combat
                                    .projectile_publications
                                    .push(ProjectilePublication {
                                        include_owner: false,
                                        wire,
                                    });
                            }
                        }
                        plan.audit
                            .projectile_reports
                            .push(json!({"report":report,"accepted":accepted.is_ok_and(|v|v)}));
                    }
                    let terminals = flights.take_due(now.as_millis() as u64, generation);
                    for terminal in terminals {
                        let faction = active.reservation.character.faction;
                        let smoke = ctx
                            .engine
                            .smoke_grenades
                            .as_deref()
                            .and_then(|catalog| catalog.variant(faction, terminal.item_id));
                        let gas = ctx
                            .engine
                            .gas_grenades
                            .as_deref()
                            .and_then(|catalog| catalog.variant(faction, terminal.item_id));
                        if ctx
                            .engine
                            .flash_grenades
                            .as_deref()
                            .is_some_and(|c| c.variant(faction, terminal.item_id).is_some())
                        {
                            let valid = flights.get(terminal.slot).is_some_and(|f| {
                                f.flash_source.as_ref().is_some_and(|source| {
                                    source.current(active, ctx.engine.round_number)
                                }) && (now.as_millis() as u64).saturating_sub(f.launched_at_ms)
                                    < f.expiry_after_ms
                            }) && ctx.accepted_owner_route
                                && h.sequence > active.latest_owner_sequence
                                && ctx.engine.round_combat_allowed(now)
                                && terminal.reason
                                    == projectile::ProjectileTerminalReason::ReportedDetonation;
                            if valid {
                                plan.combat.flash_pending.push(terminal);
                            }
                            if let Some(removal) = plan
                                .combat
                                .projectile_replicas
                                .take_removal(terminal.slot, terminal.generation)?
                            {
                                if valid {
                                    let update = projectile::ProjectileOwnerUpdate {
                                        slot: terminal.slot,
                                        object_index: active.reservation.player,
                                        fields: projectile::ProjectileOwnerFields {
                                            state: Some(2),
                                            position: terminal.position,
                                            rotation_xyzw: None,
                                        },
                                    };
                                    let wire = projectile_replication::ProjectileOwnerDelta {
                                        header: replication::UpdateHeader {
                                            sequence: 1,
                                            nak_sequence: 0,
                                            client_index: 0,
                                            object_type_mask: 1 << (16 + terminal.slot),
                                            timestamp_ms: now.as_millis() as u32,
                                        },
                                        update: &update,
                                    }
                                    .encode_with_profile(ctx.engine.wire_profile)?;
                                    plan.combat.projectile_publications.push(
                                        ProjectilePublication {
                                            include_owner: false,
                                            wire,
                                        },
                                    );
                                }
                                plan.combat.projectile_cleanup[usize::from(terminal.slot)] =
                                    Some(ProjectileCleanup {
                                        due: if valid {
                                            now.checked_add(Duration::from_millis(500))
                                                .ok_or("flash_cleanup_overflow")?
                                        } else {
                                            now
                                        },
                                        removal,
                                    });
                            }
                            continue;
                        }
                        if let Some(duration) = smoke
                            .map(|v| v.volume_duration_ms)
                            .or_else(|| gas.map(|v| v.volume_duration_ms))
                        {
                            let expired = flights.get(terminal.slot).is_some_and(|flight| {
                                (now.as_millis() as u64).saturating_sub(flight.launched_at_ms)
                                    >= flight.expiry_after_ms
                            });
                            let source = flights
                                .get(terminal.slot)
                                .and_then(|flight| flight.poison_source.as_deref());
                            let gas_source =
                                gas.zip(source.filter(|source| {
                                    source.current(active, ctx.engine.round_number)
                                }));
                            if terminal.reason
                                == projectile::ProjectileTerminalReason::ReportedDetonation
                                && (gas.is_none() || gas_source.is_some())
                                && !expired
                                && ctx.engine.round_combat_allowed(now)
                            {
                                match plan_smoke_terminal(
                                    ctx,
                                    plan,
                                    &terminal,
                                    duration,
                                    gas_source.map(|(variant, source)| (*variant, source)),
                                ) {
                                    Ok(index) => plan.audit.smoke_terminals.push(json!({
                                        "terminal":terminal,"volume_index":index,"created":true,
                                        "volume_duration_ms":duration
                                    })),
                                    Err(code) => {
                                        flights.defer_terminal(terminal.slot, terminal.generation);
                                        plan.gameplay_denial.get_or_insert(code);
                                        plan.audit.smoke_terminals.push(json!({"terminal":terminal,"created":false,"deferred":code}));
                                    }
                                }
                            } else if let Some(removal) = plan
                                .combat
                                .projectile_replicas
                                .take_removal(terminal.slot, terminal.generation)?
                            {
                                plan.combat.projectile_cleanup[usize::from(terminal.slot)] =
                                    Some(ProjectileCleanup { due: now, removal });
                                plan.audit.smoke_terminals.push(
                                    json!({"terminal":terminal,"created":false,"retired":true}),
                                );
                            }
                            // A persistent Volume terminal never enters any frag consumer.
                            continue;
                        }
                        plan.audit.projectile_terminals.push(terminal);
                        if let Some(removal) = plan
                            .combat
                            .projectile_replicas
                            .take_removal(terminal.slot, terminal.generation)?
                        {
                            let detonation = terminal.reason
                                == projectile::ProjectileTerminalReason::ReportedDetonation;
                            if detonation {
                                let update = projectile::ProjectileOwnerUpdate {
                                    slot: terminal.slot,
                                    object_index: active.reservation.player,
                                    fields: projectile::ProjectileOwnerFields {
                                        state: Some(2),
                                        position: terminal.position,
                                        rotation_xyzw: None,
                                    },
                                };
                                let wire = projectile_replication::ProjectileOwnerDelta {
                                    header: replication::UpdateHeader {
                                        sequence: 1,
                                        nak_sequence: 0,
                                        client_index: 0,
                                        object_type_mask: 1 << (16 + terminal.slot),
                                        timestamp_ms: now.as_millis() as u32,
                                    },
                                    update: &update,
                                }
                                .encode_with_profile(ctx.engine.wire_profile)?;
                                plan.combat
                                    .projectile_publications
                                    .push(ProjectilePublication {
                                        include_owner: true,
                                        wire,
                                    });
                            }
                            // Local timing policy: give native state2 a separate frame
                            // opportunity before common active=false deletion.
                            plan.combat.projectile_cleanup[usize::from(terminal.slot)] =
                                Some(ProjectileCleanup {
                                    due: if detonation {
                                        now.checked_add(Duration::from_millis(500))
                                            .ok_or("projectile_cleanup_time_overflow")?
                                    } else {
                                        now
                                    },
                                    removal,
                                });
                        }
                    }
                    plan.combat.new_projectiles = Some(flights);
                }
            }
            falling::plan_report(ctx, plan, h, &fields)?;
            owner_fields = Some(
                json!({"state":fields.state.map(|s|s.state),"pitch":fields.pitch,"yaw":fields.yaw,
                "primary_ammo":fields.primary.map(|i|i.ammo_count),"secondary_ammo":fields.secondary.map(|i|i.ammo_count),"primary_equipped":fields.primary_equipped}),
            );
            if let Some(position) = fields.position {
                reported_position = Some(position);
                position_status =
                    if !ctx.accepted_owner_route || h.client_index != 0 || h.nak_sequence != 0 {
                        "unowned_route"
                    } else if !active.life.is_alive() {
                        "not_alive"
                    } else if active.is_mounted() {
                        "mounted"
                    } else if h.sequence <= active.latest_owner_sequence {
                        "stale"
                    } else if active.tracking.correction_pending() {
                        plan.session.queued_correction_position = Some((position, h.sequence));
                        "correction_pending"
                    } else if !active.tracking.accepts(h.sequence) {
                        "stale"
                    } else {
                        "accepted"
                    };
                if position_status == "accepted" {
                    plan.session.client_position = Some((position, h.sequence));
                }
            }
        }
    }
    Ok(OwnerReport {
        reported_position,
        position_status,
        owner_fields,
        body_supported,
        accepted_stance,
    })
}

fn plan_smoke_terminal<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    terminal: &projectile::ProjectileTerminal,
    duration: u64,
    gas: Option<(gas_grenades::Variant, &gas_grenades::LaunchSource)>,
) -> Result<u16, &'static str> {
    let position = terminal
        .position
        .ok_or("smoke_terminal_position_required")?;
    let object_index = ctx
        .engine
        .free_smoke_volume_index(&plan.combat.smoke_volumes)
        .ok_or("smoke_volume_pool_full")?;
    let expires = ctx
        .now
        .checked_add(Duration::from_millis(duration))
        .ok_or("smoke_volume_time_overflow")?;
    let mut creation = plan
        .combat
        .projectile_replicas
        .creation(terminal.slot, terminal.generation)
        .ok_or("missing_smoke_parent")?;
    creation.position = position;
    let profile = ctx.engine.wire_profile;
    let update = projectile::ProjectileOwnerUpdate {
        slot: terminal.slot,
        object_index: ctx.active.reservation.player,
        fields: projectile::ProjectileOwnerFields {
            state: Some(2),
            position: Some(position),
            rotation_xyzw: None,
        },
    };
    let terminal_wire = projectile_replication::ProjectileOwnerDelta {
        header: replication::UpdateHeader {
            sequence: 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << (16 + terminal.slot),
            timestamp_ms: ctx.now.as_millis() as u32,
        },
        update: &update,
    }
    .encode_with_profile(profile)?;
    let volume_wire = smoke_volume_replication::VolumeCreation {
        object_index,
        position,
    }
    .encode_with_profile(1, ctx.now.as_millis() as u32, profile)?;
    // Validate every candidate before detaching the presentation cache. Event
    // commit installs this clone only after the common record succeeds.
    let removal = plan
        .combat
        .projectile_replicas
        .take_removal(terminal.slot, terminal.generation)?
        .ok_or("missing_smoke_parent")?;
    plan.combat.projectile_cleanup[usize::from(terminal.slot)] = Some(ProjectileCleanup {
        due: if gas.is_some() {
            expires.saturating_add(Duration::from_nanos(1))
        } else {
            expires
        },
        removal,
    });
    plan.combat.smoke_volumes.push(smoke_runtime::Volume {
        object_index,
        generation: terminal.generation,
        creation,
        expires,
        poison: gas.map(|(gas, launch)| crate::smoke_runtime::Poison {
            source: launch.gas.clone(),
            projectile_id: launch.projectile_id,
            round: launch.round,
            timing: crate::active_ability_gas_world::PulseTiming::grenade(
                ctx.now,
                duration as u32,
                Duration::from_millis(gas.warmup_ms),
                Duration::from_millis(gas.interval_ms),
            ),
            falloff: crate::active_ability_effects::BlastFalloff {
                inner_radius: gas.inner_radius_m,
                inner_scale: gas.inner_scale,
                outer_radius: gas.outer_radius_m,
                outer_scale: gas.outer_scale,
            },
            raw_hp: gas.character_hp_per_pulse,
        }),
    });
    // The admitted local Timer/Volume path already executed its effect while
    // reporting state4. Replaying remote2 to that local object can explode it
    // a second time; only peers need this presentation trigger.
    for (wire, include_owner) in [(terminal_wire, false), (volume_wire, true)] {
        plan.combat
            .projectile_publications
            .push(ProjectilePublication {
                include_owner,
                wire,
            });
    }
    Ok(object_index)
}

fn plan_grenade_blasts<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    h: replication::UpdateHeader,
    body_supported: bool,
    accepted_stance: Option<u8>,
) -> Result<std::collections::VecDeque<PendingDamage>, &'static str> {
    let active = ctx.active;
    let attacker = 0x0380_0000 | u32::from(active.reservation.player);
    let world = active.reservation.world;
    let mut grenade_queue = active.grenade_damage.clone();
    let admitted_owner_blast = ctx.accepted_owner_route
        && body_supported
        && plan.rejection.is_none()
        && h.nak_sequence == 0
        && h.client_index == 0
        && ctx.engine.round_combat_allowed(ctx.now);
    let detonated = plan.audit.projectile_terminals.iter().any(|terminal| {
        terminal.reason == projectile::ProjectileTerminalReason::ReportedDetonation
            && terminal.position.is_some()
    });
    let queue_limit = blast_visibility::MAX_BLAST_TARGETS
        * usize::from(projectile::projectile_slot_count(ctx.engine.wire_profile));
    if ctx.engine.frag_character_blast_enabled() && admitted_owner_blast && detonated {
        // Freeze the accepted motion candidate before applying any blast
        // damage to the same Ground runtime.
        let frame = crate::character_frag_damage::FragFrame {
            pose: crate::domination_structural_world::OnFootBlastPose {
                position: plan.session.client_position.map(|(position, _)| position),
                stance: accepted_stance,
            },
            ground: plan.ground_runtime.as_ref(),
            leadership: &plan.leadership,
        };
        match ctx.engine.prepare_frag_character_damage(
            active,
            &plan.audit.projectile_terminals,
            frame,
        ) {
            Ok(effect) => {
                plan.audit.native_grenade_character_checks = effect.audit;
                if grenade_queue.len() + effect.doses.len() <= queue_limit {
                    grenade_queue.extend(effect.doses);
                } else {
                    plan.gameplay_denial
                        .get_or_insert("grenade_damage_queue_full");
                    let available = queue_limit.saturating_sub(grenade_queue.len());
                    grenade_queue.extend(effect.doses.into_iter().take(available));
                }
            }
            Err(error) => {
                plan.gameplay_denial.get_or_insert(error);
                plan.audit
                    .native_grenade_character_checks
                    .push(json!({"error":error}));
            }
        }
    }
    let ground_damage_configured = ctx.engine.ground_damage_policy().is_some();
    let source_ground_frag = ctx.engine.ground_frag_areas().is_some();
    let domination = ctx.engine.domination_gameplay.as_ref();
    let source_helo_frag = domination.is_some_and(|world| world.has_helo_frag_damage_areas());
    let source_hospital_blast = domination.is_some_and(|world| world.has_hospital_blast_areas());
    let isolate_legacy_ground = source_helo_frag || source_hospital_blast;
    // All typed vehicle queries see the same accepted pre-damage motion.
    let owner_view = crate::dynamic_collision_runtime::OwnerCollisionView {
        active_position: plan.session.client_position.map(|(position, _)| position),
        ground: plan.ground_runtime.as_ref(),
    };
    let typed_helo = (admitted_owner_blast && detonated && source_helo_frag).then(|| {
        ctx.engine.prepare_domination_source_helo_frags(
            active,
            &plan.audit.projectile_terminals,
            owner_view,
            ctx.now,
        )
    });
    if admitted_owner_blast && detonated && source_hospital_blast {
        match ctx.engine.prepare_domination_hospital_frags(
            active,
            &plan.audit.projectile_terminals,
            owner_view,
            ctx.now,
        ) {
            Ok(mut effect) => {
                plan.audit.hospital_grenade_checks = std::mem::take(&mut effect.audit);
                if effect.packet.is_some() {
                    plan.hospital_blast = Some(effect);
                }
            }
            Err(error) => {
                plan.gameplay_denial.get_or_insert(error);
                plan.audit
                    .hospital_grenade_checks
                    .push(json!({"error":error}));
            }
        }
    }
    if admitted_owner_blast && detonated && source_ground_frag {
        let result = plan
            .ground_runtime
            .as_ref()
            .or(ctx.engine.ground_runtime.as_ref())
            .ok_or("ground_vehicle_runtime_required")
            .and_then(|staged| {
                ctx.engine.prepare_ground_frag(
                    active,
                    staged,
                    plan.session.client_position.map(|(position, _)| position),
                    &plan.audit.projectile_terminals,
                    ctx.now,
                )
            });
        match result {
            Ok(effect) => {
                if let Some(after) = effect.after {
                    plan.ground_runtime = Some(after);
                }
                plan.ground_damage_relay = effect.packet;
                plan.audit.ground_grenade_checks = effect.audit;
            }
            Err(error) => {
                plan.gameplay_denial.get_or_insert(error);
                plan.audit
                    .ground_grenade_checks
                    .push(json!({"error":error}));
            }
        }
    } else if admitted_owner_blast && detonated && ground_damage_configured {
        let motion_staged = plan.ground_runtime.is_some();
        let baseline = if isolate_legacy_ground {
            plan.ground_runtime.clone()
        } else {
            plan.ground_runtime.take()
        };
        let result = baseline
            .or_else(|| ctx.engine.ground_runtime.clone())
            .ok_or("ground_vehicle_runtime_required")
            .and_then(|mut candidate| {
                let (packet, audit) = ctx.engine.stage_ground_grenade_damage(
                    &mut candidate,
                    active,
                    &plan.audit.projectile_terminals,
                    ctx.now,
                )?;
                Ok((candidate, packet, audit))
            });
        match result {
            Ok((candidate, packet, audit)) => {
                if motion_staged || packet.is_some() {
                    plan.ground_runtime = Some(candidate);
                }
                plan.ground_damage_relay = packet;
                plan.audit.ground_grenade_checks = audit;
            }
            Err(error) if isolate_legacy_ground => {
                plan.gameplay_denial.get_or_insert(error);
                plan.audit
                    .ground_grenade_checks
                    .push(json!({"error":error}));
            }
            Err(error) => return Err(error),
        }
    }
    if admitted_owner_blast && detonated && (ground_damage_configured || source_helo_frag) {
        let helo = typed_helo.unwrap_or_else(|| {
            ctx.engine.prepare_domination_helo_grenades(
                active,
                &plan.audit.projectile_terminals,
                ctx.now,
            )
        });
        match helo {
            Ok(Some(helo)) => {
                plan.helo_runtime = Some(helo.fleet);
                plan.helo_damage_relay.extend(helo.packets);
                plan.helo_damage_requires_creation = source_helo_frag;
                plan.audit.helo_grenade_checks = helo.audit;
            }
            Ok(None) => (),
            Err(error) if source_ground_frag || isolate_legacy_ground => {
                plan.gameplay_denial.get_or_insert(error);
                plan.audit.helo_grenade_checks.push(json!({"error":error}));
            }
            Err(error) => return Err(error),
        }
    }

    if !ctx.engine.frag_character_blast_enabled()
        && ctx.engine.development_grenade_radius
        && body_supported
        && plan.rejection.is_none()
        && h.nak_sequence == 0
        && h.client_index == 0
    {
        let mut targets = Vec::new();
        if let Some((generation, _)) = active.life.damage_health() {
            let position = plan
                .session
                .client_position
                .map(|p| p.0)
                .or(active.tracking.sample.map(|p| p.position));
            let kit = active.equipped_loadout();
            if !active.is_mounted()
                && let (Some(feet), Some(kit)) = (position, kit)
                && let Ok(armor) =
                    armor::ArmorProfile::from_loadout_with_catalog(&kit, &ctx.engine.item_catalog)
            {
                let armor = if matches!(active.life, CharacterLife::Incapacitated { .. }) {
                    armor.for_incapacitated(ctx.engine.item_catalog.retail_combat()?)?
                } else {
                    armor
                };
                targets.push(blast_visibility::CharacterBlastTarget {
                    character_id: (0x03800000 | u32::from(active.reservation.player)),
                    life_generation: generation,
                    feet,
                    armor: armor
                        .with_modifiers(&plan.leadership.modifiers(active))
                        .with_incoming_base(
                            ctx.engine
                                .character_incoming_base(attacker, attacker, world),
                        )?,
                });
            }
        }
        for peer in ctx
            .engine
            .peers
            .values()
            .filter(|p| p.ready_for_updates && !p.is_mounted())
        {
            if let (Some((generation, _)), Some(sample), Some(kit)) = (
                peer.life.damage_health(),
                peer.tracking.sample,
                peer.equipped_loadout(),
            ) {
                let armor =
                    armor::ArmorProfile::from_loadout_with_catalog(&kit, &ctx.engine.item_catalog)?;
                let armor = if matches!(peer.life, CharacterLife::Incapacitated { .. }) {
                    armor.for_incapacitated(ctx.engine.item_catalog.retail_combat()?)?
                } else {
                    armor
                };
                targets.push(blast_visibility::CharacterBlastTarget {
                    character_id: 0x03800000 | u32::from(peer.reservation.player),
                    life_generation: generation,
                    feet: sample.position,
                    armor: armor
                        .with_modifiers(&plan.leadership.modifiers(peer))
                        .with_incoming_base(ctx.engine.character_incoming_base(
                            attacker,
                            0x0380_0000 | u32::from(peer.reservation.player),
                            world,
                        ))?,
                });
            }
        }
        for bot in
            ctx.engine.bot_combat_snapshots().into_iter().filter(|b| {
                ctx.engine.is_registry_bot(b.object_index) && b.health_quarter_points > 0
            })
        {
            let kit = ctx
                .engine
                .registry_bot_loadout(bot.object_index)
                .ok_or("missing_target_loadout")?;
            targets.push(blast_visibility::CharacterBlastTarget {
                character_id: 0x03800000 | u32::from(bot.object_index),
                life_generation: bot.generation,
                feet: bot.position,
                armor: armor::ArmorProfile::from_loadout_with_catalog(
                    kit,
                    &ctx.engine.item_catalog,
                )?
                .with_incoming_base(ctx.engine.character_incoming_base(
                    attacker,
                    0x0380_0000 | u32::from(bot.object_index),
                    world,
                ))?,
            });
        }
        if ctx.engine.development_target_replication_enabled(active)
            && active.dummy_created
            && active.dummy_health.is_some_and(|hp| hp > 0)
        {
            let target = active.development_target.as_ref();
            let kit = match target {
                Some(t) => t.loadout.clone(),
                None => ctx
                    .engine
                    .item_catalog
                    .valor_loadout(0)
                    .ok_or("missing_dummy_loadout")?,
            };
            targets.push(blast_visibility::CharacterBlastTarget {
                character_id: dummy::OBJECT_ID,
                life_generation: target
                    .and_then(|t| t.runtime.map(|r| r.generation))
                    .unwrap_or(1),
                feet: target
                    .map(|t| t.runtime.map_or(t.position, |r| r.position))
                    .or(ctx.engine.development_dummy_position)
                    .ok_or("missing_alternate_spawn")?,
                armor: armor::ArmorProfile::from_loadout_with_catalog(
                    &kit,
                    &ctx.engine.item_catalog,
                )?
                .with_incoming_base(ctx.engine.character_incoming_base(
                    attacker,
                    dummy::OBJECT_ID,
                    world,
                ))?,
            });
        }
        for terminal in &plan.audit.projectile_terminals {
            if terminal.reason != projectile::ProjectileTerminalReason::ReportedDetonation {
                continue;
            }
            let Some(blast) = terminal.position else {
                continue;
            };
            let checks = blast_visibility::resolve_grenade_blast_for_item_with_profile(
                ctx.engine.collision_world.as_ref(),
                ctx.engine.map_name(),
                terminal.slot,
                terminal.generation,
                blast,
                &targets,
                terminal.item_id,
                &ctx.engine.item_catalog,
                ctx.engine.wire_profile,
            )?;
            for check in checks {
                if check.amount_hp > 0.0 {
                    if grenade_queue.len() < queue_limit {
                        grenade_queue.push_back(PendingDamage {
                            source: PendingDamageSource::Other,
                            target_character_id: check.character_id,
                            target_reservation_id: if check.character_id
                                == (0x03800000 | u32::from(active.reservation.player))
                            {
                                Some(active.reservation.id.clone())
                            } else {
                                ctx.engine
                                    .peers
                                    .values()
                                    .find(|p| {
                                        (0x03800000 | u32::from(p.reservation.player))
                                            == check.character_id
                                    })
                                    .map(|p| p.reservation.id.clone())
                            },
                            id: format!(
                                "grenade-{}-{}-{:08x}",
                                terminal.slot, terminal.generation, check.character_id
                            ),
                            generation: check.life_generation,
                            damage: damage::ResolvedDamage {
                                amount_hp: check.amount_hp,
                                kind: damage::DamageKind::Explosive,
                                kinetic_response: check.damage_type & 8 != 0,
                                location: check.hit_location,
                                attacker_id: (0x03800000 | u32::from(active.reservation.player)),
                                source_object_id: ((16 + u32::from(terminal.slot)) << 24)
                                    | 0x00800000
                                    | u32::from(active.reservation.player),
                                user_data: 0,
                            },
                        });
                    } else {
                        plan.gameplay_denial = Some("grenade_damage_queue_full");
                    }
                }
                plan.audit.grenade_blast_checks.push(check);
            }
        }
    }
    if plan.combat.new_projectiles.is_some() {
        plan.combat.new_grenade_damage = Some(grenade_queue.clone());
    }
    Ok(grenade_queue)
}

/// Choose the next reply in the established priority order. Objective bootstrap
/// can carry multiple reliable packets in one reply.
fn plan_update_reply<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    h: replication::UpdateHeader,
    body_supported: bool,
    grenade_queue: std::collections::VecDeque<PendingDamage>,
) -> Result<(), &'static str> {
    let active = ctx.active;
    let now = ctx.now;
    let next_sequence = ctx.next_sequence;
    let history_available = ctx.history_available;
    if h.client_index != 0 {
        plan.rejection = Some("replication_client_mismatch");
    } else if plan.rejection.is_some() || !body_supported {
        // Malformed/unsupported bodies release no replies or state.
    } else if active.ready_for_updates && h.nak_sequence != 0 {
        // Replay bytes and field versions exactly, regardless of life.
        plan.outgoing = active
            .replication_history
            .iter()
            .find(|wire| {
                replication::ObjectUpdate::decode_with_profile(wire, ctx.engine.wire_profile)
                    .is_ok_and(|u| u.header.sequence == h.nak_sequence)
            })
            .cloned();
    } else if active.ready_for_updates
        && active.initial_objects.is_some()
        && history_available
        && ctx
            .engine
            .ground_vehicles
            .as_ref()
            .is_some_and(|world| active.ground_vehicle_batch < world.batch_count())
    {
        let packet = ctx.engine.current_ground_batch(
            active.ground_vehicle_batch,
            next_sequence,
            now.as_millis() as u32,
        )?;
        plan.outgoing = Some(packet.clone());
        plan.replication.ground_vehicle_creation = Some(packet);
    } else if active.ready_for_updates
        && active.initial_objects.is_some()
        && history_available
        && !active.helo_creation_sent
        && let Some(fleet) = ctx
            .engine
            .domination_gameplay
            .as_ref()
            .and_then(|world| world.state.helo.as_ref())
    {
        let carriers = fleet
            .carriers
            .iter()
            .map(|carrier| carrier.creation(next_sequence, now.as_millis() as u32))
            .collect::<Vec<_>>();
        let packet =
            domination_helo::HeloCreation::encode_list(&carriers, ctx.engine.wire_profile)?;
        plan.outgoing = Some(packet.clone());
        plan.replication.helo_creation = Some(packet);
    } else if active.ready_for_updates
        && (active.pending_damage.is_some()
            || (!grenade_queue.is_empty()
                && (!ctx.engine.frag_character_blast_enabled()
                    || (ctx.accepted_owner_route && ctx.engine.round_combat_allowed(now)))))
        && history_available
    {
        update_actions::plan_pending_damage(ctx, plan, grenade_queue)?;
    } else if active.ready_for_updates
        && active
            .healing
            .as_ref()
            .is_some_and(|effect| effect.is_due(now))
        && active.life.is_alive()
        && history_available
    {
        update_actions::plan_healing_tick(ctx, plan)?;
    } else if active.ready_for_updates
        && h.nak_sequence == 0
        && history_available
        && active.initial_objects.is_some()
        && active.objective_delta.is_none()
        && (ctx.engine.wire_profile.application_id() == 20231
            || active.emplacement_creation.is_some())
        && ctx.engine.match_profile().has_objectives
    {
        let timestamp = now.as_millis() as u32;
        let catalog = ctx
            .engine
            .objective_catalog
            .as_ref()
            .ok_or("retail_objective_catalog_required")?;
        let (packets, sequence_batches) = if let Some(world) = ctx
            .engine
            .objective_world
            .as_ref()
            .filter(|w| !w.is_pristine())
        {
            let sequences = if let Some(gameplay) = &ctx.engine.domination_gameplay {
                gameplay.current_sequences()
            } else if let Some(gameplay) = &ctx.engine.acquisition_gameplay {
                gameplay.current_structural_sequences()
            } else {
                Vec::new()
            };
            (
                world
                    .snapshot(next_sequence, timestamp)
                    .encode_bootstrap_batches(ctx.engine.wire_profile)?,
                replication::ObjectiveSequenceDelta::encode_batches(
                    &sequences,
                    timestamp,
                    ctx.engine.wire_profile,
                )?,
            )
        } else {
            (
                vec![
                    catalog
                        .snapshot(next_sequence, timestamp)
                        .encode_with_profile(ctx.engine.wire_profile)?,
                ],
                Vec::new(),
            )
        };
        if (!ctx.engine.world_tick_enabled
            && ctx.engine.maximum_players == 1
            && active.replication_history.len() + packets.len() > MAX_REPLICATION_HISTORY)
            || (!sequence_batches.is_empty()
                && (active.replication_backlogged
                    || active.ordinary_shared_len() + sequence_batches.len()
                        > crate::multiplayer::MAX_SHARED_QUEUE))
        {
            // Leave bootstrap incomplete so a later pull retries the full snapshot.
            return Ok(());
        }
        let mut packets = packets.into_iter();
        let first = packets
            .next()
            .expect("validated nonempty objective bootstrap");
        plan.outgoing = Some(first.clone());
        plan.replication.objective_delta = Some(first);
        plan.replication.objective_control_tail = packets.collect();
        plan.replication.objective_count = catalog.object_count();
        plan.replication.domination_sequence_batches = sequence_batches;
    } else if active.ready_for_updates
        && h.nak_sequence == 0
        && history_available
        && active.objective_needs_clear
        && (active.spawn_delta.is_none() || active.life.is_alive())
        && ctx.engine.match_profile().has_objectives
    {
        // Explicit local per-spawn orders refresh. The native HUD
        // fades the assignment after45 seconds; unchanged resends
        // are not proven to retrigger its watched binding.
        let packet = replication::SquadObjectiveAssignment {
            sequence: next_sequence,
            timestamp_ms: now.as_millis() as u32,
            squad_index: active.squad_catalog.squad_index(),
            objective_index: u16::MAX,
        }
        .encode_with_profile(ctx.engine.wire_profile)?;
        plan.outgoing = Some(packet.clone());
        plan.replication.objective_clear = Some(packet);
    } else if active.ready_for_updates
        && h.nak_sequence == 0
        && history_available
        && active.objective_delta.is_some()
        && !active.objective_assigned
        && (active.spawn_delta.is_none() || active.life.is_alive())
    {
        // Source-list first objective for this actor's current squad.
        let objective_index = ctx
            .engine
            .initial_objective_index(active.squad_catalog.squad_index())?;
        let assignment = replication::SquadObjectiveAssignment {
            sequence: next_sequence,
            timestamp_ms: now.as_millis() as u32,
            squad_index: active.squad_catalog.squad_index(),
            objective_index,
        }
        .encode_with_profile(ctx.engine.wire_profile)?;
        plan.outgoing = Some(assignment.clone());
        plan.replication.objective_assignment = Some(assignment);
        plan.replication.objective_target = Some(objective_index);
    } else if active.ready_for_updates
        && active.initial_objects.is_some()
        && active.emplacement_creation.is_none()
        && history_available
    {
        let mut bunkers = if ctx.engine.match_profile().has_bunkers {
            ctx.engine
                .bunker_world
                .as_ref()
                .ok_or("bunker_world_unavailable")?
                .creations(next_sequence, now.as_millis() as u32)
        } else {
            Vec::new()
        };
        for bunker in &mut bunkers {
            let ownership = ctx
                .engine
                .bunker_ownership_after(bunker.object_index, u16::MAX, None);
            bunker.owner_index = ownership.owner;
            bunker.turret_owners[0] = ownership.turret_owner;
        }
        let creation = replication::AlphaSpawnSetup {
            sequence: next_sequence,
            timestamp_ms: now.as_millis() as u32,
        }
        .encode_with_bunkers_with_profile(
            active.reservation.player,
            if ctx.engine.match_profile().game_type == 2
                && ctx.engine.match_profile().auto_assign_teams
            {
                None
            } else {
                ctx.engine.alpha_owner()
            },
            active.squad_catalog.as_ref(),
            &bunkers,
            ctx.engine.wire_profile,
        )?;
        plan.outgoing = Some(creation.clone());
        plan.replication.emplacement_creation = Some(creation);
    } else if ctx.engine.development_target_replication_enabled(active)
        && active.ready_for_updates
        && h.nak_sequence == 0
        && history_available
        && (active.objective_assigned || !ctx.engine.match_profile().has_objectives)
        && !active.dummy_created
    {
        let packet = if let Some(target) = &active.development_target {
            if target.script.is_some() {
                dummy::character_with_loadout(
                    next_sequence,
                    now.as_millis() as u32,
                    &target.loadout,
                    target.faction,
                    ctx.engine.wire_profile,
                    "BOT Mercer",
                    target.squad,
                )?
            } else {
                dummy::creation_with_named_loadout(
                    next_sequence,
                    now.as_millis() as u32,
                    &target.loadout,
                    target.faction,
                    ctx.engine.wire_profile,
                    "Target",
                )?
            }
        } else {
            dummy::creation_with_catalog_and_profile(
                next_sequence,
                now.as_millis() as u32,
                &ctx.engine.item_catalog,
                ctx.engine.wire_profile,
            )?
        };
        plan.outgoing = Some(packet.clone());
        plan.replication.dummy_delta = Some(packet);
        plan.combat.dummy_created = true;
    } else if ctx.engine.development_target_replication_enabled(active)
        && h.nak_sequence == 0
        && history_available
        && active.dummy_created
        && let Some(target) = &active.development_target
        && target.script.is_some()
        && !target.roster_published
    {
        let packet = replication::multiplayer::SquadRoster {
            sequence: next_sequence,
            timestamp_ms: now.as_millis() as u32,
            squad: target.squad.squad_index(),
            members: [
                dummy::OBJECT_INDEX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
            ],
            leader: dummy::OBJECT_INDEX,
        }
        .encode_with_profile(ctx.engine.wire_profile)?;
        plan.outgoing = Some(packet.clone());
        plan.replication.dummy_delta = Some(packet);
        plan.combat.bot_roster_published = true;
    } else if ctx.engine.development_target_replication_enabled(active)
        && active.ready_for_updates
        && h.nak_sequence == 0
        && history_available
        && active.dummy_created
        && active.dummy_health.is_none()
    {
        let packet = if let Some(target) = &active.development_target {
            dummy::respawn_with_loadout(
                next_sequence,
                now.as_millis() as u32,
                target.position,
                &target.loadout,
                ctx.engine.wire_profile,
            )?
        } else {
            dummy::respawn_at_with_catalog_and_profile(
                next_sequence,
                now.as_millis() as u32,
                ctx.engine
                    .development_dummy_position
                    .ok_or("missing_alternate_spawn")?,
                &ctx.engine.item_catalog,
                ctx.engine.wire_profile,
            )?
        };
        plan.outgoing = Some(packet.clone());
        plan.replication.dummy_delta = Some(packet);
        plan.combat.dummy_health = Some(dummy::INITIAL_HEALTH_QUARTER_POINTS);
        if active
            .development_target
            .as_ref()
            .is_some_and(|t| t.script.is_some())
        {
            plan.combat.bot_runtime = Some(development_target::BotRuntime::spawned(
                now,
                active.development_target.as_ref().unwrap().position,
            ));
            plan.audit.bot_script = Some(
                json!({"object_id":dummy::OBJECT_ID,"name":"BOT Mercer","team":active.development_target.as_ref().unwrap().squad.team(),"squad_index":active.development_target.as_ref().unwrap().squad.squad_index(),"faction":active.development_target.as_ref().unwrap().faction,"owner_life_generation":active.development_target.as_ref().unwrap().generation,"owner_index":dummy::SERVER_OWNER,"generation":1,"phase":"spawn","sequence":next_sequence,"timestamp_ms":now.as_millis() as u32,"position":active.development_target.as_ref().unwrap().position,"combat_authority":ctx.engine.item_catalog.retail_combat().is_ok()}),
            );
        }
    } else if !ctx.engine.world_tick_enabled
        && ctx.engine.development_target_replication_enabled(active)
        && h.nak_sequence == 0
        && history_available
        && active.dummy_created
        && active.dummy_health.is_some()
        && let Some(target) = &active.development_target
        && let Some(step) = {
            let actors = ctx.engine.actor_grid(now)?;
            target.script_step_with_actors(
                next_sequence,
                now,
                ctx.engine.wire_profile,
                ctx.engine.collision_world.as_ref(),
                ctx.engine.map_name(),
                ctx.engine.bot_movement_body(),
                active.dummy_health.unwrap_or(0),
                ctx.engine.fall_damage_rules.as_ref(),
                Some(crate::actor_collision::Query {
                    grid: &actors,
                    actor: target.object_index,
                }),
            )?
        }
    {
        plan.outgoing = Some(step.packet.clone());
        plan.replication.dummy_delta = Some(step.packet);
        plan.combat.dummy_health = step.health;
        plan.combat.bot_runtime = Some(step.runtime);
        plan.audit.bot_script = Some(step.receipt);
    } else if active.ready_for_updates
        && active.entered_game
        && active
            .pending_spawn_deadline
            .is_some_and(|deadline| now >= deadline)
        && active
            .squad_catalog
            .as_ref()
            .alpha(
                active
                    .selected_spawn
                    .map_or(active.squad_spawn, |s| s.spawn_point_index),
            )
            .is_some()
        && ctx.engine.actor_matches_team(active)
        && active.life.next_generation().is_some()
        && ctx.engine.match_started
        && history_available
    {
        update_actions::plan_respawn(ctx, plan)?;
    }
    // While the picker is open, publish rollover without enqueuing.
    // A queued due spawn has priority above; never move its deadline
    // forward before the character can consume that wave.
    if plan.rejection.is_none()
        && body_supported
        && plan.outgoing.is_none()
        && h.nak_sequence == 0
        && active.ready_for_updates
        && active.entered_game
        && active.life.next_generation().is_some()
        && active.pending_spawn_deadline.is_none()
        && history_available
        && ctx.engine.actor_matches_team(active)
        && let Some(clock) = ctx.engine.team_waves[usize::from(active.team())]
    {
        let deadline = clock.next_after(now)?;
        if active.published_wave_deadline != Some(deadline) {
            let packet = replication::TeamSpawnDeadline {
                sequence: next_sequence,
                timestamp_ms: now.as_millis() as u32,
                object_index: 0,
                team: active.team(),
                deadline_ms: deadline.as_millis() as u32,
            }
            .encode_with_profile(ctx.engine.wire_profile)?;
            plan.outgoing = Some(packet.clone());
            plan.replication.wave_delta = Some(packet);
            plan.session.published_wave_deadline = Some(deadline);
        }
    }
    Ok(())
}
