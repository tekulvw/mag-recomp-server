//! Source-bound Structural objective effects for Acquisition's owned match.
use crate::{
    Engine, OutboundMessage,
    acquisition_character_effects::NativeState,
    acquisition_ground_runtime as ground,
    acquisition_spawn_effects::Clearance,
    acquisition_world::Settings,
    active_abilities::Ability,
    active_ability_state::native_ability_mask,
    domination_fsp::{self as fsp, FspType, SpawnIdentity, SquadMembership, Team, VehicleHook},
    domination_gameplay_effects::Structural,
    domination_structural_effects::{Catalog, TerrainMaterialGrid},
    domination_structural_world::{
        AreaDamageState, AreaUpdate, BlastCollision, BlastSource, DamageWork,
        SequenceCollisionAssets, SequenceCollisionState,
    },
    objective_catalog::{NativeClass, ObjectiveCatalog},
    objective_control::{NativeEntry, ObjectiveWorld},
    replication::{ObjectiveSequenceDelta, ObjectiveState},
    spawn_catalog::SpawnCatalog,
    structure_catalog::RepairParameters,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[cfg(test)]
#[path = "acquisition_structural_connected_tests.rs"]
pub(crate) mod connected_tests;

#[cfg(test)]
#[path = "acquisition_structural_runtime_tests.rs"]
mod runtime_tests;

const LEGACY_BINDINGS: &str = "mag_acquisition_structural_bindings_v1";
const NATIVE_BINDINGS: &str = "mag_acquisition_structural_bindings_v2";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Files {
    effects: PathBuf,
    collision_assets: PathBuf,
    terrain_index: PathBuf,
    terrain_materials: PathBuf,
    bindings: PathBuf,
    repair_catalog: Option<PathBuf>,
}

impl Files {
    pub(crate) fn resolve(&mut self, directory: &Path) {
        for path in [
            &mut self.effects,
            &mut self.collision_assets,
            &mut self.terrain_index,
            &mut self.terrain_materials,
            &mut self.bindings,
        ] {
            *path = directory.join(&*path);
        }
        if let Some(path) = &mut self.repair_catalog {
            *path = directory.join(&*path);
        }
    }

    pub(crate) fn load(
        &self,
        objectives: &ObjectiveCatalog,
        spawns: &SpawnCatalog,
    ) -> io::Result<Setup> {
        let bindings: Bindings = serde_json::from_str(&read(&self.bindings)?)?;
        let effects = Catalog::from_json(&read(&self.effects)?).map_err(io::Error::other)?;
        if !effects.source_is(
            spawns.map_name(),
            &objectives.source().authored_catalog_sha256,
            &bindings.level_archive_sha256,
            &bindings.level_clv_sha256,
        ) {
            return Err(io::Error::other("acquisition_structural_source_mismatch"));
        }
        bindings
            .validate(&effects, objectives, spawns)
            .map_err(io::Error::other)?;
        let assets = SequenceCollisionAssets::load(&self.collision_assets, &effects)
            .map_err(io::Error::other)?;
        let terrain = TerrainMaterialGrid::load(
            &self.terrain_index,
            &self.terrain_materials,
            &effects,
            &objectives.source().source_image_sha256,
        )
        .map_err(io::Error::other)?;
        let playback =
            Structural::from_acquisition_catalog(effects, objectives).map_err(io::Error::other)?;
        let repair_tool = self
            .repair_catalog
            .as_ref()
            .map(crate::structural_repair_authority::load_retail_repair_tool)
            .transpose()?;
        let setup = Setup {
            playback,
            assets: Arc::new(assets),
            terrain: Arc::new(terrain),
            bindings: Arc::new(bindings),
            repair_tool,
        };
        setup
            .validate_spawn_routes(spawns)
            .map_err(io::Error::other)?;
        Ok(setup)
    }
}

fn read(path: &Path) -> io::Result<String> {
    const LIMIT: u64 = 256 * 1024;
    let mut text = String::new();
    File::open(path)?
        .take(LIMIT + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > LIMIT {
        return Err(io::Error::other("acquisition_structural_source_too_large"));
    }
    Ok(text)
}

/// Checked source inputs loaded through the optional Acquisition configuration.
#[derive(Clone)]
pub struct Setup {
    playback: Structural,
    assets: Arc<SequenceCollisionAssets>,
    terrain: Arc<TerrainMaterialGrid>,
    bindings: Arc<Bindings>,
    pub(crate) repair_tool: Option<RepairParameters>,
}

impl Setup {
    pub(crate) fn validate_native_objectives(
        &self,
        objectives: &ObjectiveCatalog,
    ) -> Result<(), &'static str> {
        self.bindings.validate_native_objectives(objectives)?;
        if self.repair_tool.is_none()
            && objectives.objects.iter().any(|object| {
                object.native_class == Some(NativeClass::Structural)
                    && object.repair.is_some_and(|rule| rule.can_repair)
            })
        {
            return Err("acquisition_structural_repair_catalog_required");
        }
        Ok(())
    }

    pub(crate) fn validate_ground_area_seats(
        &self,
        vehicles: &crate::ground_vehicles::Catalog,
    ) -> Result<(), &'static str> {
        if self.bindings.schema != NATIVE_BINDINGS {
            return Ok(());
        }
        for (&index, row) in &self.bindings.rows {
            if row.class != NativeClass::Structural {
                continue;
            }
            let blast = self.playback.blast_properties(index)?;
            if blast.character_damage_over_time_radius == 0.0
                || blast.character_damage_over_time_value == 0.0
            {
                continue;
            }
            // The rotating area scan can sample any occupied ground seat.
            // Check each mountable seat's visibility before the match starts.
            let qualified = vehicles.creations().all(|vehicle| {
                vehicle
                    .passenger_seats
                    .iter()
                    .copied()
                    .chain(
                        vehicles
                            .driver_seat(vehicle.index)
                            .map(|driver| driver.seat),
                    )
                    .all(|seat| {
                        vehicles
                            .seat_occupant_visible(vehicle.index, seat)
                            .is_some()
                    })
            });
            return qualified
                .then_some(())
                .ok_or("acquisition_structural_ground_seat_visibility_required");
        }
        Ok(())
    }

    fn configure_spawns(&self, spawns: &mut SpawnCatalog) -> Result<(), &'static str> {
        let unselectable = self
            .bindings
            .rows
            .values()
            .filter_map(|row| row.spawn)
            .filter(|spawn| spawn.user_class == 0)
            .map(|spawn| spawn.index)
            .collect();
        let mut category_zero = [u16::MAX; 32];
        for (&squad, row) in &self.bindings.squads {
            category_zero[usize::from(squad)] = row
                .category_zero
                .ok_or("acquisition_structural_category_zero_missing")?
                .0;
        }
        spawns.configure_acquisition_structural_spawn_scope(unselectable, category_zero)
    }

    fn validate_spawn_routes(&self, spawns: &SpawnCatalog) -> Result<(), &'static str> {
        let mut spawns = spawns.clone();
        self.configure_spawns(&mut spawns)?;
        for state in [ObjectiveState::Deactivated, ObjectiveState::Completed] {
            let mut candidate = spawns.clone();
            for row in self.bindings.rows.values() {
                let Some(spawn) = row.spawn else { continue };
                let plan = self.bindings.plan_fsp(row.token, spawn, state)?;
                (candidate, ()) =
                    fsp::stage_spawn_effects(&candidate, &plan)?.complete(|_| Ok(()))?;
            }
        }
        Ok(())
    }

    pub(crate) fn validate_ground(&self, ground: &ground::Runtime) -> Result<(), &'static str> {
        for spawn in self
            .bindings
            .rows
            .values()
            .filter_map(|row| row.spawn)
            .filter(|spawn| spawn.vehicle)
        {
            if ground
                .vehicles()
                .filter(|vehicle| {
                    vehicle.source_spawn_point_index == spawn.index
                        && vehicle.source_spawn_point_token == spawn.token
                        && vehicle.source_spawn_type == Some(7)
                })
                .count()
                != 1
            {
                return Err("acquisition_structural_motor_pool_source_missing");
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bindings {
    schema: String,
    map: String,
    metadata_sha256: String,
    initial_objective_sha256: String,
    level_archive_sha256: String,
    level_clv_sha256: String,
    rows: BTreeMap<u16, Row>,
    squads: BTreeMap<u8, Squad>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    token: u32,
    class: NativeClass,
    spawn: Option<Spawn>,
    start_deactivated: bool,
    hooks: BTreeMap<u8, Hook>,
    #[serde(default)]
    trigger_positions: Vec<[f32; 3]>,
}

impl Row {
    fn trigger_position(&self, trigger: Option<u8>) -> Result<Option<[f32; 3]>, &'static str> {
        trigger
            .map(|index| {
                self.trigger_positions
                    .get(usize::from(index))
                    .copied()
                    .ok_or("acquisition_structural_trigger_pose_required")
            })
            .transpose()
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct Spawn {
    token: u32,
    index: u16,
    team: u8,
    user_class: u8,
    spawn_type: u8,
    fsp_type: u8,
    vehicle: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Squad {
    points: Vec<(u16, u32)>,
    category_zero: Option<(u16, u32)>,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Hook {
    Antiaircraft { platoon: u16, spawns: [u16; 4] },
    Mortar { platoon: u16 },
    Sensor { platoon: u16 },
    MotorPool { spawn: u16 },
}

impl Hook {
    fn spawn_targets(&self) -> &[u16] {
        match self {
            Self::Antiaircraft { spawns, .. } => spawns,
            Self::MotorPool { spawn } => std::slice::from_ref(spawn),
            _ => &[],
        }
    }

    fn commands(&self) -> Option<(u16, bool, &[Ability])> {
        match self {
            Self::Antiaircraft { platoon, .. } => Some((
                *platoon,
                true,
                &[
                    Ability::ClusterBomb,
                    Ability::PrecisionStrike,
                    Ability::Sensor,
                    Ability::StrafingRun,
                ],
            )),
            Self::Mortar { platoon } => Some((
                *platoon,
                false,
                &[
                    Ability::ClusterBomb,
                    Ability::GasBombardment,
                    Ability::GuidedArtillery,
                ],
            )),
            Self::Sensor { platoon } => Some((*platoon, false, &[Ability::Sensor])),
            Self::MotorPool { .. } => None,
        }
    }
}

impl Bindings {
    fn plan_fsp(
        &self,
        token: u32,
        spawn: Spawn,
        state: ObjectiveState,
    ) -> Result<fsp::Candidate, &'static str> {
        let identity = |(index, token): (u16, u32)| SpawnIdentity {
            token,
            native_id: 0x2380_0000 | u32::from(index),
        };
        let team = if spawn.team == 0 {
            Team::Zero
        } else {
            Team::One
        };
        let fsp_type = if spawn.fsp_type == 0 {
            FspType::ActiveWhenIncomplete
        } else {
            FspType::ActiveWhenComplete
        };
        let points: BTreeMap<u8, Vec<SpawnIdentity>> = self
            .squads
            .iter()
            .map(|(&index, squad)| (index, squad.points.iter().copied().map(identity).collect()))
            .collect();
        let squads = std::array::from_fn(|index| {
            let squad = self.squads.get(&(index as u8))?;
            Some(SquadMembership {
                points: &points[&(index as u8)],
                category_zero: squad.category_zero.map(identity),
            })
        });
        fsp::plan_entry(
            fsp::CommittedEntry {
                objective_token: token,
                state,
                is_completed: state == ObjectiveState::Completed,
            },
            Some(fsp::AuthoredLink {
                objective_token: token,
                spawn_token: spawn.token,
                team,
                fsp_type,
            }),
            Some(fsp::ResolvedSpawn {
                identity: identity((spawn.index, spawn.token)),
                team,
                squad_routing: if spawn.user_class == 0 {
                    fsp::SquadRouting::Enabled
                } else {
                    fsp::SquadRouting::Disabled
                },
            }),
            &squads,
        )
        .map_err(|_| "acquisition_structural_fsp_invalid")
    }

    fn validate(
        &self,
        effects: &Catalog,
        objectives: &ObjectiveCatalog,
        spawns: &SpawnCatalog,
    ) -> Result<(), &'static str> {
        if !matches!(self.schema.as_str(), LEGACY_BINDINGS | NATIVE_BINDINGS)
            || self.map != spawns.map_name()
            || self.metadata_sha256 != objectives.source().authored_catalog_sha256
            || self.initial_objective_sha256 != objectives.source().binding_capture_sha256
            || spawns.match_profile().game_type != 1
            || !spawns.map_pool().runtime_indices_verified
            || self.rows.len() > 128
        {
            return Err("invalid_acquisition_structural_bindings");
        }
        let expected: BTreeSet<_> = effects.placed_objectives().collect();
        let actual: BTreeSet<_> = self
            .rows
            .values()
            .filter(|row| row.class == NativeClass::Structural)
            .map(|row| row.token)
            .collect();
        if actual != expected {
            return Err("acquisition_structural_effect_coverage_mismatch");
        }
        if self
            .rows
            .values()
            .filter(|row| row.class == NativeClass::SecureHold)
            .count()
            != 8
            || self.rows.values().any(|row| {
                row.class != NativeClass::Structural
                    && (row.class != NativeClass::SecureHold || !row.hooks.is_empty())
            })
        {
            return Err("invalid_acquisition_structural_spawn_classes");
        }
        let mut spawn_counts = BTreeMap::new();
        for spawn in self.rows.values().filter_map(|row| row.spawn) {
            *spawn_counts.entry(spawn.spawn_type).or_insert(0) += 1;
        }
        if spawn_counts != BTreeMap::from([(4, 2), (5, 4), (7, 8), (8, 8)]) {
            return Err("invalid_acquisition_structural_spawn_inventory");
        }
        for (&index, row) in &self.rows {
            if objectives.binding(row.token).map(|(id, _)| id) != Some(index)
                || row.start_deactivated != (row.class == NativeClass::Structural)
                || row.spawn.is_some_and(|spawn| {
                    spawn.team > 1
                        || !matches!(
                            (spawn.user_class, spawn.spawn_type, spawn.vehicle),
                            (0, 4 | 5 | 8, false) | (1, 7, true)
                        )
                        || spawn.fsp_type > 1
                        || spawns
                            .point(spawn.index)
                            .is_none_or(|point| point.token != spawn.token)
                })
                || row.hooks.iter().any(|(state, hook)| {
                    ![1, 6, 7].contains(state)
                        || hook
                            .spawn_targets()
                            .iter()
                            .any(|target| !self.rows.contains_key(target))
                        || hook.commands().is_some_and(|(index, attacker, _)| {
                            !(if attacker { [0, 1] } else { [4, 5] }).contains(&index)
                        })
                })
            {
                return Err("invalid_acquisition_structural_objective");
            }
        }
        if self.squads.len() != 16
            || self.squads.iter().any(|(&index, row)| {
                spawns.selected_squad(u16::from(index)).is_none()
                    || row.points.is_empty()
                    || row.points.len() > 32
                    || row
                        .points
                        .iter()
                        .any(|&(i, t)| spawns.point(i).is_none_or(|point| point.token != t))
                    || row
                        .category_zero
                        .is_some_and(|point| !row.points.contains(&point))
            })
        {
            return Err("invalid_acquisition_structural_squad_membership");
        }
        Ok(())
    }

    fn validate_native_objectives(
        &self,
        objectives: &ObjectiveCatalog,
    ) -> Result<(), &'static str> {
        let expected: BTreeSet<_> = self
            .rows
            .iter()
            .filter(|(_, row)| {
                self.schema == NATIVE_BINDINGS && row.class == NativeClass::Structural
            })
            .map(|(&index, _)| index)
            .collect();
        let actual: BTreeMap<_, _> = objectives
            .objects
            .iter()
            .filter(|object| object.native_class == Some(NativeClass::Structural))
            .map(|object| (object.object_index, object))
            .collect();
        if actual.keys().copied().ne(expected) {
            return Err("acquisition_structural_native_coverage_mismatch");
        }
        for (&index, row) in &self.rows {
            let count = actual
                .get(&index)
                .and_then(|object| object.demolition.as_ref())
                .map_or(0, |rule| rule.trigger_volumes.len());
            if objectives.binding(row.token).map(|(id, _)| id) != Some(index)
                || row.trigger_positions.len() != count
                || row
                    .trigger_positions
                    .iter()
                    .flatten()
                    .any(|p| !p.is_finite())
            {
                return Err("acquisition_structural_trigger_pose_mismatch");
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Runtime {
    setup: Setup,
    collisions: SequenceCollisionState,
    area: AreaDamageState,
    queued: BTreeMap<u16, (ObjectiveState, u16)>,
}

impl Runtime {
    pub(crate) fn new(setup: Setup) -> Self {
        Self {
            setup,
            collisions: SequenceCollisionState::default(),
            area: AreaDamageState::default(),
            queued: BTreeMap::new(),
        }
    }

    pub(crate) fn sequences(&self) -> Vec<ObjectiveSequenceDelta> {
        self.setup.playback.current_sequences()
    }

    pub(crate) fn advances_native_objectives(&self) -> bool {
        self.setup.bindings.schema == NATIVE_BINDINGS
    }

    pub(crate) fn blast_collision<'a>(
        &'a self,
        filters: &'a [u32; 32],
        physics: Settings,
    ) -> BlastCollision<'a> {
        BlastCollision {
            filter_rows: filters,
            limits: physics.query_limits,
            sequences: &self.collisions,
            material_costs: self.setup.playback.material_penetration_costs(),
            terrain_materials: Some(&self.setup.terrain),
        }
    }
}

pub(crate) struct Frame<'a, W: Write> {
    pub(crate) engine: &'a mut Engine<W>,
    pub(crate) ground: &'a mut ground::Runtime,
    pub(crate) spawns: &'a mut SpawnCatalog,
    pub(crate) native: &'a mut BTreeMap<u32, NativeState>,
    pub(crate) ground_changes: &'a mut BTreeMap<u16, u32>,
    pub(crate) audit: &'a mut Vec<Value>,
    pub(crate) messages: &'a mut Vec<OutboundMessage>,
    pub(crate) sequences: &'a mut BTreeMap<u16, ObjectiveSequenceDelta>,
    pub(crate) filter_rows: &'a [u32; 32],
    pub(crate) physics: Settings,
    pub(crate) ground_policy: Option<&'a crate::acquisition_ground_damage::Policy>,
    pub(crate) now: Duration,
}

impl Runtime {
    pub(crate) fn stage_entry<W: Write>(
        &mut self,
        entry: NativeEntry,
        frame: &mut Frame<'_, W>,
    ) -> Result<(), &'static str> {
        let index = entry.delta.object_index;
        let state = entry.delta.state_actor.state;
        let row = self
            .setup
            .bindings
            .rows
            .get(&index)
            .filter(|row| row.class == entry.class)
            .ok_or("acquisition_structural_entry_unbound")?;
        let token = row.token;
        let hook = row.hooks.get(&(state as u8)).cloned();

        if let Some(spawn) = row.spawn
            && matches!(
                state,
                ObjectiveState::Completed | ObjectiveState::Deactivated | ObjectiveState::Repaired
            )
        {
            self.stage_fsp(entry, token, spawn, frame)?;
        }
        if entry.requires_authored_explosion() {
            let request = self.setup.playback.explosion(token, entry)?;
            if request.properties.damage_value != 0. {
                let root = self.setup.playback.catalog().objective_pose(token)?;
                let blast = request.properties.instantaneous_blast(
                    root.translation.map(|value| value as f32),
                    row.trigger_position(entry.active_trigger_index)?,
                )?;
                let source = if entry.delta.state_actor.actor_index == u16::MAX {
                    u32::MAX
                } else {
                    0x0380_0000 | u32::from(entry.delta.state_actor.actor_index)
                };
                self.stage_damage(frame, |engine, collision| {
                    engine
                        .collect_domination_blast(
                            BlastSource::Structural {
                                objective_id: 0x0c80_0000 | u32::from(index),
                            },
                            source,
                            blast,
                            collision,
                        )
                        .map(|work| vec![work])
                })?;
            }
        }
        if let Some(hook) = hook {
            self.stage_hook(
                index,
                state,
                entry.delta.state_actor.actor_index,
                hook,
                frame,
            )?;
        }
        if entry.class == NativeClass::Structural {
            let root = self.setup.playback.catalog().objective_pose(token)?;
            if let Some(delta) = self.setup.playback.prepare_sequence(
                token,
                entry,
                frame.now.as_millis() as u32,
                |_, playback| {
                    self.collisions.prepare_reference_playback(
                        index,
                        root,
                        playback,
                        &self.setup.assets,
                    )
                },
            )? {
                frame.sequences.insert(index, delta);
            }
        }
        frame.audit.push(json!({
            "op":"acquisition_structural_entry", "object":index,
            "previous": entry.previous as u8, "state":state as u8,
            "explosion":entry.requires_authored_explosion(),
        }));
        Ok(())
    }

    pub(crate) fn stage_world<W: Write>(
        &mut self,
        world: &mut ObjectiveWorld,
        frame: &mut Frame<'_, W>,
        starting: bool,
    ) -> Result<BTreeMap<u16, crate::replication::ObjectiveControlDelta>, &'static str> {
        if starting {
            self.setup.configure_spawns(frame.spawns)?;
        }
        if self.advances_native_objectives() {
            world.begin_native_tick(frame.now);
        }
        let world_id = frame
            .engine
            .active
            .iter()
            .chain(frame.engine.peers.values())
            .next()
            .map(|actor| actor.reservation.world)
            .ok_or("acquisition_structural_world_missing")?;
        // The v2 projection verifies that these client slots match Sim family-12 order.
        let indices: Vec<_> = self.setup.bindings.rows.keys().copied().collect();
        let mut deltas = BTreeMap::new();
        for index in indices {
            let state_before_update = world.state(index);
            let row = &self.setup.bindings.rows[&index];
            let class = row.class;
            let request = self.queued.remove(&index).or_else(|| {
                (starting && row.start_deactivated)
                    .then_some((ObjectiveState::Deactivated, u16::MAX))
            });
            if world.native_class(index) == Some(NativeClass::Structural) {
                if let Some((state, actor)) = request
                    && world.state(index) != Some(state)
                {
                    // MotorPool hooks can request an already-current state.
                    // Preserve that no-op instead of clearing a pending strike.
                    let source = (actor != u16::MAX)
                        .then(|| crate::acquisition_gameplay_engine::objective_actors(frame.engine))
                        .and_then(|actors| actors.get(&actor).copied())
                        .map(|actor| crate::domination_objective_runtime::Actor {
                            player: actor.player,
                            generation: actor.generation.into(),
                            team: actor.team,
                        });
                    world.stage_native_state(index, state, source)?;
                }
                // Earlier entries and area tails can kill or unseat a holder.
                // Resolve the actor against the staged world at this slot.
                let actors = if world.interaction_owner(index).is_some() {
                    crate::acquisition_gameplay_engine::objective_actors(frame.engine)
                } else {
                    BTreeMap::new()
                };
                loop {
                    if let Some(delta) = world.tick_native_object(index, frame.now, &actors) {
                        deltas.insert(index, delta);
                    }
                    for entry in world.take_native_entries() {
                        self.stage_entry(entry, frame)?;
                    }
                    if !world.has_precision_transition(index) {
                        break;
                    }
                }
            } else {
                if let Some((state, actor)) = request
                    && let Some(entry) = world
                        .enter_acquisition_spawn_state(index, class, state, actor, frame.now)?
                {
                    self.stage_entry(entry, frame)?;
                    deltas.insert(index, entry.delta);
                }
                if let Some(delta) =
                    world.advance_acquisition_precision_entry(index, frame.now, |entry, _| {
                        self.stage_entry(entry, frame)
                    })?
                {
                    deltas.insert(index, delta);
                }
            }
            if class == NativeClass::Structural
                && let Some(state @ (ObjectiveState::Completed | ObjectiveState::Repairing)) =
                    state_before_update
            {
                self.stage_update_tail(index, world_id, state, frame)?;
            }
        }
        Ok(deltas)
    }

    fn stage_update_tail<W: Write>(
        &mut self,
        index: u16,
        world_id: u32,
        state: ObjectiveState,
        frame: &mut Frame<'_, W>,
    ) -> Result<(), &'static str> {
        let properties = self.setup.playback.blast_properties(index)?;
        if properties.character_damage_over_time_radius == 0.0
            || properties.character_damage_over_time_value == 0.0
        {
            return Ok(());
        }
        let token = self.setup.bindings.rows[&index].token;
        let objective_position = self
            .setup
            .playback
            .catalog()
            .objective_pose(token)?
            .translation
            .map(|value| value as f32);
        self.stage_damage(frame, |_, _| {
            Ok(vec![DamageWork::Area(AreaUpdate {
                objective_index: index,
                objective_position,
                updated_state: state,
                world: world_id,
                properties,
            })])
        })
    }

    fn stage_damage<W: Write>(
        &mut self,
        frame: &mut Frame<'_, W>,
        collect: impl FnOnce(&Engine<W>, BlastCollision<'_>) -> Result<Vec<DamageWork>, &'static str>,
    ) -> Result<(), &'static str> {
        let prior = frame.engine.ground_runtime.replace(frame.ground.clone());
        let work = collect(
            frame.engine,
            self.blast_collision(frame.filter_rows, frame.physics),
        );
        let result = work.and_then(|work| {
            frame.engine.stage_domination_damage_with_ground(
                &mut self.area,
                frame.native,
                frame.now,
                &work,
                frame.ground_policy,
            )
        });
        *frame.ground = std::mem::replace(&mut frame.engine.ground_runtime, prior)
            .ok_or("acquisition_structural_ground_runtime_missing")?;
        frame.install_damage(result?)
    }

    fn stage_hook<W: Write>(
        &mut self,
        objective: u16,
        state: ObjectiveState,
        actor: u16,
        hook: Hook,
        frame: &mut Frame<'_, W>,
    ) -> Result<(), &'static str> {
        let completed = state == ObjectiveState::Completed;
        if let Some((platoon, enable_if_completed, abilities)) = hook.commands() {
            let mask = abilities.iter().try_fold(0, |mask, &ability| {
                Ok::<_, &'static str>(mask | native_ability_mask(ability)?)
            })?;
            let enabled = completed == enable_if_completed;
            frame
                .engine
                .set_acquisition_command_enabled(platoon, mask, enabled)?;
            frame.audit.push(json!({
                "op":"acquisition_structural_command_availability", "objective":objective,
                "platoon":platoon, "mask":mask, "enabled":enabled,
            }));
        }
        let state = if completed {
            ObjectiveState::Completed
        } else {
            ObjectiveState::Deactivated
        };
        let actor = if matches!(hook, Hook::MotorPool { .. }) {
            u16::MAX
        } else {
            actor
        };
        for &target in hook.spawn_targets() {
            self.queued.insert(target, (state, actor));
        }
        Ok(())
    }

    fn stage_fsp<W: Write>(
        &self,
        entry: NativeEntry,
        token: u32,
        spawn: Spawn,
        frame: &mut Frame<'_, W>,
    ) -> Result<(), &'static str> {
        let plan = self
            .setup
            .bindings
            .plan_fsp(token, spawn, entry.delta.state_actor.state)?;
        let (spawns, ()) = fsp::stage_spawn_effects(frame.spawns, &plan)?.complete(|requests| {
            if spawn.vehicle {
                for &request in requests {
                    frame.stage_vehicle(request, entry.delta.object_index)?;
                }
            }
            Ok(())
        })?;
        *frame.spawns = spawns;
        Ok(())
    }
}

