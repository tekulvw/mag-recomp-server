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
    objective_control::{NativeEntry, ObjectiveActor, ObjectiveWorld},
    protocol::{ObjectiveInteraction, ObjectiveInteractionKind},
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

#[cfg(test)]
#[path = "acquisition_published_sequence_tests.rs"]
mod published_sequence_tests;

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
    pub(crate) fn projectile_sequence_assets(&self) -> &SequenceCollisionAssets {
        &self.assets
    }

    /// Published controller sequences in (object index, token) order. None
    /// may share an index with a bound Structural or passive spawn row.
    pub(crate) fn published_objectives(&self) -> Result<Vec<(u16, u32)>, &'static str> {
        let published: Vec<_> = self.playback.published_objectives().collect();
        if published
            .iter()
            .any(|(index, _)| self.bindings.rows.contains_key(index))
        {
            return Err("acquisition_published_sequence_overlaps_structural");
        }
        Ok(published)
    }

    pub(crate) fn validate_frag_character_inputs(
        &self,
        items: &crate::item_catalog::ItemCatalog,
        world: Option<&crate::collision::CollisionWorld>,
        row: u32,
    ) -> Result<(), &'static str> {
        crate::domination_gameplay_config::validate_frag_character_inputs(
            items,
            world,
            &self.bindings.map,
        )?;
        let world = world.ok_or("frag_character_blast_requires_static_world")?;
        let known = self
            .playback
            .material_penetration_costs()
            .keys()
            .copied()
            .collect();
        world.validate_material_ray_bindings(row, &known, Some(&self.terrain.material_slots()))
    }

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

    /// What each bound Structural row is to bots breaking a lane's defences
    /// (bot_breach), in row order. A row whose authored hooks
    /// open passive drop objectives is anti-air, with the spawn points those
    /// objectives open; other hooks (mortar, sensor, motor pool) are support.
    /// A row without hooks that is the forward spawn of an on-foot squad
    /// point is a bunker; the rest are obstacles (gates, barricades).
    pub(crate) fn bot_stage_roles(&self) -> Vec<(u16, crate::bot_breach::Role)> {
        use crate::bot_breach::Role;
        let rows = &self.bindings.rows;
        rows.iter()
            .filter(|(_, row)| row.class == NativeClass::Structural)
            .map(|(&index, row)| {
                let role = if let Some(Hook::Antiaircraft { spawns, .. }) = row
                    .hooks
                    .values()
                    .find(|hook| matches!(hook, Hook::Antiaircraft { .. }))
                {
                    let mut opened: Vec<u16> = spawns
                        .iter()
                        .filter_map(|target| rows.get(target)?.spawn.map(|s| s.index))
                        .collect();
                    opened.dedup();
                    Role::AntiAir(opened)
                } else if !row.hooks.is_empty() {
                    Role::Support
                } else if let Some(spawn) = row.spawn.filter(|s| !s.vehicle) {
                    Role::OwnerSpawn(spawn.index)
                } else if row.spawn.is_some() {
                    Role::Support
                } else {
                    Role::Obstacle
                };
                (index, role)
            })
            .collect()
    }

    /// Each squad's authored spawn points, by squad index.
    pub(crate) fn squad_spawn_points(&self) -> Vec<(u16, Vec<u16>)> {
        self.bindings
            .squads
            .iter()
            .map(|(&squad, s)| {
                (
                    u16::from(squad),
                    s.points.iter().map(|&(index, _)| index).collect(),
                )
            })
            .collect()
    }

    /// Every bound field hospital must be the forward spawn of one of this
    /// map's Structural objectives, whose FSP then drives its emplacement.
    pub(crate) fn validate_hospitals(
        &self,
        hospitals: &crate::structures::BunkerWorld,
    ) -> Result<(), &'static str> {
        self.bindings.validate_hospitals(hospitals)
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
        self.bindings.configure_spawns(spawns)
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
    fn validate_hospitals(
        &self,
        hospitals: &crate::structures::BunkerWorld,
    ) -> Result<(), &'static str> {
        for hospital in hospitals.hospitals() {
            let bound = self.rows.values().any(|row| {
                row.class == NativeClass::Structural
                    && row.spawn.is_some_and(|spawn| {
                        spawn.index == hospital.spawn_index
                            && spawn.team == hospital.team
                            && (spawn.user_class, spawn.spawn_type, spawn.vehicle) == (0, 8, false)
                    })
            });
            if !bound {
                return Err("acquisition_hospital_forward_spawn_unbound");
            }
        }
        Ok(())
    }

    fn configure_spawns(&self, spawns: &mut SpawnCatalog) -> Result<(), &'static str> {
        // Structural forward spawns (the defenders' field hospitals) a spawn
        // catalog leaves unsupported fall back locally to each squad's
        // category-zero point. A catalog that makes them selectable lets
        // players and bots use them directly; those need no fallback.
        let unselectable: BTreeSet<u16> = self
            .rows
            .values()
            .filter_map(|row| row.spawn)
            .filter(|spawn| spawn.user_class == 0)
            .map(|spawn| spawn.index)
            .filter(|&index| {
                spawns.point(index).is_none_or(|point| {
                    matches!(point.kind, crate::spawn_catalog::SpawnKind::Unsupported)
                })
            })
            .collect();
        if unselectable.is_empty() {
            return Ok(());
        }
        let mut category_zero = [u16::MAX; 32];
        for (&squad, row) in &self.squads {
            category_zero[usize::from(squad)] = row
                .category_zero
                .ok_or("acquisition_structural_category_zero_missing")?
                .0;
        }
        spawns.configure_acquisition_structural_spawn_scope(unselectable, category_zero)
    }

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

