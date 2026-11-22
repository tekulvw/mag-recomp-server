//! Shared authoritative capture and demolition for reviewed retail objective bindings.
use crate::{
    Active, domination_objective_runtime as secure, domination_repair_runtime as repair,
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

#[derive(Clone, Copy, Debug, Default)]
pub struct ActionTimingModifiers {
    pub secure_and_hold: f32,
    pub demolition: f32,
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
    contact_arming: bool,
    recipient: Option<RewardRecipient>,
    capture_actor: Option<CaptureActorIdentity>,
    player: u16,
    generation: u32,
    trigger_index: u8,
    deadline: Duration,
}

#[derive(Clone)]
pub(crate) struct CaptureActorIdentity {
    reservation_id: String,
    world: u32,
    player: u16,
    match_id: String,
    account: u32,
}

impl CaptureActorIdentity {
    fn from_active(actor: &Active) -> Self {
        Self {
            reservation_id: actor.reservation.id.clone(),
            world: actor.reservation.world,
            player: actor.reservation.player,
            match_id: actor.reward_match_id.clone(),
            account: actor.reservation.account,
        }
    }

    pub(crate) fn matches(&self, actor: &Active) -> bool {
        self.reservation_id == actor.reservation.id
            && self.world == actor.reservation.world
            && self.player == actor.reservation.player
            && self.match_id == actor.reward_match_id
            && self.account == actor.reservation.account
    }
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
    current_actor: Option<u16>,
    reward_attacker: Option<u16>,
    reward_defender: Option<u16>,
}

#[derive(Clone)]
struct NativeObjective {
    class: NativeClass,
    owner_team: Option<u8>,
    secure: Option<secure::SecureHold>,
    repair_rule: Option<ObjectiveRepairRule>,
    repair: Option<repair::StructuralRepair>,
    damage_health: f32,
    was_repaired: bool,
    autonomous_arming: bool,
    active_trigger_index: Option<u8>,
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
    Script(ObjectiveState),
    Precision(Option<crate::structure_damage::Source>),
}

#[derive(Clone, Copy)]
pub(crate) enum PrecisionRewardActor {
    Attacker(u16),
    Defender(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeEntry {
    pub class: NativeClass,
    pub previous: ObjectiveState,
    pub delta: ObjectiveControlDelta,
    pub actor_generation: Option<u32>,
    /// Qualified native stat-callback actor, independent of the wire actor.
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
                            owner_team: object
                                .interaction
                                .as_ref()
                                .map(|rule| rule.owner_team)
                                .or_else(|| object.demolition.as_ref().map(|rule| rule.owner_team))
                                .or_else(|| object.repair.map(|rule| rule.owner_team))
                                .or_else(|| object.scoring.as_ref().map(|rule| rule.owner_team)),
                            secure: object
                                .interaction
                                .as_ref()
                                .map(|rule| secure_hold(initial, rule)),
                            repair_rule: object.repair,
                            repair: new_structural_repair(initial, object.repair),
                            damage_health,
                            was_repaired: initial == ObjectiveState::Repaired,
                            autonomous_arming: false,
                            active_trigger_index: None,
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
                            current_actor: None,
                            reward_attacker: None,
                            reward_defender: None,
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
        let class = mode.class;
        if let Some(NativePending::Script(target)) = mode.pending {
            let old = previous.state_actor.state;
            let entry = script_next_state(class, old, target).map(|next| {
                let active_trigger_index = mode.active_trigger_index;
                advance_script(object, &mut mode, old, next, target, now);
                NativeEntry {
                    class,
                    previous: old,
                    delta: object.state,
                    actor_generation: None,
                    reward_actor: script_reward_actor(object, class, old, next, target),
                    active_trigger_index,
                }
            });
            object.native = Some(mode);
            return entry.and_then(|entry| {
                self.native_entries.push(entry);
                self.pristine = false;
                (entry.delta != previous).then_some(entry.delta)
            });
        }
        let precision = matches!(mode.pending, Some(NativePending::Precision(_)));
        let active_trigger_index = if precision {
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
        let reward_actor = match (class, precision) {
            (NativeClass::SecureHold, _) => Some(object.state.state_actor.actor_index),
            (NativeClass::Structural, true) => {
                precision_completion_actor(object, previous.state_actor.state)
            }
            (NativeClass::Structural, false) => {
                use ObjectiveState::{
                    Activated, Activating, Completed, Deactivated, Deactivating, Repaired,
                    Repairing,
                };
                match (previous.state_actor.state, object.state.state_actor.state) {
                    (Activating, Activated) | (Activated | Deactivating, Completed) => {
                        object.reward_attacker
                    }
                    (Deactivating, Deactivated | Repaired) | (Repairing, Repaired) => {
                        object.reward_defender
                    }
                    _ => None,
                }
            }
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
        self.stage_native_state_with_caller(index, target, actor.map(|actor| actor.player), actor)
    }

    pub(crate) fn stage_native_state_with_caller(
        &mut self,
        index: u16,
        target: ObjectiveState,
        raw_slot: Option<u16>,
        actor: Option<secure::Actor>,
    ) -> Result<(), &'static str> {
        let Some(object) = self.objects.get(&index) else {
            return Err("unknown_native_objective");
        };
        if object.native.is_none() {
            return Err("native_objective_not_configured");
        }
        if object.state.state_actor.state == target {
            return Ok(());
        }
        self.stage_native_target(index, target)?;
        let raw_slot = raw_slot.filter(|&slot| slot != u16::MAX);
        let object = self
            .objects
            .get_mut(&index)
            .expect("qualified native objective");
        let mode = object.native.as_mut().expect("qualified native objective");
        object.current_actor = raw_slot;
        mode.active_trigger_index = None;
        if let (Some(actor), Some(owner_team)) = (
            actor.filter(|a| Some(a.player) == raw_slot),
            mode.owner_team,
        ) {
            if actor.team == owner_team {
                object.reward_defender = Some(actor.player);
            } else {
                object.reward_attacker = Some(actor.player);
            }
        }
        Ok(())
    }

    pub(crate) fn stage_native_target(
        &mut self,
        index: u16,
        target: ObjectiveState,
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
        if target != ObjectiveState::Completed
            || !matches!(mode.pending, Some(NativePending::Precision(_)))
        {
            mode.pending = Some(NativePending::Script(target));
        }
        self.pristine = false;
        Ok(())
    }

    pub(crate) fn script_actor(&self, index: u16) -> Option<u16> {
        self.objects.get(&index)?.current_actor
    }

    pub(crate) fn has_script_transition(&self, index: u16) -> bool {
        matches!(
            self.objects
                .get(&index)
                .and_then(|object| object.native.as_ref())
                .and_then(|mode| mode.pending.as_ref()),
            Some(NativePending::Script(_))
        )
    }

    /// Precision executes the Structural desired-state setter. Acquisition
    /// consumes the same request through its separate objective controller.
    #[cfg(test)]
    pub(crate) fn stage_precision_strike(
        &mut self,
        index: u16,
        source: Option<crate::structure_damage::Source>,
    ) -> Result<bool, &'static str> {
        self.stage_precision_strike_with_actor(index, source, None)
    }

    #[cfg(test)]
    pub(crate) fn stage_precision_strike_with_actor(
        &mut self,
        index: u16,
        source: Option<crate::structure_damage::Source>,
        reward_actor: Option<PrecisionRewardActor>,
    ) -> Result<bool, &'static str> {
        self.stage_precision_strike_with_caller(
            index,
            source,
            reward_actor,
            source.map(|actor| actor.player),
        )
    }

    pub(crate) fn stage_precision_strike_with_caller(
        &mut self,
        index: u16,
        source: Option<crate::structure_damage::Source>,
        reward_actor: Option<PrecisionRewardActor>,
        raw_slot: Option<u16>,
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
                mode.active_trigger_index = None;
            }
            Some(_) => return Ok(false),
            None => {
                self.acquisition_precision.insert(index, source);
            }
        }
        match reward_actor {
            Some(PrecisionRewardActor::Attacker(player)) => object.reward_attacker = Some(player),
            Some(PrecisionRewardActor::Defender(player)) => object.reward_defender = Some(player),
            None => {}
        }
        object.current_actor = raw_slot.filter(|&slot| slot != u16::MAX);
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
            enter_structural_actor(object, object.state.state_actor.state);
            let entry = NativeEntry {
                class: NativeClass::Structural,
                previous,
                delta: object.state,
                actor_generation: source.map(|actor| actor.generation),
                reward_actor: precision_completion_actor(object, previous),
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
            enter_structural_actor(object, target);
            object.state.health_percent = if target == ObjectiveState::Completed {
                0
            } else {
                100
            };
        } else {
            object.current_actor = Some(actor).filter(|&slot| slot != u16::MAX);
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
    #[cfg(test)]
    pub(crate) fn activate_destroyed_trigger(
        &mut self,
        index: u16,
        trigger: u8,
        actor: crate::structure_damage::Source,
        now: Duration,
        modifier: f32,
    ) -> Result<Option<ObjectiveControlDelta>, &'static str> {
        self.activate_destroyed_trigger_with_source(index, trigger, actor, false, now, modifier)
    }

    pub(crate) fn activate_destroyed_trigger_with_source(
        &mut self,
        index: u16,
        trigger: u8,
        actor: crate::structure_damage::Source,
        source_allocated: bool,
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
        let duration = modified_action_duration(rule.arm_ms, modifier)
            .ok_or("invalid_structure_damage_arm_modifier")?;
        let deadline = now
            .checked_add(Duration::from_millis(duration.into()))
            .ok_or("structure_damage_arm_deadline_overflow")?;
        // Retail synchronous SetTargetStateId replaces an unstarted async script request.
        mode.pending = None;
        mode.autonomous_arming = true;
        mode.active_trigger_index = Some(trigger);
        // Trigger admission installs the source in the attacker slot for either team.
        object.reward_attacker = source_allocated.then_some(actor.player);
        object.fuse = None;
        object.operation = Some(Operation {
            started_at: now,
            contact_arming: false,
            recipient: None,
            capture_actor: None,
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
        match self.stage_repair_packet(target_object_id, authority)? {
            repair::Submission::Queued(_) => Ok(()),
            repair::Submission::LatchOnly => Err(repair::HEALTH_ALREADY_COVERED),
        }
    }

    pub(crate) fn stage_repair_packet(
        &mut self,
        target_object_id: u32,
        authority: repair::Authority,
    ) -> Result<repair::Submission, &'static str> {
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
        let submission = mode
            .repair
            .as_mut()
            .ok_or("native_structural_repair_not_configured")?
            .submit_packet(authority, mode.damage_health)?;
        object.reward_defender = Some(authority.actor.player);
        self.pristine = false;
        Ok(submission)
    }

    /// Snapshot the authenticated admission at action start, not at delayed completion.
    pub(crate) fn attribute_operation(&mut self, index: u16, actor: &Active) {
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
                *staged = RewardRecipient::from_active(actor);
            }
        } else if let Some(operation) = object.operation.as_mut()
            && operation.player == actor.reservation.player
            && operation
                .capture_actor
                .as_ref()
                .is_none_or(|identity| identity.matches(actor))
        {
            if operation.recipient.is_none() {
                operation.recipient = RewardRecipient::from_active(actor);
            }
            if object.native.is_none()
                && object.demolition.is_none()
                && operation.capture_actor.is_none()
            {
                operation.capture_actor = Some(CaptureActorIdentity::from_active(actor));
            }
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
            contact_arming: false,
            recipient: None,
            capture_actor: None,
            player: u16::MAX,
            generation: 0,
            trigger_index: 0,
            deadline: now.saturating_add(Duration::from_millis(rule.detonate_ms.into())),
        });
        armed_state(object, now);
        self.pristine = false;
        Ok(object.state)
    }

    pub(crate) fn native_structural_live_contact_fuse_trigger(
        &self,
        index: u16,
        now: Duration,
    ) -> Option<u8> {
        let object = self.objects.get(&index)?;
        let mode = object.native.as_ref()?;
        let fuse = object.fuse.as_ref()?;
        (mode.class == NativeClass::Structural
            && mode.pending.is_none()
            && matches!(
                object.state.state_actor.state,
                ObjectiveState::Activated | ObjectiveState::Deactivating
            )
            && fuse.contact_arming
            && now < fuse.deadline)
            .then_some(fuse.trigger_index)
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
        self.interact_with_action_timing(request, actor, now, ActionTimingModifiers::default())
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
        self.interact_with_action_timing(
            request,
            actor,
            now,
            ActionTimingModifiers {
                demolition: demolition_modifier,
                ..Default::default()
            },
        )
    }

    pub fn interact_with_action_timing(
        &mut self,
        request: ObjectiveInteraction,
        actor: ObjectiveActor,
        now: Duration,
        timings: ActionTimingModifiers,
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
            let outcome = stage_native_interaction(object, request, actor, now, timings);
            self.pristine &= !outcome.accepted;
            return outcome;
        }
        if let Some(rule) = object.demolition.clone() {
            let outcome = interact_demolition(
                object,
                &rule,
                request,
                actor,
                now,
                false,
                timings.demolition,
            );
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
                let duration = if timings.secure_and_hold == 0.0 {
                    Some(rule.activate_ms)
                } else {
                    modified_action_duration(rule.activate_ms, timings.secure_and_hold)
                };
                let Some(duration) = duration else {
                    return InteractionOutcome::denied("invalid_objective_modifier");
                };
                let deadline = now.saturating_add(Duration::from_millis(duration.into()));
                object.operation = Some(Operation {
                    started_at: now,
                    contact_arming: false,
                    recipient: None,
                    capture_actor: None,
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
        self.tick_observing_captures(now, actors, |_, _| {})
    }

    pub(crate) fn tick_with_capture_completions(
        &mut self,
        now: Duration,
        actors: &BTreeMap<u16, ObjectiveActor>,
    ) -> (Vec<ObjectiveControlDelta>, Vec<CaptureCompletion>) {
        let mut captures = Vec::new();
        let changes = self.tick_observing_captures(now, actors, |object_index, operation| {
            captures.push(CaptureCompletion {
                object_index,
                player: operation.player,
                actor: operation.capture_actor.clone(),
            });
        });
        (changes, captures)
    }

    fn tick_observing_captures(
        &mut self,
        now: Duration,
        actors: &BTreeMap<u16, ObjectiveActor>,
        mut capture_completed: impl FnMut(u16, &Operation),
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
                capture_completed(object.state.object_index, &operation);
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

pub(crate) struct CaptureCompletion {
    pub(crate) object_index: u16,
    pub(crate) player: u16,
    pub(crate) actor: Option<CaptureActorIdentity>,
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

fn modified_action_duration(base_ms: u32, modifier: f32) -> Option<u32> {
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
    timings: ActionTimingModifiers,
) -> InteractionOutcome {
    use ObjectiveInteractionKind::{Activate, Deactivate, Release};
    let mode = object.native.as_mut().expect("native objective");
    if mode.pending.is_some() {
        return InteractionOutcome::denied("native_objective_transition_pending");
    }
    if let (Some(hold), Some(rule)) = (&mut mode.secure, &object.rule) {
        let action = match request.kind {
            Activate => secure::Action::Activate,
            Deactivate => secure::Action::Deactivate,
            Release => secure::Action::Release,
            _ => return InteractionOutcome::denied("objective_interaction_unsupported"),
        };
        let contact = at_native_trigger(actor.position, rule.range(), request.trigger_index);
        let actor = secure::Actor {
            player: actor.player,
            team: actor.team,
            generation: actor.generation.into(),
        };
        let mut candidate = hold.clone();
        let transition = match candidate.request(action, actor, request.trigger_index, contact) {
            Ok(transition) => transition,
            Err(reason) => return InteractionOutcome::denied(reason),
        };
        let base = match transition.target() {
            ObjectiveState::Activating | ObjectiveState::Retaking => Some(rule.activate_ms),
            ObjectiveState::Deactivating | ObjectiveState::CancelRetaken => {
                Some(rule.deactivate_ms)
            }
            _ => None,
        };
        let duration = if let Some(base) = base {
            let Some(duration) = modified_action_duration(base, timings.secure_and_hold) else {
                return InteractionOutcome::denied("invalid_objective_modifier");
            };
            Some(duration)
        } else {
            None
        };
        *hold = candidate;
        mode.pending = Some(NativePending::Secure(transition, duration));
        return InteractionOutcome::accepted(
            "native_objective_interaction_staged",
            None,
            if action == secure::Action::Release {
                u8::MAX
            } else {
                request.trigger_index
            },
        );
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
    let modifier = timings.demolition;
    let outcome = interact_demolition(
        &mut object.clone(),
        rule,
        request,
        actor,
        now,
        was_repaired,
        modifier,
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

fn set_native_state(object: &mut Objective, target: ObjectiveState, player: u16, now: Duration) {
    object.state.state_actor = ObjectiveStateActor {
        state: target,
        actor_index: player,
        change_time_ms: wire_time(now),
    };
}

fn script_next_state(
    class: NativeClass,
    previous: ObjectiveState,
    desired: ObjectiveState,
) -> Option<ObjectiveState> {
    use ObjectiveState::*;
    // The stable desired state selects a column; the physical state selects a
    // row. Each update dispatches the selected entry before evaluating again.
    let column = match desired {
        Inactive => 0,
        Deactivated => 1,
        Completed => 2,
        Locked => 3,
        _ => return None,
    };
    let next = match class {
        NativeClass::Structural => {
            let states = match previous {
                Inactive => [Inactive, Deactivated, Deactivated],
                Deactivated | Repaired => [Inactive, Deactivated, Activating],
                Activating => [Inactive, Deactivated, Activated],
                Activated => [Inactive, Deactivating, Completed],
                Deactivating | Repairing | Completed => [Inactive, Deactivated, Completed],
                _ => return None,
            };
            *states.get(column)?
        }
        NativeClass::SecureHold => {
            let states = match previous {
                Inactive => [Inactive, Deactivated, Deactivated, Deactivated],
                Deactivated => [Inactive, Deactivated, Activating, Activating],
                Activating => [Deactivated, Deactivated, Activated, Activated],
                Activated => [Deactivating, Deactivating, Completed, Completed],
                Deactivating => [Deactivated, Deactivated, Activated, Activated],
                Completed => [Retaking, Retaking, Completed, Locked],
                Locked => [Completed, Completed, Completed, Locked],
                Retaking | CancelRetaken => [Retaken, Retaken, Completed, Completed],
                Retaken => [Deactivated, Deactivated, CancelRetaken, CancelRetaken],
                _ => return None,
            };
            states[column]
        }
    };
    Some(next)
}

fn advance_script(
    object: &mut Objective,
    mode: &mut NativeObjective,
    previous: ObjectiveState,
    next: ObjectiveState,
    desired: ObjectiveState,
    now: Duration,
) {
    use ObjectiveState::*;
    object.operation = None;
    object.fuse = None;
    let actor = match mode.class {
        NativeClass::Structural => enter_structural_actor(object, next),
        NativeClass::SecureHold => enter_secure_fields(object, next),
    };
    let change_time_ms = if next == previous {
        object.state.state_actor.change_time_ms
    } else {
        wire_time(now)
    };
    object.state.state_actor = ObjectiveStateActor {
        state: next,
        actor_index: actor.unwrap_or(u16::MAX),
        change_time_ms,
    };
    match mode.class {
        NativeClass::Structural => enter_structural_health(object, mode, previous),
        NativeClass::SecureHold => {
            if next == desired {
                mode.secure = object.rule.as_ref().map(|rule| secure_hold(next, rule));
            }
        }
    }
    match next {
        Inactive | Locked => {
            object.state.expiration_time_ms = 0;
            object.state.deactivation_time_ms = 0;
        }
        Deactivated => {
            object.state.expiration_time_ms = 0;
            if previous != Deactivating {
                object.state.deactivation_time_ms = 0;
            }
        }
        Activating | Activated | Retaking | Retaken => {
            object.state.expiration_time_ms = wire_time(now);
            object.state.deactivation_time_ms = 0;
        }
        Deactivating | CancelRetaken => object.state.deactivation_time_ms = wire_time(now),
        Completed => object.state.deactivation_time_ms = 0,
        _ => unreachable!("unsupported script entry"),
    }
    mode.pending = (next != desired).then_some(NativePending::Script(desired));
}

fn script_reward_actor(
    object: &Objective,
    class: NativeClass,
    previous: ObjectiveState,
    next: ObjectiveState,
    desired: ObjectiveState,
) -> Option<u16> {
    use ObjectiveState::*;
    match (class, previous, next) {
        (NativeClass::Structural, Activated | Deactivating, Completed)
        | (NativeClass::SecureHold, Activated | CancelRetaken, Completed) => object.reward_attacker,
        (_, Deactivating, Deactivated) | (NativeClass::SecureHold, Retaken, Deactivated) => {
            object.reward_defender
        }
        (_, Activating, Activated) if next == desired => object.reward_attacker,
        (NativeClass::SecureHold, Retaking, Retaken) if next == desired => object.reward_defender,
        _ => None,
    }
}

fn enter_secure_fields(object: &mut Objective, target: ObjectiveState) -> Option<u16> {
    use ObjectiveState::*;
    let actor = match target {
        Inactive | Locked => {
            object.reward_attacker = None;
            object.reward_defender = None;
            None
        }
        Deactivated | Retaken => {
            object.reward_attacker = None;
            object.reward_defender
        }
        Activated | Completed => {
            object.reward_defender = None;
            object.reward_attacker
        }
        Activating | CancelRetaken => object.reward_attacker,
        Deactivating | Retaking => object.reward_defender,
        _ => unreachable!("unsupported Secure entry"),
    };
    object.current_actor = actor;
    match target {
        Deactivated => object.state.health_percent = 100,
        Completed => object.state.health_percent = 0,
        _ => {}
    }
    actor
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
    enter_secure_fields(object, entry.current);
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
            contact_arming: false,
            recipient: None,
            capture_actor: None,
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
    enter_structural_actor(object, object.state.state_actor.state);
    enter_structural_health(object, mode, previous);
}

fn enter_structural_actor(object: &mut Objective, target: ObjectiveState) -> Option<u16> {
    use ObjectiveState::*;
    let actor = match target {
        Inactive => {
            object.reward_attacker = None;
            object.reward_defender = None;
            None
        }
        Deactivated => {
            object.reward_attacker = None;
            object.reward_defender
        }
        Activating => object.reward_attacker,
        Activated | Completed => {
            object.reward_defender = None;
            object.reward_attacker
        }
        Deactivating | Repairing | Repaired => object.reward_defender,
        _ => None,
    };
    object.current_actor = if target == Deactivated { None } else { actor };
    actor
}

fn enter_structural_health(
    object: &mut Objective,
    mode: &mut NativeObjective,
    previous: ObjectiveState,
) {
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
        Deactivated => {
            mode.damage_health = 100.0;
        }
        Activated => {
            mode.damage_health = 100.0;
        }
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

fn precision_completion_actor(object: &Objective, previous: ObjectiveState) -> Option<u16> {
    if object.state.state_actor.state == ObjectiveState::Completed
        && matches!(
            previous,
            ObjectiveState::Activated | ObjectiveState::Deactivating
        )
    {
        object.reward_attacker
    } else {
        None
    }
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
        Some(NativePending::Script(_)) => unreachable!("script handled before ordinary tick"),
        Some(NativePending::Precision(source)) => {
            if let Some(next) = advance_precision(object, source, now) {
                enter_structural(object, mode, previous);
                if next != ObjectiveState::Completed {
                    mode.pending = Some(NativePending::Precision(source));
                }
                return source.map(|a| a.generation);
            }
        }
        Some(NativePending::Secure(request, duration)) => {
            match request.target() {
                ObjectiveState::Activating | ObjectiveState::CancelRetaken => {
                    object.reward_attacker = Some(request.actor().player);
                }
                ObjectiveState::Deactivating | ObjectiveState::Retaking => {
                    object.reward_defender = Some(request.actor().player);
                }
                _ => {}
            }
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
            let rule = object.demolition.clone().expect("native demolition rule");
            let request = ObjectiveInteraction {
                target_object_id: 0x0c80_0000 | u32::from(object.state.object_index),
                kind,
                actor_index: actor.player,
                trigger_index: trigger,
            };
            let fuse = object.fuse.clone();
            if interact_demolition(
                object,
                &rule,
                request,
                actor,
                now,
                mode.was_repaired,
                modifier,
            )
            .changed
            .is_some()
            {
                if let Some(operation) = object.operation.as_mut() {
                    operation.recipient = recipient;
                }
                match (kind, object.state.state_actor.state) {
                    (ObjectiveInteractionKind::Activate, ObjectiveState::Activating) => {
                        object.reward_attacker = Some(actor.player);
                        mode.active_trigger_index = Some(trigger);
                    }
                    (ObjectiveInteractionKind::Deactivate, ObjectiveState::Deactivating) => {
                        object.reward_defender = Some(actor.player);
                        mode.active_trigger_index = Some(trigger);
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
    modifier: f32,
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
    let Some(duration) = modified_action_duration(duration, modifier) else {
        return InteractionOutcome::denied("invalid_demolition_modifier");
    };
    let deadline = now.saturating_add(Duration::from_millis(duration.into()));
    object.operation = Some(Operation {
        started_at: now,
        contact_arming: request.kind == Activate,
        recipient: None,
        capture_actor: None,
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
    fn structural_script_same_current_keeps_pending_desired_and_completes_through_entries() {
        let mut catalog = native_catalog();
        catalog["objects"][1]["initial_state"] = json!(1);
        let mut world =
            ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap());
        let enemy = secure::Actor {
            player: 7,
            generation: 3,
            team: 0,
        };
        let owner = secure::Actor {
            player: 8,
            generation: 4,
            team: 1,
        };
        world
            .stage_native_state(11, ObjectiveState::Completed, Some(enemy))
            .unwrap();
        world
            .stage_native_state(11, ObjectiveState::Completed, Some(owner))
            .unwrap();
        world
            .stage_native_state(11, ObjectiveState::Deactivated, None)
            .unwrap();
        assert_eq!(world.script_actor(11), Some(owner.player));

        for (state, actor, credit) in [
            (ObjectiveState::Activating, 7, None),
            (ObjectiveState::Activated, 7, None),
            (ObjectiveState::Completed, 7, Some(7)),
        ] {
            let delta = frame(&mut world, 11, 40, &BTreeMap::new()).unwrap();
            assert_eq!(delta.state_actor.state, state);
            assert_eq!(delta.state_actor.actor_index, actor);
            assert_eq!(world.script_actor(11), Some(actor));
            assert_eq!(
                world.take_native_entries().pop().unwrap().reward_actor,
                credit
            );
        }
        assert!(frame(&mut world, 11, 41, &BTreeMap::new()).is_none());
    }

    #[test]
    fn structural_direct_target_reenters_current_and_preserves_the_request_actor() {
        use ObjectiveState::{Completed, Deactivated};
        let mut catalog = native_catalog();
        catalog["objects"][1]["initial_state"] = json!(1);
        let mut world =
            ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap());
        world
            .stage_native_state_with_caller(11, Completed, Some(7), None)
            .unwrap();
        world.stage_native_target(11, Deactivated).unwrap();
        assert_eq!(world.script_actor(11), Some(7));

        assert!(frame(&mut world, 11, 100, &BTreeMap::new()).is_none());
        let entry = world.take_native_entries().pop().unwrap();
        assert_eq!(entry.previous, Deactivated);
        assert_eq!(entry.delta.state_actor.state, Deactivated);
        assert_eq!(entry.delta.state_actor.change_time_ms, 0);
        assert_eq!(entry.reward_actor, None);
        assert_eq!(world.script_actor(11), None);
        assert!(!world.has_script_transition(11));
    }

    #[test]
    fn structural_unresolved_script_caller_preserves_the_planter_and_defender() {
        use ObjectiveState::{Activated, Completed, Deactivated, Deactivating};
        let mut catalog = native_catalog();
        catalog["objects"][1]["initial_state"] = json!(1);
        let mut world =
            ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap());
        let enemy = secure::Actor {
            player: 7,
            team: 0,
            generation: 3,
        };
        let owner = secure::Actor {
            player: 8,
            team: 1,
            generation: 4,
        };
        world
            .stage_native_state(11, Completed, Some(enemy))
            .unwrap();
        frame(&mut world, 11, 100, &BTreeMap::new()).unwrap();
        assert_eq!(
            frame(&mut world, 11, 100, &BTreeMap::new())
                .unwrap()
                .state_actor
                .state,
            Activated
        );
        world.take_native_entries();

        let mut completing = world.clone();
        completing
            .stage_native_state_with_caller(11, Completed, Some(9), None)
            .unwrap();
        assert_eq!(completing.script_actor(11), Some(9));
        frame(&mut completing, 11, 101, &BTreeMap::new()).unwrap();
        let complete = completing.take_native_entries().pop().unwrap();
        assert_eq!(complete.delta.state_actor.actor_index, enemy.player);
        assert_eq!(complete.reward_actor, Some(enemy.player));
        assert_eq!(complete.delta.expiration_time_ms, 100);

        world
            .stage_native_state(11, Deactivated, Some(owner))
            .unwrap();
        world
            .stage_native_state_with_caller(11, Deactivated, Some(9), None)
            .unwrap();
        assert_eq!(world.script_actor(11), Some(9));
        let deactivating = frame(&mut world, 11, 102, &BTreeMap::new()).unwrap();
        assert_eq!(deactivating.state_actor.state, Deactivating);
        assert_eq!(deactivating.expiration_time_ms, 100);
        assert_eq!(deactivating.deactivation_time_ms, 102);
        assert_eq!(
            world.take_native_entries().pop().unwrap().reward_actor,
            None
        );
        let disarmed = frame(&mut world, 11, 102, &BTreeMap::new()).unwrap();
        let entry = world.take_native_entries().pop().unwrap();
        assert_eq!(entry.previous, Deactivating);
        assert_eq!(entry.delta.state_actor.actor_index, owner.player);
        assert_eq!(entry.reward_actor, Some(owner.player));
        assert_eq!(world.script_actor(11), None);
        assert_eq!(disarmed.expiration_time_ms, 0);
        assert_eq!(disarmed.deactivation_time_ms, 102);
    }

    #[test]
    fn ordinary_secure_retake_and_completion_publish_the_entry_health() {
        use ObjectiveInteractionKind::{Activate, Deactivate};
        use ObjectiveState::{Completed, Deactivated, Retaken, Retaking};
        let mut world = native_world();
        world.stage_native_state(10, Completed, None).unwrap();
        for _ in 0..3 {
            frame(&mut world, 10, 100, &BTreeMap::new()).unwrap();
        }
        assert_eq!(
            world
                .take_native_entries()
                .last()
                .unwrap()
                .delta
                .health_percent,
            0
        );

        let interact = |world: &mut ObjectiveWorld, player, team, kind, ms| {
            let actor = ObjectiveActor {
                player,
                team,
                generation: 1,
                position: CharacterPosition {
                    x: 100.,
                    y: 2.5,
                    z: 100.,
                },
            };
            assert!(
                world
                    .interact(
                        ObjectiveInteraction {
                            target_object_id: 0x0c80_000a,
                            kind,
                            actor_index: player,
                            trigger_index: 0,
                        },
                        actor,
                        Duration::from_millis(ms),
                    )
                    .accepted
            );
            BTreeMap::from([(player, actor)])
        };
        let owner = interact(&mut world, 8, 1, Deactivate, 101);
        assert_eq!(
            frame(&mut world, 10, 101, &owner)
                .unwrap()
                .state_actor
                .state,
            Retaking
        );
        assert_eq!(
            frame(&mut world, 10, 111, &owner)
                .unwrap()
                .state_actor
                .state,
            Retaken
        );
        let deactivated = frame(&mut world, 10, 131, &owner).unwrap();
        assert_eq!(deactivated.state_actor.state, Deactivated);
        assert_eq!(deactivated.health_percent, 100);

        let enemy = interact(&mut world, 7, 0, Activate, 132);
        frame(&mut world, 10, 132, &enemy).unwrap();
        frame(&mut world, 10, 142, &enemy).unwrap();
        let completed = frame(&mut world, 10, 162, &enemy).unwrap();
        assert_eq!(completed.state_actor.state, Completed);
        assert_eq!(completed.health_percent, 0);
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
    fn precision_completion_keeps_the_planter_when_the_strike_source_is_unresolved() {
        let mut catalog = native_catalog();
        catalog["objects"][1]["initial_state"] = json!(1);
        let mut world =
            ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap());
        let planter = ObjectiveActor {
            player: 7,
            team: 0,
            generation: 1,
            position: CharacterPosition {
                x: 100.,
                y: 2.5,
                z: 100.,
            },
        };
        let actors = BTreeMap::from([(planter.player, planter)]);
        assert!(
            world
                .interact(
                    ObjectiveInteraction {
                        target_object_id: 0x0c80_000b,
                        kind: ObjectiveInteractionKind::Activate,
                        actor_index: planter.player,
                        trigger_index: 0,
                    },
                    planter,
                    Duration::from_millis(80),
                )
                .accepted
        );
        frame(&mut world, 11, 80, &actors).unwrap();
        frame(&mut world, 11, 90, &actors).unwrap();
        assert_eq!(
            world.take_native_entries().last().unwrap().reward_actor,
            Some(7)
        );

        let strike = crate::structure_damage::Source {
            player: 4,
            generation: 3,
        };
        for (reward_actor, credited) in [
            (None, planter.player),
            (Some(PrecisionRewardActor::Defender(4)), planter.player),
            (Some(PrecisionRewardActor::Attacker(4)), 4),
        ] {
            let mut candidate = world.clone();
            assert!(
                candidate
                    .stage_precision_strike_with_actor(11, Some(strike), reward_actor)
                    .unwrap()
            );
            frame(&mut candidate, 11, 100, &BTreeMap::new()).unwrap();
            let entry = candidate.take_native_entries().pop().unwrap();
            assert_eq!(entry.delta.state_actor.actor_index, strike.player);
            assert_eq!(entry.reward_actor, Some(credited));
            assert_eq!(world.state(11), Some(ObjectiveState::Activated));
        }

        assert!(
            world
                .stage_precision_strike_with_caller(11, None, None, Some(strike.player))
                .unwrap()
        );
        assert_eq!(world.script_actor(11), Some(strike.player));
        frame(&mut world, 11, 100, &BTreeMap::new()).unwrap();
        let entry = world.take_native_entries().pop().unwrap();
        assert_eq!(entry.delta.state_actor.actor_index, u16::MAX);
        assert_eq!(entry.reward_actor, Some(planter.player));
        assert_eq!(world.script_actor(11), Some(planter.player));
    }

    #[test]
    fn precision_has_one_completion_callback_and_inactive_entry_clears_the_attacker() {
        for (initial, credited) in [(0, None), (1, Some(4)), (5, None)] {
            let mut catalog = native_catalog();
            catalog["objects"][1]["initial_state"] = json!(if initial == 5 { 7 } else { initial });
            let mut world =
                ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap());
            if initial == 5 {
                let object = world.objects.get_mut(&11).unwrap();
                set_native_state(object, ObjectiveState::Repairing, 7, Duration::ZERO);
            }
            assert!(
                world
                    .stage_precision_strike_with_actor(
                        11,
                        None,
                        Some(PrecisionRewardActor::Attacker(4))
                    )
                    .unwrap()
            );
            let mut entries = Vec::new();
            for millis in 100..104 {
                frame(&mut world, 11, millis, &BTreeMap::new());
                entries.extend(world.take_native_entries());
            }
            let (completed, intermediate) = entries.split_last().unwrap();
            assert!(
                intermediate
                    .iter()
                    .all(|entry| entry.reward_actor.is_none())
            );
            assert_eq!(completed.delta.state_actor.state, ObjectiveState::Completed);
            assert_eq!(completed.delta.state_actor.actor_index, u16::MAX);
            assert_eq!(completed.reward_actor, credited);
        }
    }

    #[test]
    fn passive_precision_uses_the_same_qualified_completion_latch() {
        for (initial, credited) in [(0, None), (1, Some(4))] {
            let mut catalog = native_catalog();
            catalog["objects"][1]
                .as_object_mut()
                .unwrap()
                .remove("native_class");
            catalog["objects"][1]
                .as_object_mut()
                .unwrap()
                .remove("repair");
            catalog["objects"][1]
                .as_object_mut()
                .unwrap()
                .remove("demolition");
            catalog["objects"][1]["initial_state"] = json!(initial);
            let mut world =
                ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap());
            assert!(
                world
                    .stage_precision_strike_with_actor(
                        11,
                        None,
                        Some(PrecisionRewardActor::Attacker(4))
                    )
                    .unwrap()
            );
            let mut entries = Vec::new();
            world
                .advance_acquisition_precision_entries(Duration::from_millis(100), |entry, _| {
                    entries.push(entry);
                    Ok(())
                })
                .unwrap();
            let (completed, intermediate) = entries.split_last().unwrap();
            assert!(
                intermediate
                    .iter()
                    .all(|entry| entry.reward_actor.is_none())
            );
            assert_eq!(completed.delta.state_actor.state, ObjectiveState::Completed);
            assert_eq!(completed.reward_actor, credited);
        }
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

    fn timing_actor(team: u8) -> ObjectiveActor {
        ObjectiveActor {
            player: u16::from(team) + 7,
            team,
            generation: 1,
            position: CharacterPosition {
                x: 100.,
                y: 3.,
                z: 100.,
            },
        }
    }

    fn timing_request(
        actor: ObjectiveActor,
        kind: ObjectiveInteractionKind,
    ) -> ObjectiveInteraction {
        ObjectiveInteraction {
            target_object_id: 0x0c80_000a,
            actor_index: actor.player,
            trigger_index: 0,
            kind,
        }
    }

    fn timing_world(initial: ObjectiveState) -> ObjectiveWorld {
        let mut catalog = native_catalog();
        catalog["objects"][0]["interaction"]["activate_ms"] = json!(37);
        catalog["objects"][0]["interaction"]["deactivate_ms"] = json!(47);
        let mut world =
            ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap());
        use ObjectiveInteractionKind::{Activate, Deactivate};
        use ObjectiveState::{Activated, Completed, Deactivated, Retaken};
        let base_state = match initial {
            Activated => Deactivated,
            Retaken => Completed,
            state => state,
        };
        let object = world.objects.get_mut(&10).unwrap();
        object.state.state_actor.state = base_state;
        object.native.as_mut().unwrap().secure =
            Some(secure_hold(base_state, object.rule.as_ref().unwrap()));

        // Establish transient completion-window states through their actual holds.
        if matches!(initial, Activated | Retaken) {
            let (actor, kind) = match initial {
                Activated => (timing_actor(0), Activate),
                _ => (timing_actor(1), Deactivate),
            };
            let start = Duration::from_millis(60);
            assert!(
                world
                    .interact(timing_request(actor, kind), actor, start)
                    .accepted
            );
            world
                .commit_pending_native_interaction(10, start, actor)
                .unwrap();
            assert_eq!(
                frame(&mut world, 10, 97, &BTreeMap::from([(actor.player, actor)]))
                    .unwrap()
                    .state_actor
                    .state,
                initial
            );
        }
        world
    }

    #[test]
    fn action_timing_demolition_modifier_does_not_shorten_capture() {
        let mut world = timing_world(ObjectiveState::Deactivated);
        let actor = timing_actor(0);
        let now = Duration::from_millis(100);
        assert!(
            world
                .interact_with_demolition_modifier(
                    timing_request(actor, ObjectiveInteractionKind::Activate),
                    actor,
                    now,
                    -0.2,
                )
                .accepted
        );
        assert_eq!(
            world
                .commit_pending_native_interaction(10, now, actor)
                .unwrap()
                .expiration_time_ms,
            137
        );
    }

    #[test]
    fn action_timing_selects_native_entering_state_and_secure_slot() {
        use ObjectiveInteractionKind::{Activate, Deactivate};
        use ObjectiveState::{
            Activated, Activating, CancelRetaken, Completed, Deactivated, Deactivating, Retaken,
            Retaking,
        };
        let now = Duration::from_millis(100);
        for (initial, kind, team, expected_state, deadline) in [
            (Deactivated, Activate, 0, Activating, 146),
            (Activated, Deactivate, 1, Deactivating, 158),
            (Completed, Deactivate, 1, Retaking, 146),
            (Retaken, Activate, 0, CancelRetaken, 158),
        ] {
            let mut world = timing_world(initial);
            let actor = timing_actor(team);
            assert!(
                world
                    .interact_with_action_timing(
                        timing_request(actor, kind),
                        actor,
                        now,
                        ActionTimingModifiers {
                            secure_and_hold: 0.25,
                            demolition: f32::NAN
                        },
                    )
                    .accepted
            );
            let delta = world
                .commit_pending_native_interaction(10, now, actor)
                .unwrap();
            let time = if matches!(expected_state, Retaking | Activating) {
                delta.expiration_time_ms
            } else {
                delta.deactivation_time_ms
            };
            assert_eq!((delta.state_actor.state, time), (expected_state, deadline));
        }
    }

    #[test]
    fn action_timing_invalid_native_duration_leaves_no_staged_hold() {
        use ObjectiveInteractionKind::{Activate, Release};
        let mut world = timing_world(ObjectiveState::Deactivated);
        let actor = timing_actor(0);
        let now = Duration::from_millis(100);
        let invalid = ActionTimingModifiers {
            secure_and_hold: f32::NAN,
            demolition: f32::NAN,
        };
        let request = timing_request(actor, Activate);
        assert_eq!(
            world
                .interact_with_action_timing(request, actor, now, invalid)
                .reason,
            "invalid_objective_modifier"
        );
        assert!(
            world
                .commit_pending_native_interaction(10, now, actor)
                .is_none()
        );
        assert!(world.interact(request, actor, now).accepted);
        let delta = world
            .commit_pending_native_interaction(10, now, actor)
            .unwrap();
        assert_eq!(delta.expiration_time_ms, 137);

        let later = now + Duration::from_millis(5);
        assert!(
            !world
                .interact_with_action_timing(request, actor, later, invalid)
                .accepted
        );
        assert!(
            world
                .interact_with_action_timing(timing_request(actor, Release), actor, later, invalid,)
                .accepted
        );
        let delta = world
            .commit_pending_native_interaction(10, later, actor)
            .unwrap();
        assert_eq!(delta.state_actor.state, ObjectiveState::Deactivated);
    }

    #[test]
    fn action_timing_legacy_capture_honors_secure_modifier_without_restarting_duplicate() {
        let mut world = world();
        let actor = timing_actor(0);
        let now = Duration::from_millis(100);
        let request = timing_request(actor, ObjectiveInteractionKind::Activate);
        let outcome = world.interact_with_action_timing(
            request,
            actor,
            now,
            ActionTimingModifiers {
                secure_and_hold: -0.2,
                demolition: f32::NAN,
            },
        );
        assert_eq!(outcome.changed.unwrap().expiration_time_ms, 108);
        let later = now + Duration::from_millis(1);
        let duplicate = world.interact_with_action_timing(
            request,
            actor,
            later,
            ActionTimingModifiers {
                secure_and_hold: f32::NAN,
                demolition: 0.0,
            },
        );
        assert!(duplicate.accepted && duplicate.changed.is_none());
        assert_eq!(world.objects[&10].state.expiration_time_ms, 108);
    }

    #[test]
    fn action_timing_zero_keeps_exact_authored_legacy_deadline() {
        let mut world = world();
        world
            .objects
            .get_mut(&10)
            .unwrap()
            .rule
            .as_mut()
            .unwrap()
            .activate_ms = u32::MAX;
        let actor = timing_actor(0);
        let now = Duration::from_millis(100);
        assert!(
            world
                .interact(
                    timing_request(actor, ObjectiveInteractionKind::Activate),
                    actor,
                    now
                )
                .accepted
        );
        assert_eq!(
            world.objects[&10].operation.as_ref().unwrap().deadline,
            now + Duration::from_millis(u64::from(u32::MAX))
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
        assert_eq!(entries[0].reward_actor, None);
        assert_eq!(entries[1].reward_actor, Some(2));
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
        assert_eq!(entry.reward_actor, Some(attacker.player));
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

#[cfg(test)]
#[path = "objective_control_secure_script_tests.rs"]
mod secure_script_tests;
