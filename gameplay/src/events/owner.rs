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
    } = plan_owner_report(ctx, plan, &update)?;
    let mut grenade_queue = plan_grenade_blasts(ctx, plan, h, body_supported)?;
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
    Ok(
        json!({"type":"object_update","sequence":h.sequence,"nak_sequence":h.nak_sequence,
        "client_index":h.client_index,"object_type_mask":h.object_type_mask,"timestamp_ms":h.timestamp_ms,
        "body_supported":body_supported,
        "reported_position":reported_position,"position_status":position_status,"owner_fields":owner_fields,
        "fall_damage":plan.audit.falling_damage,
        "applied_damage":plan.audit.applied_damage,"applied_healing":plan.audit.applied_healing,
        "projectile_reports":plan.audit.projectile_reports,"projectile_terminals":plan.audit.projectile_terminals,
        "grenade_blast_checks":plan.audit.grenade_blast_checks,
        "grenade_visibility_policy":if ctx.engine.development_grenade_radius {"disk_triangles_three_body_samples"} else {"unknown_no_damage"},
        "initial_objectives":plan.replication.objective_delta.is_some(),"objective_count":plan.replication.objective_count,
        "initial_emplacement":plan.replication.emplacement_creation.is_some(),
        "assigned_objective":plan.replication.objective_target,
        "spawn_sent":plan.replication.spawn_delta.is_some(),
        "development_target_created":plan.combat.dummy_created,
        "bot_script":plan.audit.bot_script,
        "development_target_spawned":plan.combat.dummy_health.is_some() && plan.audit.bot_script.as_ref().is_none_or(|r|r["phase"]=="spawn" || r["phase"]=="respawn"),
        "team_wave_deadline_ms":plan.session.published_wave_deadline.map(|d|d.as_millis() as u32),
        "spawn_point_index":plan.audit.spawned_point,"spawn_mounted_seat":plan.session.new_mounted_seat.flatten(),
        "life_generation":if plan.replication.spawn_delta.is_some(){active.life.next_generation()}else{None},
        "equipped_inventory_index":plan.session.new_inventory_handle.map(|h|h.inventory_index),
        "replication_sequence":if plan.replication.spawn_delta.is_some() || plan.replication.wave_delta.is_some(){Some(next_sequence)}else{None}}),
    )
}

