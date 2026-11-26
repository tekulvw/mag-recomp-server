//! Detached objective/catalog/round transaction for the original Domination entry order.
//! The owning engine supplies qualified per-class and external-world operations.
use crate::{
    acquisition_round::Round,
    domination_callbacks::{Context, InvocationEffects, Queue},
    domination_control::{self, Ability, Caller, Effect, Event, State, Transition},
    domination_fsp::{VehicleRequest, stage_spawn_effects},
    domination_fsp_bindings::Bindings,
    domination_objective_runtime::Actor,
    objective_catalog::{NativeClass, ObjectiveCatalog},
    objective_control::{ActionTimingModifiers, NativeEntry, ObjectiveActor, ObjectiveWorld},
    protocol::{ObjectiveInteraction, ObjectiveInteractionKind},
    replication::{
        ObjectiveControlDelta, ObjectiveControlSnapshot, ObjectiveSequenceDelta, ObjectiveState,
    },
    spawn_catalog::SpawnCatalog,
    wire_profile::WireProfile,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, num::NonZeroUsize, sync::Arc, time::Duration};

#[derive(Deserialize)]
struct StartupFile {
    schema: String,
    status: String,
    source: StartupSource,
    maps: Vec<StartupMap>,
}

#[derive(Deserialize)]
struct StartupSource {
    authored_metadata_sha256: String,
    native_program_sha256: String,
}

#[derive(Deserialize)]
struct LevelSource {
    archive_sha256: String,
    clv_sha256: String,
}

#[derive(Deserialize)]
struct StartupMap {
    map_name: String,
    source: LevelSource,
    objectives: Vec<StartupRow>,
}

#[derive(Deserialize)]
struct StartupRow {
    token: String,
    client_network_index: u16,
    property_class: String,
    property_active: Active,
    saved_desired_state: u8,
    transition_request: Option<u8>,
    current_network_state_after_mission_start: Option<u8>,
    sim_network_index: Option<u16>,
}

#[derive(Deserialize)]
struct Active {
    value: bool,
}

#[derive(Deserialize)]
struct SpawnSource {
    level_archive_sha256: String,
    level_clv_sha256: String,
}

#[derive(Deserialize)]
struct SimSchedule {
    schema: String,
    status: String,
    retail_elf_sha256: String,
    metadata_sha256: String,
    client_source_bindings_sha256: String,
    completed_predicate: [u8; 2],
    maps: Vec<SimScheduleMap>,
}

#[derive(Deserialize)]
struct SimScheduleMap {
    map: String,
    ordered_tokens: Vec<String>,
}

#[derive(Clone)]
pub struct Setup {
    catalog: Arc<ObjectiveCatalog>,
    map: Arc<domination_control::Map>,
    fsp: Bindings,
    spawn_sha256: String,
    tokens_by_index: BTreeMap<u16, u32>,
    indices_by_token: BTreeMap<u32, u16>,
    classes_by_index: BTreeMap<u16, NativeClass>,
    startup: Vec<(u16, ObjectiveState)>,
    update_order: Vec<u16>,
}

