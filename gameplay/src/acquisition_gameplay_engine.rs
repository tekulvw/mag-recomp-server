//! Concrete Acquisition objective transactions over owned Engine state.
use crate::{
    acquisition_atomic::{EnteredState, Entry},
    acquisition_character_effects::{Candidate as CharacterCandidate, NativeState},
    acquisition_control::Catalog,
    acquisition_gameplay::{CaptureCredit, Effect, Gameplay},
    acquisition_round::{Round, Rules, ScoreIntent},
    acquisition_spawn_effects::Clearance,
    acquisition_volume_effects::VolumeRequest,
    acquisition_world::Settings,
    dynamic_collision::QueryBox,
    objective_catalog::NativeClass,
    objective_control::{BotCaptureSource, CaptureActor, ObjectiveActor, ObjectiveWorld},
    objective_range::TriggerBox,
    replication::{ObjectiveControlDelta, ObjectiveState},
    *,
};
use std::{collections::BTreeMap, sync::Arc};

pub struct Setup {
    pub controller: Catalog,
    pub state: Gameplay,
    pub rules: Rules,
    pub filter_rows: [u32; 32],
    pub physics: Settings,
    pub volumes: BTreeMap<u32, QueryBox>,
    pub initial_active: Vec<u32>,
    pub arrival_triggers: BTreeMap<u32, TriggerBox>,
    pub arrival_static: Option<ground_vehicles::arrival_static::ArrivalStatic>,
    pub ground_capabilities: Vec<(u16, acquisition_ground_runtime::Capabilities)>,
    pub ground_damage: Option<acquisition_ground_damage::Policy>,
    pub frag_character_blast: bool,
    pub(crate) human_static_resupply: Option<Arc<human_static_resupply::Catalog>>,
    pub(crate) projectile_contacts:
        Option<Arc<crate::domination_projectile_collision::ContactBindings>>,
    pub(crate) frag_ground_areas: Option<Arc<acquisition_frag_ground::Catalog>>,
    pub vehicle_reinforcement_intervals_ms: [u32; 2],
    pub structural: Option<crate::acquisition_structural::Setup>,
}

pub(crate) enum StructuralInput {
    Interaction(protocol::ObjectiveInteraction, ObjectiveActor, f32),
    Repair(u16, domination_repair_runtime::Authority),
}

struct ObjectiveCommit {
    runtime: Runtime,
    world: ObjectiveWorld,
    ground: acquisition_ground_runtime::Runtime,
    round: Option<Round>,
    spawns: Option<spawn_catalog::SpawnCatalog>,
    changed: BTreeMap<u16, ObjectiveControlDelta>,
    ground_changes: BTreeMap<u16, u32>,
    sequence_deltas: BTreeMap<u16, replication::ObjectiveSequenceDelta>,
    transition_packets: Vec<Vec<u8>>,
    characters: CharacterCandidate,
    native: BTreeMap<u32, NativeState>,
    leadership: Option<leadership_runtime::LeadershipPlan>,
    refresh_assignments: bool,
    audit: Value,
}

struct CaptureWitness {
    completion: crate::objective_control::CaptureCompletion,
    target_version: u64,
}

impl Setup {
    pub fn has_human_static_resupply(&self) -> bool {
        self.human_static_resupply.is_some()
    }