struct OwnerReport {
    reported_position: Option<replication::CharacterPosition>,
    position_status: &'static str,
    owner_fields: Option<Value>,
    body_supported: bool,
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
            let mut fields = report.character.unwrap_or_default();
            fields.projectiles = report.projectiles;
            fields
        });
    let mut reported_position = None;
    let mut position_status = "absent";
    let mut owner_fields = None;
    let mut body_supported = true;
    match fields {
        Err(
            error @ (owner_update::OwnerUpdateError::UnsupportedCharacterField
            | owner_update::OwnerUpdateError::UnsupportedObjectType
            | owner_update::OwnerUpdateError::UnsupportedProjectileField
            | owner_update::OwnerUpdateError::UnsupportedTurretField),
        ) => {
            // Coverage gaps discard the whole body without disconnecting.
            plan.gameplay_denial.get_or_insert(error.code());
            body_supported = false;
            position_status = "unsupported";
        }
        Err(owner_update::OwnerUpdateError::Invalid(code)) => plan.rejection = Some(code),
        Ok(fields) => {
            if h.client_index == 0 && h.nak_sequence == 0 {
                plan.session.new_owner_sequence =
                    Some(active.latest_owner_sequence.max(h.sequence));
                if h.sequence > active.latest_owner_sequence && active.life.is_alive() {
                    plan.combat.observed_primary = fields.primary;
                    plan.combat.observed_secondary = fields.secondary;
                    plan.combat.primary_equipped = fields.primary_equipped;
                }
                if let CharacterLife::Alive { generation, .. } = active.life {
                    let mut flights = active.projectiles.clone();
                    for report in fields.projectiles.iter().cloned() {
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
                    plan.audit.projectile_terminals =
                        flights.take_due(now.as_millis() as u64, generation);
                    for terminal in &plan.audit.projectile_terminals {
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
                position_status = if !active.life.is_alive() {
                    "not_alive"
                } else if active.mounted_seat.is_some() {
                    "mounted"
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
    })
}

fn plan_grenade_blasts<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    h: replication::UpdateHeader,
    body_supported: bool,
) -> Result<std::collections::VecDeque<PendingDamage>, &'static str> {
    let active = ctx.active;
    let mut grenade_queue = active.grenade_damage.clone();
    if ctx.engine.development_grenade_radius
        && body_supported
        && plan.rejection.is_none()
        && h.nak_sequence == 0
        && h.client_index == 0
    {
        let mut targets = Vec::new();
        if let CharacterLife::Alive { generation, .. } = active.life {
            let position = plan
                .session
                .client_position
                .map(|p| p.0)
                .or(active.tracking.sample.map(|p| p.position));
            let kit = active.current_inventory_handle.and_then(|h| {
                active.reservation.character.replication_loadout(
                    &ctx.engine.item_catalog,
                    h.inventory_index,
                    active.wire_profile,
                )
            });
            if active.mounted_seat.is_none()
                && let (Some(feet), Some(kit)) = (position, kit)
                && let Ok(armor) =
                    armor::ArmorProfile::from_loadout_with_catalog(&kit, &ctx.engine.item_catalog)
            {
                targets.push(blast_visibility::CharacterBlastTarget {
                    character_id: (0x03800000 | u32::from(active.reservation.player)),
                    life_generation: generation,
                    feet,
                    armor: armor.with_modifiers(&plan.leadership.modifiers(active)),
                });
            }
        }
        for peer in ctx
            .engine
            .peers
            .values()
            .filter(|p| p.ready_for_updates && p.mounted_seat.is_none())
        {
            if let (CharacterLife::Alive { generation, .. }, Some(sample), Some(handle)) = (
                peer.life,
                peer.tracking.sample,
                peer.current_inventory_handle,
            ) && let Some(kit) = peer.reservation.character.replication_loadout(
                &ctx.engine.item_catalog,
                handle.inventory_index,
                peer.wire_profile,
            ) {
                targets.push(blast_visibility::CharacterBlastTarget {
                    character_id: 0x03800000 | u32::from(peer.reservation.player),
                    life_generation: generation,
                    feet: sample.position,
                    armor: armor::ArmorProfile::from_loadout_with_catalog(
                        &kit,
                        &ctx.engine.item_catalog,
                    )?
                    .with_modifiers(&plan.leadership.modifiers(peer)),
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
                )?,
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
                )?,
            });
        }
        for terminal in &plan.audit.projectile_terminals {
            if terminal.reason != projectile::ProjectileTerminalReason::ReportedDetonation {
                continue;
            }
            let Some(blast) = terminal.position else {
                continue;
            };
            let checks = blast_visibility::resolve_grenade_blast_for_item(
                ctx.engine.collision_world.as_ref(),
                ctx.engine.map_name(),
                terminal.slot,
                terminal.generation,
                blast,
                &targets,
                terminal.item_id,
                &ctx.engine.item_catalog,
            )?;
            for check in checks {
                if check.amount_hp > 0.0 {
                    if grenade_queue.len() < blast_visibility::MAX_BLAST_TARGETS * 3 {
                        grenade_queue.push_back(PendingDamage {
                            falling: false,
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

/// Choose at most one reliable reply in the established priority order. Replay,
/// damage, healing, initialization and respawn retain their original precedence.
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
        && (active.pending_damage.is_some() || !grenade_queue.is_empty())
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
        let delta = if let Some(catalog) = &ctx.engine.objective_catalog {
            plan.replication.objective_count = catalog.object_count();
            if let Some(world) = ctx
                .engine
                .objective_world
                .as_ref()
                .filter(|w| !w.is_pristine())
            {
                world
                    .snapshot(next_sequence, timestamp)
                    .encode_with_profile(ctx.engine.wire_profile)?
            } else {
                catalog
                    .snapshot(next_sequence, timestamp)
                    .encode_with_profile(ctx.engine.wire_profile)?
            }
        } else {
            return Err("retail_objective_catalog_required");
        };
        plan.outgoing = Some(delta.clone());
        plan.replication.objective_delta = Some(delta);
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
        let creation = replication::AlphaSpawnSetup {
            sequence: next_sequence,
            timestamp_ms: now.as_millis() as u32,
        }
        .encode_with_bunkers_with_profile(
            active.reservation.player,
            ctx.engine.alpha_owner(),
            active.squad_catalog.as_ref(),
            &ctx.engine
                .bunker_world
                .as_ref()
                .ok_or("bunker_world_unavailable")?
                .creations(next_sequence, now.as_millis() as u32),
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
        && !active.life.is_alive()
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