/// Decodes the standalone original MissionStart receipt. An ordinary initial
/// load catalog has no runtime authority and cannot supply this setup.
impl Setup {
    pub(crate) fn category_zero_indices(&self) -> Result<[u16; 32], &'static str> {
        self.fsp.category_zero_indices()
    }

    pub fn from_sources(
        catalog: Arc<ObjectiveCatalog>,
        spawns: &SpawnCatalog,
        controller: &domination_control::Catalog,
        startup_raw: &[u8],
        schedule_raw: &[u8],
        fsp_raw: &[u8],
    ) -> Result<Self, &'static str> {
        if startup_raw.len() > 2 * 1024 * 1024
            || schedule_raw.len() > 64 * 1024
            || !catalog.is_domination_gameplay()
        {
            return Err("domination_gameplay_requires_source_catalog");
        }
        catalog.validate_match(spawns.map_name(), spawns.match_profile())?;
        let map = controller
            .map(spawns.map_name())
            .ok_or("domination_gameplay_missing_controller_map")?;
        let spawn_source: SpawnSource = serde_json::from_str(spawns.authored_source())
            .map_err(|_| "domination_gameplay_missing_level_source")?;
        let startup: StartupFile = serde_json::from_slice(startup_raw)
            .map_err(|_| "domination_gameplay_invalid_startup")?;
        let source = catalog.source();
        if startup.schema != "mag_domination_mission_start_requests_v1"
            || startup.status != "ORIGINAL_AUTHORED_AND_STATIC_NATIVE_STARTUP_REQUESTS_BOUND"
            || source.startup_proof_sha256 != format!("{:x}", Sha256::digest(startup_raw))
            || startup.source.native_program_sha256 != source.source_image_sha256
            || startup.source.authored_metadata_sha256 != source.authored_catalog_sha256
            || controller.metadata_sha256 != source.authored_catalog_sha256
            || map.level_archive_sha256 != spawn_source.level_archive_sha256
            || map.level_clv_sha256 != spawn_source.level_clv_sha256
        {
            return Err("domination_gameplay_source_mismatch");
        }
        let mut rows = startup
            .maps
            .into_iter()
            .filter(|row| row.map_name == spawns.map_name());
        let level = rows
            .next()
            .ok_or("domination_gameplay_missing_startup_map")?;
        if rows.next().is_some()
            || level.source.archive_sha256 != map.level_archive_sha256
            || level.source.clv_sha256 != map.level_clv_sha256
            || level.objectives.len() != catalog.object_count()
        {
            return Err("domination_gameplay_startup_inventory_mismatch");
        }
        let fsp = Bindings::from_json(fsp_raw, spawns, &catalog)?;
        let indices_by_token: BTreeMap<_, _> = catalog.bound_objects().collect();
        let tokens_by_index = indices_by_token
            .iter()
            .map(|(&token, &index)| (index, token))
            .collect();
        if map
            .objective_tokens()
            .any(|token| !indices_by_token.contains_key(&token))
        {
            return Err("domination_gameplay_unbound_controller_objective");
        }
        let mut classes: BTreeMap<_, _> = catalog
            .objects
            .iter()
            .map(|object| {
                (
                    object.object_index,
                    (object.native_class, object.initial_state),
                )
            })
            .collect();
        let update_order = sim_order(schedule_raw, &catalog, spawns.map_name(), &indices_by_token)?;
        let mut qualified_classes = BTreeMap::new();
        let mut requests = Vec::with_capacity(level.objectives.len());
        for row in level.objectives {
            let objective_token = source_token(&row.token)?;
            let index = row.client_network_index;
            let class = match row.property_class.as_str() {
                "ObjectiveSecureAndHoldProperties" => NativeClass::SecureHold,
                "ObjectiveDemolitionStructuralProperties" => NativeClass::Structural,
                _ => return Err("domination_gameplay_unknown_startup_class"),
            };
            if indices_by_token.get(&objective_token) != Some(&index)
                || classes.remove(&index) != Some((Some(class), 0))
                || row.current_network_state_after_mission_start.is_some()
                || row.sim_network_index.is_some()
                || row.transition_request
                    != row.property_active.value.then_some(row.saved_desired_state)
            {
                return Err("domination_gameplay_startup_binding_mismatch");
            }
            qualified_classes.insert(index, class);
            if let Some(desired) = row.transition_request {
                requests.push((
                    index,
                    match desired {
                        0 => ObjectiveState::Inactive,
                        1 => ObjectiveState::Deactivated,
                        7 => ObjectiveState::Completed,
                        9 => ObjectiveState::Locked,
                        _ => return Err("domination_gameplay_unsupported_startup_state"),
                    },
                ));
            }
        }
        Ok(Self {
            catalog,
            map: Arc::new(map.clone()),
            fsp,
            spawn_sha256: spawns
                .source_sha256()
                .ok_or("domination_gameplay_missing_spawn_digest")?
                .to_owned(),
            tokens_by_index,
            indices_by_token,
            classes_by_index: qualified_classes,
            startup: requests,
            update_order,
        })
    }
}

fn source_token(token: &str) -> Result<u32, &'static str> {
    token
        .strip_prefix("0x")
        .filter(|digits| digits.len() == 8)
        .and_then(|digits| u32::from_str_radix(digits, 16).ok())
        .filter(|token| *token != 0)
        .ok_or("domination_gameplay_invalid_source_token")
}

fn sim_order(
    raw: &[u8],
    catalog: &ObjectiveCatalog,
    map: &str,
    client: &BTreeMap<u32, u16>,
) -> Result<Vec<u16>, &'static str> {
    let schedule: SimSchedule =
        serde_json::from_slice(raw).map_err(|_| "domination_gameplay_invalid_sim_schedule")?;
    let source = catalog.source();
    if schedule.schema != "mag_domination_structural_sim_schedule_v1"
        || schedule.status != "RETAIL_SIM_SOURCE_ORDER_AND_COMPLETION_PREDICATE_BOUND"
        || schedule.retail_elf_sha256 != source.source_image_sha256
        || schedule.metadata_sha256 != source.authored_catalog_sha256
        || schedule.client_source_bindings_sha256 != source.binding_capture_sha256
        || schedule.completed_predicate != [5, 7]
    {
        return Err("domination_gameplay_sim_source_mismatch");
    }
    let mut matches = schedule.maps.into_iter().filter(|row| row.map == map);
    let row = matches
        .next()
        .ok_or("domination_gameplay_missing_sim_schedule")?;
    if row.ordered_tokens.len() != client.len() || matches.next().is_some() {
        return Err("domination_gameplay_invalid_sim_schedule");
    }
    row.ordered_tokens
        .iter()
        .enumerate()
        .map(|(sim_slot, token)| {
            let client_slot = client
                .get(&source_token(token)?)
                .copied()
                .ok_or("domination_gameplay_unbound_sim_objective")?;
            if usize::from(client_slot) != sim_slot {
                return Err("domination_gameplay_sim_client_slot_mismatch");
            }
            Ok(client_slot)
        })
        .collect()
}

