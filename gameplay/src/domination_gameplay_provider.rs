//! Engine-owned effects for the source-bound Domination objective controller.
use crate::{
    CharacterLife, Engine, OutboundMessage,
    acquisition_character_effects::NativeState,
    acquisition_ground_runtime as ground,
    acquisition_spawn_effects::Clearance,
    acquisition_world::Settings,
    domination_callbacks::Context,
    domination_control::Ability,
    domination_fsp::{VehicleHook, VehicleRequest},
    domination_gameplay_effects::{self, OwnedAssignment, Structural},
    domination_gameplay_engine::{EffectsCommit, ExternalEffects, Runtime},
    domination_objective_runtime::Actor,
    domination_structural_world::{
        AreaDamageState, AreaUpdate, BlastCollision, BlastSource, DamageCommit, DamageWork,
        HospitalHulkAssets, SequenceCollisionAssets, SequenceCollisionState,
    },
    dynamic_collision::{Pose, QueryLimits},
    objective_catalog::ObjectiveCatalog,
    objective_control::{ActionTimingModifiers, NativeEntry, ObjectiveActor, ObjectiveWorld},
    replication::{MatchPhase, ObjectiveSequenceDelta, ObjectiveState, SquadObjectiveAssignment},
    spawn_catalog::SpawnCatalog,
    structures::BunkerWorld,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    sync::Arc,
    time::Duration,
};

#[derive(Clone)]
pub(crate) struct World {
    pub(crate) runtime: Runtime,
    pub(crate) state: State,
}

impl World {
    pub(crate) fn validate_human_static_resupply(
        &self,
        spawns: &SpawnCatalog,
        collision: Option<&crate::collision::CollisionWorld>,
        items: &crate::item_catalog::ItemCatalog,
    ) -> Result<(), &'static str> {
        if let Some(catalog) = &self.state.human_static_resupply {
            catalog.bind(spawns, collision, &self.state.objectives, items)?;
        }
        Ok(())
    }

    pub(crate) fn has_frag_ground_areas(&self) -> bool {
        self.state.frag_ground_areas.is_some()
    }

    pub(crate) fn has_helo_frag_damage_areas(&self) -> bool {
        self.state.helo_frag_damage_areas.is_some()
    }

    pub(crate) fn has_hospital_blast_areas(&self) -> bool {
        self.state.hospital_blast_areas.is_some()
    }

    pub(crate) fn validate_hospital_blast(
        &self,
        items: &crate::item_catalog::ItemCatalog,
        collision: Option<&crate::collision::CollisionWorld>,
        map: &str,
    ) -> Result<(), &'static str> {
        if !self.has_hospital_blast_areas() {
            return Ok(());
        }
        if self.state.hospital_hulks.is_none() {
            return Err("domination_hospital_blast_requires_body_sources");
        }
        self.validate_vehicle_frag_scene(
            items,
            collision,
            map,
            "domination_hospital_blast_requires_static_world",
            "domination_hospital_blast_collision_filter_required",
        )
    }

    pub(crate) fn validate_frag_ground(
        &self,
        items: &crate::item_catalog::ItemCatalog,
        collision: Option<&crate::collision::CollisionWorld>,
        map: &str,
    ) -> Result<(), &'static str> {
        if !self.has_frag_ground_areas() {
            return Ok(());
        }
        self.validate_vehicle_frag_scene(
            items,
            collision,
            map,
            "domination_frag_ground_requires_static_world",
            "domination_frag_ground_collision_filter_required",
        )
    }

    pub(crate) fn validate_frag_helo(
        &self,
        items: &crate::item_catalog::ItemCatalog,
        collision: Option<&crate::collision::CollisionWorld>,
        map: &str,
    ) -> Result<(), &'static str> {
        if !self.has_helo_frag_damage_areas() {
            return Ok(());
        }
        if self.state.helo.is_none()
            || self.state.helo_collision.is_none()
            || self.state.helo_damage.is_none()
        {
            return Err("domination_frag_helo_requires_fleet_sources");
        }
        self.validate_vehicle_frag_scene(
            items,
            collision,
            map,
            "domination_frag_helo_requires_static_world",
            "domination_frag_helo_collision_filter_required",
        )
    }

    fn validate_vehicle_frag_scene(
        &self,
        items: &crate::item_catalog::ItemCatalog,
        collision: Option<&crate::collision::CollisionWorld>,
        map: &str,
        missing_world: &'static str,
        missing_filter: &'static str,
    ) -> Result<(), &'static str> {
        let collision = collision
            .filter(|world| world.stats().triangles != 0 && world.metadata().map_name == map)
            .ok_or(missing_world)?;
        crate::domination_gameplay_config::validate_frag_character_inputs(
            items,
            Some(collision),
            map,
        )?;
        if [6, 24]
            .into_iter()
            .any(|row| self.state.filter_rows[row] & (1 << 22) == 0)
        {
            return Err(missing_filter);
        }
        collision.validate_material_ray_bindings(
            self.state.filter_rows[24],
            &self
                .state
                .structural
                .material_penetration_costs()
                .keys()
                .copied()
                .collect(),
            Some(&self.state.terrain_materials.material_slots()),
        )
    }

    pub(crate) fn validate_frag_ground_dynamic(
        &self,
        catalog: &crate::dynamic_collision::Catalog,
        bindings: &crate::dynamic_collision_runtime::RuntimeBindings,
        ground: &crate::ground_vehicles::Catalog,
    ) -> Result<(), &'static str> {
        if let Some(areas) = &self.state.frag_ground_areas {
            areas.validate_dynamic_parts(catalog, bindings, ground)?;
        }
        Ok(())
    }

    pub(crate) fn frag_blast_collision(&self) -> BlastCollision<'_> {
        BlastCollision {
            filter_rows: &self.state.filter_rows,
            limits: QueryLimits::default(),
            sequences: &self.state.collisions,
            material_costs: self.state.structural.material_penetration_costs(),
            terrain_materials: Some(&self.state.terrain_materials),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.runtime.reset();
        self.state.structural.reset();
        if let Some(fleet) = self.state.helo.as_mut() {
            fleet.reset();
        }
        self.state.collisions = (*self.state.initial_collisions).clone();
        self.state.area = AreaDamageState::default();
        self.state.vehicle_waves = None;
        self.state.last_ground_update = None;
        self.state.ability_enabled = [u16::MAX; 8];
        self.state.assignments = std::array::from_fn(|i| {
            OwnedAssignment::initial(&self.state.objectives, i as u16).ok()
        });
    }

    pub(crate) fn configure_ground(
        &self,
        runtime: &mut ground::Runtime,
    ) -> Result<(), &'static str> {
        crate::acquisition_gameplay_config::configure_ground(
            runtime,
            &self.state.ground_capabilities,
        )
    }

    pub(crate) fn current_sequences(&self) -> Vec<ObjectiveSequenceDelta> {
        self.state.structural.current_sequences()
    }

    pub(crate) fn assigned_objective(&self, squad: u16) -> Option<u16> {
        Some(self.squad_assignment(squad)?.primary_index)
    }

    pub(crate) fn squad_assignment(
        &self,
        squad: u16,
    ) -> Option<crate::acquisition_objective_runtime::Assignment> {
        Some(
            self.state
                .assignments
                .get(usize::from(squad))?
                .as_ref()?
                .assignment,
        )
    }

    pub(crate) fn set_manual_tag(
        &mut self,
        squad: u16,
        target: u32,
        world: &ObjectiveWorld,
        spawns: &SpawnCatalog,
        phase: MatchPhase,
    ) -> Result<crate::acquisition_objective_runtime::Assignment, &'static str> {
        let slot = self
            .state
            .assignments
            .get_mut(usize::from(squad))
            .and_then(Option::as_mut)
            .ok_or("domination_assignment_slot_missing")?;
        *slot =
            slot.with_manual_tag(squad, target, &self.state.objectives, world, spawns, phase)?;
        Ok(slot.assignment)
    }
}