    pub fn validate_human_static_resupply(
        &self,
        spawn: &spawn_catalog::SpawnCatalog,
        collision: Option<&collision::CollisionWorld>,
        objectives: &objective_catalog::ObjectiveCatalog,
        items: &item_catalog::ItemCatalog,
    ) -> Result<(), &'static str> {
        if let Some(catalog) = &self.human_static_resupply {
            catalog.bind(spawn, collision, objectives, items)?;
        }
        Ok(())
    }

    pub fn has_frag_ground_areas(&self) -> bool {
        self.frag_ground_areas.is_some()
    }

    pub fn validate_frag_ground(
        &self,
        items: &item_catalog::ItemCatalog,
        world: Option<&collision::CollisionWorld>,
        ground: &ground_vehicles::Catalog,
        objectives: &objective_catalog::ObjectiveCatalog,
    ) -> Result<(), &'static str> {
        let Some(areas) = &self.frag_ground_areas else {
            return Ok(());
        };
        self.structural
            .as_ref()
            .ok_or("acquisition_frag_ground_requires_structural")?
            .validate_frag_character_inputs(items, world, self.filter_rows[24])?;
        if [6, 24]
            .into_iter()
            .any(|row| self.filter_rows[row] & (1 << 22) == 0)
        {
            return Err("acquisition_frag_ground_collision_filter_required");
        }
        let policy = self
            .ground_damage
            .as_ref()
            .ok_or("acquisition_frag_ground_requires_damage_policy")?;
        policy.validate()?;
        let native_elf = &objectives.source().source_image_sha256;
        if ground.native_elf_sha256() != native_elf {
            return Err("acquisition_frag_ground_area_source_mismatch");
        }
        areas.validate(native_elf, ground)?;
        for creation in ground.creations() {
            policy.validate_vehicle(creation)?;
        }
        Ok(())
    }

    pub fn validate_frag_ground_dynamic(
        &self,
        bodies: &dynamic_collision::Catalog,
        bindings: &dynamic_collision_runtime::RuntimeBindings,
        ground: &ground_vehicles::Catalog,
    ) -> Result<(), &'static str> {
        if let Some(areas) = &self.frag_ground_areas {
            areas.validate_dynamic_parts(bodies, bindings, ground)?;
        }
        Ok(())
    }

    pub fn has_projectile_contacts(&self) -> bool {
        self.projectile_contacts.is_some()
    }

    pub fn validate_projectile_contacts(
        &self,
        world: Option<&collision::CollisionWorld>,
        catalog: Option<&dynamic_collision::Catalog>,
    ) -> Result<(), &'static str> {
        self.validate_projectile_contact_static(world)?;
        if let Some(contacts) = &self.projectile_contacts {
            contacts.validate_dynamic(
                catalog.ok_or("acquisition_projectile_contacts_require_dynamic_bodies")?,
            )?;
        }
        Ok(())
    }

    pub(crate) fn validate_projectile_contact_static(
        &self,
        world: Option<&collision::CollisionWorld>,
    ) -> Result<(), &'static str> {
        if let Some(contacts) = &self.projectile_contacts {
            contacts.validate_static(
                world.ok_or("acquisition_projectile_contacts_require_static_world")?,
            )?;
        }
        Ok(())
    }

    pub fn validate_frag_character_blast(
        &self,
        items: &item_catalog::ItemCatalog,
        world: Option<&collision::CollisionWorld>,
    ) -> Result<(), &'static str> {
        if !self.frag_character_blast {
            return Ok(());
        }
        let structural = self
            .structural
            .as_ref()
            .ok_or("acquisition_frag_character_blast_requires_structural")?;
        structural.validate_frag_character_inputs(items, world, self.filter_rows[24])
    }

    pub(crate) fn validate_structural(
        &self,
        catalog: &objective_catalog::ObjectiveCatalog,
    ) -> Result<(), &'static str> {
        if let Some(structural) = &self.structural {
            structural.validate_native_objectives(catalog)?;
        }
        for object in catalog
            .objects
            .iter()
            .filter(|object| object.native_class == Some(NativeClass::Structural))
        {
            if self.structural.is_none() {
                return Err("acquisition_native_structural_provider_required");
            }
            if self
                .state
                .objectives()
                .any(|(_, control)| control.replication.object_index == object.object_index)
            {
                return Err("acquisition_native_structural_overlaps_controller");
            }
        }
        Ok(())
    }

    /// Every capture chain must link a distinct, drivable Retail Escort.
    /// When native Structural area damage is present, all mountable ground
    /// seats must also have source-bound occupant visibility.
    pub fn validate_ground_drivers(
        &self,
        vehicles: &ground_vehicles::Catalog,
    ) -> Result<(), &'static str> {
        let mut seen = std::collections::BTreeSet::new();
        for chain in self.controller.chains() {
            let fsp = self
                .state
                .objective(chain.capture_token)
                .and_then(|capture| capture.fsp)
                .ok_or("acquisition_escort_driver_source_mismatch")?;
            let index = fsp.vehicle.index;
            if !seen.insert(index)
                || vehicles.vehicle_source(index).is_none_or(|source| {
                    source.spawn_type != 9
                        || source.spawn_point_index != fsp.source.spawn_point_index
                        || source.spawn_point_token != fsp.source.spawn_point_token
                        || source.spawn_point_token != chain.vehicle_spawn_token
                })
            {
                return Err("acquisition_escort_driver_source_mismatch");
            }
            if !vehicles
                .driver_seat(index)
                .is_some_and(|driver| driver.seat == 0)
            {
                return Err("acquisition_escort_driver_policy_required");
            }
        }
        if let Some(structural) = &self.structural {
            structural.validate_ground_area_seats(vehicles)?;
        }
        Ok(())
    }

    pub fn validate_ground_hulk_templates(
        &self,
        catalog: &dynamic_collision::Catalog,
    ) -> Result<(), &'static str> {
        for (_, capabilities) in &self.ground_capabilities {
            if capabilities.hulk_template.as_ref().is_some_and(|template| {
                catalog.template_kind(template)
                    != Some(acquisition_volume_damage::TargetKind::GroundVehicle)
            }) {
                return Err("acquisition_ground_hulk_template_missing");
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Runtime {
    pub(crate) setup: Arc<Setup>,
    pub(crate) state: Gameplay,
    started: bool,
    pub(crate) last_ground_update: Option<Duration>,
    pub(crate) vehicle_waves: Option<acquisition_vehicle_waves::VehicleWaves>,
    structural: Option<crate::acquisition_structural::Runtime>,
}
impl Runtime {
    pub(crate) fn from_setup(setup: Arc<Setup>) -> Self {
        Self {
            state: setup.state.clone(),
            structural: setup
                .structural
                .clone()
                .map(crate::acquisition_structural::Runtime::new),
            setup,
            started: false,
            last_ground_update: None,
            vehicle_waves: None,
        }
    }
    pub(crate) fn bot_regional_structural_bounds(
        &self,
        clv_sha256: &str,
        budget: &mut usize,
    ) -> Result<Vec<crate::actor_collision::Bounds>, &'static str> {
        self.structural
            .as_ref()
            .ok_or("bot_regional_structural_unavailable")?
            .bot_regional_bounds(clv_sha256, budget)
    }

    pub(crate) fn current_structural_sequences(&self) -> Vec<replication::ObjectiveSequenceDelta> {
        self.structural
            .as_ref()
            .map_or_else(Vec::new, crate::acquisition_structural::Runtime::sequences)
    }

    pub(crate) fn blast_collision(
        &self,
    ) -> Option<domination_structural_world::BlastCollision<'_>> {
        self.structural.as_ref().map(|structural| {
            structural.blast_collision(&self.setup.filter_rows, self.setup.physics)
        })
    }

    pub(crate) fn reset(&mut self) {
        self.state = self.setup.state.clone();
        self.started = false;
        self.last_ground_update = None;
        self.vehicle_waves = None;
        self.structural = self
            .setup
            .structural
            .clone()
            .map(crate::acquisition_structural::Runtime::new);
    }
}

impl<W: Write> Engine<W> {
    pub(crate) fn source_frag_owner_updates(&self) -> bool {
        self.frag_character_blast
            || self.ground_frag_areas().is_some()
            || self.domination_gameplay.as_ref().is_some_and(|world| {
                world.has_helo_frag_damage_areas() || world.has_hospital_blast_areas()
            })
    }