/// Identity of a native Structural contact hold started by the source slot walk.
#[derive(Clone, Copy)]
struct BotStructuralHold {
    player: u16,
    allocation: u32,
    life: u32,
    team: u8,
    world: u32,
    round: u64,
    trigger: u8,
    planting: bool,
}

impl BotStructuralHold {
    fn actor<W: Write>(self, engine: &Engine<W>, world: u32) -> Option<ObjectiveActor> {
        if self.world != world
            || self.round != engine.round_number
            || !engine.accepts_subscription_world(world)
            || engine
                .active
                .iter()
                .chain(engine.peers.values())
                .any(|actor| actor.reservation.player == self.player)
        {
            return None;
        }
        let bot = engine.bot_registry.bots.get(&self.player)?;
        let runtime = bot.target.runtime?;
        if bot.allocation_generation != self.allocation
            || runtime.generation != self.life
            || bot.target.squad.team() != self.team
            || !matches!(
                runtime.phase,
                crate::development_target::BotPhase::Moving
                    | crate::development_target::BotPhase::Complete
            )
            || runtime.physics_motion.is_none_or(|motion| motion.blocked)
            || !crate::bot_objectives::operator_safe(bot)
        {
            return None;
        }
        let [x, y, z] = runtime.physics_motion?.position.map(|value| value as f32);
        Some(ObjectiveActor {
            player: self.player,
            generation: self.life,
            team: self.team,
            position: crate::replication::CharacterPosition { x, y, z },
        })
    }