#[derive(Clone)]
pub(crate) struct State {
    pub(crate) objectives: Arc<ObjectiveCatalog>,
    pub(crate) structural: Structural,
    pub(crate) assets: Arc<SequenceCollisionAssets>,
    pub(crate) hospital_hulks: Option<Arc<HospitalHulkAssets>>,
    pub(crate) hospital_blast_areas: Option<Arc<crate::hospital_blast_areas::Catalog>>,
    pub(crate) hospital_turret_yaws: Arc<BTreeMap<u16, f32>>,
    pub(crate) hospital_rocket_ground: Option<Arc<crate::hospital_rocket_ground::Catalog>>,
    pub(crate) hospital_rocket_helo_areas: Option<Arc<crate::hospital_rocket_helo::Catalog>>,
    pub(crate) helo_frag_damage_areas: Option<Arc<crate::hospital_rocket_helo::Catalog>>,
    pub(crate) frag_character_blast: bool,
    pub(crate) human_static_resupply: Option<Arc<crate::human_static_resupply::Catalog>>,
    pub(crate) frag_ground_areas: Option<Arc<crate::acquisition_frag_ground::Catalog>>,
    pub(crate) projectile_contacts:
        Option<Arc<crate::domination_projectile_collision::ContactBindings>>,
    pub(crate) helo: Option<crate::domination_helo::FleetRuntime>,
    pub(crate) helo_collision: Option<Arc<crate::domination_structural_world::HeloCollisionAssets>>,
    pub(crate) helo_damage: Option<Arc<crate::domination_helo_damage::HeloDamageRules>>,
    pub(crate) root_poses: Arc<BTreeMap<u16, Pose>>,
    pub(crate) trigger_positions: Arc<BTreeMap<(u16, u8), [f32; 3]>>,
    pub(crate) filter_rows: [u32; 32],
    pub(crate) terrain_materials: Arc<crate::domination_structural_effects::TerrainMaterialGrid>,
    pub(crate) ground_respawn_physics: Option<Settings>,
    pub(crate) ground_damage: Arc<crate::acquisition_ground_damage::Policy>,
    pub(crate) ground_capabilities: Arc<[(u16, ground::Capabilities)]>,
    pub(crate) ground_material_penetration: f32,
    pub(crate) vehicle_waves: Option<crate::acquisition_vehicle_waves::VehicleWaves>,
    pub(crate) last_ground_update: Option<Duration>,
    pub(crate) known_empty_vehicle_spawns: Arc<BTreeSet<u16>>,
    pub(crate) disabled_helo_spawns: Arc<BTreeSet<u16>>,
    pub(crate) platoons: Arc<BTreeMap<u32, u8>>,
    pub(crate) assignments: [Option<OwnedAssignment>; 32],
    pub(crate) ability_enabled: [u16; 8],
    pub(crate) collisions: SequenceCollisionState,
    pub(crate) initial_collisions: Arc<SequenceCollisionState>,
    pub(crate) area: AreaDamageState,
}

struct Provider<'a, W: Write> {
    engine: &'a mut Engine<W>,
    native: &'a mut BTreeMap<u32, NativeState>,
    state: State,
    bunkers: Option<BunkerWorld>,
    ground: Option<ground::Runtime>,
    phase: MatchPhase,
    now: Duration,
    audit: Vec<Value>,
    messages: Vec<OutboundMessage>,
    packets: Vec<Vec<u8>>,
}

impl<W: Write> Provider<'_, W> {
    fn prepare_active_abilities(&mut self, world: &mut ObjectiveWorld) -> Result<(), &'static str> {
        if self.engine.active_abilities.is_none() {
            return Ok(());
        }
        let previous_ground =
            std::mem::replace(&mut self.engine.ground_runtime, self.ground.clone());
        let previous_bunkers =
            std::mem::replace(&mut self.engine.bunker_world, self.bunkers.clone());
        let result = self.engine.stage_active_ability_update_in_world(
            self.now,
            self.native,
            BlastCollision {
                filter_rows: &self.state.filter_rows,
                limits: QueryLimits::default(),
                sequences: &self.state.collisions,
                material_costs: self.state.structural.material_penetration_costs(),
                terrain_materials: Some(&self.state.terrain_materials),
            },
            Some(&self.state.ground_damage),
            Some(world),
        );
        self.ground = std::mem::replace(&mut self.engine.ground_runtime, previous_ground);
        self.bunkers = std::mem::replace(&mut self.engine.bunker_world, previous_bunkers);
        let DamageCommit {
            audit,
            messages,
            changed_ground,
            helo_doses,
        } = result?;
        self.stage_helo_radial(&helo_doses)?;
        if !changed_ground.is_empty() {
            let changes = changed_ground
                .into_iter()
                .map(|i| (i, 1 << 18))
                .collect::<Vec<_>>();
            self.packets
                .push(crate::ground_vehicles::encode_runtime_delta(
                    self.engine
                        .ground_vehicles
                        .as_ref()
                        .ok_or("domination_ground_catalog_required")?,
                    self.ground
                        .as_ref()
                        .ok_or("domination_ground_world_required")?,
                    &changes,
                    2,
                    self.now.as_millis() as u32,
                )?);
        }
        self.audit.extend(audit);
        self.messages.extend(messages);
        Ok(())
    }

    fn prepare_ground_sim(
        &mut self,
        spawns: &crate::spawn_catalog::SpawnCatalog,
    ) -> Result<(), &'static str> {
        let physics = self
            .state
            .ground_respawn_physics
            .ok_or("domination_ground_respawn_physics_required")?;
        let ground = self
            .ground
            .as_mut()
            .ok_or("ground_vehicle_runtime_required")?;
        let base = spawns.match_profile().spawn_wave_ms;
        let intervals = [
            self.engine.strategic_spawn_interval(0, base[0])?,
            self.engine.strategic_spawn_interval(1, base[1])?,
        ];
        if let Some(waves) = &mut self.state.vehicle_waves {
            // Sample the modifiers before the later Company update can expire
            // them. Both Ground and Helicopter keep their current shared stamp.
            waves.set_intervals_ms(intervals)?;
        } else {
            let epoch = self.engine.match_epoch.unwrap_or(self.now).as_millis() as u32;
            self.state.vehicle_waves = Some(crate::acquisition_vehicle_waves::VehicleWaves::start(
                epoch, intervals,
            )?);
        }
        let dt = self
            .state
            .last_ground_update
            .map_or(Ok(Duration::ZERO), |last| {
                self.now
                    .checked_sub(last)
                    .ok_or("domination_ground_update_time_regression")
            })?
            .as_secs_f32();
        let waves = self.state.vehicle_waves.as_mut().unwrap();
        let mut changes = BTreeMap::new();
        let update = self.engine.stage_ground_updates(
            crate::acquisition_ground_effects::GroundFrame {
                now: self.now,
                dt,
                waves,
                policy: Some(&self.state.ground_damage),
                filter_row: self.state.filter_rows[6],
                limits: physics.query_limits,
                domination_blast: Some((
                    BlastCollision {
                        filter_rows: &self.state.filter_rows,
                        limits: physics.query_limits,
                        sequences: &self.state.collisions,
                        material_costs: self
                            .state
                            .structural
                            .catalog()
                            .material_penetration_costs(),
                        terrain_materials: Some(&self.state.terrain_materials),
                    },
                    self.state.ground_material_penetration,
                )),
            },
            ground,
            self.native,
            &mut changes,
        )?;
        if !update.forced_fsp.is_empty() {
            return Err("unsupported_domination_ground_forced_fsp");
        }
        self.audit.extend(update.audit);
        self.messages.extend(update.messages);
        if let Some(packet) = self
            .engine
            .encode_ground_changes(ground, &changes, self.now)?
        {
            self.packets.push(packet);
        }
        self.stage_helo_radial(&update.helo_doses)?;
        self.state.last_ground_update = Some(self.now);
        Ok(())
    }

    fn prepare_ground_waves(
        &mut self,
        spawns: &crate::spawn_catalog::SpawnCatalog,
    ) -> Result<(), &'static str> {
        if self.phase != MatchPhase::Playing {
            return Ok(());
        }
        let mut changes = BTreeMap::new();
        let ground = self
            .ground
            .as_mut()
            .ok_or("ground_vehicle_runtime_required")?;
        let (audit, messages) = self.engine.stage_ground_vehicle_waves(
            crate::acquisition_vehicle_wave_effects::WaveFrame {
                waves: self
                    .state
                    .vehicle_waves
                    .as_mut()
                    .ok_or("domination_ground_waves_unavailable")?,
                physics: self
                    .state
                    .ground_respawn_physics
                    .ok_or("domination_ground_respawn_physics_required")?,
                filter_rows: self.state.filter_rows,
                staged_spawns: Some(spawns),
                now: self.now,
            },
            ground,
            self.native,
            &mut changes,
        )?;
        self.audit.extend(audit);
        self.messages.extend(messages);
        if let Some(packet) = self
            .engine
            .encode_ground_changes(ground, &changes, self.now)?
        {
            self.packets.push(packet);
        }
        Ok(())
    }
}