    /// Explicit replacement gameplay configuration, separate from the native
    /// pre-MissionStart identity catalog. No services or game process are used.
    pub fn with_acquisition_gameplay(mut self, setup: Setup) -> Result<Self, &'static str> {
        if self.active.is_some()
            || !self.peers.is_empty()
            || self.wire_profile.application_id() != 20231
            || self.match_profile().game_type != 1
        {
            return Err("acquisition_gameplay_requires_idle_retail_world");
        }
        setup.controller.validate_for(
            self.spawn_catalog
                .as_ref()
                .ok_or("spawn_catalog_unavailable")?,
        )?;
        let ground = self
            .ground_vehicles
            .as_ref()
            .ok_or("ground_vehicle_catalog_required")?;
        setup.validate_ground_drivers(ground)?;
        let catalog = self
            .objective_catalog
            .as_ref()
            .ok_or("objective_catalog_unavailable")?;
        setup.validate_structural(catalog)?;
        setup.validate_human_static_resupply(
            self.spawn_catalog
                .as_ref()
                .ok_or("spawn_catalog_unavailable")?,
            self.collision_world.as_ref(),
            catalog,
            &self.item_catalog,
        )?;
        setup.validate_projectile_contacts(
            self.collision_world.as_ref(),
            self.dynamic_body_resources
                .as_ref()
                .map(|resources| &resources.catalog),
        )?;
        setup.validate_frag_character_blast(&self.item_catalog, self.collision_world.as_ref())?;
        setup.validate_frag_ground(
            &self.item_catalog,
            self.collision_world.as_ref(),
            ground,
            catalog,
        )?;
        if setup.has_frag_ground_areas() {
            let bodies = self
                .dynamic_body_resources
                .as_ref()
                .ok_or("acquisition_frag_ground_requires_dynamic_bodies")?;
            setup.validate_frag_ground_dynamic(&bodies.catalog, bodies.bindings(), ground)?;
        }
        if setup.frag_character_blast && self.dynamic_body_resources.is_none() {
            return Err("frag_character_blast_requires_dynamic_bodies");
        }
        if setup.frag_character_blast && self.character_damage_rules.is_none() {
            return Err("acquisition_gameplay_requires_character_damage_rules");
        }
        if catalog.source().capture_qualification != "native_source_gameplay"
            || setup.rules.starting_points != self.match_profile().maximum_points
            || setup.rules.starting_points.iter().any(|v| *v <= 0)
            || !(0..=100).contains(&setup.rules.major_percent)
            || !(1..=60_000).contains(&setup.rules.end_delay_ms)
        {
            return Err("acquisition_gameplay_configuration_mismatch");
        }
        for (token, objective) in setup.state.objectives() {
            if catalog.binding(token)
                != Some((
                    objective.replication.object_index,
                    objective.state.current as u8,
                ))
            {
                return Err("acquisition_gameplay_objective_mismatch");
            }
        }
        let empty = dynamic_collision::Snapshot::capture(
            acquisition_atomic::WorldKey {
                world_id: 1,
                incarnation: 1,
            },
            0,
            &[],
            vec![],
            0,
        )?;
        for chain in setup.controller.chains() {
            let query = setup
                .volumes
                .get(&chain.volume_token)
                .ok_or("acquisition_volume_missing")?;
            empty.penetrations(*query, setup.physics.query_limits)?;
            if !setup.arrival_triggers.contains_key(&chain.arrival_token) {
                return Err("acquisition_arrival_trigger_missing");
            }
        }
        for &token in &setup.initial_active {
            if setup.state.objective(token).is_none() {
                return Err("acquisition_startup_objective_missing");
            }
        }
        if let Some(policy) = &setup.ground_damage {
            policy.validate()?;
            let runtime = self
                .ground_runtime
                .as_ref()
                .ok_or("ground_vehicle_runtime_required")?;
            for vehicle in runtime.vehicles() {
                let creation = ground
                    .vehicle(vehicle.key.index)
                    .ok_or("unknown_ground_vehicle")?;
                policy.validate_vehicle(creation)?;
            }
        }
        acquisition_gameplay_config::configure_ground(
            self.ground_runtime
                .as_mut()
                .ok_or("ground_vehicle_runtime_required")?,
            &setup.ground_capabilities,
        )?;
        self.acquisition_round = Some(Round::new(setup.rules));
        self.frag_character_blast = setup.frag_character_blast;
        self.projectile_contacts = setup.projectile_contacts.clone();
        self.infantry_rockets = None;
        self.smoke_grenades = None;
        self.gas_grenades = None;
        self.flash_grenades = None;
        self.underbarrel_launchers = None;
        self.acquisition_gameplay = Some(Runtime::from_setup(Arc::new(setup)));
        Ok(self)
    }

    pub(crate) fn acquisition_structural_repair_tool(
        &self,
    ) -> Option<structure_catalog::RepairParameters> {
        self.acquisition_gameplay
            .as_ref()?
            .setup
            .structural
            .as_ref()?
            .repair_tool
    }

    pub(crate) fn event_acquisition_structural(
        &mut self,
        event: &events::EventInput,
        input: StructuralInput,
        leadership: leadership_runtime::LeadershipPlan,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let mut runtime = self
            .acquisition_gameplay
            .clone()
            .ok_or("acquisition_gameplay_unavailable")?;
        if !runtime.started {
            return Err("acquisition_gameplay_not_started");
        }
        let assignments: Vec<_> = (0..32)
            .filter_map(|index| runtime.state.squad(index).map(|s| (index, s.assignment)))
            .collect();
        let structural = runtime
            .structural
            .as_mut()
            .ok_or("acquisition_native_structural_provider_required")?;
        let mut world = self
            .objective_world
            .clone()
            .ok_or("objective_world_unavailable")?;
        let index = match input {
            StructuralInput::Interaction(request, ..) => request.target_object_id as u16,
            StructuralInput::Repair(index, _) => index,
        };
        if world.native_class(index) != Some(NativeClass::Structural) {
            return Err("acquisition_structural_entry_unbound");
        }
        let mut changed = BTreeMap::new();
        let mut repair_credit = None;
        let (op, reply, reason) = match input {
            StructuralInput::Interaction(request, actor, modifier) => {
                let outcome = if structural.bot_owns_hold(&world, index, actor.player) {
                    crate::objective_control::InteractionOutcome {
                        accepted: false,
                        reason: if world.state(index) == Some(ObjectiveState::Activating) {
                            "acquisition_bot_plant_owned"
                        } else {
                            "acquisition_bot_defuse_owned"
                        },
                        changed: None,
                        reply_trigger_index: u8::MAX,
                    }
                } else {
                    world.interact_with_demolition_modifier(request, actor, now, modifier)
                };
                if outcome.accepted {
                    if let Some(active) = &self.active {
                        world.attribute_operation(index, active);
                    }
                    if let Some(delta) = world.commit_pending_native_interaction(index, now, actor)
                    {
                        changed.insert(index, delta);
                    }
                }
                let reply = protocol::ObjectiveInteraction {
                    target_object_id: request.target_object_id,
                    actor_index: actor.player,
                    trigger_index: outcome.reply_trigger_index,
                    kind: if outcome.accepted {
                        protocol::ObjectiveInteractionKind::Ack
                    } else {
                        protocol::ObjectiveInteractionKind::Nak
                    },
                };
                (
                    "acquisition_structural_interaction",
                    Some(reply),
                    Some(outcome.reason),
                )
            }
            StructuralInput::Repair(index, authority) => {
                use crate::domination_repair_runtime::{HEALTH_ALREADY_COVERED, Submission};
                let target = 0x0c80_0000 | u32::from(index);
                let reason = match world.stage_repair_packet(target, authority)? {
                    Submission::Queued(_) => {
                        let actor = self.active.as_ref().ok_or("no_reservation")?;
                        repair_credit = Some(self.plan_repair_credit(
                            actor,
                            leadership.modifiers(actor).value("kCharModRepairAmount"),
                            self.frago_object_position(target),
                        )?);
                        None
                    }
                    Submission::LatchOnly => Some(HEALTH_ALREADY_COVERED),
                };
                ("acquisition_structural_repair", None, reason)
            }
        };
        let mut ground = self
            .ground_runtime
            .clone()
            .ok_or("ground_vehicle_runtime_required")?;
        let mut spawns = self
            .spawn_catalog
            .clone()
            .ok_or("acquisition_structural_spawn_catalog_required")?;
        let mut ground_changes = BTreeMap::new();
        let mut sequence_deltas = BTreeMap::new();
        let native = self.owned_acquisition_characters();
        let characters =
            self.prepare_acquisition_character_callback(&native, now, |engine, native| {
                let mut audit = Vec::new();
                let mut messages = Vec::new();
                if let Some(credit) = repair_credit {
                    audit.push(credit.audit());
                    messages.extend(engine.install_repair_credit(credit));
                }
                let mut frame = crate::acquisition_structural::Frame {
                    engine,
                    assignments: &assignments,
                    ground: &mut ground,
                    spawns: &mut spawns,
                    native,
                    ground_changes: &mut ground_changes,
                    audit: &mut audit,
                    messages: &mut messages,
                    sequences: &mut sequence_deltas,
                    filter_rows: &runtime.setup.filter_rows,
                    physics: runtime.setup.physics,
                    ground_policy: runtime.setup.ground_damage.as_ref(),
                    now,
                };
                for entry in world.take_native_entries() {
                    structural.stage_entry(entry, &mut world, &mut frame)?;
                }
                if let Some(reply) = reply {
                    messages.push(OutboundMessage {
                        reservation_id: event.reservation_id.clone(),
                        world_id: event.world_id,
                        player_id: event.player_id,
                        plugin_id: 1,
                        payload_hex: protocol::encode_hex(&reply.encode()),
                    });
                }
                Ok((audit, messages))
            })?;
        let audit = json!({
            "op": op, "timestamp_ms": now.as_millis() as u64,
            "world_id": event.world_id, "player_id": event.player_id,
            "reservation_id": event.reservation_id, "object_index": index,
            "transport": event.transport, "direction": event.direction,
            "rt_type": event.rt_type, "plugin_id": event.plugin_id,
            "payload_hex": event.payload_hex.to_ascii_lowercase(),
            "reason": reason, "reply_kind": reply.map(|reply| reply.kind as u8),
            "leadership_changes": leadership.audit(),
        });
        self.commit_acquisition_objective_candidate(
            ObjectiveCommit {
                runtime,
                world,
                ground,
                round: None,
                spawns: Some(spawns),
                refresh_assignments: !changed.is_empty(),
                changed,
                ground_changes,
                sequence_deltas,
                transition_packets: Vec::new(),
                characters,
                native,
                leadership: Some(leadership),
                audit,
            },
            now,
            wall,
        )
    }

    /// The admitted replacement runtime has no temporary invulnerability
    /// producer. These values describe this owned world, not a native capture.
    /// Mounted status comes from current seats, never a default false flag.
    pub(crate) fn owned_acquisition_characters(&self) -> BTreeMap<u32, NativeState> {
        let mut states = BTreeMap::new();
        for actor in self.active.iter().chain(self.peers.values()) {
            states.insert(
                0x0380_0000 | u32::from(actor.reservation.player),
                NativeState {
                    invulnerability: Some(0),
                    associated_vehicle: Some(if actor.is_mounted() {
                        protocol::VictimWasMounted::AssociatedVehicle
                    } else {
                        protocol::VictimWasMounted::NoAssociatedVehicle
                    }),
                },
            );
        }
        for &id in self.bot_registry.bots.keys() {
            states.insert(
                0x0380_0000 | u32::from(id),
                NativeState {
                    invulnerability: Some(0),
                    associated_vehicle: Some(protocol::VictimWasMounted::NoAssociatedVehicle),
                },
            );
        }
        states
    }

    pub(crate) fn tick_acquisition_objectives(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let mut runtime = self
            .acquisition_gameplay
            .clone()
            .ok_or("acquisition_gameplay_unavailable")?;
        if !self.match_started || self.match_terminal.is_some() {
            return Ok(vec![]);
        }
        if !self
            .active
            .iter()
            .chain(self.peers.values())
            .any(|a| a.entered_game && a.ready_for_updates && a.initial_objects.is_some())
        {
            return Ok(vec![]);
        }
        let setup = Arc::clone(&runtime.setup);
        let mut world = self
            .objective_world
            .clone()
            .ok_or("objective_world_unavailable")?;
        let mut round = self
            .acquisition_round
            .clone()
            .ok_or("acquisition_round_unavailable")?;
        let before_ground = self
            .ground_runtime
            .clone()
            .ok_or("ground_vehicle_runtime_required")?;
        let mut ground = before_ground.clone();
        let prior_owners: BTreeMap<_, _> = world
            .snapshot(1, now.as_millis() as u32)
            .objectives
            .iter()
            .filter_map(|d| {
                let owner = world.interaction_owner(d.object_index)?;
                if world.acquisition_bot_capture(d.object_index).is_some()
                    || runtime.structural.as_ref().is_some_and(|structural| {
                        structural.bot_owns_hold(&world, d.object_index, owner.0)
                    })
                {
                    return None;
                }
                Some((d.object_index, (d.state_actor.state, owner)))
            })
            .collect();
        let mut changed: BTreeMap<u16, ObjectiveControlDelta> = BTreeMap::new();
        let mut entry_actors = BTreeMap::new();
        let mut capture_witnesses = BTreeMap::new();
        let mut capture_audit = Vec::new();
        let abilities_enabled = self.active_abilities.is_some();
        if !abilities_enabled {
            stage_objective_observations(
                self,
                &mut runtime,
                &mut world,
                now,
                &mut changed,
                &mut entry_actors,
                &mut capture_witnesses,
                &mut capture_audit,
            )?;
        }
        let starting = !runtime.started;
        if starting {
            if let Some(structural) = &mut runtime.structural {
                structural.stage_mission_start(&mut world)?;
            }
            for &token in &setup.initial_active {
                runtime
                    .state
                    .request_state(token, ObjectiveState::Deactivated)?;
            }
            runtime.started = true;
        }
        // Resolve the current squad spawn each time; the initial catalog alone
        // is not a claim that a later selected spawn still resolves.
        for index in (0..2).flat_map(|team| (0..8).map(move |slot| team * 16 + slot)) {
            if let Some(squad) = runtime.state.squad_mut(index) {
                squad.spawn_resolved = self.spawn_catalog.as_ref().is_some_and(|c| {
                    c.default_for_squad(index)
                        .and_then(|p| c.point(p))
                        .is_some()
                });
            }
        }
        let mut ground_changes = BTreeMap::new();
        let mut transition_packets = Vec::new();
        let mut sequence_deltas = BTreeMap::new();
        let mut structural_spawns = if runtime.structural.is_some() {
            Some(
                self.spawn_catalog
                    .clone()
                    .ok_or("acquisition_structural_spawn_catalog_required")?,
            )
        } else {
            None
        };
        let native = self.owned_acquisition_characters();
        let characters = self.prepare_acquisition_character_callback(&native, now, |engine, native| {
            let mut audit = std::mem::take(&mut capture_audit);
            let mut messages = Vec::new();
            let mut delivery_rewards = BTreeMap::new();
            let (rows, output) = engine.stage_acquisition_vehicle_waves(&mut runtime, &mut ground, now, native, &mut ground_changes)?;
            audit.extend(rows);
            messages.extend(output);
            let (rows, output) = engine.stage_acquisition_ground_updates(&mut runtime, &mut ground, now, native, &mut ground_changes, &mut entry_actors)?;
            audit.extend(rows);
            messages.extend(output);
            let (rows, output) = engine.stage_acquisition_deliveries(&mut runtime, &mut ground, now, &mut ground_changes, &mut entry_actors, &mut delivery_rewards)?;
            audit.extend(rows);
            messages.extend(output);
            if engine.active_abilities.is_some() {
                let previous = engine.ground_runtime.replace(ground.clone());
                let sequences = crate::domination_structural_world::SequenceCollisionState::default();
                let costs = BTreeMap::new();
                let collision = runtime.blast_collision().unwrap_or(
                    crate::domination_structural_world::BlastCollision {
                        filter_rows: &setup.filter_rows, limits: setup.physics.query_limits,
                        sequences: &sequences, material_costs: &costs, terrain_materials: None,
                    });
                let result = engine.stage_active_ability_update_in_world(now, native,
                    collision, setup.ground_damage.as_ref(), Some(&mut world));
                ground = std::mem::replace(&mut engine.ground_runtime, previous).ok_or("ground_vehicle_runtime_required")?;
                let commit = result?;
                audit.extend(commit.audit); messages.extend(commit.messages);
                for index in commit.changed_ground { *ground_changes.entry(index).or_default() |= 1 << 18; }
                if !commit.helo_doses.is_empty() { return Err("acquisition_unexpected_helo_strike_target"); }
            }
            if abilities_enabled {
                stage_objective_observations(engine, &mut runtime, &mut world, now, &mut changed, &mut entry_actors, &mut capture_witnesses, &mut audit)?;
            }
            audit.extend(engine.stage_acquisition_capture_rewards(world.take_reward_actions(), &runtime, &ground)?);
            let assignments: Vec<_> = (0..32).filter_map(|index| runtime.state.squad(index).map(|s|(index,s.assignment))).collect();
            if let Some(structural) = &mut runtime.structural {
                let mut frame = crate::acquisition_structural::Frame {
                    engine,
                    assignments: &assignments,
                    ground: &mut ground,
                    spawns: structural_spawns.as_mut().ok_or("acquisition_structural_spawn_catalog_required")?,
                    native,
                    ground_changes: &mut ground_changes,
                    audit: &mut audit,
                    messages: &mut messages,
                    sequences: &mut sequence_deltas,
                    filter_rows: &setup.filter_rows,
                    physics: setup.physics,
                    ground_policy: setup.ground_damage.as_ref(),
                    now,
                };
                changed.extend(structural.stage_world(&mut world, &mut frame, starting)?);
            } else {
                for delta in world.advance_acquisition_precision_entries(now, |entry, _| {
                    let assignment = engine.active.iter().chain(engine.peers.values())
                        .find(|a| Some(a.reservation.player) == entry.reward_actor)
                        .and_then(|a| runtime.state.squad(a.squad_catalog.squad_index()).map(|s|s.assignment));
                    let assignments: Vec<_> = (0..32).filter_map(|index| runtime.state.squad(index).map(|s|(index,s.assignment))).collect();
                    let effects = engine.stage_native_objective_callbacks(entry, assignment, &assignments,
                        crate::frago::Frame{ground:Some(&ground),..Default::default()})?;
                    audit.extend(effects.audit); messages.extend(effects.messages);
                    Ok(())
                })? {
                    changed.insert(delta.object_index, delta);
                }
            }
            *native = engine.owned_acquisition_characters();
            for _ in 0..256 {
                let next = runtime.state.pending_transitions().min_by_key(|(token, _)| runtime.state.objective(*token).unwrap().replication.object_index);
                let Some((token, target)) = next else {
                    for (&index, &(previous, (player, generation, trigger))) in &prior_owners {
                        if !changed.get(&index).is_some_and(|d| d.state_actor.state != previous) {
                            continue;
                        }
                        if let Some(actor) = engine.active.iter().chain(engine.peers.values()).find(|a| {
                            a.reservation.player == player && matches!(a.life, CharacterLife::Alive { generation: current, .. } if current == generation)
                        }) {
                            messages.push(events::objective_hold_exit(actor, index, player, trigger));
                        }
                    }
                    return Ok((audit, messages));
                };
                let state = objective_catalog::native_state(target as u8).ok_or("unsupported_acquisition_target_state")?;
                let capture_witness = take_capture_witness(
                    &runtime, token, &mut capture_witnesses, &mut entry_actors,
                );
                let object = runtime.state.objective(token).ok_or("unknown_acquisition_gameplay_objective")?;
                let attributed = if state == ObjectiveState::Completed { object.current_attacker } else { object.current_defender };
                let actor_index = entry_actors.remove(&token).unwrap_or({
                    if attributed & 0xffff_0000 == 0x0380_0000 { attributed as u16 } else { u16::MAX }
                });
                if let Some(witness) = delivery_rewards.remove(&token)
                    && state == ObjectiveState::Completed && actor_index == witness.player {
                    let effects=engine.stage_acquisition_delivery_reward(&runtime, &ground, token, witness)?;
                    audit.extend(effects.audit); messages.extend(effects.messages);
                }
                let mut capture_credit = CaptureCredit::None;
                let mut bot_credit = None;
                if let Some(witness) = capture_witness {
                    let valid_entry = state == ObjectiveState::Completed
                        && actor_index == witness.completion.player
                        && runtime.state.target_version(token) == Some(witness.target_version);
                    match witness.completion.actor {
                        Some(CaptureActor::Human(identity)) if valid_entry => {
                            if engine.active.iter().chain(engine.peers.values()).any(|actor| {
                                actor.reservation.player == actor_index
                                    && actor.entered_game && actor.claimed.is_some() && identity.matches(actor)
                            }) {
                                capture_credit = CaptureCredit::Human;
                            }
                        }
                        Some(CaptureActor::Bot(source)) if state == ObjectiveState::Completed => {
                            let index = witness.completion.object_index;
                            if let Some(delta) = resolve_bot_capture_completion(
                                engine, &mut runtime, &mut world,
                                token, source, now, valid_entry,
                            )? {
                                changed.insert(index, delta);
                                entry_actors.insert(token, u16::MAX);
                                audit.push(bot_capture_audit("acquisition_bot_capture_cancelled", index, source));
                                continue;
                            }
                            capture_credit = CaptureCredit::Bot;
                            bot_credit = Some(source);
                            audit.push(bot_capture_audit("acquisition_bot_capture_completed", index, source));
                        }
                        _ => {}
                    }
                }
                let callback = setup.controller.objective_tokens().any(|t| t == token);
                let delta = if callback && matches!(state, ObjectiveState::Completed | ObjectiveState::Deactivated) {
                    let entry = Entry { objective_token: token, state: if state == ObjectiveState::Completed { EnteredState::Completed } else { EnteredState::Deactivated }, actor_index, timestamp_ms: now.as_millis() as u32 };
                    let mut rates = round.rates_mut().clone();
                    let publication = runtime.state.apply_entry_with_capture(&setup.controller, &entry, &mut ground, &mut rates, capture_credit, |ground, effect| {
                        let previous = engine.ground_runtime.replace(ground.clone()).ok_or("ground_vehicle_runtime_required")?;
                        let result = match effect {
                            Effect::CaptureCompleted { object_index, actor_index, squads, vehicle_capture } => {
                                let assignments: Vec<_> = squads.iter().map(|(&index,squad)|(index,squad.assignment)).collect();
                                let effects = if let Some(source) = bot_credit {
                                    engine.stage_objective_counters_for_team(
                                        source.world, source.team, None, object_index, &[], true, &assignments,
                                    )?
                                } else {
                                    engine.stage_objective_counters(actor_index, object_index, if vehicle_capture { &[58] } else { &[] }, true, &assignments)?
                                };
                                Ok((effects.audit,effects.messages))
                            }
                            Effect::Blockers(source) => engine.stage_respawn_clearance(Clearance { source, physics: setup.physics, filter_rows: setup.filter_rows, now }, native, &mut ground_changes),
                            Effect::Volume { target, arguments, damage_type_symbol, damage_source_symbol } => {
                                acquisition_damage_metadata::Metadata::from_symbols(damage_type_symbol, damage_source_symbol)?;
                                engine.stage_acquisition_volume(VolumeRequest {
                                    query: *setup.volumes.get(&target).ok_or("acquisition_volume_missing")?,
                                    arguments: acquisition_volume_damage::Arguments { player_damage: arguments[0] as f32, ground_vehicle_damage: arguments[1] as f32, mine_damage: arguments[2] as f32, apply_to_team: arguments[3] },
                                    row6: setup.filter_rows[6], limits: setup.physics.query_limits, now,
                                }, native, &mut ground_changes)
                            }
                            Effect::ScoreDeduction { team, amount } => {
                                round.apply_score(ScoreIntent { team, delta: amount.wrapping_neg() })?;
                                Ok((vec![], vec![]))
                            }
                        };
                        *ground = engine.ground_runtime.replace(previous).unwrap();
                        let (rows, output) = result?;
                        audit.extend(rows);
                        messages.extend(output);
                        Ok(())
                    })?;
                    *round.rates_mut() = rates;
                    for event in &publication.lifecycle {
                        let mask = match event.effect { acquisition_ground_runtime::Effect::ForcePosition(_) => 1 << 25, acquisition_ground_runtime::Effect::ForceRotation(_) => 1 << 26, acquisition_ground_runtime::Effect::DamageReset => 1 << 18, acquisition_ground_runtime::Effect::StateChanged(_) => 1 << 5, _ => 0 };
                        if mask != 0 { *ground_changes.entry(event.vehicle.index).or_default() |= mask; }
                    }
                    audit.push(json!({"objective":token,"current":state as u8,"commands":publication.plan.commands,"calls":publication.plan.unbound_calls}));
                    publication.objective
                } else {
                    runtime.state.commit_state(token, state, actor_index, now.as_millis() as u32)?
                };
                transition_packets.push(replication::ObjectiveControlSnapshot {
                    sequence: 1,
                    timestamp_ms: now.as_millis() as u32,
                    objectives: vec![delta],
                }.encode_with_profile(engine.wire_profile)?);
                world.install_acquisition_delta(delta)?;
                changed.insert(delta.object_index, delta);
            }
            Err("acquisition_transition_limit")
        });
        self.ground_runtime = Some(before_ground);
        let characters = characters?;
        if changed.is_empty()
            && !starting
            && ground_changes.is_empty()
            && characters.audit().is_empty()
            && characters.messages().is_empty()
        {
            // A scheduling cursor carries no gameplay result. Keep it fresh
            // during idle ticks so new critical damage never ages retroactively.
            if let Some(live) = &mut self.acquisition_gameplay {
                live.last_ground_update = Some(now);
            }
            if runtime
                .structural
                .as_ref()
                .is_some_and(crate::acquisition_structural::Runtime::advances_native_objectives)
            {
                // Native repair integrates each quiet frame before the next
                // replicated health batch. Keep that timer with its world.
                self.objective_world = Some(world);
            }
            return Ok(vec![]);
        }
        let audit = json!({
            "op":"acquisition_objective_tick", "timestamp_ms":now.as_millis() as u64,
            "startup":starting, "controller_sha256":setup.controller.fingerprint(),
            "scores":round.points(),
        });
        self.commit_acquisition_objective_candidate(
            ObjectiveCommit {
                runtime,
                world,
                ground,
                round: Some(round),
                spawns: structural_spawns,
                changed,
                ground_changes,
                sequence_deltas,
                transition_packets,
                characters,
                native,
                leadership: None,
                refresh_assignments: true,
                audit,
            },
            now,
            wall,
        )
    }

    fn commit_acquisition_objective_candidate(
        &mut self,
        candidate: ObjectiveCommit,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let ObjectiveCommit {
            runtime,
            world,
            ground,
            round,
            spawns,
            changed,
            ground_changes,
            sequence_deltas,
            transition_packets,
            characters,
            mut native,
            leadership,
            refresh_assignments,
            mut audit,
        } = candidate;
        let packet = if changed.is_empty() {
            None
        } else {
            Some(
                replication::ObjectiveControlSnapshot {
                    sequence: 1,
                    timestamp_ms: now.as_millis() as u32,
                    objectives: changed.values().copied().collect(),
                }
                .encode_with_profile(self.wire_profile)?,
            )
        };
        let ground_packet = self.encode_ground_changes(&ground, &ground_changes, now)?;
        let sequence_packets = replication::ObjectiveSequenceDelta::encode_batches(
            &sequence_deltas.into_values().collect::<Vec<_>>(),
            now.as_millis() as u32,
            self.wire_profile,
        )?;
        let mut spawn_packets = Vec::new();
        let spawn_writes = if let Some(next) = &spawns {
            self.prepare_runtime_spawn_publication(
                self.spawn_catalog
                    .as_ref()
                    .ok_or("acquisition_structural_spawn_catalog_required")?,
                next,
                now.as_millis() as u32,
                &mut spawn_packets,
            )?
        } else {
            Vec::new()
        };
        self.check_acquisition_character_effects(&characters, &native)?;
        audit["effects"] = json!(characters.audit());
        audit["messages"] = json!(characters.messages());
        audit["objective_updates"] = json!(
            transition_packets
                .iter()
                .map(|p| protocol::encode_hex(p))
                .collect::<Vec<_>>()
        );
        audit["objectives"] = json!(
            changed
                .values()
                .map(|d| json!({"index":d.object_index,"state":d.state_actor.state as u8}))
                .collect::<Vec<_>>()
        );
        audit["spawn_updates"] = json!(
            spawn_packets
                .iter()
                .map(|p| protocol::encode_hex(p))
                .collect::<Vec<_>>()
        );
        audit["personal_spawn_updates"] = json!(
            spawn_writes
                .iter()
                .filter_map(|write| write
                    .packet
                    .as_ref()
                    .map(|p| json!({"player":write.player, "payload_hex":protocol::encode_hex(p)})))
                .collect::<Vec<_>>()
        );
        audit["objective_snapshot"] = json!(packet.as_ref().map(|p| protocol::encode_hex(p)));
        audit["sequence_updates"] = json!(
            sequence_packets
                .iter()
                .map(|p| protocol::encode_hex(p))
                .collect::<Vec<_>>()
        );
        audit["ground_update"] = json!(ground_packet.as_ref().map(|p| protocol::encode_hex(p)));
        self.record(audit, wall)?;
        let messages = self.install_character_effects_after_record(characters, &mut native);
        self.ground_runtime = Some(ground);
        self.objective_world = Some(world);
        if let Some(round) = round {
            self.acquisition_round = Some(round);
        }
        self.acquisition_gameplay = Some(runtime);
        if let Some(spawns) = spawns {
            self.spawn_catalog = Some(spawns);
        }
        if let Some(leadership) = leadership {
            self.commit_leadership(leadership);
            if let Some(active) = &mut self.active {
                active.last_event = now;
            }
        }
        self.install_runtime_spawn_publication(&spawn_writes);
        for packet in spawn_packets {
            self.broadcast_bot_packet(packet);
        }
        for packet in transition_packets {
            self.broadcast_bot_packet(packet);
        }
        if let Some(packet) = packet {
            self.broadcast_bot_packet(packet);
        }
        for packet in sequence_packets {
            self.broadcast_bot_packet(packet);
        }
        if let Some(packet) = ground_packet {
            self.broadcast_bot_packet(packet);
        }
        if refresh_assignments {
            for actor in self.active.iter_mut().chain(self.peers.values_mut()) {
                actor.objective_assigned = false;
                actor.objective_needs_clear = true;
            }
        }
        Ok(messages)
    }
}

