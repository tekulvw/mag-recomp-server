//! Structural objective repair. The engine supplies server-owned actor and
//! inventory authority, the linked damage model, and the replicated health field.

use crate::{domination_objective_runtime::Actor, replication::ObjectiveState};

const BATCH_MS: u32 = 1_000;
const MAX_HEALTH: f32 = 100.0;
const MIN_DURATION_SECONDS: f32 = 0.1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    pub owner_team: u8,
    pub can_repair: bool,
    pub repair_ms: u32,
    pub minimum_repair_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Authority {
    pub actor: Actor,
    pub live: bool,
    pub tool_active: bool,
    /// The authoritative tool rate after applying this actor's repair modifier.
    pub repair_rate: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contribution {
    pub actor: Actor,
    pub amount: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Submission {
    Queued(Contribution),
    LatchOnly,
}

pub(crate) const HEALTH_ALREADY_COVERED: &str = "structural_repair_health_already_covered";

#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub contributions: Vec<Contribution>,
    pub health_before: f32,
    /// The requested value for the structural objective's damage model.
    pub health_after: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    revision: u64,
    pub source: ObjectiveState,
    pub target: ObjectiveState,
    pub actor: Actor,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    None,
    Transition(Request),
    Repair(Batch),
}

#[derive(Clone, Debug)]
pub struct StructuralRepair {
    rules: Rules,
    current: ObjectiveState,
    pending: Option<Request>,
    revision: u64,
    repairs: Vec<Contribution>,
    repair_total: f32,
    last_actor: Option<Actor>,
    remaining_ms: u32,
}

fn duration_seconds(ms: u32) -> f32 {
    (ms as f32 * 0.001_f32).max(MIN_DURATION_SECONDS)
}

fn validate_health(health: f32) -> Result<(), &'static str> {
    if (0.0..=MAX_HEALTH).contains(&health) {
        Ok(())
    } else {
        Err("invalid_structural_damage_model_health")
    }
}