impl<W: Write> Provider<'_, W> {
    fn prepare_hospital_damage_presence(&mut self) -> Result<(), &'static str> {
        let Some(bunkers) = self.bunkers.as_mut() else {
            return Ok(());
        };
        for (update, attribution) in bunkers.advance_domination_hospital_damage(self.now)? {
            self.stage_hospital_update(update, attribution)?;
        }
        Ok(())
    }

    fn stage_hospital_update(
        &mut self,
        update: crate::structures::DominationBunkerUpdate,
        damage_id: u32,
    ) -> Result<(), &'static str> {
        let mut bunkers = self
            .bunkers
            .take()
            .ok_or("domination_hospital_world_required")?;
        let result = self.stage_hospital_presence(&mut bunkers, update, damage_id);
        self.bunkers = Some(bunkers);
        result
    }

    fn stage_hospital_presence(
        &mut self,
        bunkers: &mut BunkerWorld,
        update: crate::structures::DominationBunkerUpdate,
        damage_id: u32,
    ) -> Result<(), &'static str> {
        if update.kills_occupants() {
            let previous = self.engine.bunker_world.replace(bunkers.clone());
            let result = self.engine.stage_bunker_occupants(
                update.object_index(),
                damage_id,
                self.native,
                self.now,
            );
            self.engine.bunker_world = previous;
            let (audit, messages) = result?;
            self.audit.extend(audit);
            self.messages.extend(messages);
        }
        if update.restores_turret() {
            let assets = self
                .state
                .hospital_hulks
                .as_ref()
                .ok_or("domination_hospital_hulk_assets_required")?;
            let bunker = bunkers
                .domination_bunker(update.object_index())
                .ok_or("domination_hospital_body_source_missing")?;
            self.state
                .collisions
                .install_operational_hospital(bunker, assets)?;
        }
        if update.destroys_turret() {
            let assets = self
                .state
                .hospital_hulks
                .as_ref()
                .cloned()
                .ok_or("domination_hospital_hulk_assets_required")?;
            let yaw = bunkers
                .hospital_turrets
                .get(&update.object_index())
                .and_then(crate::hospital_turrets::Runtime::yaw)
                .or_else(|| {
                    self.state
                        .hospital_turret_yaws
                        .get(&update.object_index())
                        .copied()
                })
                .ok_or("domination_hospital_turret_yaw_required")?;
            let bunker = bunkers
                .domination_bunker(update.object_index())
                .ok_or("domination_hospital_hulk_source_missing")?;
            let faction = bunker.faction;
            let transform = bunker
                .hulk_transform(yaw)
                .ok_or("domination_hospital_hulk_pose_invalid")?;
            let origin = [
                transform.position.x,
                transform.position.y,
                transform.position.z,
            ];
            let blast = assets.hospital_blast(faction, origin)?;
            let previous_ground =
                std::mem::replace(&mut self.engine.ground_runtime, self.ground.clone());
            let previous_bunkers = self.engine.bunker_world.replace(bunkers.clone());
            let result = (|| {
                let work = self.engine.collect_domination_blast(
                    BlastSource::Hospital {
                        body_id: 0x0d80_0000 | u32::from(update.object_index()),
                    },
                    damage_id,
                    blast,
                    BlastCollision {
                        filter_rows: &self.state.filter_rows,
                        limits: QueryLimits::default(),
                        sequences: &self.state.collisions,
                        material_costs: self.state.structural.material_penetration_costs(),
                        terrain_materials: Some(&self.state.terrain_materials),
                    },
                )?;
                self.engine.stage_domination_damage_with_ground(
                    &mut self.state.area,
                    self.native,
                    self.now,
                    &[work],
                    Some(&self.state.ground_damage),
                )
            })();
            self.ground = std::mem::replace(&mut self.engine.ground_runtime, previous_ground);
            *bunkers = std::mem::replace(&mut self.engine.bunker_world, previous_bunkers)
                .ok_or("domination_hospital_blast_world_missing")?;
            let DamageCommit {
                audit,
                messages,
                changed_ground,
                helo_doses,
            } = result?;
            self.stage_helo_radial_with_catalog(&helo_doses, bunkers.damage_catalog())?;
            if !changed_ground.is_empty() {
                let changes = changed_ground
                    .into_iter()
                    .map(|index| (index, 1 << 18))
                    .collect::<Vec<_>>();
                self.packets
                    .push(crate::ground_vehicles::encode_runtime_delta(
                        self.engine
                            .ground_vehicles
                            .as_ref()
                            .ok_or("domination_ground_catalog_required")?,
                        self.ground
                            .as_ref()
                            .ok_or("domination_ground_world_required")?,
                        &changes,
                        2,
                        self.now.as_millis() as u32,
                    )?);
            }
            self.audit.extend(audit);
            self.messages.extend(messages);
            self.state
                .collisions
                .remove_operational_hospital(update.object_index());
            let [_, sin, _, cos] = transform.rotation_xyzw.map(f64::from);
            self.state.collisions.install_hospital_hulk(
                update.object_index(),
                faction,
                Pose {
                    translation: [
                        transform.position.x,
                        transform.position.y,
                        transform.position.z,
                    ]
                    .map(f64::from),
                    rotation: [
                        [1.0 - 2.0 * sin * sin, 0., 2.0 * sin * cos],
                        [0., 1., 0.],
                        [-2.0 * sin * cos, 0., 1.0 - 2.0 * sin * sin],
                    ],
                },
                &assets,
            )?;
        }
        if let Some(packet) =
            update.encode(1, self.now.as_millis() as u32, self.engine.wire_profile)?
        {
            self.packets.push(packet);
        }
        Ok(())
    }

    fn stage_helo_radial(
        &mut self,
        doses: &[crate::domination_helo_damage::RadialDose],
    ) -> Result<(), &'static str> {
        if doses.is_empty() {
            return Ok(());
        }
        let catalog = self
            .bunkers
            .as_ref()
            .ok_or("domination_helo_armor_catalog_required")?
            .damage_catalog()
            .clone();
        self.stage_helo_radial_with_catalog(doses, &catalog)
    }

    fn stage_helo_radial_with_catalog(
        &mut self,
        doses: &[crate::domination_helo_damage::RadialDose],
        catalog: &crate::structure_catalog::StructureCatalog,
    ) -> Result<(), &'static str> {
        if doses.is_empty() {
            return Ok(());
        }
        let fleet = self
            .state
            .helo
            .as_mut()
            .ok_or("domination_helo_world_required")?;
        let rules = self
            .state
            .helo_damage
            .as_deref()
            .ok_or("domination_helo_damage_rules_required")?;
        let (packets, audit) = self
            .engine
            .stage_helo_radial_doses(fleet, rules, catalog, doses, self.now)?;
        self.packets.extend(packets);
        self.audit.extend(audit);
        Ok(())
    }

    fn prepare_helo_damage(&mut self) -> Result<(), &'static str> {
        use crate::domination_helo::{FlightState, HeloUpdate};
        use crate::domination_helo_damage::{HullTransition, damage_delta};
        let Some(fleet) = self.state.helo.as_ref() else {
            return Ok(());
        };
        let dt = fleet
            .last_tick
            .map_or(0., |last| self.now.saturating_sub(last).as_secs_f32());
        let rules = self
            .state
            .helo_damage
            .clone()
            .ok_or("domination_helo_damage_rules_required")?;
        let thresholds = self
            .bunkers
            .as_ref()
            .ok_or("domination_helo_armor_catalog_required")?
            .damage_catalog()
            .rules();
        let wave = self
            .state
            .vehicle_waves
            .as_ref()
            .ok_or("domination_helo_spawn_waves_required")?
            .next_timestamp(0)?;
        let count = fleet.carriers.len();
        for slot in 0..count {
            let carrier = &mut self.state.helo.as_mut().unwrap().carriers[slot];
            let profile = rules.profile(carrier.source.faction)?;
            let index = carrier.source.object_index;
            if carrier.observed_wave.replace(wave) == Some(wave) {
                carrier.spawn_cycle_age += dt;
            } else {
                carrier.spawn_cycle_age = 0.;
                if carrier
                    .respawn_wave
                    .is_some_and(|previous| previous != wave)
                {
                    carrier.respawn_wave = None;
                }
            }
            let changed = carrier.damage.tick(profile, thresholds, dt)?;
            let transition = carrier.damage.update_state(profile, dt)?;
            let wire = carrier.damage.wire;
            if changed
                || matches!(
                    transition,
                    HullTransition::Critical
                        | HullTransition::Destroyed
                        | HullTransition::Recovered
                )
            {
                let state = matches!(
                    transition,
                    HullTransition::Critical | HullTransition::Destroyed
                )
                .then_some(carrier.damage.state);
                self.packets.push(damage_delta(
                    index,
                    state,
                    Some(wire),
                    self.now,
                    self.engine.wire_profile,
                )?);
            }
            match transition {
                HullTransition::Destroyed => {
                    let position = carrier
                        .current_pose
                        .ok_or("domination_helo_destroyed_pose_missing")?
                        .position;
                    let passengers = carrier.motion.passengers().collect::<Vec<_>>();
                    self.stage_helo_destruction(
                        index,
                        position,
                        &passengers,
                        wire.last_attacker,
                        profile.destruction,
                    )?;
                }
                HullTransition::Recovered => {
                    carrier.motion.reset_destroyed_flight();
                    carrier.respawn_wave = Some(wave);
                    carrier.current_pose = None;
                    carrier.path_elapsed = 0;
                    carrier.published_state = FlightState::Dormant;
                    carrier.live_basis = None;
                    self.packets.push(
                        HeloUpdate {
                            object_index: index,
                            path_elapsed: Some(0),
                            flight_state: Some(FlightState::Dormant),
                        }
                        .encode(
                            2,
                            self.now.as_millis() as u32,
                            self.engine.wire_profile,
                        )?,
                    );
                }
                HullTransition::None
                    if carrier.spawn_cycle_age > 1.
                        && carrier.spawn_cycle_age < 2.
                        && carrier.damage.wants_retreat(profile) =>
                {
                    carrier.motion.request_retreat_if_hovering();
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn stage_helo_destruction(
        &mut self,
        index: u16,
        position: crate::replication::CharacterPosition,
        passengers: &[(u8, u16)],
        attacker: u32,
        destruction: crate::domination_helo_damage::Destruction,
    ) -> Result<(), &'static str> {
        use crate::acquisition_character_effects::{Request, Target};
        use crate::domination_structural_effects::InstantaneousBlast;
        let origin = [position.x, position.y, position.z];
        for &(seat, player) in passengers {
            let actor = self
                .engine
                .active
                .iter()
                .chain(self.engine.peers.values())
                .find(|actor| {
                    actor.reservation.player == player
                        && actor.helo_seat
                            == Some(crate::domination_helo::PassengerSeat {
                                object_index: index,
                                seat,
                            })
                })
                .ok_or("domination_helo_destroyed_rider_missing")?;
            let Some((generation, _)) = actor.life.damage_health() else {
                continue;
            };
            let target = Target {
                character_id: 0x0380_0000 | u32::from(player),
                generation,
                world: actor.reservation.world,
            };
            let state = self
                .native
                .get_mut(&target.character_id)
                .ok_or("domination_helo_destroyed_rider_state_missing")?;
            let plan = self.engine.plan_acquisition_character_effect(
                target,
                Request::VehicleDeathOccupant {
                    last_attacker: attacker,
                    vehicle_vector: origin,
                },
                *state,
                self.now,
            )?;
            self.audit.push(json!({"op":"domination_helo_occupant_damage","index":index,"seat":seat,"effect":plan.audit()}));
            self.messages.extend(
                self.engine
                    .commit_acquisition_character_effect(plan, state)?,
            );
        }
        let before_ground = std::mem::replace(&mut self.engine.ground_runtime, self.ground.clone());
        let before_bunkers = std::mem::replace(&mut self.engine.bunker_world, self.bunkers.clone());
        let outcome = (|| {
            let work = self.engine.collect_domination_blast(
                BlastSource::Helo {
                    body_id: 0x0280_0000 | u32::from(index),
                },
                attacker,
                InstantaneousBlast {
                    origins: vec![origin],
                    inner_radius: destruction.inner_radius,
                    outer_radius: destruction.outer_radius,
                    damage: destruction.damage,
                    material_penetration: destruction.material_penetration,
                },
                BlastCollision {
                    filter_rows: &self.state.filter_rows,
                    limits: QueryLimits::default(),
                    sequences: &self.state.collisions,
                    material_costs: self.state.structural.material_penetration_costs(),
                    terrain_materials: Some(&self.state.terrain_materials),
                },
            )?;
            self.engine.stage_domination_damage_with_ground(
                &mut self.state.area,
                self.native,
                self.now,
                &[work],
                Some(&self.state.ground_damage),
            )
        })();
        self.ground = std::mem::replace(&mut self.engine.ground_runtime, before_ground);
        self.bunkers = std::mem::replace(&mut self.engine.bunker_world, before_bunkers);
        let DamageCommit {
            audit,
            messages,
            changed_ground,
            helo_doses,
        } = outcome?;
        self.audit.extend(audit);
        self.messages.extend(messages);
        if !changed_ground.is_empty() {
            let changes = changed_ground
                .into_iter()
                .map(|index| (index, 1 << 18))
                .collect();
            if let Some(packet) = self.engine.encode_ground_changes(
                self.ground
                    .as_ref()
                    .ok_or("domination_ground_world_required")?,
                &changes,
                self.now,
            )? {
                self.packets.push(packet);
            }
        }
        self.stage_helo_radial(&helo_doses)?;
        self.state.collisions.remove_helo(index);
        Ok(())
    }

    fn prepare_helo_flight(&mut self) -> Result<(), &'static str> {
        let Some(fleet) = self.state.helo.as_mut() else {
            return Ok(());
        };
        let assets = self
            .state
            .helo_collision
            .as_deref()
            .ok_or("domination_helo_collision_assets_required")?;
        let now = self.now;
        let dt = fleet
            .last_tick
            .replace(now)
            .map_or(0.0, |previous| now.saturating_sub(previous).as_secs_f32());
        let active = self
            .engine
            .active
            .iter()
            .chain(self.engine.peers.values())
            .filter(|actor| {
                actor.helo_seat.is_some()
                    && !matches!(
                        actor.life,
                        CharacterLife::AwaitingFirstSpawn | CharacterLife::AwaitingRespawn { .. }
                    )
            })
            .map(|actor| actor.reservation.player)
            .collect::<BTreeSet<_>>();
        let stale = fleet
            .carriers
            .iter()
            .flat_map(|carrier| carrier.motion.passengers())
            .map(|(_, player)| player)
            .filter(|player| !active.contains(player))
            .collect::<Vec<_>>();
        for player in stale {
            fleet.remove_passenger(player);
        }
        for carrier in &mut fleet.carriers {
            if carrier.damage.state == crate::replication::VehicleState::Destroyed
                && (carrier.damage.wire.health_quarter_points == 0
                    || carrier.respawn_wave.is_some())
            {
                continue;
            }
            let (changed, exited) = self.engine.stage_helo_frame(
                carrier,
                crate::domination_helo_engine::HeloFrame {
                    dt,
                    row6: self.state.filter_rows[6],
                    now,
                    collisions: &mut self.state.collisions,
                    assets,
                    random: &mut fleet.random,
                },
            )?;
            for player in exited {
                let value = self
                    .native
                    .get_mut(&(0x0380_0000 | u32::from(player)))
                    .ok_or("domination_helo_rider_native_state_required")?;
                value.associated_vehicle =
                    Some(crate::protocol::VictimWasMounted::NoAssociatedVehicle);
            }
            if changed {
                self.audit.push(serde_json::json!({"op":"domination_helo_flight", "object": carrier.source.object_index,
                                                    "path_elapsed":carrier.path_elapsed}));
            }
        }
        fleet.schedule_ended()
    }

    fn prepare_ground_vehicle(
        &mut self,
        request: VehicleRequest,
        objective: u32,
    ) -> Result<bool, &'static str> {
        let spawn = request.point.native_id as u16;
        let Some(runtime) = self.ground.as_mut() else {
            return Ok(false);
        };
        let Some(vehicle) = runtime
            .vehicles()
            .find(|vehicle| vehicle.source_spawn_point_index == spawn)
        else {
            return Ok(false);
        };
        if vehicle.source_spawn_point_token != request.point.token
            || vehicle.source_spawn_type != Some(7)
        {
            return Err("domination_motor_pool_binding_mismatch");
        }
        let key = vehicle.key;
        if request.hook == VehicleHook::OnFspDeactivate {
            runtime.disable_cycle(key)?;
            return Ok(true);
        }
        let catalog = self
            .engine
            .ground_vehicles
            .as_ref()
            .ok_or("domination_ground_catalog_required")?;
        let creation = catalog
            .vehicle(key.index)
            .ok_or("domination_ground_vehicle_missing")?;
        if creation.spawn_point_index != spawn || creation.spawn_point_token != request.point.token
        {
            return Err("domination_motor_pool_creation_mismatch");
        }
        runtime.enable_cycle(key, objective)?;
        let events = runtime.respawn_existing(
            key,
            ground::QualifiedRespawn {
                spawn_point_index: spawn,
                spawn_point_token: request.point.token,
                active: true,
                pose: ground::Pose {
                    position: creation.position,
                    rotation: creation.rotation,
                },
            },
            false,
        )?;
        let mut changes = BTreeMap::new();
        for event in events {
            self.audit.push(json!({"op":"domination_motor_pool_effect","vehicle":event.vehicle.object_id(),"effect":format!("{:?}",event.effect)}));
            let field = match event.effect {
                ground::Effect::ForcePosition(_) => 1 << 25,
                ground::Effect::ForceRotation(_) => 1 << 26,
                ground::Effect::DamageReset => 1 << 18,
                ground::Effect::StateChanged(_) => 1 << 5,
                ground::Effect::KillSpawnBlockers => {
                    let physics = self
                        .state
                        .ground_respawn_physics
                        .ok_or("domination_motor_pool_clearance_required")?;
                    let previous = self.engine.ground_runtime.replace(runtime.clone());
                    let result = self.engine.stage_respawn_clearance(
                        Clearance {
                            source: key,
                            physics,
                            filter_rows: self.state.filter_rows,
                            now: self.now,
                        },
                        self.native,
                        &mut changes,
                    );
                    let updated = std::mem::replace(&mut self.engine.ground_runtime, previous)
                        .ok_or("domination_ground_clearance_runtime_missing")?;
                    let (audit, messages) = result?;
                    *runtime = updated;
                    self.audit.extend(audit);
                    self.messages.extend(messages);
                    0
                }
                _ => 0,
            };
            if field != 0 {
                *changes.entry(event.vehicle.index).or_default() |= field;
            }
        }
        if !changes.is_empty() {
            self.packets
                .push(crate::ground_vehicles::encode_runtime_delta(
                    self.engine
                        .ground_vehicles
                        .as_ref()
                        .ok_or("domination_ground_catalog_required")?,
                    runtime,
                    &changes.into_iter().collect::<Vec<_>>(),
                    2,
                    self.now.as_millis() as u32,
                )?);
        }
        Ok(true)
    }
}