    fn request(self, index: u16, kind: ObjectiveInteractionKind) -> ObjectiveInteraction {
        ObjectiveInteraction {
            target_object_id: 0x0c80_0000 | u32::from(index),
            actor_index: self.player,
            trigger_index: self.trigger,
            kind,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Runtime {
    setup: Setup,
    collisions: SequenceCollisionState,
    area: AreaDamageState,
    queued: BTreeMap<u16, (ObjectiveState, u16)>,
    mission_start_staged: bool,
    bot_holds: BTreeMap<u16, BotStructuralHold>,
}

impl Runtime {
    pub(crate) fn bot_regional_bounds(
        &self,
        clv_sha256: &str,
        budget: &mut usize,
    ) -> Result<Vec<crate::actor_collision::Bounds>, &'static str> {
        if self.setup.bindings.level_clv_sha256 != clv_sha256 {
            return Err("bot_regional_structural_source_mismatch");
        }
        self.collisions.bot_regional_bounds(budget)
    }

    pub(crate) fn new(setup: Setup) -> Self {
        Self {
            setup,
            collisions: SequenceCollisionState::default(),
            area: AreaDamageState::default(),
            queued: BTreeMap::new(),
            mission_start_staged: false,
            bot_holds: BTreeMap::new(),
        }
    }

    pub(crate) fn sequences(&self) -> Vec<ObjectiveSequenceDelta> {
        self.setup.playback.current_sequences()
    }

    /// Publish (never replay) the authored sequence a committed controller
    /// state selects. Other objectives and unchanged selections yield None.
    pub(crate) fn stage_published_sequence(
        &mut self,
        index: u16,
        state: ObjectiveState,
        game_time_ms: u32,
    ) -> Result<Option<ObjectiveSequenceDelta>, &'static str> {
        self.setup
            .playback
            .prepare_published_sequence(index, state, game_time_ms)
    }

    /// LOCAL DIAGNOSTIC (headless simulation only): request a Structural
    /// state the way an authored script request does, for the next tick.
    pub(crate) fn queue_scripted_state(
        &mut self,
        index: u16,
        state: ObjectiveState,
    ) -> Result<(), &'static str> {
        if !self
            .setup
            .bindings
            .rows
            .get(&index)
            .is_some_and(|row| row.class == NativeClass::Structural)
        {
            return Err("acquisition_structural_entry_unbound");
        }
        self.queued.insert(index, (state, u16::MAX));
        Ok(())
    }

