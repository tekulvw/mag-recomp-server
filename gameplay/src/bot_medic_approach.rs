//! Bot medics walk to downed teammates (docs/bot-healing.md, "Reaching downed
//! teammates").
//!
//! Each tick, every downed teammate (human or bot) that a medic can still
//! revive gets the nearest free bot medic of its side within the downed-bot
//! medic radius (`incapacitation.medic_radius_m`, the same 30 m that keeps a
//! downed bot waiting). The medic leaves its ordinary objective movement and
//! walks to the patient; once the patient is inside its medkit's reach it
//! stops, and the ordinary medkit use revives the patient. One medic serves
//! one patient. The approach ends when the patient is revived, bleeds out or
//! leaves, when the medic dies, goes down or takes a higher duty (an
//! armed-objective, blast, defuse, repair, demolition or interaction duty,
//! a supply action or a passage), or when it makes no progress.
use super::*;
use crate::development_target::BotPhase;
use std::collections::{BTreeMap, BTreeSet};

/// A downed patient a medic is sent to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Patient {
    Bot(u16),
    /// A human, by player index.
    Human(u16),
}
impl Patient {
    fn audit(self) -> Value {
        match self {
            Self::Bot(id) => json!({"kind":"bot","object_index":id}),
            Self::Human(player) => json!({"kind":"human","player":player}),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Approach {
    pub(crate) patient: Patient,
    /// The patient's life, so a later life is never mistaken for this one.
    pub(crate) generation: u32,
    pub(crate) since: Duration,
    /// Closest the medic has come to the patient, and when.
    pub(crate) best_distance_m: f64,
    pub(crate) progress_at: Duration,
    pub(crate) arrived: bool,
}

/// No progress for this long ends an approach (a route that cannot be walked,
/// a passage the medic may not take, a body in the way).
const STALL: Duration = Duration::from_secs(6);
/// At most this many new routes per tick.
const NEW_APPROACHES_PER_TICK: usize = 4;

struct DownedPatient {
    patient: Patient,
    generation: u32,
    team: u8,
    feet: [f64; 3],
    position: replication::CharacterPosition,
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
fn horizontal(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}

impl<W: Write> Engine<W> {
    fn downed_patients(&self, now: Duration) -> Vec<DownedPatient> {
        let mut patients = Vec::new();
        for a in self.active.iter().chain(self.peers.values()) {
            let CharacterLife::Incapacitated { generation, downed } = a.life else {
                continue;
            };
            if downed.is_expired(now)
                || !a.entered_game
                || !a.ready_for_updates
                || a.is_mounted()
                || !self.accepts_subscription_world(a.reservation.world)
            {
                continue;
            }
            let Some(sample) = a.tracking.sample else {
                continue;
            };
            let p = sample.position;
            patients.push(DownedPatient {
                patient: Patient::Human(a.reservation.player),
                generation,
                team: a.team(),
                feet: [p.x.into(), p.y.into(), p.z.into()],
                position: p,
            });
        }
        for (&id, bot) in &self.bot_registry.bots {
            let (Some(generation), Some(r)) = (self.bot_revivable(id, now), bot.target.runtime)
            else {
                continue;
            };
            patients.push(DownedPatient {
                patient: Patient::Bot(id),
                generation,
                team: bot.target.squad.team(),
                feet: r.physical_position(),
                position: r.position,
            });
        }
        patients
    }

    fn approach_patient_current(&self, approach: Approach, now: Duration) -> bool {
        match approach.patient {
            Patient::Bot(id) => self.bot_revivable(id, now) == Some(approach.generation),
            Patient::Human(player) => self.active.iter().chain(self.peers.values()).any(|a| {
                a.reservation.player == player
                    && matches!(a.life, CharacterLife::Incapacitated { generation, downed }
                        if generation == approach.generation && !downed.is_expired(now))
            }),
        }
    }

    fn approach_patient_alive(&self, approach: Approach) -> bool {
        match approach.patient {
            Patient::Bot(id) => self.bot_registry.bots.get(&id).is_some_and(|b| {
                b.health > 0
                    && b.target
                        .runtime
                        .is_some_and(|r| r.generation == approach.generation)
            }),
            Patient::Human(player) => self.active.iter().chain(self.peers.values()).any(|a| {
                a.reservation.player == player
                    && matches!(a.life, CharacterLife::Alive { generation, .. }
                        if generation == approach.generation)
            }),
        }
    }

    /// Stop a medic's approach walk and hand it back to its objective task.
    fn end_medic_approach(&mut self, id: u16, reason: &'static str, now: Duration) -> Value {
        let bot = self.bot_registry.bots.get_mut(&id).unwrap();
        let approach = bot.medic_approach.take();
        if bot.health > 0 {
            bot.target.pending_move = None;
            if let Some(r) = bot.target.runtime.as_mut()
                && matches!(r.phase, BotPhase::Relocating)
            {
                r.phase = BotPhase::Complete;
            }
            // The objective task plans again from where the medic stands.
            bot.task = Default::default();
        }
        json!({"op":"bot_medic_approach_ended","timestamp_ms":now.as_millis() as u64,
            "object_index":id,"reason":reason,
            "patient":approach.map(|a| a.patient.audit()),
            "approach_ms":approach.map(|a| now.saturating_sub(a.since).as_millis() as u64)})
    }

    /// Release, assign and advance medic approaches. `busy` holds the bots
    /// with a higher duty this tick. Returns the medics now approaching, which
    /// ordinary objective movement leaves alone.
    pub(crate) fn tick_bot_medic_approaches(
        &mut self,
        now: Duration,
        busy: &BTreeSet<u16>,
        receipts: &mut Vec<Value>,
    ) -> Result<BTreeSet<u16>, &'static str> {
        let policy = self.bot_registry.config.as_ref().map(|c| c.healing.clone());
        let enabled = policy
            .as_ref()
            .is_some_and(|p| p.enabled && p.teammates && p.revive && p.approach_downed)
            && self.incapacitation_rules.is_some()
            && self.wire_profile.application_id() == 20231
            && self.match_started
            && self.match_terminal.is_none();
        let team_item = crate::bot_healing::team_item(&self.item_catalog);
        let radius = self
            .bot_incapacitation_policy()
            .map_or(30., |p| p.medic_radius_m);
        self.bot_registry
            .director
            .medic_failures
            .retain(|_, until| *until > now);
        // Release approaches that no longer hold.
        let current: Vec<(u16, Approach)> = self
            .bot_registry
            .bots
            .iter()
            .filter_map(|(&id, b)| b.medic_approach.map(|a| (id, a)))
            .collect();
        for (id, approach) in current {
            let bot = &self.bot_registry.bots[&id];
            let r = bot.target.runtime.ok_or("bot_runtime_missing")?;
            let reason = if !enabled {
                Some("disabled")
            } else if bot.health == 0 || matches!(r.phase, BotPhase::Dead) {
                Some("medic_down")
            } else if !self.approach_patient_current(approach, now) {
                Some(if self.approach_patient_alive(approach) {
                    "patient_revived"
                } else {
                    "patient_lost"
                })
            } else if busy.contains(&id) {
                Some("higher_duty")
            } else if bot.has_supply_action() {
                Some("supply_action")
            } else if bot.drop_committed || bot.drop_permit.is_some() || bot.drop_waiting.is_some()
            {
                Some("passage")
            } else if !approach.arrived && now.saturating_sub(approach.progress_at) >= STALL {
                Some("no_progress")
            } else {
                None
            };
            if let Some(reason) = reason {
                if reason == "no_progress" {
                    // Let another medic, or this one later, try instead.
                    self.bot_registry
                        .director
                        .medic_failures
                        .insert((id, approach.patient), now + STALL);
                }
                let receipt = self.end_medic_approach(id, reason, now);
                receipts.push(receipt);
            }
        }
        let (true, Some(team_item)) = (enabled, team_item) else {
            return Ok(BTreeSet::new());
        };
        let reach = team_item.forward_offset_m + team_item.radius_m;
        // Stop where the patient falls well inside the medkit's sphere.
        let arrive_m = (team_item.forward_offset_m + team_item.radius_m * 0.5).max(0.75);
        let patients = self.downed_patients(now);
        let served: BTreeSet<Patient> = self
            .bot_registry
            .bots
            .values()
            .filter_map(|b| b.medic_approach.map(|a| a.patient))
            .collect();
        // Nearest free medic per unserved patient, nearest pairs first.
        let mut pairs = Vec::new();
        for patient in patients.iter().filter(|p| !served.contains(&p.patient)) {
            for (&id, bot) in &self.bot_registry.bots {
                let Some(r) = bot.target.runtime else {
                    continue;
                };
                if patient.patient == Patient::Bot(id)
                    || bot.medic_approach.is_some()
                    || busy.contains(&id)
                    || bot.health == 0
                    || bot.downed.is_some()
                    || matches!(r.phase, BotPhase::Dead)
                    || bot.target.squad.team() != patient.team
                    || !r.physics_motion.is_some_and(|m| m.grounded)
                    || r.navmesh_state.on_player
                    || bot.has_supply_action()
                    || bot.drop_committed
                    || bot.drop_permit.is_some()
                    || bot.drop_waiting.is_some()
                    || self
                        .bot_registry
                        .director
                        .medic_failures
                        .get(&(id, patient.patient))
                        .is_some_and(|until| *until > now)
                    || crate::bot_healing::medkit_slot(
                        &self.item_catalog,
                        team_item,
                        &bot.target.loadout,
                    )
                    .is_none()
                {
                    continue;
                }
                let d = distance(r.physical_position(), patient.feet);
                if d <= radius {
                    pairs.push((d, id, patient));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut taken_medics = BTreeSet::new();
        let mut taken_patients = BTreeSet::new();
        let mut started = 0;
        let map = self.map_name().to_owned();
        for (d, id, patient) in pairs {
            if started >= NEW_APPROACHES_PER_TICK
                || taken_medics.contains(&id)
                || taken_patients.contains(&patient.patient)
            {
                continue;
            }
            taken_medics.insert(id);
            taken_patients.insert(patient.patient);
            let sprint = self
                .bot_registry
                .config
                .as_ref()
                .and_then(|c| c.slots.get(self.bot_registry.bots[&id].slot))
                .is_some_and(|slot| slot.sprint);
            let mesh = self.navmesh_world.clone();
            let bot = self.bot_registry.bots.get_mut(&id).unwrap();
            let me = bot.target.runtime.unwrap().physical_position();
            let walk = horizontal(me, patient.feet) > arrive_m;
            if walk {
                let Some(mesh) = mesh else {
                    break;
                };
                let old_pending = bot.target.pending_move.take();
                let old_runtime = bot.target.runtime;
                let result = bot.target.begin_registry_move(
                    mesh,
                    &map,
                    BotMove {
                        mode: BotMoveMode::Navmesh,
                        destination: patient.position,
                        duration_ms: 60_000,
                        update_ms: 250,
                        yaw: None,
                        sprint,
                    },
                    now,
                    bot.health,
                    self.fall_damage_rules.as_ref(),
                );
                started += 1;
                if let Err(reason) = result {
                    bot.target.pending_move = old_pending;
                    bot.target.runtime = old_runtime;
                    self.bot_registry
                        .director
                        .medic_failures
                        .insert((id, patient.patient), now + STALL);
                    receipts.push(json!({"op":"bot_medic_approach_unroutable","timestamp_ms":now.as_millis() as u64,
                        "object_index":id,"patient":patient.patient.audit(),"reason":reason}));
                    continue;
                }
            } else {
                bot.target.pending_move = None;
                bot.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
            }
            bot.task = Default::default();
            bot.medic_approach = Some(Approach {
                patient: patient.patient,
                generation: patient.generation,
                since: now,
                best_distance_m: horizontal(me, patient.feet),
                progress_at: now,
                arrived: !walk,
            });
            receipts.push(
                json!({"op":"bot_medic_approach_started","timestamp_ms":now.as_millis() as u64,
                "object_index":id,"patient":patient.patient.audit(),"distance_m":d,"walk":walk}),
            );
        }
        // Advance: note progress and stop on arrival.
        let feet: BTreeMap<Patient, [f64; 3]> =
            patients.iter().map(|p| (p.patient, p.feet)).collect();
        let mut approaching = BTreeSet::new();
        for (&id, bot) in self.bot_registry.bots.iter_mut() {
            let Some(approach) = bot.medic_approach.as_mut() else {
                continue;
            };
            approaching.insert(id);
            let (Some(target), Some(r)) = (feet.get(&approach.patient), bot.target.runtime) else {
                continue;
            };
            let me = r.physical_position();
            let d = horizontal(me, *target);
            if d + 0.25 < approach.best_distance_m {
                approach.best_distance_m = d;
                approach.progress_at = now;
            }
            if !approach.arrived && d <= arrive_m && (me[1] - target[1]).abs() <= 2. {
                approach.arrived = true;
                approach.progress_at = now;
                bot.target.pending_move = None;
                bot.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
                receipts.push(json!({"op":"bot_medic_approach_arrived","timestamp_ms":now.as_millis() as u64,
                    "object_index":id,"patient":approach.patient.audit(),"distance_m":d,"reach_m":reach,
                    "approach_ms":now.saturating_sub(approach.since).as_millis() as u64}));
            }
        }
        Ok(approaching)
    }
}

#[cfg(test)]
#[path = "bot_medic_approach_tests.rs"]
mod tests;