/// All methods prepare a detached provider candidate. Implementations must
/// resolve original runtime identities, collision, effects and assignment rules
/// or return an error. None of these methods may publish to the live engine.
pub trait ExternalEffects {
    fn prepare_class_prefix(
        &mut self,
        entry: NativeEntry,
        world: &ObjectiveWorld,
        spawns: &SpawnCatalog,
    ) -> Result<(), &'static str>;

    fn prepare_vehicles(&mut self, requests: &[VehicleRequest]) -> Result<(), &'static str>;

    fn prepare_class_entry(
        &mut self,
        entry: NativeEntry,
        world: &ObjectiveWorld,
        spawns: &SpawnCatalog,
    ) -> Result<(), &'static str>;

    fn prepare_explosion(
        &mut self,
        entry: NativeEntry,
        world: &ObjectiveWorld,
    ) -> Result<(), &'static str>;

    fn prepare_structural_update_tail(
        &mut self,
        _index: u16,
        _updated_state: ObjectiveState,
    ) -> Result<(), &'static str> {
        Ok(())
    }

    fn prepare_ability(
        &mut self,
        context: Context,
        platoon_token: u32,
        ability: Ability,
        enabled: bool,
    ) -> Result<(), &'static str>;

    fn callback_actor(&self, original_object_id: u32) -> Result<Option<Actor>, &'static str>;

    /// Refresh assignments through the native ordered squad vectors and
    /// qualified eligibility/spawn context.
    fn refresh_assignments(
        &mut self,
        entry: NativeEntry,
        catalog: &ObjectiveCatalog,
        world: &ObjectiveWorld,
        spawns: &SpawnCatalog,
    ) -> Result<(), &'static str>;

    /// After assignments, finish the native sequence selection for this entry.
    /// Return no update only for an independently qualified leave-unchanged ordinal.
    fn prepare_sequence(
        &mut self,
        entry: NativeEntry,
        objective_token: u32,
        game_time_ms: u32,
        world: &ObjectiveWorld,
    ) -> Result<Option<ObjectiveSequenceDelta>, &'static str>;
}

#[derive(Clone)]
pub struct Runtime {
    setup: Arc<Setup>,
    callbacks: Queue,
    started: bool,
}

#[must_use = "record and install the complete Domination candidate atomically"]
pub struct Candidate<E> {
    pub runtime: Runtime,
    pub world: ObjectiveWorld,
    pub spawns: SpawnCatalog,
    pub round: Round,
    pub effects: E,
    pub deltas: Vec<ObjectiveControlDelta>,
    pub sequences: Vec<ObjectiveSequenceDelta>,
}

pub struct Frame<'a> {
    pub now: Duration,
    pub world: &'a ObjectiveWorld,
    pub spawns: &'a SpawnCatalog,
    pub round: &'a Round,
}

pub struct PlayerInteraction {
    pub request: ObjectiveInteraction,
    pub actor: ObjectiveActor,
    /// Actor's DemolitionTime modifier for arm and defuse.
    pub timer_modifier: f32,
}

#[must_use = "record the reply and accepted effects before installing the transaction"]
pub struct InteractionCandidate<E> {
    pub candidate: Candidate<E>,
    pub reply: ObjectiveInteraction,
    pub reason: &'static str,
}

