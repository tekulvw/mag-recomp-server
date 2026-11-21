//! Shared authoritative capture and demolition for reviewed retail objective bindings.
use crate::{
    domination_objective_runtime as secure, domination_repair_runtime as repair,
    objective_catalog::{
        ObjectiveCatalog, ObjectiveDemolitionRule, ObjectiveInteractionRule,
        ObjectiveProgressionRule, ObjectiveRepairRule, native_state,
    },
    progression::{RewardKind, RewardRecipient},
    protocol::{ObjectiveInteraction, ObjectiveInteractionKind},
    replication::{
        CharacterPosition, ObjectiveControlDelta, ObjectiveControlSnapshot, ObjectiveState,
        ObjectiveStateActor,
    },
};
use std::{collections::BTreeMap, time::Duration};

pub use crate::objective_catalog::NativeClass;

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
    native: Option<NativeObjective>,
}

#[derive(Clone)]
struct NativeObjective {
    class: NativeClass,
    secure: Option<secure::SecureHold>,
    repair_rule: Option<ObjectiveRepairRule>,
    repair: Option<repair::StructuralRepair>,
    damage_health: f32,
    was_repaired: bool,
    autonomous_arming: bool,
    reward_attacker: Option<u16>,
    reward_defender: Option<u16>,
    pending: Option<NativePending>,
}

