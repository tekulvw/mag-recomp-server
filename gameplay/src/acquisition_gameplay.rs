//! Owned Acquisition objective state used inside the Engine's callback candidate.
//! The caller stages collision, character and round effects in the same
//! transaction and records the publication before installing any authority.
use crate::{
    acquisition_atomic::{EnteredState, Entry},
    acquisition_control::{Catalog, Command, ObservedState, Plan},
    acquisition_ground_runtime::{self as ground, Key, LifecycleEvent, QualifiedRespawn, Runtime},
    acquisition_objective_runtime::{self as assignment, Assignment, RefreshResult, State},
    acquisition_score_rate::{self as score_rate, Rates},
    replication::{ObjectiveControlDelta, ObjectiveState, ObjectiveStateActor},
};
use std::collections::{BTreeMap, BTreeSet};

/// Source-bound type-9 point and the existing Escort allocation. The supplied
/// pose includes the vehicle-specific placement offset exactly once.
#[derive(Clone, Copy, Debug)]
pub struct Fsp {
    pub vehicle: Key,
    pub source: QualifiedRespawn,
}

#[derive(Clone, Debug)]
pub struct Objective {
    pub state: State,
    pub owner_team: u8,
    pub activation_score_value: u16,
    pub current_attacker: u32,
    pub current_defender: u32,
    pub replication: ObjectiveControlDelta,
    pub fsp: Option<Fsp>,
}

impl Objective {
    pub fn object_id(&self) -> u32 {
        0x0c80_0000 | u32::from(self.replication.object_index)
    }

    fn observe(&self) -> ObservedState {
        match self.state.current {
            7 => ObservedState::Completed,
            1 => ObservedState::Deactivated,
            _ => ObservedState::Other,
        }
    }

    fn commit_state(&mut self, state: ObjectiveState, actor_index: u16, timestamp_ms: u32) {
        let old = self.replication.state_actor;
        self.state.current = state as i32;
        self.replication.state_actor = ObjectiveStateActor {
            state,
            actor_index,
            change_time_ms: if old.state == state {
                old.change_time_ms
            } else {
                timestamp_ms
            },
        };
    }
}

/// Source slots and order are preserved. A null slot is a real input.
#[derive(Clone, Copy, Debug)]
pub struct OrderSource {
    pub objective_token: Option<u32>,
    pub order: i32,
}

#[derive(Clone, Debug)]
pub struct Squad {
    pub team: u8,
    pub gate_value: u16,
    pub spawn_resolved: bool,
    pub normal: Option<Vec<OrderSource>>,
    pub frago: Vec<OrderSource>,
    pub manual_tag: u32,
    pub assignment: Assignment,
}

#[derive(Clone, Debug)]
pub struct Gameplay {
    controller: String,
    callback_tokens: BTreeSet<u32>,
    objectives: BTreeMap<u32, Objective>,
    squads: BTreeMap<u16, Squad>,
    target_versions: BTreeMap<u32, u64>,
}

/// Operations requiring Engine collision and character authority. The
/// consumer must execute each immediately against the shared owned candidate.
#[derive(Clone, Copy, Debug)]
pub enum Effect<'a> {
    Blockers(Key),
    ScoreDeduction {
        team: u8,
        /// Native signed-low16 operand. A positive value deducts points.
        amount: i16,
    },
    Volume {
        target: u32,
        arguments: [i32; 4],
        damage_type_symbol: &'a str,
        damage_source_symbol: &'a str,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct AssignmentUpdate {
    pub squad_index: u16,
    pub assignment: Assignment,
    pub result: RefreshResult,
}

#[derive(Clone, Debug)]
pub struct Publication {
    pub objective: ObjectiveControlDelta,
    pub fsp_active: Option<(u16, bool)>,
    pub lifecycle: Vec<LifecycleEvent>,
    pub assignments: Vec<AssignmentUpdate>,
    pub plan: Plan,
}