impl Runtime {
    pub(crate) fn category_zero_indices(&self) -> Result<[u16; 32], &'static str> {
        self.setup.category_zero_indices()
    }

    /// What completing each structural objective changes, for bot planning.
    pub(crate) fn structure_hints(&self) -> BTreeMap<u16, crate::bot_demolition::Hint> {
        use crate::domination_fsp::{FspType, SquadRouting};
        let mut hints: BTreeMap<u16, crate::bot_demolition::Hint> = BTreeMap::new();
        for (index, point, team, fsp_type, routing) in self.setup.fsp.linked_spawns() {
            let hint = hints.entry(index).or_default();
            match (fsp_type, routing) {
                (FspType::ActiveWhenIncomplete, SquadRouting::Enabled) => {
                    hint.closes_spawn_of = Some(team as u8);
                    hint.spawn_point = Some(point);
                }
                (FspType::ActiveWhenIncomplete, SquadRouting::Disabled) => {
                    hint.closes_vehicle_spawn_of = Some(team as u8)
                }
                (FspType::ActiveWhenComplete, _) => hint.opens_spawn_of = Some(team as u8),
            }
        }
        for (&index, &token) in &self.setup.tokens_by_index {
            if self.setup.classes_by_index.get(&index) == Some(&NativeClass::Structural)
                && self.setup.map.toggles_abilities(token, Event::Completed)
            {
                hints.entry(index).or_default().toggles_abilities = true;
            }
        }
        hints
    }

    pub(crate) fn is_started(&self) -> bool {
        self.started
    }

    pub(crate) fn reset(&mut self) {
        self.started = false;
        self.callbacks.clear();
    }

    pub fn new(setup: Arc<Setup>, callback_capacity: NonZeroUsize) -> Self {
        let callbacks = Queue::new(setup.map.clone(), callback_capacity);
        Self {
            setup,
            callbacks,
            started: false,
        }
    }

    fn candidate<E>(&self, frame: &Frame<'_>, effects: E) -> Result<Candidate<E>, &'static str> {
        let mut candidate = self.candidate_preserving_entries(frame, effects)?;
        if !candidate.world.take_native_entries().is_empty() {
            return Err("domination_gameplay_unconsumed_native_entries");
        }
        Ok(candidate)
    }

    fn candidate_preserving_entries<E>(
        &self,
        frame: &Frame<'_>,
        effects: E,
    ) -> Result<Candidate<E>, &'static str> {
        if frame.spawns.source_sha256() != Some(self.setup.spawn_sha256.as_str()) {
            return Err("domination_gameplay_runtime_spawn_mismatch");
        }
        if frame
            .world
            .native_object_indices()
            .ne(self.setup.tokens_by_index.keys().copied())
            || self
                .setup
                .classes_by_index
                .iter()
                .any(|(&index, &class)| frame.world.native_class(index) != Some(class))
        {
            return Err("domination_gameplay_runtime_objective_mismatch");
        }
        Ok(Candidate {
            runtime: self.clone(),
            world: frame.world.clone(),
            spawns: frame.spawns.clone(),
            round: frame.round.clone(),
            effects,
            deltas: Vec::new(),
            sequences: Vec::new(),
        })
    }

    /// Stage MissionStart requests as desired states only. Ordinary object
    /// processing commits each transition before dispatching its callback.
    pub fn stage_mission_start<E>(
        &self,
        frame: Frame<'_>,
        effects: E,
    ) -> Result<Candidate<E>, &'static str> {
        let mut next = self.candidate(&frame, effects)?;
        if !self.started {
            for &(index, state) in &self.setup.startup {
                next.world.stage_native_target(index, state)?;
            }
            next.runtime.started = true;
        }
        Ok(next)
    }

    /// Advance original active Sim family-12 slots using the independently
    /// qualified schedule and verified per-token client-slot equivalence.
    pub fn stage_objective_update<E: ExternalEffects>(
        &self,
        frame: Frame<'_>,
        actors: &BTreeMap<u16, ObjectiveActor>,
        effects: E,
    ) -> Result<Candidate<E>, &'static str> {
        if !self.started {
            return Err("domination_gameplay_not_started");
        }
        let mut next = self.candidate(&frame, effects)?;
        next.world.begin_native_tick(frame.now);
        for &index in &self.setup.update_order {
            let state_before_update = next.world.state(index);
            let mut published = None;
            let mut transitions = 0;
            loop {
                let delta = next.world.tick_native_object(index, frame.now, actors);
                let progressed = delta.is_some() || next.world.has_native_entries();
                if let Some(delta) = delta {
                    published = Some(delta);
                }
                // The structural state machine queues each entry before
                // continuing its desired-state chain in this same update.
                for entry in next.world.take_native_entries() {
                    next.prepare_entry(entry, frame.now.as_millis() as u32)?;
                }
                if !progressed
                    || !(next.world.has_precision_transition(index)
                        || next.world.has_script_transition(index))
                {
                    break;
                }
                transitions += 1;
                if transitions >= 256 {
                    return Err("native_objective_transition_limit");
                }
            }
            next.deltas.extend(published);
            // The original Structural Completed/Repairing update can transition
            // first, then tail-call sustained damage for the state that ran.
            if next.world.native_class(index) == Some(NativeClass::Structural)
                && let Some(state @ (ObjectiveState::Completed | ObjectiveState::Repairing)) =
                    state_before_update
            {
                next.effects.prepare_structural_update_tail(index, state)?;
            }
        }
        Ok(next)
    }

    /// Publish each synchronous strike-triggered activation through its ordinary
    /// entry effects before the next objective tick.
    pub fn stage_structure_damage_entries<E: ExternalEffects>(
        &self,
        frame: Frame<'_>,
        effects: E,
    ) -> Result<Candidate<E>, &'static str> {
        if !self.started {
            return Err("domination_gameplay_not_started");
        }
        let mut next = self.candidate_preserving_entries(&frame, effects)?;
        for entry in next.world.take_native_entries() {
            if entry.class != NativeClass::Structural
                || !matches!(
                    entry.previous,
                    ObjectiveState::Deactivated | ObjectiveState::Repaired
                )
                || entry.delta.state_actor.state != ObjectiveState::Activating
                || entry.actor_generation.is_none()
                || entry.active_trigger_index.is_none()
            {
                return Err("domination_gameplay_unexpected_structure_damage_entry");
            }
            next.deltas.push(entry.delta);
            next.prepare_entry(entry, frame.now.as_millis() as u32)?;
        }
        Ok(next)
    }

    /// Stage a player's synchronous START/RELEASE and its ACK/NAK together.
    /// The normal event owner must record the reply and complete candidate
    /// before installing any authority. Other objectives do not tick here.
    pub fn stage_interaction<E: ExternalEffects>(
        &self,
        frame: Frame<'_>,
        input: PlayerInteraction,
        effects: E,
    ) -> Result<InteractionCandidate<E>, &'static str> {
        self.stage_interaction_with_timing(
            frame,
            input.request,
            input.actor,
            ActionTimingModifiers {
                demolition: input.timer_modifier,
                ..Default::default()
            },
            effects,
        )
    }

    pub(crate) fn stage_interaction_with_timing<E: ExternalEffects>(
        &self,
        frame: Frame<'_>,
        request: ObjectiveInteraction,
        actor: ObjectiveActor,
        timings: ActionTimingModifiers,
        effects: E,
    ) -> Result<InteractionCandidate<E>, &'static str> {
        if !self.started {
            return Err("domination_gameplay_not_started");
        }
        let mut candidate = self.candidate(&frame, effects)?;
        let outcome = candidate
            .world
            .interact_with_action_timing(request, actor, frame.now, timings);
        if outcome.accepted {
            candidate
                .deltas
                .extend(candidate.world.commit_pending_native_interaction(
                    request.target_object_id as u16,
                    frame.now,
                    actor,
                ));
            for entry in candidate.world.take_native_entries() {
                candidate.prepare_entry(entry, frame.now.as_millis() as u32)?;
            }
        }
        let reply = ObjectiveInteraction {
            target_object_id: request.target_object_id,
            actor_index: actor.player,
            trigger_index: outcome.reply_trigger_index,
            kind: if outcome.accepted {
                ObjectiveInteractionKind::Ack
            } else {
                ObjectiveInteractionKind::Nak
            },
        };
        Ok(InteractionCandidate {
            candidate,
            reply,
            reason: outcome.reason,
        })
    }

    pub(crate) fn stage_repair(
        &self,
        frame: Frame<'_>,
        target: u32,
        authority: crate::domination_repair_runtime::Authority,
    ) -> Result<(Candidate<()>, crate::domination_repair_runtime::Submission), &'static str> {
        if !self.started {
            return Err("domination_gameplay_not_started");
        }
        let mut candidate = self.candidate(&frame, ())?;
        let submission = candidate.world.stage_repair_packet(target, authority)?;
        Ok((candidate, submission))
    }

    /// An actual ScriptObjectExecutor update samples the game clock once.
    /// The caller controls its placement relative to the Sim objective update.
    pub fn stage_callback_update<E: ExternalEffects>(
        &self,
        frame: Frame<'_>,
        effects: E,
    ) -> Result<Candidate<E>, &'static str> {
        if !self.started {
            return Err("domination_gameplay_not_started");
        }
        let mut next = self.candidate(&frame, effects)?;
        let game_time = frame.now.as_millis() as u32;
        loop {
            let current = next.current();
            let applied =
                next.runtime
                    .callbacks
                    .consume_next_due(game_time, &current, |batch| {
                        apply_effects(
                            &next.runtime.setup,
                            &mut next.world,
                            &mut next.round,
                            &mut next.effects,
                            game_time,
                            batch,
                        )
                    })?;
            if applied.is_none() {
                return Ok(next);
            }
        }
    }
}