impl StructuralRepair {
    pub fn new(current: ObjectiveState, rules: Rules) -> Result<Self, &'static str> {
        if rules.owner_team > 1
            || !matches!(
                current,
                ObjectiveState::Completed | ObjectiveState::Repairing | ObjectiveState::Repaired
            )
        {
            return Err("invalid_structural_repair_state_or_owner");
        }
        Ok(Self {
            rules,
            current,
            pending: None,
            revision: 0,
            repairs: Vec::new(),
            repair_total: 0.0,
            last_actor: None,
            remaining_ms: BATCH_MS,
        })
    }

    pub fn current(&self) -> ObjectiveState {
        self.current
    }

    pub fn desired(&self) -> ObjectiveState {
        self.pending.map_or(self.current, |request| request.target)
    }

    /// Add one accepted packet. Damage-model health and replicated health are
    /// separate observations. The receive gate uses damage-model health plus
    /// contributions already queued at the time of this packet.
    pub fn submit(
        &mut self,
        authority: Authority,
        damage_model_health: f32,
    ) -> Result<Contribution, &'static str> {
        match self.submit_packet(authority, damage_model_health)? {
            Submission::Queued(contribution) => Ok(contribution),
            Submission::LatchOnly => Err(HEALTH_ALREADY_COVERED),
        }
    }

    pub(crate) fn submit_packet(
        &mut self,
        authority: Authority,
        damage_model_health: f32,
    ) -> Result<Submission, &'static str> {
        validate_health(damage_model_health)?;
        if !self.rules.can_repair
            || !matches!(
                self.current,
                ObjectiveState::Completed | ObjectiveState::Repairing
            )
        {
            return Err("structural_objective_not_repairable");
        }
        if !authority.live || authority.actor.team != self.rules.owner_team {
            return Err("structural_repair_requires_live_owner");
        }
        if !authority.tool_active {
            return Err("structural_repair_tool_not_active");
        }
        let amount = (authority.repair_rate * MAX_HEALTH) / duration_seconds(self.rules.repair_ms);
        if !amount.is_finite() || amount <= 0.0 {
            return Err("invalid_structural_repair_contribution");
        }
        // Retail latches the validated owner character before evaluating the
        // queued-health ceiling. Keep that credit candidate independently.
        self.last_actor = Some(authority.actor);
        if damage_model_health + self.repair_total >= MAX_HEALTH {
            return Ok(Submission::LatchOnly);
        }
        let contribution = Contribution {
            actor: authority.actor,
            amount,
        };
        self.repair_total += amount;
        self.repairs.push(contribution);
        Ok(Submission::Queued(contribution))
    }

    fn request(&mut self, target: ObjectiveState) -> Option<Request> {
        let actor = self.last_actor?;
        self.revision = self.revision.checked_add(1)?;
        let request = Request {
            revision: self.revision,
            source: self.current,
            target,
            actor,
        };
        self.pending = Some(request);
        Some(request)
    }

    /// Process one native update opportunity. An empty queue freezes the
    /// remaining batch time. An elapsed batch drains once and resets to a
    /// full interval. Already accepted packets survive the actor going away.
    pub fn update(
        &mut self,
        elapsed_ms: u32,
        damage_model_health: f32,
        target_idle: bool,
    ) -> Result<Update, &'static str> {
        validate_health(damage_model_health)?;
        if self.repairs.is_empty() {
            return Ok(Update::None);
        }
        if self.current == ObjectiveState::Completed && target_idle && self.pending.is_none() {
            self.remaining_ms = BATCH_MS;
            return Ok(self
                .request(ObjectiveState::Repairing)
                .map_or(Update::None, Update::Transition));
        }
        if self.current != ObjectiveState::Repairing {
            return Ok(Update::None);
        }
        self.remaining_ms = self.remaining_ms.saturating_sub(elapsed_ms);
        if self.remaining_ms > 0 {
            return Ok(Update::None);
        }
        self.remaining_ms = BATCH_MS;
        let maximum = MAX_HEALTH / duration_seconds(self.rules.minimum_repair_ms);
        let amount = std::mem::take(&mut self.repair_total).min(maximum);
        let contributions = std::mem::take(&mut self.repairs);
        Ok(Update::Repair(Batch {
            contributions,
            health_before: damage_model_health,
            health_after: (damage_model_health + amount).min(MAX_HEALTH),
        }))
    }

    /// Evaluate after applying the batch and refreshing the net objective.
    /// The native state tests the replicated byte independently of the current
    /// damage-model value and disables completion while a transition is active.
    pub fn observe_completion(
        &mut self,
        replicated_health_percent: u8,
        target_idle: bool,
    ) -> Option<Request> {
        if self.current != ObjectiveState::Repairing
            || self.pending.is_some()
            || !target_idle
            || f32::from(replicated_health_percent) != MAX_HEALTH
        {
            return None;
        }
        self.request(ObjectiveState::Repaired)
    }

    /// Commit only the staged transition. The Repairing entry discards the
    /// packets that initiated it. Repaired entry also clears pending repairs.
    pub fn commit(&mut self, request: Request) -> Result<(), &'static str> {
        if self.pending != Some(request) || request.source != self.current {
            return Err("stale_structural_repair_transition");
        }
        self.repairs.clear();
        self.repair_total = 0.0;
        self.current = request.target;
        self.pending = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ObjectiveState::{Completed, Repaired, Repairing};

    fn rules() -> Rules {
        Rules {
            owner_team: 1,
            can_repair: true,
            repair_ms: 8_000,
            minimum_repair_ms: 4_000,
        }
    }

    fn authority(player: u16, generation: u64, repair_rate: f32) -> Authority {
        Authority {
            actor: Actor {
                player,
                generation,
                team: 1,
            },
            live: true,
            tool_active: true,
            repair_rate,
        }
    }

    #[test]
    fn takes_one_start_packet_then_batches_to_the_full_second() {
        let mut repair = StructuralRepair::new(Completed, rules()).unwrap();
        let initial_actor = authority(4, 3, 1.0);
        assert_eq!(repair.submit(initial_actor, 10.0).unwrap().amount, 12.5);
        assert_eq!(repair.update(17, 10.0, false), Ok(Update::None));
        let Update::Transition(request) = repair.update(17, 10.0, true).unwrap() else {
            panic!("expected Repairing target request");
        };
        assert_eq!((repair.current(), repair.desired()), (Completed, Repairing));
        assert_eq!(request.actor, initial_actor.actor);
        assert_eq!(repair.update(2_000, 10.0, true), Ok(Update::None));
        repair.commit(request).unwrap();
        assert_eq!(repair.update(9_000, 10.0, true), Ok(Update::None));

        let first = repair.submit(authority(4, 3, 1.0), 10.0).unwrap();
        let next = repair.submit(authority(4, 4, 1.0), 10.0).unwrap();
        let other = repair.submit(authority(9, 1, 1.0), 10.0).unwrap();
        assert_eq!(repair.update(999, 10.0, true), Ok(Update::None));
        assert_eq!(
            repair.update(1, 10.0, true),
            Ok(Update::Repair(Batch {
                contributions: vec![first, next, other],
                health_before: 10.0,
                health_after: 35.0,
            }))
        );
        assert_eq!(repair.current(), Repairing);
        assert_eq!(repair.observe_completion(35, true), None);
    }

    #[test]
    fn gates_and_normalizes_each_packet_before_batching() {
        let mut repair = StructuralRepair::new(Repairing, rules()).unwrap();
        let valid = authority(4, 3, 1.0);
        let enemy = Authority {
            actor: Actor {
                team: 0,
                ..valid.actor
            },
            ..valid
        };
        assert_eq!(
            repair.submit(enemy, 0.0),
            Err("structural_repair_requires_live_owner")
        );
        assert_eq!(
            repair.submit(
                Authority {
                    live: false,
                    ..valid
                },
                0.0
            ),
            Err("structural_repair_requires_live_owner")
        );
        assert_eq!(
            repair.submit(
                Authority {
                    tool_active: false,
                    ..valid
                },
                0.0
            ),
            Err("structural_repair_tool_not_active")
        );
        for rate in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::MAX] {
            assert_eq!(
                repair.submit(
                    Authority {
                        repair_rate: rate,
                        ..valid
                    },
                    0.0
                ),
                Err("invalid_structural_repair_contribution")
            );
        }
        assert_eq!(
            repair.submit(valid, f32::NAN),
            Err("invalid_structural_damage_model_health")
        );

        let mut repair = StructuralRepair::new(
            Repairing,
            Rules {
                repair_ms: 5,
                ..rules()
            },
        )
        .unwrap();
        let contribution = repair.submit(authority(4, 3, 0.01), 95.0).unwrap();
        assert_eq!(contribution.amount, 10.0);
        assert_eq!(
            repair.submit(valid, 95.0),
            Err("structural_repair_health_already_covered")
        );
        assert_eq!(
            repair.update(1_000, 95.0, true),
            Ok(Update::Repair(Batch {
                contributions: vec![contribution],
                health_before: 95.0,
                health_after: 100.0,
            }))
        );
    }

    #[test]
    fn finish_requires_replicated_full_health_and_explicit_current_commit() {
        let mut repair = StructuralRepair::new(Repairing, rules()).unwrap();
        let healer = authority(6, 1, 1.0);
        repair.submit(healer, 95.0).unwrap();
        let Update::Repair(batch) = repair.update(5_000, 95.0, true).unwrap() else {
            panic!("expected one applied batch");
        };
        assert_eq!(batch.health_after, 100.0);
        assert_eq!(repair.observe_completion(99, true), None);
        assert_eq!(repair.observe_completion(100, false), None);
        let request = repair.observe_completion(100, true).unwrap();
        assert_eq!((repair.current(), repair.desired()), (Repairing, Repaired));
        assert_eq!(request.actor, healer.actor);
        assert_eq!(repair.observe_completion(100, true), None);
        repair.commit(request).unwrap();
        assert_eq!(repair.current(), Repaired);
        assert_eq!(
            repair.commit(request),
            Err("stale_structural_repair_transition")
        );
    }
}