impl<W: Write> ExternalEffects for Provider<'_, W> {
    fn prepare_class_prefix(
        &mut self,
        entry: NativeEntry,
        _: &ObjectiveWorld,
        _: &SpawnCatalog,
    ) -> Result<(), &'static str> {
        if let Some(rule) = self
            .state
            .objectives
            .objects
            .iter()
            .find(|object| object.object_index == entry.delta.object_index)
            .and_then(|object| object.scoring.as_ref())
            && (rule.activation_score != 0 || rule.deactivation_score != 0)
        {
            return Err("domination_unimplemented_native_match_score");
        }
        Ok(())
    }

    fn prepare_vehicles(&mut self, requests: &[VehicleRequest]) -> Result<(), &'static str> {
        for request in requests {
            let spawn = request.point.native_id as u16;
            if self.state.known_empty_vehicle_spawns.contains(&spawn) {
                continue;
            }
            if let Some(carrier) = self
                .state
                .helo
                .as_mut()
                .and_then(|fleet| fleet.by_spawn_mut(spawn))
            {
                let (objective, _) = self
                    .state
                    .objectives
                    .binding(request.objective_token)
                    .ok_or("domination_helo_objective_binding_missing")?;
                carrier
                    .motion
                    .set_cycle(request.hook == VehicleHook::OnFspActivate, Some(objective));
                continue;
            }
            if self.state.disabled_helo_spawns.contains(&spawn) {
                continue;
            }
            let (objective, _) = self
                .state
                .objectives
                .binding(request.objective_token)
                .ok_or("domination_vehicle_objective_binding_missing")?;
            let damage_id = 0x0c80_0000 | u32::from(objective);
            if self.prepare_ground_vehicle(*request, damage_id)? {
                continue;
            }
            let Some(bunkers) = self.bunkers.as_mut() else {
                return Err("domination_fsp_vehicle_world_required");
            };
            if bunkers.domination_bunker_at_spawn(spawn).is_none() {
                return Err("domination_fsp_linked_vehicle_backend_required");
            }
            let update = bunkers.apply_domination_cycle(
                spawn,
                request.hook == VehicleHook::OnFspActivate,
                damage_id,
            )?;
            self.stage_hospital_update(update, damage_id)?;
        }
        Ok(())
    }

    fn prepare_class_entry(
        &mut self,
        entry: NativeEntry,
        _: &ObjectiveWorld,
        _: &SpawnCatalog,
    ) -> Result<(), &'static str> {
        let assignment = self
            .engine
            .active
            .iter()
            .chain(self.engine.peers.values())
            .find(|a| Some(a.reservation.player) == entry.reward_actor)
            .and_then(|a| {
                self.state
                    .assignments
                    .get(usize::from(a.squad_catalog.squad_index()))
            })
            .and_then(|a| a.as_ref())
            .map(|a| a.assignment);
        let assignments: Vec<_> = self
            .state
            .assignments
            .iter()
            .enumerate()
            .filter_map(|(index, value)| value.map(|v| (index as u16, v.assignment)))
            .collect();
        let effects = self.engine.stage_native_objective_callbacks(
            entry,
            assignment,
            &assignments,
            crate::frago::Frame {
                ground: self.ground.as_ref(),
                helo: self.state.helo.as_ref(),
                ..Default::default()
            },
        )?;
        self.audit.extend(effects.audit);
        self.messages.extend(effects.messages);
        Ok(())
    }

    fn prepare_explosion(
        &mut self,
        entry: NativeEntry,
        _: &ObjectiveWorld,
    ) -> Result<(), &'static str> {
        let index = entry.delta.object_index;
        let token = self
            .state
            .objectives
            .bound_objects()
            .find(|&(_, candidate)| candidate == index)
            .map(|(token, _)| token)
            .ok_or("domination_blast_objective_binding_missing")?;
        let root = self
            .state
            .root_poses
            .get(&index)
            .ok_or("domination_blast_live_pose_missing")?;
        let request = self.state.structural.explosion(token, entry)?;
        let trigger_position = entry
            .active_trigger_index
            .map(|trigger| {
                self.state
                    .trigger_positions
                    .get(&(index, trigger))
                    .copied()
                    .ok_or("domination_blast_live_trigger_pose_missing")
            })
            .transpose()?;
        let blast = request
            .properties
            .instantaneous_blast(root.translation.map(|v| v as f32), trigger_position)?;
        let previous_ground =
            std::mem::replace(&mut self.engine.ground_runtime, self.ground.clone());
        let previous_bunkers =
            std::mem::replace(&mut self.engine.bunker_world, self.bunkers.clone());
        let result = (|| {
            let work = self.engine.collect_domination_blast(
                BlastSource::Structural {
                    objective_id: 0x0c80_0000 | u32::from(index),
                },
                0x0380_0000 | u32::from(entry.delta.state_actor.actor_index),
                blast,
                BlastCollision {
                    filter_rows: &self.state.filter_rows,
                    limits: QueryLimits::default(),
                    sequences: &self.state.collisions,
                    material_costs: self.state.structural.material_penetration_costs(),
                    terrain_materials: Some(&self.state.terrain_materials),
                },
            )?;
            self.engine.stage_domination_damage_with_ground(
                &mut self.state.area,
                self.native,
                self.now,
                &[work],
                Some(&self.state.ground_damage),
            )
        })();
        self.ground = std::mem::replace(&mut self.engine.ground_runtime, previous_ground);
        self.bunkers = std::mem::replace(&mut self.engine.bunker_world, previous_bunkers);
        let DamageCommit {
            audit,
            messages,
            changed_ground,
            helo_doses,
        } = result?;
        self.stage_helo_radial(&helo_doses)?;
        if !changed_ground.is_empty() {
            let changes = changed_ground
                .into_iter()
                .map(|index| (index, 1 << 18))
                .collect::<Vec<_>>();
            self.packets
                .push(crate::ground_vehicles::encode_runtime_delta(
                    self.engine
                        .ground_vehicles
                        .as_ref()
                        .ok_or("domination_ground_catalog_required")?,
                    self.ground
                        .as_ref()
                        .ok_or("domination_ground_world_required")?,
                    &changes,
                    2,
                    self.now.as_millis() as u32,
                )?);
        }
        self.audit.extend(audit);
        self.messages.extend(messages);
        Ok(())
    }

    fn prepare_structural_update_tail(
        &mut self,
        index: u16,
        updated_state: ObjectiveState,
    ) -> Result<(), &'static str> {
        let properties = self.state.structural.blast_properties(index)?;
        if properties.character_damage_over_time_radius == 0.0
            || properties.character_damage_over_time_value == 0.0
        {
            return Ok(());
        }
        let root = self
            .state
            .root_poses
            .get(&index)
            .ok_or("domination_area_live_pose_missing")?;
        let world = self
            .engine
            .active
            .iter()
            .chain(self.engine.peers.values())
            .next()
            .map(|actor| actor.reservation.world)
            .ok_or("domination_area_world_id_missing")?;
        let work = DamageWork::Area(AreaUpdate {
            objective_index: index,
            objective_position: root.translation.map(|v| v as f32),
            updated_state,
            world,
            properties,
        });
        let previous = std::mem::replace(&mut self.engine.bunker_world, self.bunkers.clone());
        let result = self.engine.stage_domination_structural_damage(
            &mut self.state.area,
            self.native,
            self.now,
            &[work],
        );
        self.bunkers = std::mem::replace(&mut self.engine.bunker_world, previous);
        let (audit, messages) = result?;
        self.audit.extend(audit);
        self.messages.extend(messages);
        Ok(())
    }

    fn prepare_ability(
        &mut self,
        _: Context,
        platoon_token: u32,
        ability: Ability,
        enabled: bool,
    ) -> Result<(), &'static str> {
        if platoon_token == 0 {
            return Ok(());
        }
        let platoon = *self
            .state
            .platoons
            .get(&platoon_token)
            .ok_or("domination_ability_platoon_unbound")?;
        let native = match ability {
            Ability::ClusterBomb => 1,
            Ability::PrecisionStrike => 2,
            Ability::Sensor => 3,
            Ability::GasBombardment => 4,
            Ability::StrafingRun => 5,
            Ability::GuidedArtillery => 6,
        };
        let mask = self
            .state
            .ability_enabled
            .get_mut(usize::from(platoon))
            .ok_or("domination_ability_invalid_platoon_slot")?;
        let bit = 1_u16 << (native - 1);
        if enabled {
            *mask |= bit;
        } else {
            *mask &= !bit;
        }
        self.audit
            .push(json!({"op":"domination_ability_toggle","platoon":platoon,
            "ability":native,"enabled":enabled,"enabled_mask":*mask}));
        Ok(())
    }

    fn callback_actor(&self, original_object_id: u32) -> Result<Option<Actor>, &'static str> {
        if original_object_id & 0xffff_0000 != 0x0380_0000 {
            return Err("domination_callback_invalid_actor_id");
        }
        let player = original_object_id as u16;
        let world = self
            .engine
            .active
            .iter()
            .chain(self.engine.peers.values())
            .next()
            .map(|a| a.reservation.world);
        Ok(world.and_then(|world| self.engine.objective_script_character(player, world)))
    }

    fn refresh_assignments(
        &mut self,
        _: NativeEntry,
        catalog: &ObjectiveCatalog,
        world: &ObjectiveWorld,
        spawns: &SpawnCatalog,
    ) -> Result<(), &'static str> {
        for change in domination_gameplay_effects::stage_owned_assignments(
            catalog,
            world,
            spawns,
            &self.state.assignments,
            self.phase,
        )? {
            let assignment = self.state.assignments[usize::from(change.squad_index)]
                .as_mut()
                .ok_or("domination_assignment_slot_missing")?;
            if assignment.assignment.primary_index != change.assignment.primary_index {
                self.packets.push(
                    SquadObjectiveAssignment {
                        sequence: 2,
                        timestamp_ms: self.now.as_millis() as u32,
                        squad_index: change.squad_index,
                        objective_index: change.assignment.primary_index,
                    }
                    .encode_with_profile(self.engine.wire_profile)?,
                );
            }
            assignment.assignment = change.assignment;
        }
        Ok(())
    }

    fn prepare_sequence(
        &mut self,
        entry: NativeEntry,
        objective_token: u32,
        game_time_ms: u32,
        _: &ObjectiveWorld,
    ) -> Result<Option<ObjectiveSequenceDelta>, &'static str> {
        let index = entry.delta.object_index;
        let root = self.state.root_poses.get(&index);
        self.state.structural.prepare_sequence(
            objective_token,
            entry,
            game_time_ms,
            |_, playback| {
                self.state.collisions.prepare_reference_playback(
                    index,
                    *root.ok_or("domination_sequence_live_pose_missing")?,
                    playback,
                    &self.state.assets,
                )
            },
        )
    }
}