fn controller_state(state: ObjectiveState) -> State {
    match state {
        ObjectiveState::Completed => State::Completed,
        ObjectiveState::Deactivated => State::Deactivated,
        ObjectiveState::Locked => State::Locked,
        _ => State::Other,
    }
}

impl<E> Candidate<E> {
    /// Split a candidate from a provider that borrows the staged Engine. The
    /// caller retains its prepared operations and can finish validation before
    /// relinquishing that borrow.
    pub fn detach_effects(self) -> (Candidate<()>, E) {
        let Self {
            runtime,
            world,
            spawns,
            round,
            effects,
            deltas,
            sequences,
        } = self;
        (
            Candidate {
                runtime,
                world,
                spawns,
                round,
                effects: (),
                deltas,
                sequences,
            },
            effects,
        )
    }

    fn objective_packets(
        &self,
        timestamp_ms: u32,
        profile: WireProfile,
    ) -> Result<Vec<Vec<u8>>, &'static str> {
        // Bound complete packets under the existing 1397-byte replication
        // budget. Worst case is 216 bits per object with both sequence fields.
        const OBJECTIVES_PER_PACKET: usize = 48;
        if self
            .deltas
            .windows(2)
            .any(|pair| pair[0].object_index >= pair[1].object_index)
        {
            return Err("invalid_objective_control_delta");
        }
        if self
            .sequences
            .windows(2)
            .any(|pair| pair[0].object_index >= pair[1].object_index)
        {
            return Err("invalid_objective_sequence_links");
        }

