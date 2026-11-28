//! Ground-vehicle impact reports (`0x020d`) from the driving client.
//!
//! The driver's client names its vehicle and the character it touched. The
//! server checks that the sender is that vehicle's current driver, finds the
//! character, and applies the damage of `crate::vehicle_collision::Rules` as
//! ordinary typed damage: ExternalKinetic, at hit location 0, through the
//! victim's armor, friendly-fire base and owned modifiers, attributed to the
//! driver with the vehicle as its source object.
use super::*;
use crate::vehicle_collision::{ReportKind, VehicleReport, closing_speed};

/// A character on foot that the vehicle can hit: a human in this world or a
/// registered bot.
enum Victim<'a> {
    Human(&'a Active),
    Bot(bot_registry::BotCombatSnapshot),
}

struct Impact {
    amount: f32,
    damage: replication::CharacterDamage,
    calculation: Value,
}

pub(super) fn plan<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    report: VehicleReport,
) -> Result<Value, &'static str> {
    let mut audit = json!({"type":"vehicle_report","kind":report.kind.name(),
        "vehicle_id":report.attacker,"target_id":report.defender});
    if let Err(code) = apply(ctx, plan, report, &mut audit) {
        plan.gameplay_denial = Some(code);
    }
    Ok(audit)
}

fn apply<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    report: VehicleReport,
    audit: &mut Value,
) -> Result<(), &'static str> {
    let engine = ctx.engine;
    let active = ctx.active;
    if !ctx.accepted_client_route {
        return Err("vehicle_report_requires_native_client_route");
    }
    if report.kind == ReportKind::Stuck {
        return Err("vehicle_stuck_report_unsupported");
    }
    // Retail helicopters inherit the base vehicle's handler, which does nothing.
    if report.attacker >> 24 == 2 {
        audit["outcome"] = json!("helicopter_collisions_have_no_effect");
        return Ok(());
    }
    if report.attacker & 0xffff_0000 != 0x0100_0000 {
        return Err("vehicle_report_requires_ground_vehicle");
    }
    let rules = engine
        .vehicle_collision_rules
        .as_ref()
        .ok_or("vehicle_collision_rules_missing")?;
    let index = report.attacker as u16;
    if !engine.ground_driver_authority(active, index) {
        return Err("vehicle_report_requires_vehicle_driver");
    }
    if !engine.match_started || !engine.round_combat_allowed(ctx.now) {
        return Err("vehicle_collision_requires_active_round");
    }
    if !ctx.history_available {
        return Err("replication_history_limit");
    }
    let vehicle = engine
        .ground_runtime
        .as_ref()
        .and_then(|runtime| runtime.get(index))
        .ok_or("unknown_ground_vehicle")?;
    let vehicle_team = engine
        .ground_vehicles
        .as_ref()
        .and_then(|catalog| catalog.vehicle(index))
        .ok_or("unknown_ground_vehicle")?
        .team;
    let world = active.reservation.world;
    let victim = engine
        .peers
        .values()
        .find(|peer| {
            0x0380_0000 | u32::from(peer.reservation.player) == report.defender
                && peer.reservation.world == world
                && peer.ready_for_updates
                && peer.entered_game
                && !peer.is_mounted()
                && peer.life.damage_health().is_some()
        })
        .map(Victim::Human)
        .or_else(|| {
            engine
                .bot_combat_snapshots()
                .into_iter()
                .find(|bot| {
                    engine.is_registry_bot(bot.object_index)
                        && 0x0380_0000 | u32::from(bot.object_index) == report.defender
                        && bot.health_quarter_points > 0
                })
                .map(Victim::Bot)
        })
        .ok_or("unsupported_vehicle_collision_target")?;
    let now_ms = ctx.now.as_millis() as u64;
    // Someone who just got out of this vehicle is not run over by it.
    if let Victim::Human(peer) = &victim
        && let Some((left, at)) = peer.ground_exit
        && left == index
        && ctx.now.saturating_sub(at) < Duration::from_millis(rules.recent_exit_grace_ms)
    {
        audit["outcome"] = json!("recent_occupant");
        return Ok(());
    }
    let (previous, latest) = vehicle.driver_motion();
    let vehicle_velocity = rules.estimated_velocity(previous, latest, now_ms);
    let victim_velocity = match &victim {
        Victim::Human(peer) => {
            let (previous, latest) = peer.tracking.client_motion();
            rules.estimated_velocity(previous, latest, now_ms)
        }
        // Bot motion has no sampled history here; it counts as standing still.
        Victim::Bot(_) => None,
    };
    let speed = closing_speed(vehicle_velocity, victim_velocity);
    let victim_team = match &victim {
        Victim::Human(peer) => peer.team(),
        Victim::Bot(bot) => bot.team,
    };
    let friendly_fire = (victim_team == vehicle_team).then(|| {
        engine
            .character_damage_rules
            .as_ref()
            .map_or(1.0, |rules| rules.friendly_fire_multiplier())
    });
    audit["closing_speed_mps"] = json!(speed);
    audit["vehicle_velocity"] = json!(vehicle_velocity);
    audit["target_velocity"] = json!(victim_velocity);
    let Some(raw) = rules.raw_damage(report.kind, speed, friendly_fire) else {
        audit["outcome"] = json!("below_minimum_speed");
        return Ok(());
    };
    audit["raw_damage_hp"] = json!(raw);
    let driver = 0x0380_0000 | u32::from(active.reservation.player);
    let Some(impact) = resolve(ctx, plan, &victim, raw, driver, report)? else {
        audit["outcome"] = json!("no_damage");
        return Ok(());
    };
    let damage = impact.damage;
    let lethal = damage.health_quarter_points == 0;
    let mut life = None;
    if let Victim::Human(peer) = &victim {
        let (model, next) = engine.plan_human_damage(peer, impact.amount, damage, ctx.now)?;
        life = Some((model, next));
    }
    let bot_kd = match &victim {
        Victim::Bot(bot) if lethal && engine.match_terminal.is_none() => {
            Some(bot_firearm_kd::Plan::prepare(
                engine,
                world,
                None,
                Some(bot_firearm_kd::Life {
                    index: bot.object_index,
                    generation: bot.generation,
                    team: bot.team,
                    health: bot.health_quarter_points,
                }),
            )?)
        }
        _ => None,
    };
    let damage = life.map_or(damage, |(model, _)| model);
    let notice = match &victim {
        Victim::Human(peer) if !peer.life.is_alive() => None,
        _ if !lethal => None,
        _ => Some(engine.plan_vehicle_impact_death_notice(
            world,
            report.defender as u16,
            driver,
            report.attacker,
            rules.damage_source_type,
        )?),
    };
    let packet = engine
        .character_damage_update(
            ctx.next_sequence,
            now_ms as u32,
            report.defender as u16,
            damage,
            lethal,
        )
        .encode_with_profile(engine.wire_profile)?;

    // Everything is resolved; stage the effects.
    match &victim {
        Victim::Human(peer) => {
            let (_, next) = life.unwrap();
            plan.combat
                .peer_lives
                .push((peer.reservation.id.clone(), next));
            if matches!(peer.life, CharacterLife::Incapacitated { downed, .. } if !downed.is_expired(ctx.now))
                && matches!(next, CharacterLife::AwaitingRespawn { .. })
            {
                statistics::finished(plan, active, peer.team());
                if peer.team() != active.team()
                    && let Some(recipient) = progression::RewardRecipient::from_active(active)
                {
                    plan.combat.new_rewards.push(recipient.event(
                        format!(
                            "finish-{}-{}-{}",
                            active.reservation.id, peer.reservation.id, ctx.next_sequence
                        ),
                        progression::RewardKind::EnemyFinished,
                        1,
                    ));
                }
            }
            if let Some((generation, _)) = peer.life.damage_health() {
                plan.combat
                    .peer_damage
                    .push((peer.reservation.id.clone(), generation, damage));
            }
        }
        Victim::Bot(bot) => {
            plan.combat
                .bot_damage
                .push((bot.object_index, bot.generation, damage));
            if lethal {
                statistics::target_killed(plan, active, bot.team);
                if active.team() != bot.team
                    && let Some(recipient) = progression::RewardRecipient::from_active(active)
                {
                    let reward = engine.frago_combat_reward(
                        recipient.bot_killed(bot.object_index, bot.generation),
                        active,
                        Some(crate::frago::point(bot.position)),
                        plan.frago_frame(engine),
                    );
                    plan.combat.new_rewards.push(reward);
                }
            }
            plan.combat.bot_firearm_kd = bot_kd;
        }
    }
    audit["damage_calculation"] = impact.calculation;
    audit["applied_damage"] = json!({"amount_hp":impact.amount,
        "health_after_quarter_points":damage.health_quarter_points,
        "replication_sequence":ctx.next_sequence});
    audit["death_notice"] = json!(notice.as_ref().map(|n| n.audit()));
    plan.combat.vehicle_impact_notice = notice;
    plan.replication.damage_delta = Some(packet.clone());
    plan.outgoing = Some(packet);
    Ok(())
}