impl<W: Write> Frame<'_, W> {
    fn install_damage(
        &mut self,
        commit: crate::domination_structural_world::DamageCommit,
    ) -> Result<(), &'static str> {
        if !commit.helo_doses.is_empty() {
            return Err("acquisition_structural_unexpected_helo");
        }
        for index in commit.changed_ground {
            *self.ground_changes.entry(index).or_default() |= 1 << 18;
        }
        self.audit.extend(commit.audit);
        self.messages.extend(commit.messages);
        Ok(())
    }

    fn stage_vehicle(
        &mut self,
        request: fsp::VehicleRequest,
        index: u16,
    ) -> Result<(), &'static str> {
        let spawn = request.point.native_id as u16;
        let vehicle = self
            .ground
            .vehicles()
            .find(|vehicle| {
                vehicle.source_spawn_point_index == spawn
                    && vehicle.source_spawn_point_token == request.point.token
                    && vehicle.source_spawn_type == Some(7)
            })
            .ok_or("acquisition_structural_motor_pool_missing")?;
        let key = vehicle.key;
        if request.hook == VehicleHook::OnFspDeactivate {
            return self.ground.disable_cycle(key);
        }
        let creation = self
            .engine
            .ground_vehicles
            .as_ref()
            .and_then(|catalog| catalog.vehicle(key.index))
            .filter(|creation| {
                creation.spawn_point_index == spawn
                    && creation.spawn_point_token == request.point.token
            })
            .ok_or("acquisition_structural_motor_pool_creation_missing")?;
        let pose = ground::Pose {
            position: creation.position,
            rotation: creation.rotation,
        };
        self.ground
            .enable_cycle(key, 0x0c80_0000 | u32::from(index))?;
        let events = self.ground.respawn_existing(
            key,
            ground::QualifiedRespawn {
                spawn_point_index: spawn,
                spawn_point_token: request.point.token,
                active: true,
                pose,
            },
            false,
        )?;
        for event in events {
            self.audit.push(json!({"op":"acquisition_structural_motor_pool", "vehicle":event.vehicle.object_id(),
                "effect":format!("{:?}",event.effect)}));
            let flags = match event.effect {
                ground::Effect::ForcePosition(_) => 1 << 25,
                ground::Effect::ForceRotation(_) => 1 << 26,
                ground::Effect::DamageReset => 1 << 18,
                ground::Effect::StateChanged(_) => 1 << 5,
                ground::Effect::KillSpawnBlockers => {
                    let previous = self.engine.ground_runtime.replace(self.ground.clone());
                    let output = self.engine.stage_respawn_clearance(
                        Clearance {
                            source: key,
                            physics: self.physics,
                            filter_rows: *self.filter_rows,
                            now: self.now,
                        },
                        self.native,
                        self.ground_changes,
                    );
                    *self.ground = std::mem::replace(&mut self.engine.ground_runtime, previous)
                        .ok_or("acquisition_structural_ground_runtime_missing")?;
                    let (audit, messages) = output?;
                    self.audit.extend(audit);
                    self.messages.extend(messages);
                    0
                }
                _ => 0,
            };
            if flags != 0 {
                *self.ground_changes.entry(key.index).or_default() |= flags;
            }
        }
        Ok(())
    }
}