#[derive(Clone)]
enum NativePending {
    Secure(secure::Request, Option<u32>),
    Demolition {
        kind: ObjectiveInteractionKind,
        actor: ObjectiveActor,
        trigger: u8,
        modifier: f32,
        recipient: Option<RewardRecipient>,
    },
    Script(ObjectiveState, Option<secure::Actor>),
    Precision(Option<crate::structure_damage::Source>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeEntry {
    pub class: NativeClass,
    pub previous: ObjectiveState,
    pub delta: ObjectiveControlDelta,
    pub actor_generation: Option<u32>,
    /// Qualified ordinary stat-callback actor, independent of the wire actor.
    /// Precision, scripts and damage-trigger latch attribution remain unqualified.
    pub reward_actor: Option<u16>,
    pub active_trigger_index: Option<u8>,
}

impl NativeEntry {
    /// Armed completion also dispatches the authored server explosion.
    pub fn requires_authored_explosion(&self) -> bool {
        self.class == NativeClass::Structural
            && self.delta.state_actor.state == ObjectiveState::Completed
            && matches!(
                self.previous,
                ObjectiveState::Activated | ObjectiveState::Deactivating
            )
    }
}

#[derive(Clone)]
pub struct ObjectiveWorld {
    reward_actions: Vec<ObjectiveRewardAction>,
    objects: BTreeMap<u16, Objective>,
    pristine: bool,
    progression: Option<ObjectiveProgressionRule>,
    progression_started: bool,
    delayed_deadline: Option<Duration>,
    native_entries: Vec<NativeEntry>,
    acquisition_precision: BTreeMap<u16, Option<crate::structure_damage::Source>>,
    native_tick_time: Option<Duration>,
    native_elapsed_ms: u32,
}
impl ObjectiveWorld {
    pub fn new(catalog: &ObjectiveCatalog) -> Self {
        Self {
            reward_actions: Vec::new(),
            objects: catalog
                .objects
                .iter()
                .map(|object| {
                    let initial =
                        native_state(object.initial_state).expect("validated objective state");
                    let native = object.native_class.map(|class| {
                        let damage_health = if class == NativeClass::Structural
                            && initial == ObjectiveState::Completed
                        {
                            0.0
                        } else {
                            100.0
                        };
                        NativeObjective {
                            class,
                            secure: object
                                .interaction
                                .as_ref()
                                .map(|rule| secure_hold(initial, rule)),
                            repair_rule: object.repair,
                            repair: new_structural_repair(initial, object.repair),
                            damage_health,
                            was_repaired: initial == ObjectiveState::Repaired,
                            autonomous_arming: false,
                            reward_attacker: None,
                            reward_defender: None,
                            pending: None,
                        }
                    });
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
                                    state: initial,
                                    change_time_ms: 0,
                                    actor_index: u16::MAX,
                                },
                                expiration_time_ms: 0,
                                deactivation_time_ms: 0,
                                health_percent: native
                                    .as_ref()
                                    .map_or(100, |state| state.damage_health as u8),
                            },
                            operation: None,
                            native,
                        },
                    )
                })
                .collect(),
            pristine: true,
            progression: catalog.progression.clone(),
            progression_started: false,
            delayed_deadline: None,
            native_entries: Vec::new(),
            acquisition_precision: BTreeMap::new(),
            native_tick_time: None,
            native_elapsed_ms: 0,
        }
    }

    pub(crate) fn has_native_entries(&self) -> bool {
        !self.native_entries.is_empty()
    }

    pub fn take_native_entries(&mut self) -> Vec<NativeEntry> {
        std::mem::take(&mut self.native_entries)
    }

    pub fn native_class(&self, index: u16) -> Option<NativeClass> {
        Some(self.objects.get(&index)?.native.as_ref()?.class)
    }

    pub fn native_object_indices(&self) -> impl Iterator<Item = u16> + '_ {
        self.objects
            .iter()
            .filter_map(|(&index, object)| object.native.as_ref().map(|_| index))
    }

    /// Start an objective update frame. The backend updates each qualified
    /// object once and dispatches its entry before advancing to the next.
    pub fn begin_native_tick(&mut self, now: Duration) {
        let elapsed = self
            .native_tick_time
            .map_or(Duration::ZERO, |last| now.saturating_sub(last));
        self.native_tick_time = Some(now);
        self.native_elapsed_ms = elapsed.as_millis().min(u128::from(u32::MAX)) as u32;
    }

    pub fn tick_native_object(
        &mut self,
        index: u16,
        now: Duration,
        actors: &BTreeMap<u16, ObjectiveActor>,
    ) -> Option<ObjectiveControlDelta> {
        self.tick_native_object_with_elapsed(index, now, self.native_elapsed_ms, actors)
    }

    /// Retail player START/RELEASE advances only its admitted objective with
    /// dt=0 inside the request stack. Script desired-state changes wait for
    /// the original Sim manager pass.
    pub fn commit_pending_native_interaction(
        &mut self,
        index: u16,
        now: Duration,
        actor: ObjectiveActor,
    ) -> Option<ObjectiveControlDelta> {
        let pending = self.objects.get(&index)?.native.as_ref()?.pending.as_ref();
        if !matches!(
            pending,
            Some(NativePending::Secure(..) | NativePending::Demolition { .. })
        ) {
            return None;
        }
        self.tick_native_object_with_elapsed(
            index,
            now,
            0,
            &BTreeMap::from([(actor.player, actor)]),
        )
    }

    fn tick_native_object_with_elapsed(
        &mut self,
        index: u16,
        now: Duration,
        elapsed_ms: u32,
        actors: &BTreeMap<u16, ObjectiveActor>,
    ) -> Option<ObjectiveControlDelta> {
        let object = self.objects.get_mut(&index)?;
        let mut mode = object.native.take()?;
        let previous = object.state;
        let scripted = matches!(mode.pending, Some(NativePending::Script(..)));
        let active_trigger_index = if matches!(mode.pending, Some(NativePending::Precision(_))) {
            None
        } else {
            object.fuse.as_ref().map(|fuse| fuse.trigger_index)
        };
        let generation = tick_native(
            object,
            &mut mode,
            now,
            elapsed_ms,
            actors,
            &mut self.reward_actions,
        );
        let class = mode.class;
        let reward_actor = match class {
            NativeClass::SecureHold if !scripted => Some(object.state.state_actor.actor_index),
            NativeClass::Structural => match object.state.state_actor.state {
                ObjectiveState::Activated | ObjectiveState::Completed => mode.reward_attacker,
                ObjectiveState::Deactivated | ObjectiveState::Repaired => mode.reward_defender,
                _ => None,
            },
            _ => None,
        };
        object.native = Some(mode);
        if previous == object.state {
            return None;
        }
        self.pristine = false;
        if previous.state_actor.state != object.state.state_actor.state {
            self.native_entries.push(NativeEntry {
                class,
                previous: previous.state_actor.state,
                delta: object.state,
                actor_generation: generation,
                reward_actor,
                active_trigger_index,
            });
        }
        Some(object.state)
    }

    pub fn stage_native_state(
        &mut self,
        index: u16,
        target: ObjectiveState,
        actor: Option<secure::Actor>,
    ) -> Result<(), &'static str> {
        let object = self
            .objects
            .get_mut(&index)
            .ok_or("unknown_native_objective")?;
        let mode = object
            .native
            .as_mut()
            .ok_or("native_objective_not_configured")?;
        if !matches!(
            target,
            ObjectiveState::Inactive
                | ObjectiveState::Deactivated
                | ObjectiveState::Completed
                | ObjectiveState::Locked
        ) {
            return Err("unsupported_native_script_state");
        }
        if matches!(
            mode.pending,
            Some(NativePending::Secure(..) | NativePending::Demolition { .. })
        ) {
            return Err("native_objective_transition_pending");
        }
        if object.state.state_actor.state != target {
            mode.pending = Some(NativePending::Script(target, actor));
            self.pristine = false;
        } else {
            mode.pending = None;
        }
        Ok(())
    }

    /// Precision executes the Structural desired-state setter. Acquisition
    /// consumes the same request through its separate objective controller.
    pub(crate) fn stage_precision_strike(
        &mut self,
        index: u16,
        source: Option<crate::structure_damage::Source>,
    ) -> Result<bool, &'static str> {
        let Some(object) = self.objects.get_mut(&index) else {
            return Ok(false);
        };
        if object.state.state_actor.state == ObjectiveState::Completed {
            return Ok(false);
        }
        match object.native.as_mut() {
            Some(mode) if mode.class == NativeClass::Structural => {
                mode.pending = Some(NativePending::Precision(source));
            }
            Some(_) => return Ok(false),
            None => {
                self.acquisition_precision.insert(index, source);
            }
        }
        self.pristine = false;
        Ok(true)
    }

    pub(crate) fn has_precision_transition(&self, index: u16) -> bool {
        self.objects.get(&index).is_some_and(|object| {
            matches!(
                object
                    .native
                    .as_ref()
                    .and_then(|mode| mode.pending.as_ref()),
                Some(NativePending::Precision(_))
            )
        })
    }

    /// These source-qualified Acquisition structures have ordinary family-12
    /// entries and are outside the Secure and Arrival controller's hook list.
    pub(crate) fn advance_acquisition_precision_entries(
        &mut self,
        now: Duration,
        mut prepare: impl FnMut(NativeEntry, &mut Self) -> Result<(), &'static str>,
    ) -> Result<Vec<ObjectiveControlDelta>, &'static str> {
        let mut updates = Vec::new();
        for index in self
            .acquisition_precision
            .keys()
            .copied()
            .collect::<Vec<_>>()
        {
            updates.extend(self.advance_acquisition_precision_entry(index, now, &mut prepare)?);
        }
        Ok(updates)
    }

    pub(crate) fn advance_acquisition_precision_entry(
        &mut self,
        index: u16,
        now: Duration,
        mut prepare: impl FnMut(NativeEntry, &mut Self) -> Result<(), &'static str>,
    ) -> Result<Option<ObjectiveControlDelta>, &'static str> {
        let Some(source) = self.acquisition_precision.remove(&index) else {
            return Ok(None);
        };
        let mut published = None;
        while let Some(object) = self.objects.get_mut(&index) {
            let previous = object.state.state_actor.state;
            if advance_precision(object, source, now).is_none() {
                break;
            }
            let entry = NativeEntry {
                class: NativeClass::Structural,
                previous,
                delta: object.state,
                actor_generation: source.map(|actor| actor.generation),
                reward_actor: None,
                active_trigger_index: None,
            };
            published = Some(entry.delta);
            prepare(entry, self)?;
        }
        Ok(published)
    }

    /// Source-projected passive spawn objectives have only the script-requested
    /// stable states. Their class entry effects are staged by the caller.
    pub(crate) fn enter_acquisition_spawn_state(
        &mut self,
        index: u16,
        class: NativeClass,
        target: ObjectiveState,
        actor: u16,
        now: Duration,
    ) -> Result<Option<NativeEntry>, &'static str> {
        let object = self
            .objects
            .get_mut(&index)
            .ok_or("acquisition_spawn_objective_missing")?;
        let previous = object.state.state_actor.state;
        if target == previous {
            return Ok(None);
        }
        if !matches!(
            previous,
            ObjectiveState::Inactive | ObjectiveState::Deactivated | ObjectiveState::Completed
        ) || !matches!(
            target,
            ObjectiveState::Deactivated | ObjectiveState::Completed
        ) {
            return Err("unqualified_acquisition_spawn_state");
        }
        set_native_state(object, target, actor, now);
        object.state.expiration_time_ms = 0;
        object.state.deactivation_time_ms = 0;
        if class == NativeClass::Structural {
            object.state.health_percent = if target == ObjectiveState::Completed {
                0
            } else {
                100
            };
        }

        self.pristine = false;
        Ok(Some(NativeEntry {
            class,
            previous,
            delta: object.state,
            actor_generation: None,
            reward_actor: None,
            active_trigger_index: None,
        }))
    }

    /// A destroyed type4/5 trigger invokes the owner's ordinary Activating
    /// entry synchronously. Its source need not stand in an interaction volume.
    pub(crate) fn activate_destroyed_trigger(
        &mut self,
        index: u16,
        trigger: u8,
        actor: crate::structure_damage::Source,
        now: Duration,
        modifier: f32,
    ) -> Result<Option<ObjectiveControlDelta>, &'static str> {
        let object = self
            .objects
            .get_mut(&index)
            .ok_or("unknown_structure_damage_owner")?;
        let mode = object
            .native
            .as_mut()
            .filter(|m| m.class == NativeClass::Structural)
            .ok_or("unsupported_structure_damage_owner")?;
        let previous = object.state.state_actor.state;
        if !matches!(
            previous,
            ObjectiveState::Deactivated | ObjectiveState::Repaired
        ) {
            return Ok(None);
        }
        let rule = object
            .demolition
            .as_ref()
            .ok_or("structure_damage_demolition_rules_missing")?;
        if trigger >= rule.trigger_count {
            return Err("structure_damage_trigger_index_out_of_range");
        }
        let duration = modified_demolition_duration(rule.arm_ms, modifier)
            .ok_or("invalid_structure_damage_arm_modifier")?;
        let deadline = now
            .checked_add(Duration::from_millis(duration.into()))
            .ok_or("structure_damage_arm_deadline_overflow")?;
        // Retail synchronous SetTargetStateId replaces an unstarted async script request.
        mode.pending = None;
        mode.autonomous_arming = true;
        mode.reward_attacker = None;
        mode.reward_defender = None;
        object.fuse = None;
        object.operation = Some(Operation {
            started_at: now,
            recipient: None,
            player: actor.player,
            generation: actor.generation,
            trigger_index: trigger,
            deadline,
        });
        set_native_state(object, ObjectiveState::Activating, actor.player, now);
        object.state.expiration_time_ms = wire_time(deadline);
        object.state.deactivation_time_ms = 0;
        let mut mode = object.native.take().expect("validated native structure");
        enter_structural(object, &mut mode, previous);
        object.native = Some(mode);
        let delta = object.state;
        self.native_entries.push(NativeEntry {
            class: NativeClass::Structural,
            previous,
            delta,
            actor_generation: Some(actor.generation),
            reward_actor: None,
            active_trigger_index: Some(trigger),
        });
        self.pristine = false;
        Ok(Some(delta))
    }

    pub fn stage_repair(
        &mut self,
        target_object_id: u32,
        authority: repair::Authority,
    ) -> Result<(), &'static str> {
        let index = target_object_id as u16;
        if target_object_id != 0x0c80_0000 | u32::from(index) {
            return Err("objective_invalid_target");
        }
        let object = self
            .objects
            .get_mut(&index)
            .ok_or("unknown_native_objective")?;
        let mode = object
            .native
            .as_mut()
            .ok_or("native_objective_not_configured")?;
        mode.repair
            .as_mut()
            .ok_or("native_structural_repair_not_configured")?
            .submit(authority, mode.damage_health)?;
        self.pristine = false;
        Ok(())
    }

    /// Snapshot the authenticated admission at action start, not at delayed completion.
    pub(crate) fn attribute_operation(&mut self, index: u16, recipient: Option<RewardRecipient>) {
        let Some(object) = self.objects.get_mut(&index) else {
            return;
        };
        if let Some(NativePending::Demolition {
            recipient: staged, ..
        }) = object
            .native
            .as_mut()
            .and_then(|mode| mode.pending.as_mut())
        {
            if staged.is_none() {
                *staged = recipient;
            }
        } else if let Some(operation) = object.operation.as_mut()
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

    /// Apply a validated entry to the isolated Acquisition world. A controller
    /// transition cancels any old capture/fuse operation on that objective.
    pub(crate) fn install_acquisition_delta(
        &mut self,
        delta: ObjectiveControlDelta,
    ) -> Result<(), &'static str> {
        let object = self
            .objects
            .get_mut(&delta.object_index)
            .ok_or("unknown_acquisition_gameplay_objective")?;
        object.state = delta;
        object.operation = None;
        object.fuse = None;
        self.pristine = false;
        Ok(())
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
            ObjectiveState::Activating
                | ObjectiveState::Deactivating
                | ObjectiveState::Retaking
                | ObjectiveState::CancelRetaken
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
        if object.native.is_some() {
            let outcome =
                stage_native_interaction(object, request, actor, now, demolition_modifier);
            self.pristine &= !outcome.accepted;
            return outcome;
        }
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
            let outcome = interact_demolition(object, &rule, request, actor, now, false);
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
            if object.native.is_some() {
                continue;
            }
            if let Some(rule) = object.demolition.clone() {
                if tick_demolition(object, &rule, now, actors, &mut self.reward_actions, None) {
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
    rule.range().contains(actor.position)
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

fn secure_hold(initial: ObjectiveState, rule: &ObjectiveInteractionRule) -> secure::SecureHold {
    secure::SecureHold::new(
        initial,
        secure::Rules {
            owner_team: rule.owner_team,
            trigger_count: rule.trigger_count,
            can_deactivate: rule.can_deactivate,
            activate_ms: rule.activate_ms,
            complete_ms: rule.complete_ms,
            deactivate_ms: rule.deactivate_ms,
        },
    )
    .expect("validated secure hold")
}

fn new_structural_repair(
    initial: ObjectiveState,
    rule: Option<ObjectiveRepairRule>,
) -> Option<repair::StructuralRepair> {
    repair::StructuralRepair::new(initial, rule?.into()).ok()
}

fn at_native_trigger(
    position: CharacterPosition,
    range: crate::objective_range::ObjectiveRange<'_>,
    index: u8,
) -> bool {
    range.volumes.get(usize::from(index)).is_some_and(|volume| {
        crate::objective_range::ObjectiveRange {
            volumes: std::slice::from_ref(volume),
            ..range
        }
        .contains(position)
    })
}

fn stage_native_interaction(
    object: &mut Objective,
    request: ObjectiveInteraction,
    actor: ObjectiveActor,
    now: Duration,
    modifier: f32,
) -> InteractionOutcome {
    use ObjectiveInteractionKind::{Activate, Deactivate, Release};
    let mode = object.native.as_mut().expect("native objective");
    if mode.pending.is_some() {
        return InteractionOutcome::denied("native_objective_transition_pending");
    }
    if let (Some(hold), Some(rule)) = (&mut mode.secure, &object.rule) {
        let (action, base) = match request.kind {
            Activate => (secure::Action::Activate, Some(rule.activate_ms)),
            Deactivate => (secure::Action::Deactivate, Some(rule.deactivate_ms)),
            Release => (secure::Action::Release, None),
            _ => return InteractionOutcome::denied("objective_interaction_unsupported"),
        };
        let duration = match base {
            Some(base) => match modified_demolition_duration(base, modifier) {
                Some(duration) => Some(duration),
                None => return InteractionOutcome::denied("invalid_objective_modifier"),
            },
            None => None,
        };
        let contact = at_native_trigger(actor.position, rule.range(), request.trigger_index);
        let actor = secure::Actor {
            player: actor.player,
            team: actor.team,
            generation: actor.generation.into(),
        };
        return match hold.request(action, actor, request.trigger_index, contact) {
            Ok(transition) => {
                mode.pending = Some(NativePending::Secure(transition, duration));
                InteractionOutcome::accepted(
                    "native_objective_interaction_staged",
                    None,
                    if action == secure::Action::Release {
                        u8::MAX
                    } else {
                        request.trigger_index
                    },
                )
            }
            Err(reason) => InteractionOutcome::denied(reason),
        };
    }
    let Some(rule) = object.demolition.as_ref() else {
        return InteractionOutcome::denied("objective_interaction_not_configured");
    };
    if actor.team > 1
        || request.kind != Release
            && !at_native_trigger(actor.position, rule.range(), request.trigger_index)
    {
        return InteractionOutcome::denied("objective_invalid_native_contact");
    }
    if request.kind == Deactivate
        && object
            .fuse
            .as_ref()
            .is_some_and(|fuse| fuse.trigger_index != request.trigger_index)
    {
        return InteractionOutcome::denied("objective_active_trigger_mismatch");
    }
    if matches!(
        (request.kind, object.state.state_actor.state),
        (Activate, ObjectiveState::Activating) | (Deactivate, ObjectiveState::Deactivating)
    ) && object
        .operation
        .as_ref()
        .is_some_and(|operation| operation.trigger_index != request.trigger_index)
    {
        // A duplicate may preserve an existing hold, but cannot acknowledge a
        // different box while the operation still observes its original one.
        return InteractionOutcome::denied("objective_active_trigger_mismatch");
    }
    let was_repaired = mode.was_repaired;
    let Some(adjusted) = adjusted_demolition_rule(rule, request.kind, modifier) else {
        return InteractionOutcome::denied("invalid_demolition_modifier");
    };
    let outcome = interact_demolition(
        &mut object.clone(),
        &adjusted,
        request,
        actor,
        now,
        was_repaired,
    );
    if outcome.changed.is_some() {
        object.native.as_mut().expect("native objective").pending =
            Some(NativePending::Demolition {
                kind: request.kind,
                actor,
                trigger: request.trigger_index,
                modifier,
                recipient: None,
            });
    }
    InteractionOutcome {
        changed: None,
        ..outcome
    }
}

fn adjusted_demolition_rule(
    rule: &ObjectiveDemolitionRule,
    kind: ObjectiveInteractionKind,
    modifier: f32,
) -> Option<ObjectiveDemolitionRule> {
    let mut rule = rule.clone();
    match kind {
        ObjectiveInteractionKind::Activate => {
            rule.arm_ms = modified_demolition_duration(rule.arm_ms, modifier)?
        }
        ObjectiveInteractionKind::Deactivate => {
            rule.defuse_ms = modified_demolition_duration(rule.defuse_ms, modifier)?
        }
        _ => {}
    }
    Some(rule)
}

fn set_native_state(object: &mut Objective, target: ObjectiveState, player: u16, now: Duration) {
    object.state.state_actor = ObjectiveStateActor {
        state: target,
        actor_index: player,
        change_time_ms: wire_time(now),
    };
}

fn enter_secure(
    object: &mut Objective,
    hold: &mut secure::SecureHold,
    request: secure::Request,
    duration: Option<u32>,
    now: Duration,
) -> Option<u32> {
    let entry = hold
        .commit(request, wire_time(now), duration)
        .expect("staged secure hold transition");
    set_native_state(object, entry.current, entry.actor.player, now);
    object.state.expiration_time_ms = entry.expiration_ms.unwrap_or_default();
    object.state.deactivation_time_ms = entry.deactivation_ms.unwrap_or_default();
    let generation = u32::try_from(entry.actor.generation).ok();
    object.operation = if matches!(
        entry.current,
        ObjectiveState::Activating
            | ObjectiveState::Deactivating
            | ObjectiveState::Retaking
            | ObjectiveState::CancelRetaken
    ) {
        Some(Operation {
            started_at: now,
            recipient: None,
            player: entry.actor.player,
            generation: generation.expect("world actor generation"),
            trigger_index: request.trigger_index(),
            deadline: now,
        })
    } else {
        None
    };
    generation
}

fn enter_structural(object: &mut Objective, mode: &mut NativeObjective, previous: ObjectiveState) {
    use ObjectiveState::{Activated, Completed, Deactivated, Deactivating, Repaired, Repairing};
    let target = object.state.state_actor.state;
    if target != ObjectiveState::Activating {
        mode.autonomous_arming = false;
    }
    match target {
        Completed => {
            mode.damage_health = 0.0;
            if matches!(previous, Activated | Deactivating) {
                mode.was_repaired = false;
            }
        }
        Repaired => {
            mode.damage_health = 100.0;
            if matches!(previous, Repairing | Deactivating) {
                mode.was_repaired = true;
            }
        }
        Deactivated | Activated => mode.damage_health = 100.0,
        _ => {}
    }
    if target != Repairing {
        mode.repair = new_structural_repair(target, mode.repair_rule);
    }
    object.state.health_percent = mode.damage_health as u8;
}

/// Apply the next Structural state on a desired-Completed transition. The
/// intervening Activating and Activated entries skip arm/fuse delays because
/// their state IDs differ from the requested final state.
fn advance_precision(
    object: &mut Objective,
    source: Option<crate::structure_damage::Source>,
    now: Duration,
) -> Option<ObjectiveState> {
    use ObjectiveState::{
        Activated, Activating, Completed, Deactivated, Deactivating, Inactive, Repaired, Repairing,
    };
    let next = match object.state.state_actor.state {
        Inactive => Deactivated,
        Deactivated | Repaired => Activating,
        Activating => Activated,
        Activated | Deactivating | Repairing => Completed,
        _ => return None,
    };
    let player = if next == Deactivated {
        u16::MAX
    } else {
        source.map_or(u16::MAX, |actor| actor.player)
    };
    set_native_state(object, next, player, now);
    object.state.expiration_time_ms = 0;
    object.state.deactivation_time_ms = 0;
    if next == Completed {
        object.state.health_percent = 0;
    } else if matches!(next, Deactivated | Activated) {
        object.state.health_percent = 100;
    }
    object.operation = None;
    object.fuse = None;
    Some(next)
}

fn tick_native(
    object: &mut Objective,
    mode: &mut NativeObjective,
    now: Duration,
    elapsed_ms: u32,
    actors: &BTreeMap<u16, ObjectiveActor>,
    rewards: &mut Vec<ObjectiveRewardAction>,
) -> Option<u32> {
    let previous = object.state.state_actor.state;
    match mode.pending.take() {
        Some(NativePending::Script(target, actor)) => {
            mode.reward_attacker = None;
            mode.reward_defender = None;
            set_native_state(
                object,
                target,
                actor.map_or(u16::MAX, |actor| actor.player),
                now,
            );
            object.state.expiration_time_ms = 0;
            object.state.deactivation_time_ms = 0;
            object.operation = None;
            object.fuse = None;
            if mode.class == NativeClass::Structural {
                enter_structural(object, mode, previous);
            } else {
                mode.secure = object.rule.as_ref().map(|rule| secure_hold(target, rule));
            }
            return actor.and_then(|actor| u32::try_from(actor.generation).ok());
        }
        Some(NativePending::Precision(source)) => {
            // The desired-Completed cascade bypasses intermediate activation
            // stats. Its final actor requires side-resolved native latch data.
            mode.reward_attacker = None;
            mode.reward_defender = None;
            if let Some(next) = advance_precision(object, source, now) {
                enter_structural(object, mode, previous);
                if next != ObjectiveState::Completed {
                    mode.pending = Some(NativePending::Precision(source));
                }
                return source.map(|a| a.generation);
            }
        }
        Some(NativePending::Secure(request, duration)) => {
            return enter_secure(
                object,
                mode.secure.as_mut().expect("native secure hold"),
                request,
                duration,
                now,
            );
        }
        Some(NativePending::Demolition {
            kind,
            actor,
            trigger,
            modifier,
            recipient,
        }) => {
            let rule = adjusted_demolition_rule(
                object.demolition.as_ref().expect("native demolition"),
                kind,
                modifier,
            )
            .expect("validated demolition modifier");
            let request = ObjectiveInteraction {
                target_object_id: 0x0c80_0000 | u32::from(object.state.object_index),
                kind,
                actor_index: actor.player,
                trigger_index: trigger,
            };
            let fuse = object.fuse.clone();
            if interact_demolition(object, &rule, request, actor, now, mode.was_repaired)
                .changed
                .is_some()
            {
                if let Some(operation) = object.operation.as_mut() {
                    operation.recipient = recipient;
                }
                match (kind, object.state.state_actor.state) {
                    (ObjectiveInteractionKind::Activate, ObjectiveState::Activating) => {
                        mode.reward_attacker = Some(actor.player)
                    }
                    (ObjectiveInteractionKind::Deactivate, ObjectiveState::Deactivating) => {
                        mode.reward_defender = Some(actor.player)
                    }
                    _ => {}
                }
                enter_structural(object, mode, previous);
                return source_generation(object, Some(actor.generation), fuse.as_ref());
            }
            return None;
        }
        None => {}
    }

    if let (Some(hold), Some(rule)) = (mode.secure.as_mut(), object.rule.as_ref()) {
        let request = hold.due(wire_time(now), |source| {
            actors
                .get(&source.player)
                .zip(object.operation.as_ref())
                .is_some_and(|(actor, operation)| {
                    u64::from(actor.generation) == source.generation
                        && actor.team == source.team
                        && at_native_trigger(actor.position, rule.range(), operation.trigger_index)
                })
        })?;
        return enter_secure(object, hold, request, None, now);
    }

    if mode.autonomous_arming && previous == ObjectiveState::Activating {
        let operation = object.operation.as_ref().expect("autonomous arm operation");
        if now < operation.deadline {
            return None;
        }
        let operation = operation.clone();
        let rule = object
            .demolition
            .as_ref()
            .expect("autonomous demolition rules");
        object.fuse = Some(Operation {
            deadline: now.saturating_add(Duration::from_millis(rule.detonate_ms.into())),
            ..operation.clone()
        });
        armed_state(object, now);
        enter_structural(object, mode, previous);
        return Some(operation.generation);
    }

    if let Some(rule) = object.demolition.clone() {
        let operation = object.operation.clone();
        let fuse = object.fuse.clone();
        if tick_demolition(object, &rule, now, actors, rewards, Some(mode.was_repaired)) {
            enter_structural(object, mode, previous);
            return source_generation(
                object,
                operation.as_ref().map(|operation| operation.generation),
                fuse.as_ref(),
            );
        }
    }

    let core = mode.repair.as_mut()?;
    let request = match core
        .update(elapsed_ms, mode.damage_health, true)
        .expect("validated structural health")
    {
        repair::Update::None => return None,
        repair::Update::Transition(request) => request,
        repair::Update::Repair(batch) => {
            mode.damage_health = batch.health_after;
            object.state.health_percent = batch.health_after as u8;
            core.observe_completion(object.state.health_percent, true)?
        }
    };
    core.commit(request)
        .expect("staged structural repair transition");
    set_native_state(object, request.target, request.actor.player, now);
    object.state.expiration_time_ms = 0;
    object.state.deactivation_time_ms = 0;
    enter_structural(object, mode, previous);
    u32::try_from(request.actor.generation).ok()
}

fn source_generation(
    object: &Objective,
    generation: Option<u32>,
    fuse: Option<&Operation>,
) -> Option<u32> {
    let player = object.state.state_actor.actor_index;
    if player == u16::MAX {
        None
    } else if matches!(
        object.state.state_actor.state,
        ObjectiveState::Activated | ObjectiveState::Completed
    ) && let Some(fuse) = fuse.filter(|fuse| fuse.player == player)
    {
        Some(fuse.generation)
    } else {
        generation
    }
}

fn interact_demolition(
    object: &mut Objective,
    rule: &ObjectiveDemolitionRule,
    request: ObjectiveInteraction,
    actor: ObjectiveActor,
    now: Duration,
    was_repaired: bool,
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
            if was_repaired {
                object.state.state_actor.state = ObjectiveState::Repaired;
            }
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
            if was_repaired {
                ObjectiveState::Repaired
            } else {
                ObjectiveState::Deactivated
            },
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
    if state != expected
        && !(was_repaired
            && expected == ObjectiveState::Repaired
            && state == ObjectiveState::Deactivated)
    {
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
    native_was_repaired: Option<bool>,
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
        if state == ObjectiveState::Activated && native_was_repaired.is_none() {
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
                && native_was_repaired.map_or_else(
                    || demolition_in_range(*actor, rule),
                    |_| at_native_trigger(actor.position, rule.range(), operation.trigger_index),
                )
        });
        if !valid {
            if state == ObjectiveState::Deactivating {
                armed_state(object, now);
            } else {
                cancel(object, now);
                if native_was_repaired == Some(true) {
                    object.state.state_actor.state = ObjectiveState::Repaired;
                }
            }
        } else if now >= operation.deadline {
            if state == ObjectiveState::Activating {
                // Native class-entry consumers settle stats in callback order.
                // The legacy queue is drained only by the legacy objective driver.
                if native_was_repaired.is_none() {
                    emit_objective_reward(object, &operation, RewardKind::ChargePlanted, rewards);
                    emit_objective_reward(
                        object,
                        &operation,
                        RewardKind::PrimaryObjectiveActivated,
                        rewards,
                    );
                }
                object.fuse = Some(Operation {
                    deadline: now.saturating_add(Duration::from_millis(rule.detonate_ms.into())),
                    ..operation
                });
                // Native fresh Activated entry starts its fuse at actual entry time.
                armed_state(object, now);
            } else {
                if native_was_repaired.is_none() {
                    emit_objective_reward(object, &operation, RewardKind::ChargeDefused, rewards);
                    emit_objective_reward(
                        object,
                        &operation,
                        RewardKind::PrimaryObjectiveActivated,
                        rewards,
                    );
                }
                object.state.state_actor = ObjectiveStateActor {
                    state: if native_was_repaired == Some(true) {
                        ObjectiveState::Repaired
                    } else {
                        ObjectiveState::Deactivated
                    },
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
    rule.range().contains(actor.position)
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

#[cfg(test)]
pub(crate) mod trigger_box_tests {
    use super::*;
    use serde_json::json;

    // Invented catalog: one capture objective with one authored 5 x 2 x 2 box.
    fn world() -> ObjectiveWorld {
        let catalog = json!({"schema_version":1,"format":"mag_retail_objective_catalog","application_id":20231,
            "protocol":crate::protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":0,"map_name":"synthetic_arena",
            "entity_family":12,"network_type":12,
            "source":{"authored_catalog_sha256":"0".repeat(64),"binding_capture_sha256":"1".repeat(64),
                "startup_proof_sha256":"2".repeat(64),"source_image_sha256":"3".repeat(64),
                "capture_qualification":"bounded_concurrent_token_slot_capture"},
            "objects":[{"token":"0x0000000a","object_index":10,"initial_state":1,
                "interaction":{"owner_team":1,"activate_ms":10,"complete_ms":20,"deactivate_ms":10,
                    "can_deactivate":true,"position":[100.,3.,100.],"trigger_count":1,
                    "maximum_distance_m":5.,"distance_policy":"authored_trigger_box",
                    "trigger_volumes":[{"centre":[100.,3.,100.],"axes":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],
                        "half_extents":[2.5,1.,1.]}]}}],
            "squads":(0..2).flat_map(|team| (0..4).map(move |slot| json!({"team":team,
                "squad_index":team*16+slot,"initial_objective_index":10,
                "authored_objectives":[{"object_index":10,"order":1}]}))).collect::<Vec<_>>()});
        ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap())
    }
    fn activate(world: &mut ObjectiveWorld, z: f32) -> InteractionOutcome {
        world.interact(
            ObjectiveInteraction {
                target_object_id: 0x0c80_000a,
                kind: ObjectiveInteractionKind::Activate,
                actor_index: 7,
                trigger_index: 0,
            },
            ObjectiveActor {
                player: 7,
                team: 0,
                generation: 1,
                position: CharacterPosition { x: 100., y: 2.5, z },
            },
            Duration::from_secs(1),
        )
    }

    pub(crate) fn native_catalog() -> serde_json::Value {
        json!({
            "schema_version":1,"format":"mag_retail_objective_catalog","application_id":20231,
            "protocol":crate::protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":2,"map_name":"synthetic_arena",
            "entity_family":12,"network_type":12,
            "source":{"authored_catalog_sha256":"0".repeat(64),"binding_capture_sha256":"1".repeat(64),
                "startup_proof_sha256":"2".repeat(64),"source_image_sha256":"3".repeat(64),
                "capture_qualification":"native_source_gameplay"},
            "objects":[{"token":"0x0000000a","object_index":10,"initial_state":1,"native_class":"secure_hold",
                "interaction":{"owner_team":1,"activate_ms":10,"complete_ms":20,"deactivate_ms":10,
                    "can_deactivate":true,"position":[100.,3.,100.],"trigger_count":2,
                    "maximum_distance_m":5.,"distance_policy":"authored_trigger_box",
                    "trigger_volumes":[
                        {"centre":[100.,3.,100.],"axes":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],"half_extents":[2.5,1.,1.]},
                        {"centre":[100.,3.,106.],"axes":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],"half_extents":[2.5,1.,1.]}
                    ]}},
                {"token":"0x0000000b","object_index":11,"initial_state":7,"native_class":"structural",
                 "repair":{"owner_team":1,"can_repair":true,"repair_ms":1000,"minimum_repair_ms":1000},
                 "demolition":{"owner_team":1,"arm_ms":10,"detonate_ms":30,"defuse_ms":10,
                    "can_defuse":true,"activation_score":0,"deactivation_score":0,
                    "position":[100.,3.,100.],"trigger_count":1,"maximum_distance_m":5.,
                    "distance_policy":"authored_trigger_box","trigger_volumes":[
                        {"centre":[100.,3.,100.],"axes":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],"half_extents":[2.5,1.,1.]}
                    ]}}],
            "squads":(0..2).flat_map(|team| (0..16).map(move |slot| json!({"team":team,
                "squad_index":team*16+slot,"initial_objective_index":10,
                "authored_objectives":[{"object_index":10,"order":1}]}))).collect::<Vec<_>>()
        })
    }

    fn native_world() -> ObjectiveWorld {
        ObjectiveWorld::new(&ObjectiveCatalog::from_json(&native_catalog().to_string()).unwrap())
    }

    fn frame(
        world: &mut ObjectiveWorld,
        index: u16,
        ms: u64,
        actors: &BTreeMap<u16, ObjectiveActor>,
    ) -> Option<ObjectiveControlDelta> {
        let now = Duration::from_millis(ms);
        world.begin_native_tick(now);
        world.tick_native_object(index, now, actors)
    }

    #[test]
    fn precision_strike_queues_one_ordinary_structural_completion() {
        let mut catalog = native_catalog();
        catalog["objects"][1]["initial_state"] = json!(1);
        let live = ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap());
        let mut candidate = live.clone();
        let source = crate::structure_damage::Source {
            player: 4,
            generation: 3,
        };
        assert!(candidate.stage_precision_strike(11, Some(source)).unwrap());
        assert_eq!(candidate.state(11), Some(ObjectiveState::Deactivated));
        assert!(!candidate.has_native_entries());
        for (ms, state) in [
            (98, ObjectiveState::Activating),
            (99, ObjectiveState::Activated),
        ] {
            let delta = frame(&mut candidate, 11, ms, &BTreeMap::new()).unwrap();
            assert_eq!(delta.state_actor.state, state);
            assert_eq!(delta.expiration_time_ms, 0);
            assert_eq!(candidate.take_native_entries().len(), 1);
        }
        let completed = frame(&mut candidate, 11, 100, &BTreeMap::new()).unwrap();
        assert_eq!(completed.state_actor.state, ObjectiveState::Completed);
        assert_eq!(completed.state_actor.actor_index, 4);
        assert_eq!(completed.health_percent, 0);
        let entries = candidate.take_native_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor_generation, Some(3));
        assert_eq!(entries[0].active_trigger_index, None);
        assert!(!candidate.stage_precision_strike(11, Some(source)).unwrap());
        assert!(frame(&mut candidate, 11, 101, &BTreeMap::new()).is_none());
        assert_eq!(live.state(11), Some(ObjectiveState::Deactivated));
    }

    #[test]
    fn precision_does_not_force_a_locked_structure_to_complete() {
        let mut catalog = native_catalog();
        catalog["objects"][1]["initial_state"] = json!(9);
        let mut world =
            ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap());
        assert!(world.stage_precision_strike(11, None).unwrap());
        assert!(frame(&mut world, 11, 100, &BTreeMap::new()).is_none());
        assert_eq!(world.state(11), Some(ObjectiveState::Locked));
        assert!(!world.has_native_entries());
    }

    #[test]
    fn destroyed_damage_trigger_arms_structural_owner_without_a_human_hold() {
        let mut world = native_world();
        world
            .stage_native_state(11, ObjectiveState::Deactivated, None)
            .unwrap();
        frame(&mut world, 11, 1, &BTreeMap::new());
        world.take_native_entries();
        let source = crate::structure_damage::Source {
            player: 7,
            generation: 9,
        };
        let delta = world
            .activate_destroyed_trigger(11, 0, source, Duration::from_millis(10), -0.5)
            .unwrap()
            .unwrap();
        assert_eq!(delta.state_actor.state, ObjectiveState::Activating);
        assert_eq!(delta.expiration_time_ms, 15);
        let entries = world.take_native_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor_generation, Some(9));
        assert!(frame(&mut world, 11, 14, &BTreeMap::new()).is_none());
        let armed = frame(&mut world, 11, 15, &BTreeMap::new()).unwrap();
        assert_eq!(armed.state_actor.state, ObjectiveState::Activated);
        assert_eq!(armed.expiration_time_ms, 45);
        assert!(world.take_reward_actions().is_empty());
        assert_eq!(
            frame(&mut world, 11, 45, &BTreeMap::new())
                .unwrap()
                .state_actor
                .state,
            ObjectiveState::Completed
        );
        assert!(
            world
                .activate_destroyed_trigger(11, 0, source, Duration::from_millis(46), 0.)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn admitted_strike_replaces_queued_unstarted_script_target() {
        let mut world = native_world();
        world
            .stage_native_state(11, ObjectiveState::Deactivated, None)
            .unwrap();
        frame(&mut world, 11, 1, &BTreeMap::new());
        world.take_native_entries();
        world
            .stage_native_state(11, ObjectiveState::Locked, None)
            .unwrap();
        let source = crate::structure_damage::Source {
            player: 7,
            generation: 9,
        };
        let activated = world
            .activate_destroyed_trigger(11, 0, source, Duration::from_millis(10), 0.)
            .unwrap()
            .unwrap();
        assert_eq!(activated.state_actor.state, ObjectiveState::Activating);
        assert!(frame(&mut world, 11, 19, &BTreeMap::new()).is_none());
        assert_eq!(
            frame(&mut world, 11, 20, &BTreeMap::new())
                .unwrap()
                .state_actor
                .state,
            ObjectiveState::Activated
        );
    }

    #[test]
    fn native_secure_hold_stages_before_committing_one_object() {
        let mut world = native_world();
        assert_eq!(world.native_object_indices().collect::<Vec<_>>(), [10, 11]);
        assert_eq!(world.native_class(10), Some(NativeClass::SecureHold));
        assert!(!activate(&mut world, 106.).accepted);
        let outcome = activate(&mut world, 100.);
        assert!(outcome.accepted && outcome.changed.is_none());
        assert_eq!(world.state(10), Some(ObjectiveState::Deactivated));
        world
            .stage_native_state(11, ObjectiveState::Inactive, None)
            .unwrap();
        assert!(
            world
                .tick(Duration::from_secs(1), &BTreeMap::new())
                .is_empty()
        );
        let actor = ObjectiveActor {
            player: 7,
            team: 0,
            generation: 1,
            position: CharacterPosition {
                x: 100.,
                y: 2.5,
                z: 100.,
            },
        };
        let actors = BTreeMap::from([(7, actor)]);
        assert_eq!(
            frame(&mut world, 10, 1000, &actors)
                .unwrap()
                .expiration_time_ms,
            1010
        );
        assert_eq!(world.state(11), Some(ObjectiveState::Completed));
        let entries = world.take_native_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].previous, ObjectiveState::Deactivated);
        assert_eq!(entries[0].actor_generation, Some(1));
        assert_eq!(
            frame(&mut world, 11, 1000, &actors)
                .unwrap()
                .state_actor
                .state,
            ObjectiveState::Inactive
        );
        assert!(frame(&mut world, 10, 1009, &actors).is_none());
        assert_eq!(
            frame(&mut world, 10, 1010, &actors)
                .unwrap()
                .state_actor
                .state,
            ObjectiveState::Activated
        );
    }

    #[test]
    fn native_structural_repair_reports_net_health_and_entries() {
        let mut world = native_world();
        let mut authority = repair::Authority {
            actor: secure::Actor {
                player: 2,
                generation: 8,
                team: 0,
            },
            live: true,
            tool_active: true,
            repair_rate: 1.,
        };
        assert!(world.stage_repair(0x0c80_000b, authority).is_err());
        authority.actor.team = 1;
        world.stage_repair(0x0c80_000b, authority).unwrap();
        assert_eq!(world.state(11), Some(ObjectiveState::Completed));
        let empty = BTreeMap::new();
        let delta = frame(&mut world, 11, 20, &empty).unwrap();
        assert_eq!(delta.state_actor.state, ObjectiveState::Repairing);
        assert_eq!(delta.health_percent, 0);
        world.stage_repair(0x0c80_000b, authority).unwrap();
        assert!(frame(&mut world, 11, 1019, &empty).is_none());
        let delta = frame(&mut world, 11, 1020, &empty).unwrap();
        assert_eq!(delta.state_actor.state, ObjectiveState::Repaired);
        assert_eq!(delta.health_percent, 100);
        let entries = world.take_native_entries();
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .all(|entry| entry.actor_generation == Some(8))
        );
        assert!(!entries[1].requires_authored_explosion());
    }

    #[test]
    fn structural_defuse_restores_repaired_until_fuse_completes() {
        use ObjectiveInteractionKind::{Activate, Deactivate};
        let mut catalog = native_catalog();
        catalog["objects"][1]["initial_state"] = json!(6);
        let mut world =
            ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap());
        let attacker = ObjectiveActor {
            player: 7,
            team: 0,
            generation: 1,
            position: CharacterPosition {
                x: 100.,
                y: 2.5,
                z: 100.,
            },
        };
        let defender = ObjectiveActor {
            player: 8,
            team: 1,
            generation: 2,
            ..attacker
        };
        let actors = BTreeMap::from([(7, attacker), (8, defender)]);
        let interact = |world: &mut ObjectiveWorld, actor: ObjectiveActor, kind, ms| {
            let outcome = world.interact(
                ObjectiveInteraction {
                    target_object_id: 0x0c80_000b,
                    actor_index: actor.player,
                    trigger_index: 0,
                    kind,
                },
                actor,
                Duration::from_millis(ms),
            );
            assert!(
                outcome.accepted && outcome.changed.is_none(),
                "{}",
                outcome.reason
            );
        };

        interact(&mut world, attacker, Activate, 100);
        assert_eq!(world.state(11), Some(ObjectiveState::Repaired));
        assert_eq!(
            frame(&mut world, 11, 100, &actors)
                .unwrap()
                .state_actor
                .state,
            ObjectiveState::Activating
        );
        assert_eq!(
            frame(&mut world, 11, 110, &actors)
                .unwrap()
                .expiration_time_ms,
            140
        );
        interact(&mut world, defender, Deactivate, 115);
        frame(&mut world, 11, 115, &actors);
        assert_eq!(
            frame(&mut world, 11, 125, &actors)
                .unwrap()
                .state_actor
                .state,
            ObjectiveState::Repaired
        );
        assert!(
            world
                .take_native_entries()
                .iter()
                .all(|entry| !entry.requires_authored_explosion())
        );

        interact(&mut world, attacker, Activate, 200);
        frame(&mut world, 11, 200, &actors);
        frame(&mut world, 11, 210, &actors);
        interact(&mut world, defender, Deactivate, 230);
        frame(&mut world, 11, 230, &actors);
        let delta = frame(&mut world, 11, 240, &actors).unwrap();
        assert_eq!(delta.state_actor.state, ObjectiveState::Completed);
        assert_eq!(delta.health_percent, 0);
        let entry = world.take_native_entries().pop().unwrap();
        assert_eq!(entry.actor_generation, Some(1));
        assert!(entry.requires_authored_explosion());
    }

    #[test]
    fn authored_box_rejects_requests_the_development_sphere_accepted() {
        let mut far = world();
        let outcome = activate(&mut far, 104.);
        assert!(!outcome.accepted);
        assert_eq!(outcome.reason, "objective_out_of_range");
        let mut near = world();
        assert!(activate(&mut near, 101.2).accepted);
    }

    #[test]
    fn authored_box_policy_requires_matching_volumes() {
        let mut value: serde_json::Value = serde_json::from_str(
            &json!({"centre":[0.,0.,0.],"axes":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],"half_extents":[1.,1.,1.]}).to_string(),
        )
        .unwrap();
        value["half_extents"][0] = json!(-1.);
        let bad: crate::objective_range::TriggerBox = serde_json::from_value(value).unwrap();
        assert!(!crate::objective_range::ObjectiveRange::validate(
            crate::objective_range::AUTHORED_TRIGGER_BOX,
            5.,
            1,
            &[bad]
        ));
    }
}