fn human_objective_actors<W: Write>(engine: &Engine<W>) -> BTreeMap<u16, ObjectiveActor> {
    engine
        .active
        .iter()
        .chain(engine.peers.values())
        .filter_map(|a| {
            let CharacterLife::Alive { generation, .. } = a.life else {
                return None;
            };
            if !a.entered_game || !a.ready_for_updates || a.is_mounted() {
                return None;
            }
            Some((
                a.reservation.player,
                ObjectiveActor {
                    player: a.reservation.player,
                    team: a.team(),
                    generation,
                    position: a.tracking.sample.as_ref()?.position,
                },
            ))
        })
        .collect()
}

pub(crate) fn objective_actors<W: Write>(engine: &Engine<W>) -> BTreeMap<u16, ObjectiveActor> {
    let mut actors = human_objective_actors(engine);
    for (&id, bot) in &engine.bot_registry.bots {
        if let Some(r) = bot.target.runtime
            && bot.health > 0
        {
            actors.insert(
                id,
                ObjectiveActor {
                    player: id,
                    team: bot.target.squad.team(),
                    generation: r.generation,
                    position: r.position,
                },
            );
        }
    }
    actors
}

fn bot_capture_actor<W: Write>(
    engine: &Engine<W>,
    runtime: &Runtime,
    index: u16,
    source: BotCaptureSource,
    now: Duration,
) -> Option<ObjectiveActor> {
    if !runtime.started
        || source.round != engine.round_number
        || !engine.round_combat_allowed(now)
        || !engine.accepts_subscription_world(source.world)
        || engine
            .active
            .iter()
            .chain(engine.peers.values())
            .any(|actor| actor.reservation.player == source.player)
        || !engine.bot_registry.config.as_ref().is_some_and(|config| {
            config.autonomous
                && config
                    .objective_policy
                    .as_ref()
                    .is_some_and(|policy| policy.interactions_enabled)
        })
    {
        return None;
    }
    let bot = engine.bot_registry.bots.get(&source.player)?;
    let motion = bot.target.runtime?;
    let physics = motion.physics_motion?;
    let squad = runtime.state.squad(bot.target.squad.squad_index())?;
    if bot.allocation_generation != source.allocation
        || motion.generation != source.life
        || bot.target.squad.team() != source.team
        || squad.assignment.primary_index != index
        || !bot.task.is_operator_for(index, source.life)
        || bot.task.recovering
        || !matches!(motion.phase, crate::development_target::BotPhase::Complete)
        || physics.blocked
        || !crate::bot_objectives::operator_safe(bot)
    {
        return None;
    }
    let [x, y, z] = physics.position.map(|value| value as f32);
    Some(ObjectiveActor {
        player: source.player,
        team: source.team,
        generation: source.life,
        position: replication::CharacterPosition { x, y, z },
    })
}

