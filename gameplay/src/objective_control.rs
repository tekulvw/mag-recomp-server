//! Shared authoritative capture and demolition for reviewed retail objective bindings.
use crate::{
    objective_catalog::{
        ObjectiveCatalog, ObjectiveDemolitionRule, ObjectiveInteractionRule,
        ObjectiveProgressionRule, native_state,
    },
    progression::{RewardKind, RewardRecipient},
    protocol::{ObjectiveInteraction, ObjectiveInteractionKind},
    replication::{
        CharacterPosition, ObjectiveControlDelta, ObjectiveControlSnapshot, ObjectiveState,
        ObjectiveStateActor,
    },
};
use std::{collections::BTreeMap, time::Duration};

#[derive(Clone, Copy, Debug)]
pub struct ObjectiveActor {
    pub player: u16,
    pub team: u8,
    pub generation: u32,
    pub position: CharacterPosition,
}

pub struct InteractionOutcome {
    pub accepted: bool,
    pub reason: &'static str,
    pub changed: Option<ObjectiveControlDelta>,
    pub reply_trigger_index: u8,
}
impl InteractionOutcome {
    fn denied(reason: &'static str) -> Self {
        Self {
            accepted: false,
            reason,
            changed: None,
            reply_trigger_index: u8::MAX,
        }
    }
    fn accepted(
        reason: &'static str,
        changed: Option<ObjectiveControlDelta>,
        reply_trigger_index: u8,
    ) -> Self {
        Self {
            accepted: true,
            reason,
            changed,
            reply_trigger_index,
        }
    }
}

#[derive(Clone)]
struct Operation {
    started_at: Duration,
    recipient: Option<RewardRecipient>,
    player: u16,
    generation: u32,
    trigger_index: u8,
    deadline: Duration,
}
#[derive(Clone)]
struct Objective {
    reward: Option<crate::objective_catalog::ObjectiveRewardRule>,
    rule: Option<ObjectiveInteractionRule>,
    demolition: Option<ObjectiveDemolitionRule>,
    fuse: Option<Operation>,
    state: ObjectiveControlDelta,
    operation: Option<Operation>,
}
#[derive(Clone)]
pub struct ObjectiveWorld {
    reward_actions: Vec<ObjectiveRewardAction>,
    objects: BTreeMap<u16, Objective>,
    pristine: bool,
    progression: Option<ObjectiveProgressionRule>,
    progression_started: bool,
    delayed_deadline: Option<Duration>,
}
impl ObjectiveWorld {
    pub fn new(catalog: &ObjectiveCatalog) -> Self {
        Self {
            reward_actions: Vec::new(),
            objects: catalog
                .objects
                .iter()
                .map(|object| {
                    (
                        object.object_index,
                        Objective {
                            reward: object.reward.clone(),
                            rule: object.interaction.clone(),
                            demolition: object.demolition.clone(),
                            fuse: None,
                            state: ObjectiveControlDelta {
                                object_index: object.object_index,
                                state_actor: ObjectiveStateActor {
                                    state: native_state(object.initial_state)
                                        .expect("validated objective state"),
                                    change_time_ms: 0,
                                    actor_index: u16::MAX,
                                },
                                expiration_time_ms: 0,
                                deactivation_time_ms: 0,
                                health_percent: 100,
                            },
                            operation: None,
                        },
                    )
                })
                .collect(),
            pristine: true,
            progression: catalog.progression.clone(),
            progression_started: false,
            delayed_deadline: None,
        }
    }

    /// Snapshot the authenticated admission at action start, not at delayed completion.
    pub(crate) fn attribute_operation(&mut self, index: u16, recipient: Option<RewardRecipient>) {
        if let Some(operation) = self
            .objects
            .get_mut(&index)
            .and_then(|object| object.operation.as_mut())
            && operation.recipient.is_none()
        {
            operation.recipient = recipient;
        }
    }

    pub(crate) fn take_reward_actions(&mut self) -> Vec<ObjectiveRewardAction> {
        std::mem::take(&mut self.reward_actions)
    }