/// The victim's adjusted damage: friendly-fire base and owned modifiers, then
/// its armor at hit location 0 with no armor reduction.
fn resolve<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &EventPlan,
    victim: &Victim<'_>,
    raw: f32,
    driver: u32,
    report: VehicleReport,
) -> Result<Option<Impact>, &'static str> {
    let engine = ctx.engine;
    let catalog = engine.item_catalog.retail_combat()?;
    let (kit, modifiers, health) = match victim {
        Victim::Human(peer) => (
            peer.equipped_loadout()
                .or_else(|| {
                    if peer.current_inventory_handle.is_none() {
                        peer.reservation.character.replication_loadout(
                            &engine.item_catalog,
                            0,
                            peer.wire_profile,
                        )
                    } else {
                        None
                    }
                })
                .ok_or("missing_target_loadout")?,
            plan.leadership.modifiers(peer),
            peer.life
                .damage_health()
                .ok_or("vehicle_collision_target_not_alive")?
                .1,
        ),
        Victim::Bot(bot) => (
            engine
                .registry_bot_loadout(bot.object_index)
                .ok_or("missing_target_loadout")?
                .clone(),
            skill_catalog::OwnedModifiers::default(),
            bot.health_quarter_points,
        ),
    };
    let armor = armor::ArmorProfile::from_retail_loadout(&kit, catalog)?
        .with_modifiers(&modifiers)
        .with_incoming_base(engine.character_incoming_base(
            driver,
            report.defender,
            ctx.active.reservation.world,
        ))?;
    let armor = match victim {
        Victim::Human(peer) if matches!(peer.life, CharacterLife::Incapacitated { .. }) => {
            armor.for_incapacitated(catalog)?
        }
        _ => armor,
    };
    let hit = armor::RawCharacterDamage {
        amount_hp: raw,
        armor_rating_redux: 0.0,
        armor_prevention_redux: 0.0,
        hit_location: 0,
    };
    let kind = damage::DamageKind::ExternalKinetic;
    let calculation = armor.calculation(hit, kind)?;
    let amount = armor.adjust_for_kind(hit, kind)?;
    if amount == 0.0 {
        return Ok(None);
    }
    let damage = damage::ResolvedDamage {
        amount_hp: amount,
        kind,
        // Retail sends type 5 alone: a lethal impact can down, like a bullet.
        kinetic_response: false,
        location: 0,
        attacker_id: driver,
        source_object_id: report.attacker,
        user_data: 0,
    }
    .apply(health)?;
    Ok(Some(Impact {
        amount,
        damage,
        calculation,
    }))
}