fn take_capture_witness(
    runtime: &Runtime,
    token: u32,
    capture_witnesses: &mut BTreeMap<u32, CaptureWitness>,
    entry_actors: &mut BTreeMap<u32, u16>,
) -> Option<CaptureWitness> {
    let witness = capture_witnesses.remove(&token)?;
    if matches!(witness.completion.actor, Some(CaptureActor::Bot(_)))
        && runtime.state.target_version(token) != Some(witness.target_version)
    {
        // The newer controller target and its actor own entry.
        entry_actors.remove(&token);
        return None;
    }
    Some(witness)
}

fn resolve_bot_capture_completion<W: Write>(
    engine: &Engine<W>,
    runtime: &mut Runtime,
    world: &mut ObjectiveWorld,
    token: u32,
    source: BotCaptureSource,
    now: Duration,
    valid_entry: bool,
) -> Result<Option<ObjectiveControlDelta>, &'static str> {
    let index = runtime
        .state
        .objective(token)
        .ok_or("unknown_acquisition_gameplay_objective")?
        .replication
        .object_index;
    if valid_entry
        && bot_capture_actor(engine, runtime, index, source, now)
            .is_some_and(|actor| world.acquisition_capture_trigger(index, actor, source.trigger))
    {
        return Ok(None);
    }
    let delta = world.cancel_acquisition_bot_completion(index, now)?;
    runtime
        .state
        .request_state(token, ObjectiveState::Deactivated)?;
    Ok(Some(delta))
}