        // Same-state entries can select playback without changing control fields.
        // Keep each changed object's sequence with its control row.
        let (linked, sequence_only): (Vec<_>, Vec<_>) =
            self.sequences.iter().copied().partition(|row| {
                self.deltas
                    .binary_search_by_key(&row.object_index, |delta| delta.object_index)
                    .is_ok()
            });
        let mut packets = self
            .deltas
            .chunks(OBJECTIVES_PER_PACKET)
            .map(|entries| {
                let first = entries.first().unwrap().object_index;
                let last = entries.last().unwrap().object_index;
                let start = linked.partition_point(|row| row.object_index < first);
                let end = linked.partition_point(|row| row.object_index <= last);
                ObjectiveControlSnapshot {
                    sequence: 1,
                    timestamp_ms,
                    objectives: entries.to_vec(),
                }
                .encode_with_sequences_with_profile(&linked[start..end], profile)
            })
            .collect::<Result<Vec<_>, _>>()?;
        packets.extend(ObjectiveSequenceDelta::encode_batches(
            &sequence_only,
            timestamp_ms,
            profile,
        )?);
        Ok(packets)
    }

    /// Encode one control snapshot with any linked playback changes.
    pub fn objective_packet(
        &self,
        sequence: u32,
        game_time_ms: u32,
        profile: WireProfile,
    ) -> Result<Option<Vec<u8>>, &'static str> {
        if self.deltas.is_empty() && self.sequences.is_empty() {
            return Ok(None);
        }
        ObjectiveControlSnapshot {
            sequence,
            timestamp_ms: game_time_ms,
            objectives: self.deltas.clone(),
        }
        .encode_with_sequences_with_profile(&self.sequences, profile)
        .map(Some)
    }
}

impl<E: ExternalEffects> Candidate<E> {
    fn current(&self) -> BTreeMap<u32, State> {
        self.runtime
            .setup
            .indices_by_token
            .iter()
            .filter_map(|(&token, &index)| {
                self.world
                    .state(index)
                    .map(|value| (token, controller_state(value)))
            })
            .collect()
    }

    fn prepare_entry(&mut self, entry: NativeEntry, now: u32) -> Result<(), &'static str> {
        let setup = &self.runtime.setup;
        let token = *setup
            .tokens_by_index
            .get(&entry.delta.object_index)
            .ok_or("domination_gameplay_unbound_entry")?;
        let state = entry.delta.state_actor.state;
        let event = match state {
            ObjectiveState::Completed => Some(Event::Completed),
            ObjectiveState::Deactivated => Some(Event::Deactivated),
            ObjectiveState::Repaired => Some(Event::Repaired),
            _ => None,
        };
        self.effects
            .prepare_class_prefix(entry, &self.world, &self.spawns)?;
        if let Some(event) = event {
            // Both bound retail classes use Objective::GetIsCompleted
            // (virtual +0x84) for Completed7 or Repairing5.
            let is_completed =
                matches!(state, ObjectiveState::Completed | ObjectiveState::Repairing);
            let candidate = setup.fsp.plan_entry(token, state, is_completed)?;
            (self.spawns, ()) =
                stage_spawn_effects(&self.spawns, &candidate)?.complete(|requests| {
                    if requests.is_empty() {
                        Ok(())
                    } else {
                        self.effects.prepare_vehicles(requests)
                    }
                })?;
            self.effects
                .prepare_class_entry(entry, &self.world, &self.spawns)?;
            if entry.requires_authored_explosion() {
                self.effects.prepare_explosion(entry, &self.world)?;
            }
            // Lua Caller is the source objective identity. Its current actor is
            // resolved by each setter invocation, including delayed callbacks.
            let caller_id = 0x0c80_0000 | u32::from(entry.delta.object_index);
            let current = self.current();
            self.runtime.callbacks.execute_event(
                Transition {
                    objective_token: token,
                    event,
                },
                caller_id,
                now,
                &current,
                |batch| {
                    apply_effects(
                        setup,
                        &mut self.world,
                        &mut self.round,
                        &mut self.effects,
                        now,
                        batch,
                    )
                },
            )?;
        } else {
            self.effects
                .prepare_class_entry(entry, &self.world, &self.spawns)?;
        }
        self.effects
            .refresh_assignments(entry, &setup.catalog, &self.world, &self.spawns)?;
        if let Some(delta) = self
            .effects
            .prepare_sequence(entry, token, now, &self.world)?
        {
            if delta.object_index != entry.delta.object_index || delta.sequence_index < -1 {
                return Err("domination_gameplay_sequence_binding_mismatch");
            }
            if let Some(prior) = self.sequences.last_mut()
                && prior.object_index == delta.object_index
            {
                *prior = delta;
            } else {
                self.sequences.push(delta);
            }
        }
        Ok(())
    }
}