    /// The authored blast of a structural objective, when it has one.
    pub(crate) fn blast_properties(
        &self,
        index: u16,
    ) -> Option<crate::domination_structural_effects::BlastProperties> {
        self.setup.playback.blast_properties(index).ok()
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
    pub(crate) assignments: &'a [(u16, crate::acquisition_objective_runtime::Assignment)],
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
    /// The staged field-hospital emplacements (copied on their first change)
    /// and the family-13 updates their forward spawns produce.
    pub(crate) hospitals: &'a mut Option<crate::structures::BunkerWorld>,
    pub(crate) hospital_packets: &'a mut Vec<Vec<u8>>,
    /// The team vehicle waves, once started. A motor-pool activation cancels
    /// the entry of the vehicle it respawns directly.
    pub(crate) vehicle_waves: Option<&'a mut crate::acquisition_vehicle_waves::VehicleWaves>,
    pub(crate) now: Duration,
}

impl Runtime {
    pub(crate) fn stage_entry<W: Write>(
        &mut self,
        entry: NativeEntry,
        world: &mut ObjectiveWorld,
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
        let assignment = frame
            .engine
            .active
            .iter()
            .chain(frame.engine.peers.values())
            .find(|a| Some(a.reservation.player) == entry.reward_actor)
            .and_then(|a| {
                frame
                    .assignments
                    .iter()
                    .find(|(index, _)| *index == a.squad_catalog.squad_index())
                    .map(|(_, assignment)| *assignment)
            });
        let effects = frame.engine.stage_native_objective_callbacks(
            entry,
            assignment,
            frame.assignments,
            crate::frago::Frame {
                ground: Some(frame.ground),
                ..Default::default()
            },
        )?;
        frame.audit.extend(effects.audit);
        frame.messages.extend(effects.messages);
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
                        .map(|mut work| {
                            if let DamageWork::Blast { bot_plant, .. } = &mut work {
                                *bot_plant = entry.bot_plant;
                            }
                            vec![work]
                        })
                })?;
            }
        }
        if let Some(hook) = hook {
            self.stage_hook(index, state, world.script_actor(index), hook, world, frame)?;
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

    pub(crate) fn stage_mission_start(
        &mut self,
        world: &mut ObjectiveWorld,
    ) -> Result<(), &'static str> {
        if !self.mission_start_staged {
            for (&index, row) in &self.setup.bindings.rows {
                if row.start_deactivated && world.native_class(index).is_some() {
                    world.stage_native_target(index, ObjectiveState::Deactivated)?;
                }
            }
            self.mission_start_staged = true;
        }
        Ok(())
    }

    pub(crate) fn bot_owns_hold(&self, world: &ObjectiveWorld, index: u16, player: u16) -> bool {
        self.bot_holds.get(&index).is_some_and(|hold| {
            hold.player == player
                && world.interaction_owner(index) == Some((player, hold.life, hold.trigger))
        })
    }

    fn stage_bot_hold<W: Write>(
        &mut self,
        index: u16,
        world_id: u32,
        world: &mut ObjectiveWorld,
        frame: &mut Frame<'_, W>,
        script_pending: bool,
    ) -> BTreeMap<u16, ObjectiveActor> {
        let owner = world.interaction_owner(index);
        let mut actors = if owner.is_some() {
            crate::acquisition_gameplay_engine::objective_actors(frame.engine)
        } else {
            BTreeMap::new()
        };
        let live_fuse = world.native_structural_live_contact_fuse_trigger(index, frame.now);
        let enabled = frame.engine.round_combat_allowed(frame.now)
            && frame
                .engine
                .bot_registry
                .config
                .as_ref()
                .is_some_and(|config| {
                    config.autonomous
                        && config
                            .objective_policy
                            .as_ref()
                            .is_some_and(|policy| policy.interactions_enabled)
                });
        if let Some(hold) = self.bot_holds.get(&index).copied() {
            if owner == Some((hold.player, hold.life, hold.trigger)) {
                // An actor with a reused packed index cannot sustain this native hold.
                actors.remove(&hold.player);
                if let Some(actor) = hold.actor(frame.engine, world_id) {
                    actors.insert(hold.player, actor);
                    let bot = &frame.engine.bot_registry.bots[&hold.player];
                    if !script_pending
                        && (hold.planting || live_fuse == Some(hold.trigger))
                        && (!enabled
                            || bot.task.recovering
                            || !bot.task.is_operator_for(index, hold.life))
                    {
                        let released = world.interact(
                            hold.request(index, ObjectiveInteractionKind::Release),
                            actor,
                            frame.now,
                        );
                        if released.accepted {
                            frame.audit.push(
                                json!({"op":if hold.planting { "acquisition_bot_plant_release" } else { "acquisition_bot_defuse_release" },"object":index,
                                "player":hold.player,"allocation":hold.allocation,"life":hold.life,
                                "trigger":hold.trigger}),
                            );
                        }
                    }
                }
                return actors;
            }
            self.bot_holds.remove(&index);
        }
        if !enabled || script_pending || owner.is_some() {
            return actors;
        }
        let planting = live_fuse.is_none()
            && matches!(
                world.state(index),
                Some(ObjectiveState::Deactivated | ObjectiveState::Repaired)
            );
        if live_fuse.is_none() && !planting {
            return actors;
        }
        let Some(rule) = frame
            .engine
            .objective_catalog
            .as_ref()
            .and_then(|catalog| {
                catalog
                    .objects
                    .iter()
                    .find(|object| object.object_index == index)
            })
            .and_then(|object| object.demolition.as_ref())
        else {
            return actors;
        };
        let owner_team = rule.owner_team;
        if !planting && !rule.can_defuse {
            return actors;
        }
        let selected = frame
            .engine
            .bot_registry
            .bots
            .iter()
            .find_map(|(&player, bot)| {
                let life = bot.target.runtime?.generation;
                let team = bot.target.squad.team();
                if (team == owner_team) == planting
                    || bot.task.recovering
                    || !bot.task.is_operator_for(index, life)
                {
                    return None;
                }
                let mut hold = BotStructuralHold {
                    player,
                    allocation: bot.allocation_generation,
                    life,
                    team,
                    world: world_id,
                    round: frame.engine.round_number,
                    trigger: live_fuse.unwrap_or_default(),
                    planting,
                };
                let actor = hold.actor(frame.engine, world_id)?;
                let accepted = if planting {
                    hold.trigger = (0..rule.trigger_count).find(|&trigger| {
                        crate::objective_control::at_native_trigger(
                            actor.position,
                            rule.range(),
                            trigger,
                        )
                    })?;
                    world.stage_native_bot_plant(
                        hold.request(index, ObjectiveInteractionKind::Activate),
                        actor,
                        crate::objective_control::BotPlantSource {
                            player: hold.player,
                            allocation: hold.allocation,
                            life: hold.life,
                            team: hold.team,
                            world: hold.world,
                            round: hold.round,
                        },
                        frame.now,
                    )
                } else {
                    world
                        .interact(
                            hold.request(index, ObjectiveInteractionKind::Deactivate),
                            actor,
                            frame.now,
                        )
                        .accepted
                };
                accepted.then_some((hold, actor))
            });
        if let Some((hold, actor)) = selected {
            self.bot_holds.insert(index, hold);
            actors.insert(hold.player, actor);
            let bot = frame
                .engine
                .bot_registry
                .bots
                .get_mut(&hold.player)
                .expect("selected registered bot");
            let runtime = bot.target.runtime.as_mut().expect("selected live bot");
            runtime.phase = crate::development_target::BotPhase::Complete;
            bot.target.pending_move = None;
            bot.task.goal = Some(actor.position);
            bot.task.role_goal = Some(actor.position);
            frame
                .audit
                .push(json!({"op":if hold.planting { "acquisition_bot_plant_started" } else { "acquisition_bot_defuse_started" },"object":index,
                "world":hold.world,"round":hold.round,"player":hold.player,
                "allocation":hold.allocation,"life":hold.life,"trigger":hold.trigger}));
        }
        actors
    }

    pub(crate) fn stage_world<W: Write>(
        &mut self,
        world: &mut ObjectiveWorld,
        frame: &mut Frame<'_, W>,
        starting: bool,
    ) -> Result<BTreeMap<u16, crate::replication::ObjectiveControlDelta>, &'static str> {
        if starting {
            self.stage_mission_start(world)?;
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
            let script_request = self.queued.remove(&index);
            let request = script_request.or_else(|| {
                (starting && row.start_deactivated && world.native_class(index).is_none())
                    .then_some((ObjectiveState::Deactivated, u16::MAX))
            });
            if world.native_class(index) == Some(NativeClass::Structural) {
                if let Some((state, slot)) = script_request {
                    let raw_slot = (slot != u16::MAX).then_some(slot);
                    let source_world = frame
                        .engine
                        .active
                        .iter()
                        .chain(frame.engine.peers.values())
                        .next()
                        .map(|a| a.reservation.world);
                    let resolved = raw_slot.zip(source_world).and_then(|(player, world)| {
                        frame.engine.objective_script_character(player, world)
                    });
                    world.stage_native_state_with_caller(index, state, raw_slot, resolved)?;
                }
                // Earlier entries and area tails can kill or unseat a holder.
                // Resolve the actor against the staged world at this slot.
                let actors =
                    self.stage_bot_hold(index, world_id, world, frame, script_request.is_some());
                let mut transitions = 0;
                loop {
                    let delta = world.tick_native_object(index, frame.now, &actors);
                    let progressed = delta.is_some() || world.has_native_entries();
                    if let Some(delta) = delta {
                        deltas.insert(index, delta);
                    }
                    for entry in world.take_native_entries() {
                        self.stage_entry(entry, world, frame)?;
                    }
                    if !progressed
                        || !(world.has_precision_transition(index)
                            || world.has_script_transition(index))
                    {
                        break;
                    }
                    transitions += 1;
                    if transitions >= 256 {
                        return Err("native_objective_transition_limit");
                    }
                }
                if let Some(hold) = self.bot_holds.get(&index)
                    && world.interaction_owner(index)
                        != Some((hold.player, hold.life, hold.trigger))
                {
                    self.bot_holds.remove(&index);
                }
            } else {
                if let Some((state, actor)) = request
                    && let Some(entry) = world
                        .enter_acquisition_spawn_state(index, class, state, actor, frame.now)?
                {
                    self.stage_entry(entry, world, frame)?;
                    deltas.insert(index, entry.delta);
                }
                if let Some(delta) = world.advance_acquisition_precision_entry(
                    index,
                    frame.now,
                    |entry, world| self.stage_entry(entry, world, frame),
                )? {
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
        actor: Option<u16>,
        hook: Hook,
        world: &mut ObjectiveWorld,
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
        let raw_slot = if matches!(hook, Hook::MotorPool { .. }) {
            None
        } else {
            actor
        };
        let source_world = frame
            .engine
            .active
            .iter()
            .chain(frame.engine.peers.values())
            .next()
            .map(|a| a.reservation.world);
        for &target in hook.spawn_targets() {
            if world.native_class(target).is_some() {
                let resolved = raw_slot.zip(source_world).and_then(|(player, world)| {
                    frame.engine.objective_script_character(player, world)
                });
                world.stage_native_state_with_caller(target, state, raw_slot, resolved)?;
            } else {
                self.queued
                    .insert(target, (state, raw_slot.unwrap_or(u16::MAX)));
            }
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
            for &request in requests {
                if spawn.vehicle {
                    frame.stage_vehicle(request, entry.delta.object_index)?;
                } else {
                    frame.stage_hospital(request, entry.delta.object_index)?;
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

    /// A field hospital's forward spawn drives its emplacement the way a
    /// Domination hospital's does: deactivation destroys the body and kills
    /// its seated occupants through the shared character pipeline, and
    /// activation restores it. Points without a bound emplacement have no
    /// vehicle work.
    fn stage_hospital(
        &mut self,
        request: fsp::VehicleRequest,
        objective: u16,
    ) -> Result<(), &'static str> {
        let spawn = request.point.native_id as u16;
        // Stage a copy of the emplacements only once a bound one changes.
        if self.hospitals.is_none() {
            *self.hospitals = self
                .engine
                .bunker_world
                .as_ref()
                .filter(|world| world.hospital_at_spawn(spawn).is_some())
                .cloned();
        }
        let Some(hospitals) = self
            .hospitals
            .as_mut()
            .filter(|world| world.hospital_at_spawn(spawn).is_some())
        else {
            return Ok(());
        };
        let operational = request.hook == VehicleHook::OnFspActivate;
        let damage_id = 0x0c80_0000 | u32::from(objective);
        let update = hospitals.apply_hospital_cycle(spawn, operational, damage_id)?;
        if update.kills_occupants() {
            let previous = self.engine.bunker_world.replace(hospitals.clone());
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
        let packet = update.encode(1, self.now.as_millis() as u32, self.engine.wire_profile)?;
        if packet.is_some() || update.kills_occupants() {
            // No hulk or destruction blast is bound for Acquisition: the
            // Structural objective's own sequence supplies the wreck.
            self.audit.push(json!({
                "op":"acquisition_hospital_cycle", "objective":objective,
                "spawn_point_index":spawn, "bunker_index":update.object_index(),
                "operational":operational, "occupants_killed":update.kills_occupants(),
                "hulk":"structural_sequence",
            }));
        }
        self.hospital_packets.extend(packet);
        Ok(())
    }

    pub(crate) fn stage_vehicle(
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
        if let Some((team, queued)) = self
            .vehicle_waves
            .as_deref_mut()
            .and_then(|waves| waves.cancel(key.index))
        {
            self.audit.push(
                json!({"op":"acquisition_structural_motor_pool_wave_entry_cancelled",
                "vehicle":key.object_id(),"team":team,"queued_generation":queued.generation}),
            );
        }
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

#[cfg(test)]
#[path = "acquisition_field_hospital_tests.rs"]
mod field_hospital_tests;

#[cfg(test)]
#[path = "acquisition_lalo_tests.rs"]
mod lalo_tests;