fn bot_capture_audit(operation: &str, index: u16, source: BotCaptureSource) -> Value {
    json!({
        "op":operation, "object":index, "world":source.world, "round":source.round,
        "player":source.player, "allocation":source.allocation, "life":source.life,
        "team":source.team, "trigger":source.trigger,
    })
}

fn stage_bot_captures<W: Write>(
    engine: &Engine<W>,
    runtime: &Runtime,
    world: &mut ObjectiveWorld,
    actors: &mut BTreeMap<u16, ObjectiveActor>,
    now: Duration,
    changed: &mut BTreeMap<u16, ObjectiveControlDelta>,
    audit: &mut Vec<Value>,
) {
    let world_id = engine
        .active
        .iter()
        .chain(engine.peers.values())
        .next()
        .map(|actor| actor.reservation.world);
    for (token, object) in runtime.state.objectives() {
        if object.fsp.is_none() || !runtime.state.has_callback(token) {
            continue;
        }
        let index = object.replication.object_index;
        if let Some(source) = world.acquisition_bot_capture(index) {
            if let Some(actor) = bot_capture_actor(engine, runtime, index, source, now)
                .filter(|actor| world.acquisition_capture_trigger(index, *actor, source.trigger))
            {
                actors.insert(source.player, actor);
            } else if let Some(delta) = world.cancel_acquisition_bot_capture(index, now) {
                changed.insert(index, delta);
                audit.push(bot_capture_audit(
                    "acquisition_bot_capture_cancelled",
                    index,
                    source,
                ));
            }
            continue;
        }
        let Some(world_id) = world_id else {
            continue;
        };
        if world.state(index) != Some(ObjectiveState::Deactivated) {
            continue;
        }
        let selected = engine.bot_registry.bots.iter().find_map(|(&player, bot)| {
            let mut source = BotCaptureSource {
                player,
                allocation: bot.allocation_generation,
                life: bot.target.runtime?.generation,
                team: bot.target.squad.team(),
                world: world_id,
                round: engine.round_number,
                trigger: 0,
            };
            if !bot.task.is_operator_for(index, source.life) {
                return None;
            }
            let actor = bot_capture_actor(engine, runtime, index, source, now)?;
            source.trigger = (0..world.acquisition_capture_trigger_count(index))
                .find(|&trigger| world.acquisition_capture_trigger(index, actor, trigger))?;
            let delta = world.start_acquisition_bot_capture(index, actor, source, now)?;
            Some((source, actor, delta))
        });
        if let Some((source, actor, delta)) = selected {
            actors.insert(source.player, actor);
            changed.insert(index, delta);
            audit.push(bot_capture_audit(
                "acquisition_bot_capture_started",
                index,
                source,
            ));
        }
    }
}