impl Gameplay {
    pub fn new(
        catalog: &Catalog,
        objectives: BTreeMap<u32, Objective>,
        squads: BTreeMap<u16, Squad>,
    ) -> Result<Self, &'static str> {
        let expected: BTreeSet<_> = catalog.objective_tokens().collect();
        let captures: BTreeMap<_, _> = catalog.capture_spawn_bindings().collect();
        let mut objective_indices = BTreeSet::new();
        let mut points = BTreeSet::new();
        let mut vehicles = BTreeSet::new();
        if !expected.iter().all(|token| objectives.contains_key(token))
            || objectives.iter().any(|(token, objective)| {
                objective.owner_team > 1
                    || objective.replication.object_index >= 128
                    || objective.replication.health_percent > 100
                    || objective.state.current != objective.replication.state_actor.state as i32
                    || !(0..=12).contains(&objective.state.target)
                    || !objective_indices.insert(objective.replication.object_index)
                    || match (captures.get(token), objective.fsp) {
                        (Some(&spawn_token), Some(fsp)) => {
                            fsp.source.spawn_point_token != spawn_token
                                || objective.activation_score_value != 0
                                || !points.insert(fsp.source.spawn_point_index)
                                || !vehicles.insert(fsp.vehicle.index)
                        }
                        (None, None) => false,
                        _ => true,
                    }
            })
        {
            return Err("invalid_acquisition_gameplay_objectives");
        }
        if squads.values().any(|squad| {
            squad.team > 1
                || squad
                    .normal
                    .iter()
                    .flatten()
                    .chain(&squad.frago)
                    .filter_map(|order| order.objective_token)
                    .any(|id| !objectives.contains_key(&id))
        }) {
            return Err("invalid_acquisition_gameplay_squads");
        }
        Ok(Self {
            controller: catalog.fingerprint().into(),
            callback_tokens: expected,
            target_versions: objectives.keys().map(|&token| (token, 0)).collect(),
            objectives,
            squads,
        })
    }

    pub fn objective(&self, token: u32) -> Option<&Objective> {
        self.objectives.get(&token)
    }

    pub fn token_for_index(&self, index: u16) -> Option<u32> {
        self.objectives()
            .find(|(_, obj)| obj.replication.object_index == index)
            .map(|(token, _)| token)
    }

    pub fn has_callback(&self, token: u32) -> bool {
        self.callback_tokens.contains(&token)
    }

    pub fn squad(&self, index: u16) -> Option<&Squad> {
        self.squads.get(&index)
    }

    /// Refresh inputs from current authoritative squad state. Objective-list
    /// slots remain source ordered, including nulls and unrecognized orders.
    pub fn squad_mut(&mut self, index: u16) -> Option<&mut Squad> {
        self.squads.get_mut(&index)
    }

    pub fn objectives(&self) -> impl Iterator<Item = (u32, &Objective)> {
        self.objectives
            .iter()
            .map(|(&token, objective)| (token, objective))
    }

    /// Pending native target IDs. The objective tick owns transition admission,
    /// timing and dispatch of any callback entry.
    pub fn pending_transitions(&self) -> impl Iterator<Item = (u32, i32)> + '_ {
        self.objectives()
            .filter(|(_, obj)| obj.state.current != obj.state.target)
            .map(|(token, obj)| (token, obj.state.target))
    }

    /// Record a target requested by the authoritative objective state machine.
    pub fn request_state(
        &mut self,
        token: u32,
        target: ObjectiveState,
    ) -> Result<(), &'static str> {
        let version = self
            .target_versions
            .get_mut(&token)
            .ok_or("unknown_acquisition_gameplay_objective")?;
        let next = version
            .checked_add(1)
            .ok_or("acquisition_target_version_exhausted")?;
        self.objectives
            .get_mut(&token)
            .ok_or("unknown_acquisition_gameplay_objective")?
            .state
            .target = target as i32;
        *version = next;
        Ok(())
    }

    /// Replacement-only request identity; scripts cannot reuse an old delivery witness.
    pub(crate) fn target_version(&self, token: u32) -> Option<u64> {
        self.target_versions.get(&token).copied()
    }

    /// Handle the forced type-9 ground death notification. The caller resolves
    /// the damage model's actor team, supplying native fallback team 0 when
    /// the actor does not resolve. State entry remains a later objective tick.
    pub fn request_forced_fsp_deactivation(
        &mut self,
        object_id: u32,
        actor_id: u32,
        actor_team: u8,
    ) -> Result<(), &'static str> {
        if actor_team > 1 {
            return Err("invalid_acquisition_fsp_actor_team");
        }
        let (&token, objective) = self
            .objectives
            .iter_mut()
            .find(|(_, objective)| objective.fsp.is_some() && objective.object_id() == object_id)
            .ok_or("unknown_acquisition_fsp_objective")?;
        if actor_team != objective.owner_team {
            objective.current_attacker = actor_id;
        } else {
            objective.current_defender = actor_id;
        }
        // GetIsCompleted includes Repairing in addition to Completed, unlike
        // the Lua graph, whose predicates read the literal current state.
        let version = self
            .target_versions
            .get_mut(&token)
            .ok_or("unknown_acquisition_gameplay_objective")?;
        *version = version
            .checked_add(1)
            .ok_or("acquisition_target_version_exhausted")?;
        objective.state.target = if matches!(objective.state.current, 5 | 7) {
            ObjectiveState::Deactivated as i32
        } else {
            ObjectiveState::Completed as i32
        };
        Ok(())
    }

    /// Commit an ordinary state entry. Hooked Completed and Deactivated entries
    /// use the callback path. Secondary SecureAndHold assignment objectives
    /// use this path at startup because they have no graph hooks.
    pub fn commit_state(
        &mut self,
        token: u32,
        state: ObjectiveState,
        actor_index: u16,
        timestamp_ms: u32,
    ) -> Result<ObjectiveControlDelta, &'static str> {
        if self.has_callback(token)
            && matches!(
                state,
                ObjectiveState::Completed | ObjectiveState::Deactivated
            )
        {
            return Err("acquisition_entry_requires_callback");
        }
        let objective = self
            .objectives
            .get_mut(&token)
            .ok_or("unknown_acquisition_gameplay_objective")?;
        objective.commit_state(state, actor_index, timestamp_ms);
        Ok(objective.replication)
    }

    /// Apply one entry to already isolated Engine candidates. If any step
    /// fails, the caller discards all candidates, including effects built by
    /// the closure. Match scope, entry admission and callback replay are owned
    /// by Engine. This does not award native medals, ribbons or objective XP.
    pub fn apply_entry(
        &mut self,
        catalog: &Catalog,
        entry: &Entry,
        ground: &mut Runtime,
        rates: &mut Rates,
        mut apply: impl FnMut(&mut Runtime, Effect<'_>) -> Result<(), &'static str>,
    ) -> Result<Publication, &'static str> {
        if self.controller != catalog.fingerprint() {
            return Err("acquisition_gameplay_controller_mismatch");
        }
        if !self.has_callback(entry.objective_token) {
            return Err("unknown_acquisition_callback_objective");
        }
        let objective = self
            .objectives
            .get_mut(&entry.objective_token)
            .ok_or("unknown_acquisition_gameplay_objective")?;
        let previous = objective.replication.state_actor.state;
        let state = match entry.state {
            EnteredState::Completed => ObjectiveState::Completed,
            EnteredState::Deactivated => ObjectiveState::Deactivated,
        };
        objective.state.current = state as i32;

        let mut lifecycle = Vec::new();
        let objective_id = objective.object_id();
        let fsp_active = if let Some(fsp) = &mut objective.fsp {
            let vehicle = ground
                .get(fsp.vehicle.index)
                .ok_or("acquisition_fsp_vehicle_missing")?;
            if vehicle.key != fsp.vehicle
                || vehicle.source_spawn_type != Some(9)
                || vehicle.source_spawn_point_index != fsp.source.spawn_point_index
                || vehicle.source_spawn_point_token != fsp.source.spawn_point_token
            {
                return Err("acquisition_fsp_vehicle_mismatch");
            }
            let active = state == ObjectiveState::Completed;
            fsp.source.active = active;
            if active {
                ground.enable_cycle(fsp.vehicle, objective_id)?;
                lifecycle = ground.respawn_existing(fsp.vehicle, fsp.source, false)?;
                for event in &lifecycle {
                    if event.effect == ground::Effect::KillSpawnBlockers {
                        apply(ground, Effect::Blockers(event.vehicle))?;
                    }
                }
            } else {
                ground.disable_cycle(fsp.vehicle)?;
            }
            Some((fsp.source.spawn_point_index, active))
        } else {
            None
        };

        if entry.state == EnteredState::Completed
            && objective.fsp.is_none()
            && entry.actor_index != u16::MAX
            && objective.activation_score_value != 0
        {
            apply(
                ground,
                Effect::ScoreDeduction {
                    team: objective.owner_team,
                    amount: objective.activation_score_value as i16,
                },
            )?;
        }
        objective.commit_state(state, entry.actor_index, entry.timestamp_ms);

        match entry.state {
            EnteredState::Completed => {
                objective.current_attacker = character_object_id(entry.actor_index);
                objective.current_defender = u32::MAX;
                objective.replication.health_percent = 0;
                objective.replication.deactivation_time_ms = 0;
            }
            EnteredState::Deactivated => {
                objective.current_defender = character_object_id(entry.actor_index);
                objective.current_attacker = u32::MAX;
                objective.replication.health_percent = 100;
                objective.replication.expiration_time_ms = 0;
                if objective.fsp.is_none() || previous != ObjectiveState::Deactivating {
                    objective.replication.deactivation_time_ms = 0;
                }
            }
        }
        let objective = objective.replication;

        // Snapshot only committed current states. Target requests from one
        // command cannot satisfy another group's predicate in this callback.
        let observations = self
            .objectives()
            .map(|(token, obj)| (token, obj.observe()))
            .collect();
        let plan = catalog.plan(&observations)?;
        for command in &plan.commands {
            match command {
                Command::Objective { target, operation } => {
                    let objective = self
                        .objectives
                        .get_mut(target)
                        .ok_or("unknown_acquisition_gameplay_objective")?;
                    if let Some(requested) = objective.state.request(*operation) {
                        let version = self
                            .target_versions
                            .get_mut(target)
                            .ok_or("unknown_acquisition_gameplay_objective")?;
                        *version = version
                            .checked_add(1)
                            .ok_or("acquisition_target_version_exhausted")?;
                        objective.state.target = requested;
                    }
                }
                Command::DamageVolume {
                    target,
                    arguments,
                    damage_type_symbol,
                    damage_source_symbol,
                } => {
                    apply(
                        ground,
                        Effect::Volume {
                            target: *target,
                            arguments: *arguments,
                            damage_type_symbol,
                            damage_source_symbol,
                        },
                    )?;
                }
            }
        }
        for call in &plan.unbound_calls {
            rates.configure(score_rate::Request::from_call(call)?, entry.timestamp_ms)?;
        }

        Ok(Publication {
            objective,
            fsp_active,
            lifecycle,
            assignments: self.refresh_assignments()?,
            plan,
        })
    }

    /// Update the stored manual override and rerun this squad's authored refresh.
    pub fn set_manual_tag(
        &mut self,
        squad_index: u16,
        target: u32,
    ) -> Result<Assignment, &'static str> {
        let squad = self
            .squads
            .get_mut(&squad_index)
            .ok_or("unknown_acquisition_squad")?;
        Self::refresh_assignment(&self.objectives, squad, target)?;
        squad.manual_tag = target;
        Ok(squad.assignment)
    }

    pub fn refresh_assignments(&mut self) -> Result<Vec<AssignmentUpdate>, &'static str> {
        let mut updates = Vec::new();
        for (&squad_index, squad) in &mut self.squads {
            if let Some(result) =
                Self::refresh_assignment(&self.objectives, squad, squad.manual_tag)?
            {
                updates.push(AssignmentUpdate {
                    squad_index,
                    assignment: squad.assignment,
                    result,
                });
            }
        }
        Ok(updates)
    }

    fn refresh_assignment(
        objectives: &BTreeMap<u32, Objective>,
        squad: &mut Squad,
        manual_tag: u32,
    ) -> Result<Option<assignment::RefreshResult>, &'static str> {
        let Some(normal) = &squad.normal else {
            return Ok(None);
        };
        if squad.gate_value == 3 || !squad.spawn_resolved {
            return Ok(None);
        }
        let resolve = |orders: &[OrderSource]| {
            orders
                .iter()
                .map(|order| {
                    let objective = order
                        .objective_token
                        .map(|token| {
                            let objective = objectives
                                .get(&token)
                                .ok_or("unknown_acquisition_gameplay_order_objective")?;
                            assignment::Objective::new(
                                objective.object_id(),
                                objective.state.current,
                                squad.team != objective.owner_team,
                            )
                        })
                        .transpose()?;
                    Ok(assignment::Order {
                        objective,
                        order: order.order,
                    })
                })
                .collect::<Result<Vec<_>, &'static str>>()
        };
        let normal = resolve(normal)?;
        let frago = resolve(&squad.frago)?;
        Ok(Some(squad.assignment.refresh(assignment::Refresh {
            gate_value: squad.gate_value,
            spawn_resolved: squad.spawn_resolved,
            normal: Some(&normal),
            frago: &frago,
            manual_tag,
        })))
    }
}

fn character_object_id(actor_index: u16) -> u32 {
    if actor_index == u16::MAX {
        u32::MAX
    } else {
        0x0380_0000 | u32::from(actor_index)
    }
}