impl<W: Write> Engine<W> {
    /// Dispatch one native objective-manager pass in the normal match tick.
    /// A provider failure leaves the entire previous authority installed.
    pub(crate) fn tick_domination_objectives(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if !self.match_started || self.match_terminal.is_some() {
            return Ok(Vec::new());
        }
        // Like Acquisition, an empty world (after the last release) or one
        // whose humans have not yet entered stays idle: no ticks, no records.
        if !self
            .active
            .iter()
            .chain(self.peers.values())
            .any(|a| a.entered_game && a.ready_for_updates && a.initial_objects.is_some())
        {
            return Ok(Vec::new());
        }
        let mut world = self
            .domination_gameplay
            .take()
            .ok_or("domination_gameplay_configuration_required")?;
        let result = self.tick_domination_gameplay_world(&mut world, now, wall);
        self.domination_gameplay = Some(world);
        result
    }

    fn tick_domination_gameplay_world(
        &mut self,
        world: &mut World,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        self.commit_domination_mission_start(&mut world.runtime, now, wall)?;
        let actors_before: BTreeMap<u16, ObjectiveActor> = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter_map(|actor| {
                let CharacterLife::Alive { generation, .. } = actor.life else {
                    return None;
                };
                if !actor.entered_game || !actor.ready_for_updates || actor.is_mounted() {
                    return None;
                }
                let player = actor.reservation.player;
                Some((
                    player,
                    ObjectiveActor {
                        player,
                        team: actor.team(),
                        generation,
                        position: actor.tracking.sample.as_ref()?.position,
                    },
                ))
            })
            .collect();
        let native = self.owned_acquisition_characters();
        let state = &mut world.state;
        self.commit_domination_gameplay(
            &mut world.runtime,
            now,
            wall,
            native,
            |engine, native, runtime, frame| {
                let bunkers = engine.bunker_world.clone();
                let ground = engine.ground_runtime.clone();
                let phase = match frame.round.phase() {
                    crate::acquisition_round::Phase::Playing => MatchPhase::Playing,
                    crate::acquisition_round::Phase::Ending { .. } => MatchPhase::EndCountdown,
                    crate::acquisition_round::Phase::Finished => MatchPhase::PostGame,
                };
                let provider = Provider {
                    engine,
                    native,
                    state: state.clone(),
                    bunkers,
                    ground,
                    phase,
                    now,
                    audit: Vec::new(),
                    messages: Vec::new(),
                    packets: Vec::new(),
                };
                // Retail GameServer::Update uses its in-game script executor
                // branch before TimedUpdate dispatches the Sim manager.
                let mut before_sim = runtime.stage_callback_update(frame, provider)?;
                // The native Sim manager visits ground (1), Helo (2),
                // GameState's spawn waves (7), then objectives (12).
                before_sim.effects.prepare_ground_sim(&before_sim.spawns)?;
                before_sim.effects.prepare_helo_damage()?;
                before_sim.effects.prepare_helo_flight()?;
                before_sim
                    .effects
                    .prepare_ground_waves(&before_sim.spawns)?;
                before_sim
                    .effects
                    .prepare_active_abilities(&mut before_sim.world)?;
                if before_sim.world.has_native_entries() {
                    before_sim = before_sim.runtime.stage_structure_damage_entries(
                        crate::domination_gameplay_engine::Frame {
                            now,
                            world: &before_sim.world,
                            spawns: &before_sim.spawns,
                            round: &before_sim.round,
                        },
                        before_sim.effects,
                    )?;
                }
                let structure_deltas = std::mem::take(&mut before_sim.deltas);
                let structure_sequences = std::mem::take(&mut before_sim.sequences);
                let actors = if before_sim.effects.engine.active_abilities.is_some() {
                    before_sim
                        .effects
                        .engine
                        .peers
                        .values()
                        .chain(before_sim.effects.engine.active.iter())
                        .filter_map(|actor| {
                            let CharacterLife::Alive { generation, .. } = actor.life else {
                                return None;
                            };
                            if !actor.entered_game || !actor.ready_for_updates || actor.is_mounted()
                            {
                                return None;
                            }
                            let player = actor.reservation.player;
                            Some((
                                player,
                                ObjectiveActor {
                                    player,
                                    team: actor.team(),
                                    generation,
                                    position: actor.tracking.sample.as_ref()?.position,
                                },
                            ))
                        })
                        .collect()
                } else {
                    actors_before.clone()
                };
                let (mut candidate, mut provider) = before_sim
                    .runtime
                    .stage_objective_update(
                        crate::domination_gameplay_engine::Frame {
                            now,
                            world: &before_sim.world,
                            spawns: &before_sim.spawns,
                            round: &before_sim.round,
                        },
                        &actors,
                        before_sim.effects,
                    )?
                    .detach_effects();
                // Native replication samples final fields. A zero-duration arm
                // can enter twice in this pass; publish the final value once.
                candidate.deltas = structure_deltas
                    .into_iter()
                    .chain(candidate.deltas)
                    .map(|delta| (delta.object_index, delta))
                    .collect::<BTreeMap<_, _>>()
                    .into_values()
                    .collect();
                candidate.sequences = structure_sequences
                    .into_iter()
                    .chain(candidate.sequences)
                    .map(|delta| (delta.object_index, delta))
                    .collect::<BTreeMap<_, _>>()
                    .into_values()
                    .collect();
                provider.prepare_hospital_damage_presence()?;
                let Provider {
                    state: next_state,
                    bunkers,
                    ground,
                    audit,
                    messages,
                    packets,
                    ..
                } = provider;
                Ok((
                    candidate,
                    EffectsCommit {
                        audit,
                        messages,
                        packets,
                        install: |engine: &mut Self| {
                            *state = next_state;
                            engine.bunker_world = bunkers;
                            engine.ground_runtime = ground;
                        },
                    },
                ))
            },
        )
    }
}