fn apply_effects<E: ExternalEffects>(
    setup: &Setup,
    world: &mut ObjectiveWorld,
    round: &mut Round,
    owner: &mut E,
    now: u32,
    batch: InvocationEffects,
) -> Result<(), &'static str> {
    for effect in batch.effects {
        match effect {
            Effect::SetState {
                objective_token,
                state,
                caller,
            } => {
                let index = *setup
                    .indices_by_token
                    .get(&objective_token)
                    .ok_or("domination_gameplay_unbound_script_objective")?;
                let desired = match state {
                    State::Completed => ObjectiveState::Completed,
                    State::Deactivated => ObjectiveState::Deactivated,
                    State::Locked => ObjectiveState::Locked,
                    State::Other => return Err("domination_gameplay_unknown_script_target"),
                };
                let caller_id = batch.context.caller_object_id;
                let raw_slot = match caller {
                    Caller::Original if caller_id & 0xffff_0000 == 0x0c80_0000 => {
                        world.script_actor(caller_id as u16)
                    }
                    _ => None,
                };
                let actor = raw_slot
                    .map(|slot| owner.callback_actor(0x0380_0000 | u32::from(slot)))
                    .transpose()?
                    .flatten();
                world.stage_native_state_with_caller(index, desired, raw_slot, actor)?;
            }
            Effect::ScoreRate(rate) => round.rates_mut().configure(rate, now)?,
            Effect::Ability {
                platoon_token,
                ability,
                enabled,
            } => {
                owner.prepare_ability(batch.context, platoon_token, ability, enabled)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "domination_gameplay_engine_tests.rs"]
pub(crate) mod tests;

/// Collects the concrete provider's outbound traffic and owned world changes.
/// The install closure only moves already-prepared worlds into their owners.
pub(crate) struct EffectsCommit<I> {
    pub audit: Vec<serde_json::Value>,
    pub messages: Vec<crate::OutboundMessage>,
    pub packets: Vec<Vec<u8>>,
    pub install: I,
}

impl<W: std::io::Write> crate::Engine<W> {
    /// Process the native MissionStart callback when the match owner reaches
    /// that lifecycle event. This publishes no current objective states.
    pub(crate) fn commit_domination_mission_start(
        &mut self,
        runtime: &mut Runtime,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        if runtime.started {
            return Ok(());
        }
        self.commit_domination_gameplay(
            runtime,
            now,
            wall,
            BTreeMap::new(),
            |_, _, runtime, frame| {
                Ok((
                    runtime.stage_mission_start(frame, ())?,
                    EffectsCommit {
                        audit: vec![serde_json::json!({"op": "domination_mission_start"})],
                        messages: Vec::new(),
                        packets: Vec::new(),
                        install: |_: &mut Self| {},
                    },
                ))
            },
        )?;
        Ok(())
    }

    /// The outer transaction shared by objective ticks and synchronous player
    /// interactions. Preparation runs inside the existing character rollback
    /// boundary. The provider owns all other effects until the record succeeds.
    pub(crate) fn commit_domination_gameplay<I>(
        &mut self,
        runtime: &mut Runtime,
        now: Duration,
        wall: u64,
        mut native: BTreeMap<u32, crate::acquisition_character_effects::NativeState>,
        prepare: impl FnOnce(
            &mut Self,
            &mut BTreeMap<u32, crate::acquisition_character_effects::NativeState>,
            &Runtime,
            Frame<'_>,
        ) -> Result<(Candidate<()>, EffectsCommit<I>), &'static str>,
    ) -> Result<Vec<crate::OutboundMessage>, &'static str>
    where
        I: FnOnce(&mut Self),
    {
        if self.match_profile().game_type != 2 {
            return Err("domination_gameplay_requires_retail_mode2");
        }
        let world = self
            .objective_world
            .clone()
            .ok_or("missing_domination_objective_world")?;
        let spawns = self
            .spawn_catalog
            .clone()
            .ok_or("missing_domination_spawn_catalog")?;
        let round = self
            .domination_round
            .clone()
            .ok_or("missing_domination_round")?;
        let mut staged = None;
        let characters =
            self.prepare_acquisition_character_callback(&native, now, |engine, native| {
                let (candidate, effects) = prepare(
                    engine,
                    native,
                    runtime,
                    Frame {
                        now,
                        world: &world,
                        spawns: &spawns,
                        round: &round,
                    },
                )?;
                staged = Some((candidate, effects.install, effects.packets));
                Ok((effects.audit, effects.messages))
            })?;
        let (candidate, install, packets) =
            staged.ok_or("missing_domination_gameplay_candidate")?;
        let time = now.as_millis() as u32;
        let mut shared = candidate.objective_packets(time, self.wire_profile)?;
        shared.extend(packets);
        let spawn_writes =
            self.prepare_runtime_spawn_publication(&spawns, &candidate.spawns, time, &mut shared)?;
        self.check_acquisition_character_effects(&characters, &native)?;
        self.record(
            serde_json::json!({
                "op": "domination_gameplay",
                "objective_updates": candidate.deltas.len(),
                "sequence_updates": candidate.sequences.len(),
                "shared_packets": shared.iter().map(|packet| crate::protocol::encode_hex(packet)).collect::<Vec<_>>(),
                "personal_spawn_packets": spawn_writes.iter().filter_map(|write| write.packet.as_ref().map(|packet|
                    serde_json::json!({"player": write.player, "payload_hex": crate::protocol::encode_hex(packet)})
                )).collect::<Vec<_>>(),
                "character_effects": characters.audit(),
                "messages": characters.messages(),
            }),
            wall,
        )?;
        let messages = self.install_character_effects_after_record(characters, &mut native);
        self.objective_world = Some(candidate.world);
        self.spawn_catalog = Some(candidate.spawns);
        self.domination_round = Some(candidate.round);
        *runtime = candidate.runtime;
        install(self);
        self.install_runtime_spawn_publication(&spawn_writes);
        for actor in self.peers.values_mut().chain(self.active.iter_mut()) {
            if actor.initial_objects.is_some() {
                for packet in &shared {
                    actor.queue_shared(packet.clone());
                }
            }
        }
        Ok(messages)
    }

    pub(crate) fn install_runtime_spawn_publication(&mut self, spawn_writes: &[SpawnWrite]) {
        for actor in self.peers.values_mut().chain(self.active.iter_mut()) {
            if let Some(write) = spawn_writes
                .iter()
                .find(|write| write.player == actor.reservation.player)
            {
                actor.squad_catalog = write.catalog.clone();
                actor.squad_spawn = write.default;
                if let Some(selected) = write.selection {
                    actor.selected_spawn = Some(selected);
                }
                if actor.initial_objects.is_some()
                    && let Some(packet) = &write.packet
                {
                    actor.queue_shared(packet.clone());
                }
            }
        }
    }

    pub(crate) fn prepare_runtime_spawn_publication(
        &self,
        current: &SpawnCatalog,
        next: &SpawnCatalog,
        time: u32,
        shared: &mut Vec<Vec<u8>>,
    ) -> Result<Vec<SpawnWrite>, &'static str> {
        use crate::replication::{
            CharacterSpawnAssignment, SpawnAvailability, SquadSpawnAssignment,
        };
        let encode = if current.match_profile().game_type == 1 {
            SpawnAvailability::encode_update_with_profile
        } else {
            SpawnAvailability::encode_domination_update
        };
        let old_availability = encode(1, time, current, self.wire_profile)?;
        let new_availability = encode(1, time, next, self.wire_profile)?;
        let availability_changed = old_availability != new_availability;
        if availability_changed {
            shared.push(new_availability);
        }
        let mut changed = [false; 32];
        for (index, changed) in changed.iter_mut().enumerate() {
            let squad = index as u16;
            let next_default = next.default_for_squad(squad);
            if current.default_for_squad(squad) == next_default
                && current.available_for_squad(squad) == next.available_for_squad(squad)
            {
                continue;
            }
            *changed = true;
            if current.default_for_squad(squad) != next_default {
                let spawn_point_index =
                    next_default.ok_or("domination_squad_spawn_default_missing")?;
                shared.push(
                    SquadSpawnAssignment {
                        sequence: 2,
                        timestamp_ms: time,
                        spawn_point_index,
                    }
                    .encode_bound_for_squad(squad, self.wire_profile)?,
                );
            }
        }
        let mut writes = Vec::new();
        for actor in self.active.iter().chain(self.peers.values()) {
            let squad = actor.squad_catalog.squad_index();
            if !availability_changed && !changed.get(usize::from(squad)).copied().unwrap_or(false) {
                continue;
            }
            let catalog = next.for_squad(squad)?;
            let default = catalog.default_spawn();
            let raw_next = next.default_for_squad(squad);
            let hidden_native_default =
                raw_next != current.default_for_squad(squad) && raw_next != Some(default);
            let should_follow = match actor.selected_spawn {
                None => hidden_native_default,
                Some(selected) => {
                    ((default != actor.squad_spawn || hidden_native_default)
                        && selected.spawn_point_index == actor.squad_spawn)
                        || catalog.alpha(selected.spawn_point_index).is_none()
                }
            };
            let selection = should_follow.then_some(crate::protocol::CharacterSpawnSelection {
                spawn_point_index: default,
            });
            let packet = selection
                .map(|selected| {
                    CharacterSpawnAssignment {
                        sequence: 2,
                        timestamp_ms: time,
                        spawn_point_index: selected.spawn_point_index,
                    }
                    .encode_for_player_with_catalog_with_profile(
                        actor.reservation.player,
                        &catalog,
                        self.wire_profile,
                    )
                })
                .transpose()?;
            writes.push(SpawnWrite {
                player: actor.reservation.player,
                catalog: Arc::new(catalog),
                default,
                selection,
                packet,
            });
        }
        Ok(writes)
    }
}

pub(crate) struct SpawnWrite {
    pub(crate) player: u16,
    catalog: Arc<SpawnCatalog>,
    default: u16,
    selection: Option<crate::protocol::CharacterSpawnSelection>,
    pub(crate) packet: Option<Vec<u8>>,
}
