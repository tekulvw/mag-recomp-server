//! Sabotage adapter for the shared retail SecureHold FSM. The outer world owns
//! recording, legacy capture attribution and the existing A/B script progression.
use super::*;
use ObjectiveState::*;

#[derive(Clone)]
pub(super) struct Capture {
    hold: secure::SecureHold,
    // Deactivating/CancelRetaken temporarily replace the operation. Preserve the
    // accepted passive-window actor/attribution and deadline for exact release.
    resting: Option<Operation>,
}
impl Capture {
    pub(super) fn new(initial: ObjectiveState, rule: &ObjectiveInteractionRule) -> Self {
        Self {
            hold: secure_hold(initial, rule),
            resting: None,
        }
    }

    pub(super) fn interact(
        &mut self,
        object: &mut Objective,
        request: ObjectiveInteraction,
        actor: ObjectiveActor,
        now: Duration,
        timings: ActionTimingModifiers,
    ) -> InteractionOutcome {
        use ObjectiveInteractionKind::{Activate, Deactivate, Release};
        let action = match request.kind {
            Activate => secure::Action::Activate,
            Deactivate => secure::Action::Deactivate,
            Release => secure::Action::Release,
            _ => return InteractionOutcome::denied("objective_interaction_unsupported"),
        };
        let rule = object.rule.as_ref().expect("Sabotage capture rule");
        let state = object.state.state_actor.state;
        if request.kind != Release {
            if request.trigger_index >= rule.trigger_count {
                return InteractionOutcome::denied("objective_invalid_trigger");
            }
            if !contact(actor, rule, request.trigger_index) {
                return InteractionOutcome::denied("objective_out_of_range");
            }
        }
        // A retransmission acknowledges the same hold, never resets its timer.
        if matches!(
            (request.kind, state),
            (Activate, Activating | CancelRetaken) | (Deactivate, Deactivating | Retaking)
        ) {
            return if object.operation.as_ref().is_some_and(|op| {
                op.player == actor.player
                    && op.generation == actor.generation
                    && op.trigger_index == request.trigger_index
            }) && actor.team <= 1
                && (actor.team == rule.owner_team) == (request.kind == Deactivate)
                && contact(actor, rule, request.trigger_index)
            {
                InteractionOutcome::accepted(
                    "objective_hold_duplicate",
                    None,
                    request.trigger_index,
                )
            } else {
                InteractionOutcome::denied("objective_busy")
            };
        }
        if request.kind == Release
            && !matches!(state, Activating | Deactivating | Retaking | CancelRetaken)
        {
            return InteractionOutcome::accepted("objective_release_idle", None, u8::MAX);
        }
        let mut candidate = self.hold.clone();
        let transition = match candidate.request(
            action,
            secure::Actor {
                player: actor.player,
                team: actor.team,
                generation: actor.generation.into(),
            },
            request.trigger_index,
            contact(actor, rule, request.trigger_index),
        ) {
            Ok(t) => t,
            Err(reason) => return InteractionOutcome::denied(reason),
        };
        let base = match transition.target() {
            Activating | Retaking => Some(rule.activate_ms),
            Deactivating | CancelRetaken => Some(rule.deactivate_ms),
            _ => None,
        };
        let duration = match base {
            Some(base) if timings.secure_and_hold == 0.0 => Some(base),
            Some(base) => match modified_action_duration(base, timings.secure_and_hold) {
                Some(d) => Some(d),
                None => return InteractionOutcome::denied("invalid_objective_modifier"),
            },
            None => None,
        };
        self.hold = candidate;
        if matches!(
            transition.target(),
            Activating | Deactivating | Retaking | CancelRetaken
        ) {
            self.resting = object.operation.clone();
            object.operation = Some(Operation {
                started_at: now,
                contact_arming: false,
                recipient: None,
                capture_actor: None,
                bot_plant: None,
                player: actor.player,
                generation: actor.generation,
                trigger_index: request.trigger_index,
                deadline: now,
            });
        }
        self.enter(object, transition, now, duration);
        InteractionOutcome::accepted(
            "sabotage_secure_hold_interaction",
            Some(object.state),
            if request.kind == Release {
                u8::MAX
            } else {
                request.trigger_index
            },
        )
    }