    pub fn is_pristine(&self) -> bool {
        self.pristine
    }

    pub fn state(&self, object_index: u16) -> Option<ObjectiveState> {
        self.objects
            .get(&object_index)
            .map(|object| object.state.state_actor.state)
    }

    /// Explicit development fixture: plant without impersonating a human actor.
    /// Normal defuse and fuse ticking own all subsequent transitions.
    pub fn development_arm(
        &mut self,
        object_index: u16,
        now: Duration,
    ) -> Result<ObjectiveControlDelta, &'static str> {
        let object = self
            .objects
            .get_mut(&object_index)
            .ok_or("development_demolition_not_configured")?;
        let rule = object
            .demolition
            .as_ref()
            .ok_or("development_demolition_not_configured")?;
        if object.state.state_actor.state != ObjectiveState::Deactivated {
            return Err("development_demolition_requires_deactivated");
        }
        object.fuse = Some(Operation {
            started_at: now,
            recipient: None,
            player: u16::MAX,
            generation: 0,
            trigger_index: 0,
            deadline: now.saturating_add(Duration::from_millis(rule.detonate_ms.into())),
        });
        armed_state(object, now);
        self.pristine = false;
        Ok(object.state)
    }

    pub fn interaction_owner(&self, object_index: u16) -> Option<(u16, u32, u8)> {
        let object = self.objects.get(&object_index)?;
        if !matches!(
            object.state.state_actor.state,
            ObjectiveState::Activating | ObjectiveState::Deactivating
        ) {
            return None;
        }
        let operation = object.operation.as_ref()?;
        Some((
            operation.player,
            operation.generation,
            operation.trigger_index,
        ))
    }

    pub fn snapshot(&self, sequence: u32, timestamp_ms: u32) -> ObjectiveControlSnapshot {
        ObjectiveControlSnapshot {
            sequence,
            timestamp_ms,
            objectives: self.objects.values().map(|object| object.state).collect(),
        }
    }

    pub fn interact(
        &mut self,
        request: ObjectiveInteraction,
        actor: ObjectiveActor,
        now: Duration,
    ) -> InteractionOutcome {
        self.interact_with_demolition_modifier(request, actor, now, 0.0)
    }

    /// Native GetModifiedTime uses single-precision fused base*modifier+base
    /// followed by truncation. Snapshot the duration when the action starts;
    /// later range/leader changes must not restart its accepted countdown.
    pub fn interact_with_demolition_modifier(
        &mut self,
        request: ObjectiveInteraction,
        actor: ObjectiveActor,
        now: Duration,
        demolition_modifier: f32,
    ) -> InteractionOutcome {
        let index = request.target_object_id as u16;
        if request.target_object_id != 0x0c80_0000 | u32::from(index) {
            return InteractionOutcome::denied("objective_invalid_target");
        }
        if request.actor_index != actor.player {
            return InteractionOutcome::denied("objective_actor_mismatch");
        }
        let Some(object) = self.objects.get_mut(&index) else {
            return InteractionOutcome::denied("objective_unknown_target");
        };
        if let Some(mut rule) = object.demolition.clone() {
            // Release remains possible independently of modifier validity.
            if request.kind != ObjectiveInteractionKind::Release {
                let Some(arm) = modified_demolition_duration(rule.arm_ms, demolition_modifier)
                else {
                    return InteractionOutcome::denied("invalid_demolition_modifier");
                };
                let Some(defuse) =
                    modified_demolition_duration(rule.defuse_ms, demolition_modifier)
                else {
                    return InteractionOutcome::denied("invalid_demolition_modifier");
                };
                rule.arm_ms = arm;
                rule.defuse_ms = defuse;
            }
            let outcome = interact_demolition(object, &rule, request, actor, now);
            self.pristine &= outcome.changed.is_none();
            return outcome;
        }
        let Some(rule) = object.rule.as_ref() else {
            return InteractionOutcome::denied("objective_interaction_not_configured");
        };
        // A release must remain possible after stepping out of the trigger volume.
        // FF is also accepted for release, matching the no-trigger ACK sentinel.
        if request.kind != ObjectiveInteractionKind::Release
            && request.trigger_index >= rule.trigger_count
        {
            return InteractionOutcome::denied("objective_invalid_trigger");
        }
        if actor.team == rule.owner_team {
            return InteractionOutcome::denied("objective_defender_interaction_unsupported");
        }
        if request.kind != ObjectiveInteractionKind::Release && !in_range(actor, rule) {
            return InteractionOutcome::denied("objective_out_of_range");
        }
        match request.kind {
            ObjectiveInteractionKind::Activate => {
                if object.state.state_actor.state == ObjectiveState::Activating {
                    return if object.operation.as_ref().is_some_and(|operation| {
                        operation.player == actor.player && operation.generation == actor.generation
                    }) {
                        InteractionOutcome::accepted(
                            "objective_activation_duplicate",
                            None,
                            request.trigger_index,
                        )
                    } else {
                        InteractionOutcome::denied("objective_busy")
                    };
                }
                if object.state.state_actor.state != ObjectiveState::Deactivated {
                    return InteractionOutcome::denied("objective_not_deactivated");
                }
                let deadline = now.saturating_add(Duration::from_millis(rule.activate_ms.into()));
                object.operation = Some(Operation {
                    started_at: now,
                    recipient: None,
                    player: actor.player,
                    generation: actor.generation,
                    trigger_index: request.trigger_index,
                    deadline,
                });
                object.state.state_actor = ObjectiveStateActor {
                    state: ObjectiveState::Activating,
                    change_time_ms: wire_time(now),
                    actor_index: actor.player,
                };
                object.state.expiration_time_ms = wire_time(deadline);
                object.state.deactivation_time_ms = 0;
                object.state.health_percent = 100;
                self.pristine = false;
                InteractionOutcome::accepted(
                    "objective_activation_started",
                    Some(object.state),
                    request.trigger_index,
                )
            }
            ObjectiveInteractionKind::Release => {
                if object.state.state_actor.state != ObjectiveState::Activating {
                    return InteractionOutcome::accepted("objective_release_idle", None, u8::MAX);
                }
                if !object.operation.as_ref().is_some_and(|operation| {
                    operation.player == actor.player && operation.generation == actor.generation
                }) {
                    return InteractionOutcome::denied("objective_release_not_owner");
                }
                cancel(object, now);
                self.pristine = false;
                InteractionOutcome::accepted("objective_released", Some(object.state), u8::MAX)
            }
            _ => InteractionOutcome::denied("objective_interaction_unsupported"),
        }
    }

    /// The caller supplies only living, entered actors. A new life invalidates a hold.
    pub fn tick(
        &mut self,
        now: Duration,
        actors: &BTreeMap<u16, ObjectiveActor>,
    ) -> Vec<ObjectiveControlDelta> {
        let mut changed = BTreeMap::new();
        for object in self.objects.values_mut() {
            if let Some(rule) = object.demolition.clone() {
                if tick_demolition(object, &rule, now, actors, &mut self.reward_actions) {
                    changed.insert(object.state.object_index, object.state);
                }
                continue;
            }
            let (Some(rule), Some(operation)) = (object.rule.as_ref(), object.operation.clone())
            else {
                continue;
            };
            if object.state.state_actor.state == ObjectiveState::Activating {
                let actor_valid = actors.get(&operation.player).is_some_and(|actor| {
                    actor.player == operation.player
                        && actor.generation == operation.generation
                        && actor.team != rule.owner_team
                        && in_range(*actor, rule)
                });
                if !actor_valid {
                    cancel(object, now);
                    changed.insert(object.state.object_index, object.state);
                    continue;
                }
                if now < operation.deadline {
                    continue;
                }
                emit_objective_reward(
                    object,
                    &operation,
                    RewardKind::PrimaryObjectiveActivated,
                    &mut self.reward_actions,
                );
                let deadline = operation
                    .deadline
                    .saturating_add(Duration::from_millis(rule.complete_ms.into()));
                object.state.state_actor.state = ObjectiveState::Activated;
                object.state.state_actor.change_time_ms = wire_time(operation.deadline);
                object.state.expiration_time_ms = wire_time(deadline);
                object.operation = Some(Operation {
                    deadline,
                    ..operation
                });
            } else if object.state.state_actor.state != ObjectiveState::Activated
                || now < operation.deadline
            {
                continue;
            }
            // A late tick may cross both deadlines; emit the final state once.
            if let Some(operation) = object.operation.clone()
                && now >= operation.deadline
            {
                emit_objective_reward(
                    object,
                    &operation,
                    RewardKind::PrimaryObjectiveSecured,
                    &mut self.reward_actions,
                );
                object.state.state_actor.state = ObjectiveState::Completed;
                object.state.state_actor.change_time_ms = wire_time(operation.deadline);
                object.state.expiration_time_ms = 0;
                object.operation = None;
            }
            changed.insert(object.state.object_index, object.state);
        }
        self.tick_progression(now, &mut changed);
        if !changed.is_empty() {
            self.pristine = false;
        }
        changed.into_values().collect()
    }

    fn tick_progression(
        &mut self,
        now: Duration,
        changed: &mut BTreeMap<u16, ObjectiveControlDelta>,
    ) {
        let Some(rule) = self.progression.as_ref() else {
            return;
        };
        if self.progression_started {
            if let Some(deadline) = self.delayed_deadline
                && now >= deadline
            {
                let index = rule.delayed_objective;
                self.set_progression_state(index, ObjectiveState::Deactivated, deadline, changed);
                self.delayed_deadline = None;
            }
            return;
        }
        if !rule
            .prerequisites
            .iter()
            .all(|index| self.state(*index) == Some(ObjectiveState::Completed))
        {
            return;
        }
        let rule = rule.clone();
        self.progression_started = true;
        self.delayed_deadline =
            Some(now.saturating_add(Duration::from_millis(rule.delay_ms.into())));
        for index in rule.lock {
            self.set_progression_state(index, ObjectiveState::Locked, now, changed);
        }
        for index in rule.complete {
            self.set_progression_state(index, ObjectiveState::Completed, now, changed);
        }
    }

    fn set_progression_state(
        &mut self,
        index: u16,
        state: ObjectiveState,
        now: Duration,
        changed: &mut BTreeMap<u16, ObjectiveControlDelta>,
    ) {
        let Some(object) = self.objects.get_mut(&index) else {
            return;
        };
        object.state.state_actor = ObjectiveStateActor {
            state,
            change_time_ms: wire_time(now),
            actor_index: u16::MAX,
        };
        object.state.expiration_time_ms = 0;
        object.state.deactivation_time_ms = 0;
        object.state.health_percent = 100;
        object.operation = None;
        changed.insert(index, object.state);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ObjectiveRewardAction {
    pub object_index: u16,
    pub started_at: Duration,
    pub recipient: RewardRecipient,
    pub kind: RewardKind,
}
fn emit_objective_reward(
    object: &Objective,
    operation: &Operation,
    kind: RewardKind,
    actions: &mut Vec<ObjectiveRewardAction>,
) {
    let Some(rule) = &object.reward else {
        return;
    };
    if matches!(kind, RewardKind::ChargePlanted | RewardKind::ChargeDefused) {
        if !rule.demolition {
            return;
        }
    } else if rule.category > 1 {
        return;
    }
    if let Some(recipient) = &operation.recipient {
        actions.push(ObjectiveRewardAction {
            object_index: object.state.object_index,
            started_at: operation.started_at,
            recipient: recipient.clone(),
            kind,
        });
    }
}

fn demolition_in_range(actor: ObjectiveActor, rule: &ObjectiveDemolitionRule) -> bool {
    let dx = actor.position.x - rule.position[0];
    let dy = actor.position.y - rule.position[1];
    let dz = actor.position.z - rule.position[2];
    dx * dx + dy * dy + dz * dz <= rule.maximum_distance_m * rule.maximum_distance_m
}

fn armed_state(object: &mut Objective, now: Duration) {
    let fuse = object.fuse.as_ref().expect("armed demolition fuse");
    object.state.state_actor = ObjectiveStateActor {
        state: ObjectiveState::Activated,
        change_time_ms: wire_time(now),
        actor_index: fuse.player,
    };
    object.state.expiration_time_ms = wire_time(fuse.deadline);
    object.state.deactivation_time_ms = 0;
    object.state.health_percent = 100;
    object.operation = None;
}

fn modified_demolition_duration(base_ms: u32, modifier: f32) -> Option<u32> {
    let base = base_ms as f32;
    let duration = modifier.mul_add(base, base);
    // Native takes low32 after signed64 truncation. Only ordinary finite,
    // nonnegative, representable durations are supported by this adapter.
    (modifier.is_finite() && duration.is_finite() && (0.0..4294967296.0).contains(&duration))
        .then_some(duration as u32)
}

fn interact_demolition(
    object: &mut Objective,
    rule: &ObjectiveDemolitionRule,
    request: ObjectiveInteraction,
    actor: ObjectiveActor,
    now: Duration,
) -> InteractionOutcome {
    use ObjectiveInteractionKind::{Activate, Deactivate, Release};
    let state = object.state.state_actor.state;
    if request.kind == Release {
        if !matches!(
            state,
            ObjectiveState::Activating | ObjectiveState::Deactivating
        ) {
            return InteractionOutcome::accepted("objective_release_idle", None, u8::MAX);
        }
        if !object
            .operation
            .as_ref()
            .is_some_and(|op| op.player == actor.player && op.generation == actor.generation)
        {
            return InteractionOutcome::denied("objective_release_not_owner");
        }
        if state == ObjectiveState::Deactivating {
            armed_state(object, now);
        } else {
            cancel(object, now);
        }
        return InteractionOutcome::accepted("objective_released", Some(object.state), u8::MAX);
    }
    if request.trigger_index >= rule.trigger_count {
        return InteractionOutcome::denied("objective_invalid_trigger");
    }
    if !demolition_in_range(actor, rule) {
        return InteractionOutcome::denied("objective_out_of_range");
    }
    let (expected, working, duration, reason) = match request.kind {
        Activate if actor.team != rule.owner_team => (
            ObjectiveState::Deactivated,
            ObjectiveState::Activating,
            rule.arm_ms,
            "demolition_arming_started",
        ),
        Deactivate if actor.team == rule.owner_team && rule.can_defuse => (
            ObjectiveState::Activated,
            ObjectiveState::Deactivating,
            rule.defuse_ms,
            "demolition_defusing_started",
        ),
        Activate | Deactivate => {
            return InteractionOutcome::denied("objective_wrong_team_or_capability");
        }
        _ => return InteractionOutcome::denied("objective_interaction_unsupported"),
    };
    if state == working {
        return if object
            .operation
            .as_ref()
            .is_some_and(|op| op.player == actor.player && op.generation == actor.generation)
        {
            InteractionOutcome::accepted(
                "objective_interaction_duplicate",
                None,
                request.trigger_index,
            )
        } else {
            InteractionOutcome::denied("objective_busy")
        };
    }
    if state != expected {
        return InteractionOutcome::denied("objective_invalid_demolition_state");
    }
    let deadline = now.saturating_add(Duration::from_millis(duration.into()));
    object.operation = Some(Operation {
        started_at: now,
        recipient: None,
        player: actor.player,
        generation: actor.generation,
        trigger_index: request.trigger_index,
        deadline,
    });
    object.state.state_actor = ObjectiveStateActor {
        state: working,
        change_time_ms: wire_time(now),
        actor_index: actor.player,
    };
    if working == ObjectiveState::Activating {
        object.state.expiration_time_ms = wire_time(deadline);
        object.state.deactivation_time_ms = 0;
    } else {
        object.state.deactivation_time_ms = wire_time(deadline);
    }
    object.state.health_percent = 100;
    InteractionOutcome::accepted(reason, Some(object.state), request.trigger_index)
}

fn tick_demolition(
    object: &mut Objective,
    rule: &ObjectiveDemolitionRule,
    now: Duration,
    actors: &BTreeMap<u16, ObjectiveActor>,
    rewards: &mut Vec<ObjectiveRewardAction>,
) -> bool {
    let previous = object.state;
    let state = object.state.state_actor.state;
    // Native Deactivating checks the planted fuse before the defuse deadline.
    if matches!(
        state,
        ObjectiveState::Activated | ObjectiveState::Deactivating
    ) && object
        .fuse
        .as_ref()
        .is_some_and(|fuse| now >= fuse.deadline)
    {
        let fuse = object.fuse.take().unwrap();
        if state == ObjectiveState::Activated {
            emit_objective_reward(object, &fuse, RewardKind::PrimaryObjectiveSecured, rewards);
        }
        object.state.state_actor = ObjectiveStateActor {
            state: ObjectiveState::Completed,
            change_time_ms: wire_time(now),
            actor_index: fuse.player,
        };
        // SetStateCompleted preserves the bomb expiration, clears defuse time,
        // and destroys the objective. It is not the A/B health100 completion.
        object.state.expiration_time_ms = wire_time(fuse.deadline);
        object.state.deactivation_time_ms = 0;
        object.state.health_percent = 0;
        object.operation = None;
        return true;
    }
    if matches!(
        state,
        ObjectiveState::Activating | ObjectiveState::Deactivating
    ) && let Some(operation) = object.operation.clone()
    {
        let valid = actors.get(&operation.player).is_some_and(|actor| {
            actor.generation == operation.generation
                && (actor.team == rule.owner_team) == (state == ObjectiveState::Deactivating)
                && demolition_in_range(*actor, rule)
        });
        if !valid {
            if state == ObjectiveState::Deactivating {
                armed_state(object, now);
            } else {
                cancel(object, now);
            }
        } else if now >= operation.deadline {
            if state == ObjectiveState::Activating {
                emit_objective_reward(object, &operation, RewardKind::ChargePlanted, rewards);
                emit_objective_reward(
                    object,
                    &operation,
                    RewardKind::PrimaryObjectiveActivated,
                    rewards,
                );
                object.fuse = Some(Operation {
                    deadline: now.saturating_add(Duration::from_millis(rule.detonate_ms.into())),
                    ..operation
                });
                // Native fresh Activated entry starts its fuse at actual entry time.
                armed_state(object, now);
            } else {
                emit_objective_reward(object, &operation, RewardKind::ChargeDefused, rewards);
                emit_objective_reward(
                    object,
                    &operation,
                    RewardKind::PrimaryObjectiveActivated,
                    rewards,
                );
                object.state.state_actor = ObjectiveStateActor {
                    state: ObjectiveState::Deactivated,
                    change_time_ms: wire_time(now),
                    actor_index: operation.player,
                };
                object.state.expiration_time_ms = 0;
                // Native defuse retains the elapsed field4 deadline.
                object.state.deactivation_time_ms = wire_time(operation.deadline);
                object.state.health_percent = 100;
                object.operation = None;
                object.fuse = None;
            }
        }
    }
    object.state != previous
}

fn wire_time(time: Duration) -> u32 {
    time.as_millis() as u32
}
fn in_range(actor: ObjectiveActor, rule: &ObjectiveInteractionRule) -> bool {
    let dx = actor.position.x - rule.position[0];
    let dy = actor.position.y - rule.position[1];
    let dz = actor.position.z - rule.position[2];
    dx * dx + dy * dy + dz * dz <= rule.maximum_distance_m * rule.maximum_distance_m
}
fn cancel(object: &mut Objective, now: Duration) {
    object.state.state_actor = ObjectiveStateActor {
        state: ObjectiveState::Deactivated,
        change_time_ms: wire_time(now),
        actor_index: u16::MAX,
    };
    object.state.expiration_time_ms = 0;
    object.state.deactivation_time_ms = 0;
    object.operation = None;
}
