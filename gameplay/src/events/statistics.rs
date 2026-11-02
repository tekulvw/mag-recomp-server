//! Match combat counters are gameplay state, separate from account XP receipts.
//! Native0204 uses absolute values and the low16 character index.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

// Original retail CharacterStatType table / scoreboard receiver. These are wire
// identities, not configurable balance values. Assist awards require their own
// contribution resolver; a zero here does not manufacture an assist.
const KILLS: u8 = 10;
const ASSISTS: u8 = 11;
const DEATHS: u8 = 12;
const SUICIDES: u8 = 13;
const FRIENDLY_KILLS: u8 = 14;

#[derive(Default)]
pub(super) struct StatisticsPlan {
    increments: BTreeMap<(String, u8), i32>,
    updates: BTreeMap<String, BTreeMap<u8, i32>>,
}
fn increment(plan: &mut EventPlan, actor: &Active, stat: u8) {
    let value = plan
        .statistics
        .increments
        .entry((actor.reservation.id.clone(), stat))
        .or_default();
    *value = value.saturating_add(1);
}
/// The represented opposing target has no authenticated victim account. Its
/// accepted lethal hit still counts for the attacking character, as native does.
pub(super) fn target_killed(plan: &mut EventPlan, killer: &Active, victim_team: u8) {
    increment(
        plan,
        killer,
        if killer.team() == victim_team {
            FRIENDLY_KILLS
        } else {
            KILLS
        },
    );
}
pub(super) fn resuscitated(plan: &mut EventPlan, healer: &Active) {
    increment(plan, healer, 34);
}
pub(super) fn finished(plan: &mut EventPlan, killer: &Active, victim_team: u8) {
    if killer.team() != victim_team {
        increment(plan, killer, 24);
    }
}
fn died<W: Write>(
    engine: &Engine<W>,
    plan: &mut EventPlan,
    victim: &Active,
    damage: replication::CharacterDamage,
) {
    increment(plan, victim, DEATHS);
    let victim_id = 0x03800000 | u32::from(victim.reservation.player);
    if damage.last_attacker == victim_id {
        increment(plan, victim, SUICIDES);
    } else if let Some(killer) = engine.active.iter().chain(engine.peers.values()).find(|a| {
        a.reservation.world == victim.reservation.world
            && (0x03800000 | u32::from(a.reservation.player)) == damage.last_attacker
    }) {
        target_killed(plan, killer, victim.team());
    }
}
/// World bot damage has no authenticated killer account. Reuse the native
/// death statistic and normal scoreboard row, without granting human kill/XP.
pub(crate) fn bot_killed_human<W: Write>(
    engine: &mut Engine<W>,
    reservation: &str,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let victim = engine
        .active
        .as_mut()
        .filter(|a| a.reservation.id == reservation)
        .or_else(|| engine.peers.get_mut(reservation))
        .ok_or("missing_bot_victim")?;
    let deaths = victim.reward_statistics.entry(DEATHS).or_default();
    *deaths = deaths.saturating_add(1);
    let world = victim.reservation.world;
    let wire = protocol::StatUpdate {
        character_index: victim.reservation.player,
        stats: [KILLS, ASSISTS, DEATHS]
            .into_iter()
            .map(|stat| protocol::StatValue {
                stat,
                value: victim.reward_statistics.get(&stat).copied().unwrap_or(0),
            })
            .collect(),
    }
    .encode_with_profile(victim.wire_profile)?;
    Ok(engine
        .active
        .iter()
        .chain(engine.peers.values())
        .filter(|a| {
            a.reservation.world == world && a.ready_for_updates && a.initial_objects.is_some()
        })
        .map(|a| OutboundMessage {
            reservation_id: a.reservation.id.clone(),
            world_id: world,
            player_id: a.reservation.player,
            plugin_id: 1,
            payload_hex: protocol::encode_hex(&wire),
        })
        .collect())
}
pub(super) fn stage<W: Write>(
    engine: &Engine<W>,
    plan: &mut EventPlan,
) -> Result<(), &'static str> {
    if plan.rejection.is_some() {
        return Ok(());
    }
    let active = engine.active.as_ref().ok_or("no_reservation")?;
    // Current scoreboard repair targets retail. Legacy prototype fixtures retain
    // their earlier single-reply behavior; they are not live retail acceptance.
    if active.wire_profile.application_id() != 20231 {
        return Ok(());
    }
    if active.life.is_alive()
        && plan.combat.new_life.is_some_and(|life| !life.is_alive())
        && let Some(damage) = plan.combat.new_damage_model
    {
        died(engine, plan, active, damage);
    }
    let mut seen = BTreeSet::new();
    for (id, generation, damage) in plan.combat.peer_damage.clone() {
        if damage.health_quarter_points != 0 || !seen.insert((id.clone(), generation)) {
            continue;
        }
        if let Some(victim) = engine.peers.get(&id)
            && matches!(victim.life, CharacterLife::Alive { generation:g,.. } if g == generation)
        {
            died(engine, plan, victim, damage);
        }
    }
    let actors: Vec<_> = engine.active.iter().chain(engine.peers.values()).collect();
    for ((id, stat), delta) in &plan.statistics.increments {
        let Some(actor) = actors.iter().find(|a| &a.reservation.id == id) else {
            continue;
        };
        let value = actor
            .reward_statistics
            .get(stat)
            .copied()
            .unwrap_or(0)
            .saturating_add(*delta);
        plan.statistics
            .updates
            .entry(id.clone())
            .or_default()
            .insert(*stat, value);
    }
    let initial = plan.replication.initial_objects.is_some();
    for actor in &actors {
        let changed = plan.statistics.updates.get(&actor.reservation.id);
        if changed.is_none() && !initial {
            continue;
        }
        if actor.initial_objects.is_none()
            && !(initial && actor.reservation.id == active.reservation.id)
        {
            continue;
        }
        let mut stat_ids: BTreeSet<_> = [KILLS, ASSISTS, DEATHS]
            .into_iter()
            .chain(changed.into_iter().flat_map(|m| m.keys().copied()))
            .collect();
        if initial {
            stat_ids.extend(actor.reward_statistics.keys().copied());
            if actor.match_experience > 0 {
                stat_ids.insert(2);
            }
        }
        let stats = stat_ids
            .into_iter()
            .map(|stat| protocol::StatValue {
                stat,
                value: if stat == 2 {
                    actor.match_experience.min(i32::MAX as u32) as i32
                } else {
                    changed
                        .and_then(|m| m.get(&stat))
                        .or_else(|| actor.reward_statistics.get(&stat))
                        .copied()
                        .unwrap_or(0)
                },
            })
            .collect();
        let wire = protocol::StatUpdate {
            character_index: actor.reservation.player,
            stats,
        }
        .encode_with_profile(actor.wire_profile)?;
        for recipient in &actors {
            let newly_ready = initial && recipient.reservation.id == active.reservation.id;
            if recipient.reservation.world != actor.reservation.world
                || (!newly_ready
                    && (!recipient.ready_for_updates || recipient.initial_objects.is_none()))
            {
                continue;
            }
            // On join, existing recipients need only the newcomer's current row;
            // the newcomer receives every existing row's current totals.
            if changed.is_none()
                && initial
                && !newly_ready
                && actor.reservation.id != active.reservation.id
            {
                continue;
            }
            plan.messages.push(OutboundMessage {
                reservation_id: recipient.reservation.id.clone(),
                world_id: recipient.reservation.world,
                player_id: recipient.reservation.player,
                plugin_id: 1,
                payload_hex: protocol::encode_hex(&wire),
            });
        }
    }
    Ok(())
}
pub(super) fn commit<W: Write>(engine: &mut Engine<W>, plan: StatisticsPlan) {
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        if let Some(stats) = plan.updates.get(&actor.reservation.id) {
            actor.reward_statistics.extend(stats);
        }
    }
}