    fn enter(
        &mut self,
        object: &mut Objective,
        transition: secure::Request,
        now: Duration,
        duration: Option<u32>,
    ) {
        let entry = self
            .hold
            .commit(transition, wire_time(now), duration)
            .expect("staged Sabotage transition");
        object.state.state_actor = ObjectiveStateActor {
            state: entry.current,
            change_time_ms: wire_time(now),
            actor_index: if entry.current == Deactivated && entry.previous == Activating {
                u16::MAX
            } else {
                entry.actor.player
            },
        };
        object.state.expiration_time_ms = entry.expiration_ms.unwrap_or_default();
        object.state.deactivation_time_ms = entry.deactivation_ms.unwrap_or_default();
        match (entry.previous, entry.current) {
            (Deactivating, Activated) | (CancelRetaken, Retaken) => {
                object.operation = self.resting.take();
            }
            (_, Activated | Retaken) => {
                self.resting = None;
            }
            (_, Completed | Deactivated) => {
                object.operation = None;
                self.resting = None;
            }
            _ => {}
        }
        if let Some(op) = &mut object.operation {
            let rule = object.rule.as_ref().expect("Sabotage capture rule");
            let base = match entry.current {
                Activating | Retaking => Some(duration.unwrap_or(rule.activate_ms)),
                Deactivating | CancelRetaken => Some(duration.unwrap_or(rule.deactivate_ms)),
                Activated if entry.previous == Activating => Some(rule.complete_ms),
                Retaken if entry.previous == Retaking => Some(rule.complete_ms),
                _ => None, // Restored passive windows retain their full deadline.
            };
            if let Some(ms) = base {
                op.deadline = now.saturating_add(Duration::from_millis(u64::from(ms)));
            }
        }
    }

    pub(super) fn tick(
        &mut self,
        object: &mut Objective,
        now: Duration,
        actors: &BTreeMap<u16, ObjectiveActor>,
        rewards: &mut Vec<ObjectiveRewardAction>,
        capture_completed: &mut impl FnMut(u16, &Operation),
    ) -> bool {
        let mut changed = false;
        // Preserve legacy catch-up from the accepted deadline, including a late
        // activation tick crossing the completion window. Releases stop here so
        // an already-due restored passive state runs at its next update.
        for _ in 0..2 {
            let previous = object.state.state_actor.state;
            let rule = object.rule.as_ref().expect("Sabotage capture rule");
            let operation = object.operation.clone();
            let holding = matches!(
                previous,
                Activating | Deactivating | Retaking | CancelRetaken
            );
            let current = !holding
                || operation.as_ref().is_some_and(|op| {
                    actors.get(&op.player).is_some_and(|a| {
                        a.player == op.player
                            && a.generation == op.generation
                            && a.team <= 1
                            && (a.team == rule.owner_team)
                                == matches!(previous, Deactivating | Retaking)
                            && contact(*a, rule, op.trigger_index)
                    })
                });
            // Keep legacy Duration deadlines even when their wire u32 wraps.
            // Only ask the common FSM to evaluate the exact accepted due time.
            if current && operation.as_ref().is_some_and(|op| now < op.deadline) {
                break;
            }
            let due_time = if current {
                operation.as_ref().map_or(now, |op| op.deadline)
            } else {
                now
            };
            let request = self.hold.due(wire_time(due_time), |_| current);
            let Some(request) = request else {
                break;
            };
            let when = if current {
                operation.as_ref().map_or(now, |op| op.deadline)
            } else {
                now
            };
            // Keep existing capture rewards/counters on their original forward
            // activation/completion edges. This does not introduce new XP kinds.
            if current && let Some(op) = &operation {
                match (previous, request.target()) {
                    (Activating, Activated) => emit_objective_reward(
                        object,
                        op,
                        RewardKind::PrimaryObjectiveActivated,
                        rewards,
                    ),
                    (Activated, Completed) => {
                        emit_objective_reward(
                            object,
                            op,
                            RewardKind::PrimaryObjectiveSecured,
                            rewards,
                        );
                        capture_completed(object.state.object_index, op);
                    }
                    _ => {}
                }
            }
            self.enter(object, request, when, None);
            changed = true;
            if !current {
                break;
            }
        }
        changed
    }
}

// Retain the catalog's explicit legacy point-distance admission when no authored
// per-trigger volumes exist; do not silently require Domination trigger geometry.
fn contact(actor: ObjectiveActor, rule: &ObjectiveInteractionRule, trigger: u8) -> bool {
    trigger < rule.trigger_count
        && if rule.range().volumes.is_empty() {
            in_range(actor, rule)
        } else {
            at_native_trigger(actor.position, rule.range(), trigger)
        }
}