impl<W: Write> Engine<W> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn event_domination_interaction(
        &mut self,
        event: &crate::events::EventInput,
        request: crate::protocol::ObjectiveInteraction,
        actor: ObjectiveActor,
        timings: ActionTimingModifiers,
        leadership: crate::leadership_runtime::LeadershipPlan,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let mut world = self
            .domination_gameplay
            .take()
            .ok_or("domination_gameplay_configuration_required")?;
        let result = self.run_domination_interaction_world(
            &mut world, event, request, actor, timings, leadership, now, wall,
        );
        self.domination_gameplay = Some(world);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn run_domination_interaction_world(
        &mut self,
        world: &mut World,
        event: &crate::events::EventInput,
        request: crate::protocol::ObjectiveInteraction,
        actor: ObjectiveActor,
        timings: ActionTimingModifiers,
        leadership: crate::leadership_runtime::LeadershipPlan,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let native = self.owned_acquisition_characters();
        let state = &mut world.state;
        self.commit_domination_gameplay(
            &mut world.runtime,
            now,
            wall,
            native,
            |engine, native, runtime, frame| {
                let bunkers = engine.bunker_world.clone();
                let ground = engine.ground_runtime.clone();
                let provider = Provider {
                    engine,
                    native,
                    state: state.clone(),
                    bunkers,
                    ground,
                    phase: MatchPhase::Playing,
                    now,
                    audit: Vec::new(),
                    messages: Vec::new(),
                    packets: Vec::new(),
                };
                let outcome = runtime
                    .stage_interaction_with_timing(frame, request, actor, timings, provider)?;
                let (candidate, provider) = outcome.candidate.detach_effects();
                let Provider {
                    state: next_state,
                    bunkers,
                    ground,
                    mut audit,
                    mut messages,
                    packets,
                    ..
                } = provider;
                audit.push(
                    json!({"op":"objective_interaction","world_id":event.world_id,
                "player_id":event.player_id,"reservation_id":event.reservation_id,
                "transport":event.transport,"direction":event.direction,
                "rt_type":event.rt_type,"plugin_id":event.plugin_id,
                "payload_hex":event.payload_hex.to_ascii_lowercase(),
                "reason":outcome.reason,"reply_kind":outcome.reply.kind as u8,
                    "leadership_changes":leadership.audit()}),
                );
                messages.push(OutboundMessage {
                    reservation_id: event.reservation_id.clone(),
                    world_id: event.world_id,
                    player_id: event.player_id,
                    plugin_id: 1,
                    payload_hex: crate::protocol::encode_hex(&outcome.reply.encode()),
                });
                Ok((
                    candidate,
                    EffectsCommit {
                        audit,
                        messages,
                        packets,
                        install: |engine: &mut Self| {
                            *state = next_state;
                            engine.bunker_world = bunkers;
                            engine.ground_runtime = ground;
                            engine.commit_leadership(leadership);
                            if let Some(active) = &mut engine.active {
                                active.last_event = now;
                            }
                        },
                    },
                ))
            },
        )
    }
}