// Keep the established objective phase for default-off worlds; the opted-in
// native pass calls this after squad/platoon effects have changed actor state.
#[allow(clippy::too_many_arguments)]
fn stage_objective_observations<W: Write>(
    engine: &Engine<W>,
    runtime: &mut Runtime,
    world: &mut ObjectiveWorld,
    now: Duration,
    changed: &mut BTreeMap<u16, ObjectiveControlDelta>,
    entry_actors: &mut BTreeMap<u32, u16>,
    capture_witnesses: &mut BTreeMap<u32, CaptureWitness>,
    audit: &mut Vec<Value>,
) -> Result<(), &'static str> {
    let mut actors = human_objective_actors(engine);
    stage_bot_captures(engine, runtime, world, &mut actors, now, changed, audit);
    let (changes, captures) = world.tick_with_capture_completions(now, &actors);
    changed.extend(changes.into_iter().map(|d| (d.object_index, d)));
    for (token, object) in runtime.state.objectives().collect::<Vec<_>>() {
        if let Some(delta) = changed.get(&object.replication.object_index) {
            entry_actors.insert(token, delta.state_actor.actor_index);
        }
    }
    let observations: Vec<_> = runtime
        .state
        .objectives()
        .filter_map(|(token, object)| {
            changed
                .get(&object.replication.object_index)
                .map(|&delta| (token, delta))
        })
        .collect();
    for (token, delta) in observations {
        if matches!(
            delta.state_actor.state,
            ObjectiveState::Completed | ObjectiveState::Deactivated
        ) {
            runtime
                .state
                .request_state(token, delta.state_actor.state)?;
        } else {
            runtime
                .state
                .request_state(token, delta.state_actor.state)?;
            runtime.state.commit_state(
                token,
                delta.state_actor.state,
                delta.state_actor.actor_index,
                delta.state_actor.change_time_ms,
            )?;
        }
    }
    for completion in captures {
        if let Some(token) = runtime.state.token_for_index(completion.object_index)
            && changed.get(&completion.object_index).is_some_and(|delta| {
                delta.state_actor.state == ObjectiveState::Completed
                    && delta.state_actor.actor_index == completion.player
            })
            && let Some(target_version) = runtime.state.target_version(token)
        {
            capture_witnesses.insert(
                token,
                CaptureWitness {
                    completion,
                    target_version,
                },
            );
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn structural_fixture(
        runtime: &mut Runtime,
    ) -> &mut crate::acquisition_structural::Runtime {
        runtime.structural.as_mut().unwrap()
    }

    pub(crate) fn disable_structural_fixture(runtime: &mut Runtime) {
        Arc::get_mut(&mut runtime.setup).unwrap().structural = None;
        runtime.structural = None;
    }

    pub(crate) fn install_structural_fixture(
        runtime: &mut Runtime,
        setup: crate::acquisition_structural::Setup,
    ) {
        Arc::get_mut(&mut runtime.setup).unwrap().structural = Some(setup.clone());
        runtime.structural = Some(crate::acquisition_structural::Runtime::new(setup));
        runtime.started = true;
    }
}

#[cfg(test)]
mod capture_callback_tests {
    use super::*;
    use crate::spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::registered_capture_operator;

    #[test]
    fn newer_source_target_drops_a_stale_bot_capture_witness_and_actor() {
        let (mut engine, _) = registered_capture_operator();
        engine
            .tick_acquisition_objectives(Duration::from_millis(200), 2)
            .unwrap();
        let mut runtime = engine.acquisition_gameplay.clone().unwrap();
        let mut world = engine.objective_world.clone().unwrap();
        let mut changed = BTreeMap::new();
        let mut actors = BTreeMap::new();
        let mut witnesses = BTreeMap::new();
        stage_objective_observations(
            &engine,
            &mut runtime,
            &mut world,
            Duration::from_millis(3200),
            &mut changed,
            &mut actors,
            &mut witnesses,
            &mut Vec::new(),
        )
        .unwrap();
        let (&token, witness) = witnesses.first_key_value().unwrap();
        assert_eq!(actors.get(&token), Some(&witness.completion.player));
        let object_id = runtime.state.objective(token).unwrap().object_id();
        // The source Ground notification can request Completed again on the same slot.
        runtime
            .state
            .request_forced_fsp_deactivation(object_id, 0x0380_0000, 0)
            .unwrap();
        assert!(take_capture_witness(&runtime, token, &mut witnesses, &mut actors).is_none());
        assert!(!actors.contains_key(&token));
        assert_eq!(
            runtime.state.objective(token).unwrap().current_attacker,
            0x0380_0000
        );
        assert_eq!(
            runtime
                .state
                .pending_transitions()
                .find(|(id, _)| *id == token),
            Some((token, ObjectiveState::Completed as i32))
        );
    }

    #[test]
    fn lost_bot_after_clock_resets_capture_candidate_before_source_entry() {
        let (mut engine, player) = registered_capture_operator();
        engine
            .tick_acquisition_objectives(Duration::from_millis(200), 2)
            .unwrap();
        let mut runtime = engine.acquisition_gameplay.clone().unwrap();
        let mut world = engine.objective_world.clone().unwrap();
        let now = Duration::from_millis(3200);
        let mut changed = BTreeMap::new();
        let mut actors = BTreeMap::new();
        let mut witnesses = BTreeMap::new();
        let mut audit = Vec::new();
        stage_objective_observations(
            &engine,
            &mut runtime,
            &mut world,
            now,
            &mut changed,
            &mut actors,
            &mut witnesses,
            &mut audit,
        )
        .unwrap();
        let (token, witness) = witnesses.pop_first().unwrap();
        let index = witness.completion.object_index;
        let Some(CaptureActor::Bot(source)) = witness.completion.actor else {
            panic!("expected an exact registered bot completion");
        };
        assert_eq!(
            runtime.state.target_version(token),
            Some(witness.target_version)
        );
        assert_eq!(world.state(index), Some(ObjectiveState::Completed));
        engine.bot_registry.bots.get_mut(&player).unwrap().health = 0;
        let delta = resolve_bot_capture_completion(
            &engine,
            &mut runtime,
            &mut world,
            token,
            source,
            now,
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(delta.state_actor.actor_index, u16::MAX);
        assert_eq!(world.state(index), Some(ObjectiveState::Deactivated));
        assert_eq!(world.interaction_owner(index), None);
        assert_eq!(
            runtime
                .state
                .pending_transitions()
                .find(|(target, _)| *target == token),
            Some((token, ObjectiveState::Deactivated as i32))
        );
        assert_eq!(
            engine
                .ground_runtime
                .as_ref()
                .unwrap()
                .get(4)
                .unwrap()
                .state,
            0
        );
    }
}