impl<W: Write> Engine<W> {
    pub(crate) fn event_domination_structural_repair(
        &mut self,
        event: &crate::events::EventInput,
        request: crate::protocol::Repair,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let tool = self
            .bunker_world
            .as_ref()
            .map(BunkerWorld::repair_tool_profile);
        let (authority, leadership) = self.plan_structural_repair_authority(tool, now)?;
        let active = self.active.as_ref().ok_or("no_reservation")?;
        // The live Domination runtime supplies the squad's current FRAGO assignment.
        let credit = self.plan_repair_credit(
            active,
            leadership.modifiers(active).value("kCharModRepairAmount"),
            self.frago_object_position(request.target_object_id),
        );
        let mut world = self
            .domination_gameplay
            .take()
            .ok_or("domination_gameplay_configuration_required")?;
        let result = self.commit_domination_gameplay(
            &mut world.runtime, now, wall, BTreeMap::new(),
            |engine, _, runtime, frame| {
                use crate::domination_repair_runtime::{HEALTH_ALREADY_COVERED, Submission};
                let (candidate, submission) = runtime.stage_repair(frame, request.target_object_id, authority)?;
                let (credit, reason) = match submission {
                    Submission::Queued(_) => (Some(credit?), None),
                    Submission::LatchOnly => (None, Some(HEALTH_ALREADY_COVERED)),
                };
                let audit = json!({"op":"domination_structural_repair",
                    "reservation_id":event.reservation_id,"world_id":event.world_id,"player_id":event.player_id,
                    "target":request.target_object_id,"payload_hex":event.payload_hex.to_ascii_lowercase(),
                    "reason":reason,"leadership_changes":leadership.audit(),"repair_credit":credit.as_ref().map(|credit| credit.audit())});
                let messages = credit.map_or_else(Vec::new, |credit| engine.install_repair_credit(credit));
                Ok((candidate, EffectsCommit {
                    audit: vec![audit],
                    messages,
                    packets: Vec::new(),
                    install: |engine: &mut Self| {
                        engine.commit_leadership(leadership);
                        if let Some(active) = &mut engine.active {
                            active.last_event = now;
                        }
                    },
                }))
            },
        );
        self.domination_gameplay = Some(world);
        result
    }
}
