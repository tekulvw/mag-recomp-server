//! Explicit private source paths for the retail Domination provider.
use crate::{
    Engine, acquisition_ground_damage,
    domination_control::Catalog as Controller,
    domination_gameplay_effects::{OwnedAssignment, Structural},
    domination_gameplay_engine::{Runtime, Setup},
    domination_gameplay_provider::{State, World},
    domination_structural_effects::{Catalog as StructuralEffects, TerrainMaterialGrid},
    domination_structural_world::{
        HospitalHulkAssets, SequenceCollisionAssets, SequenceCollisionState,
    },
    dynamic_collision::Pose,
    ground_vehicles,
    objective_catalog::ObjectiveCatalog,
    spawn_catalog::SpawnCatalog,
    structures::BunkerWorld,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Read, Write},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    format: String,
    schema_version: u8,
    controller: PathBuf,
    mission_start: PathBuf,
    sim_schedule: PathBuf,
    forward_spawns: PathBuf,
    structural_effects: PathBuf,
    sequence_assets: PathBuf,
    initial_assignments: PathBuf,
    core: Option<Core>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Core {
    scope: String,
    aircraft_links: PathBuf,
    hospital_hulks: PathBuf,
    hospital_turrets: Option<PathBuf>,
    hospital_rockets: Option<PathBuf>,
    hospital_rocket_ground: Option<PathBuf>,
    #[serde(default)]
    frag_character_blast: bool,
    projectile_contacts: Option<PathBuf>,
    ground_turrets: Option<PathBuf>,
    initial_filters: PathBuf,
    qualified_poses: PathBuf,
    ground_damage: PathBuf,
    ground_damage_sha256: String,
    ground_lifecycle: PathBuf,
    ground_lifecycle_sha256: String,
    terrain_material_index: PathBuf,
    terrain_material_pixels: PathBuf,
    helo: Option<Helo>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Helo {
    fleet: PathBuf,
    models: PathBuf,
    paths: PathBuf,
    damage: PathBuf,
    hospital_rocket_damage_areas: Option<PathBuf>,
    random_seed: u32,
}

impl Core {
    fn validate_scope(&self) -> io::Result<()> {
        if matches!(
            (self.scope.as_str(), self.helo.as_ref()),
            ("core_without_helo_insertion", None) | ("core_with_helo_insertion", Some(_))
        ) {
            if self
                .helo
                .as_ref()
                .is_some_and(|helo| helo.hospital_rocket_damage_areas.is_some())
                && self.hospital_rockets.is_none()
            {
                return Err(io::Error::other("hospital_rocket_helo_requires_rockets"));
            }
            if self.hospital_rocket_ground.is_some() && self.hospital_rockets.is_none() {
                return Err(io::Error::other("hospital_rocket_ground_requires_rockets"));
            }
            if self.hospital_rockets.is_some()
                && (self.hospital_turrets.is_none() || self.projectile_contacts.is_none())
            {
                return Err(io::Error::other(
                    "hospital_rockets_require_turrets_and_contacts",
                ));
            }
            Ok(())
        } else {
            Err(io::Error::other("unknown_domination_gameplay_scope"))
        }
    }
}

#[derive(Deserialize)]
struct GroundLifecycle {
    schema: String,
    native_elf_sha256: String,
    team_interval_ms: [u32; 2],
    material_penetration: f32,
    profiles: Vec<GroundLifecycleProfile>,
    vehicles: Vec<GroundLifecycleVehicle>,
}

#[derive(Deserialize)]
struct GroundLifecycleProfile {
    spawn_sha256: String,
    ground_sha256: String,
    spawn_types: BTreeMap<u16, u8>,
}

#[derive(Deserialize)]
struct GroundLifecycleVehicle {
    vehicle_type_token: u32,
    faction: u8,
    maximum_health_quarter_points: u16,
    capabilities: crate::acquisition_ground_runtime::Capabilities,
}

#[derive(Deserialize)]
struct AircraftLinks {
    schema: String,
    retail_elf_sha256: String,
    initial_spawn_bindings_sha256: String,
    maps: Vec<AircraftMap>,
}

#[derive(Deserialize)]
struct AircraftMap {
    map: String,
    helo_points: Vec<AircraftPoint>,
    lalo_points: Vec<AircraftPoint>,
}

#[derive(Deserialize)]
struct AircraftPoint {
    point_token: u32,
    packed_family35_id: u32,
    spawn_type: u8,
    server_link: Option<String>,
}

#[derive(Deserialize)]
struct SpawnBindings {
    native_spawn_bindings_sha256: String,
}

#[derive(Deserialize)]
struct FilterProof {
    schema: String,
    retail_sha256: String,
    initial_filter_enable_pairs: Vec<[u8; 2]>,
    initial_filter6_allowed_layers: Vec<u8>,
}

#[derive(Deserialize)]
struct Poses {
    schema: String,
    retail_elf_sha256: String,
    metadata_sha256: String,
    client_source_bindings_sha256: String,
    map: String,
    level_archive_sha256: String,
    level_clv_sha256: String,
    objective_poses: Vec<ObjectivePose>,
    trigger_positions: Vec<TriggerPosition>,
}

#[derive(Deserialize)]
struct ObjectivePose {
    token: u32,
    index: u16,
    pose: Pose,
}

#[derive(Deserialize)]
struct TriggerPosition {
    objective_index: u16,
    trigger_index: u8,
    position: [f32; 3],
}

pub struct Sources {
    pub runtime: Runtime,
    pub structural: Structural,
    pub sequence_assets: SequenceCollisionAssets,
    pub platoons: BTreeMap<u32, u8>,
}

#[derive(Deserialize)]
struct Assignments {
    schema: String,
    retail_elf_sha256: String,
    native_manual_tag: u32,
    maps: Vec<AssignmentMap>,
}

#[derive(Deserialize)]
struct AssignmentMap {
    map: String,
    archive_sha256: String,
    clv_sha256: String,
    platoons: Vec<Platoon>,
}

#[derive(Deserialize)]
struct Platoon {
    source_token: u32,
    team: u8,
    platoon: u8,
    slot: u8,
}

impl Configuration {
    /// Read an operator-selected manifest. Nested paths are relative to it.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let mut value: Self =
            serde_json::from_slice(&read(path, 32 * 1024)?).map_err(io::Error::other)?;
        if value.format != "mag_domination_gameplay_configuration_v1" || value.schema_version != 1 {
            return Err(io::Error::other(
                "invalid_domination_gameplay_configuration",
            ));
        }
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other("missing_domination_configuration_directory"))?;
        for input in [
            &mut value.controller,
            &mut value.mission_start,
            &mut value.sim_schedule,
            &mut value.forward_spawns,
            &mut value.structural_effects,
            &mut value.sequence_assets,
            &mut value.initial_assignments,
        ] {
            if input.as_os_str().is_empty() {
                return Err(io::Error::other("empty_domination_gameplay_source_path"));
            }
            *input = directory.join(&*input);
        }
        if let Some(core) = &mut value.core {
            core.validate_scope()?;
            for input in [
                &mut core.aircraft_links,
                &mut core.hospital_hulks,
                &mut core.initial_filters,
                &mut core.qualified_poses,
                &mut core.ground_damage,
                &mut core.ground_lifecycle,
                &mut core.terrain_material_index,
                &mut core.terrain_material_pixels,
            ] {
                if input.as_os_str().is_empty() {
                    return Err(io::Error::other("empty_domination_gameplay_core_path"));
                }
                *input = directory.join(&*input);
            }
            if let Some(path) = &mut core.hospital_turrets {
                if path.as_os_str().is_empty() {
                    return Err(io::Error::other(
                        "empty_domination_hospital_turret_source_path",
                    ));
                }
                *path = directory.join(&*path);
            }
            for (input, error) in [
                (
                    &mut core.hospital_rocket_ground,
                    "empty_domination_hospital_rocket_ground_path",
                ),
                (
                    &mut core.hospital_rockets,
                    "empty_domination_hospital_rocket_source_path",
                ),
                (
                    &mut core.projectile_contacts,
                    "empty_domination_projectile_contact_source_path",
                ),
            ] {
                if let Some(path) = input {
                    if path.as_os_str().is_empty() {
                        return Err(io::Error::other(error));
                    }
                    *path = directory.join(&*path);
                }
            }
            if let Some(path) = &mut core.ground_turrets {
                if path.as_os_str().is_empty() {
                    return Err(io::Error::other(
                        "empty_domination_ground_turret_source_path",
                    ));
                }
                *path = directory.join(&*path);
            }
            if let Some(helo) = &mut core.helo {
                for input in [
                    &mut helo.fleet,
                    &mut helo.models,
                    &mut helo.paths,
                    &mut helo.damage,
                ] {
                    if input.as_os_str().is_empty() {
                        return Err(io::Error::other("empty_domination_helo_source_path"));
                    }
                    *input = directory.join(&*input);
                }
                if let Some(path) = &mut helo.hospital_rocket_damage_areas {
                    if path.as_os_str().is_empty() {
                        return Err(io::Error::other("empty_domination_helo_rocket_areas_path"));
                    }
                    *path = directory.join(&*path);
                }
            }
        }
        Ok(value)
    }

    /// Validate the existing source-bound controller, schedule, FSP and
    /// Structural assets for the chosen profile before preparing the world.
    pub fn load_sources(
        &self,
        objectives: Arc<ObjectiveCatalog>,
        spawns: &SpawnCatalog,
        callback_capacity: NonZeroUsize,
    ) -> io::Result<Sources> {
        let controller = Controller::from_json(&read(&self.controller, 2 * 1024 * 1024)?)
            .map_err(io::Error::other)?;
        let setup = Setup::from_sources(
            objectives.clone(),
            spawns,
            &controller,
            &read(&self.mission_start, 2 * 1024 * 1024)?,
            &read(&self.sim_schedule, 64 * 1024)?,
            &read(&self.forward_spawns, 128 * 1024)?,
        )
        .map_err(io::Error::other)?;
        let map = controller
            .map(spawns.map_name())
            .ok_or_else(|| io::Error::other("missing_domination_controller_map"))?;
        let raw = String::from_utf8(read(&self.structural_effects, 2 * 1024 * 1024)?)
            .map_err(io::Error::other)?;
        let structural = Structural::from_json(
            &raw,
            &objectives,
            spawns.map_name(),
            &map.level_archive_sha256,
            &map.level_clv_sha256,
        )
        .map_err(io::Error::other)?;
        let catalog = StructuralEffects::from_json(&raw).map_err(io::Error::other)?;
        let sequence_assets = SequenceCollisionAssets::load(&self.sequence_assets, &catalog)
            .map_err(io::Error::other)?;
        let initial: Assignments =
            serde_json::from_slice(&read(&self.initial_assignments, 256 * 1024)?)
                .map_err(io::Error::other)?;
        if initial.schema != "mag_domination_initial_squad_assignment_v1"
            || initial.retail_elf_sha256 != objectives.source().source_image_sha256
            || initial.native_manual_tag != u32::MAX
        {
            return Err(io::Error::other("domination_assignment_source_mismatch"));
        }
        let mut rows = initial.maps.into_iter().filter(|row| row.map == map.name);
        let original = rows
            .next()
            .ok_or_else(|| io::Error::other("missing_domination_assignment_map"))?;
        if rows.next().is_some()
            || original.archive_sha256 != map.level_archive_sha256
            || original.clv_sha256 != map.level_clv_sha256
            || original.platoons.len() != 8
        {
            return Err(io::Error::other(
                "domination_assignment_map_source_mismatch",
            ));
        }
        let mut platoons = BTreeMap::new();
        let mut slots = BTreeSet::new();
        for row in original.platoons {
            if row.source_token == 0
                || row.team > 1
                || row.platoon > 3
                || row.slot != row.team * 4 + row.platoon
                || !slots.insert(row.slot)
                || platoons.insert(row.source_token, row.slot).is_some()
            {
                return Err(io::Error::other(
                    "invalid_domination_ability_platoon_identity",
                ));
            }
        }
        Ok(Sources {
            runtime: Runtime::new(Arc::new(setup), callback_capacity),
            structural,
            sequence_assets,
            platoons,
        })
    }
}

fn read(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("domination_gameplay_source_too_large"));
    }
    Ok(bytes)
}

impl Configuration {
    pub(crate) fn hospital_turret_source(&self) -> io::Result<Option<Vec<u8>>> {
        self.core
            .as_ref()
            .and_then(|core| core.hospital_turrets.as_ref())
            .map(|path| read(path, 256 * 1024))
            .transpose()
    }

    pub(crate) fn hospital_rocket_source(&self) -> io::Result<Option<Vec<u8>>> {
        self.core
            .as_ref()
            .and_then(|core| core.hospital_rockets.as_ref())
            .map(|path| read(path, 1024 * 1024))
            .transpose()
    }

    pub(crate) fn ground_turret_source(&self) -> io::Result<Option<Vec<u8>>> {
        self.core
            .as_ref()
            .and_then(|core| core.ground_turrets.as_ref())
            .map(|path| read(path, 256 * 1024))
            .transpose()
    }

    pub(crate) fn build_for(
        &self,
        objectives: Arc<ObjectiveCatalog>,
        spawns: &mut SpawnCatalog,
        bunkers: &BunkerWorld,
        grounds: &ground_vehicles::Catalog,
    ) -> io::Result<World> {
        let core = self
            .core
            .as_ref()
            .ok_or_else(|| io::Error::other("domination_gameplay_requires_explicit_core_scope"))?;
        core.validate_scope()?;
        bunkers
            .validate_domination_profile(spawns)
            .map_err(io::Error::other)?;
        let sources =
            self.load_sources(objectives.clone(), spawns, NonZeroUsize::new(128).unwrap())?;
        sources
            .structural
            .validate_ground_area_seats(grounds)
            .map_err(io::Error::other)?;
        let ground_raw = read(&core.ground_damage, 256 * 1024)?;
        if format!("{:x}", Sha256::digest(&ground_raw)) != core.ground_damage_sha256 {
            return Err(io::Error::other("domination_ground_damage_source_mismatch"));
        }
        let ground_damage: acquisition_ground_damage::Policy =
            serde_json::from_slice(&ground_raw).map_err(io::Error::other)?;
        ground_damage.validate().map_err(io::Error::other)?;
        let lifecycle_raw = read(&core.ground_lifecycle, 256 * 1024)?;
        if format!("{:x}", Sha256::digest(&lifecycle_raw)) != core.ground_lifecycle_sha256 {
            return Err(io::Error::other(
                "domination_ground_lifecycle_source_mismatch",
            ));
        }
        let lifecycle: GroundLifecycle =
            serde_json::from_slice(&lifecycle_raw).map_err(io::Error::other)?;
        let lifecycle_profile = lifecycle.profiles.iter().find(|profile| {
            spawns.source_sha256() == Some(profile.spawn_sha256.as_str())
                && grounds.raw_sha256() == profile.ground_sha256
        });
        if lifecycle.schema != "mag_domination_ground_lifecycle_v1"
            || lifecycle.native_elf_sha256 != objectives.source().source_image_sha256
            || lifecycle.team_interval_ms != spawns.match_profile().spawn_wave_ms
            || !lifecycle.material_penetration.is_finite()
            || lifecycle.material_penetration < 0.0
            || lifecycle.vehicles.len() != 6
            || lifecycle.profiles.len() != 6
        {
            return Err(io::Error::other(
                "invalid_domination_ground_lifecycle_source",
            ));
        }
        let lifecycle_profile = lifecycle_profile
            .ok_or_else(|| io::Error::other("domination_ground_lifecycle_profile_required"))?;
        let mut capabilities = Vec::new();
        for vehicle in grounds.creations() {
            ground_damage
                .validate_vehicle(vehicle)
                .map_err(io::Error::other)?;
            let source_type = grounds.source_spawn_type(vehicle);
            if !matches!(source_type, Some(3 | 7))
                || source_type != lifecycle_profile.spawn_types.get(&vehicle.index).copied()
            {
                return Err(io::Error::other("unsupported_domination_ground_spawn_type"));
            }
            let mut matching = lifecycle.vehicles.iter().filter(|source| {
                source.vehicle_type_token == vehicle.vehicle_type_token
                    && source.faction == vehicle.faction
            });
            let source = matching.next().filter(|source| {
                source.maximum_health_quarter_points == vehicle.health_quarter_points
                    && source.capabilities.destruction.is_some()
                    && source.capabilities.hulk_template.is_some()
                    && source.capabilities.delivered_inert_seconds.is_none()
            });
            if matching.next().is_some() || source.is_none() {
                return Err(io::Error::other(
                    "unbound_domination_ground_lifecycle_vehicle",
                ));
            }
            capabilities.push((vehicle.index, source.unwrap().capabilities.clone()));
        }
        let mut ground = crate::acquisition_ground_runtime::Runtime::from_catalog(grounds)
            .map_err(io::Error::other)?;
        crate::acquisition_gameplay_config::configure_ground(&mut ground, &capabilities)
            .map_err(io::Error::other)?;
        let original = objectives.source();
        let native_elf = &original.source_image_sha256;
        let terrain_materials = TerrainMaterialGrid::load(
            &core.terrain_material_index,
            &core.terrain_material_pixels,
            sources.structural.catalog(),
            native_elf,
        )
        .map_err(io::Error::other)?;
        let spawn_proof: SpawnBindings =
            serde_json::from_str(spawns.authored_source()).map_err(io::Error::other)?;
        let aircraft_links_raw = read(&core.aircraft_links, 128 * 1024)?;
        let planes: AircraftLinks =
            serde_json::from_slice(&aircraft_links_raw).map_err(io::Error::other)?;
        if planes.schema != "mag_private_retail_domination_aircraft_links_v1"
            || planes.retail_elf_sha256 != *native_elf
            || planes.initial_spawn_bindings_sha256 != spawn_proof.native_spawn_bindings_sha256
        {
            return Err(io::Error::other("domination_aircraft_source_mismatch"));
        }
        let mut maps = planes
            .maps
            .into_iter()
            .filter(|map| map.map == spawns.map_name());
        let map = maps
            .next()
            .ok_or_else(|| io::Error::other("missing_domination_aircraft_source_map"))?;
        if maps.next().is_some() || map.helo_points.len() != 4 || map.lalo_points.len() != 8 {
            return Err(io::Error::other(
                "invalid_domination_aircraft_source_coverage",
            ));
        }
        let verify = |row: AircraftPoint, spawn_type: u8| -> io::Result<u16> {
            let slot = row.packed_family35_id as u16;
            if row.spawn_type != spawn_type
                || row.packed_family35_id & 0xffff_0000 != 0x2380_0000
                || spawns
                    .point(slot)
                    .is_none_or(|point| point.token != row.point_token)
            {
                return Err(io::Error::other("domination_aircraft_spawn_mismatch"));
            }
            if spawn_type == 5
                && !matches!(
                    row.server_link.as_deref(),
                    Some("class3_factory_returns_null" | "absent_registry_returns_null")
                )
            {
                return Err(io::Error::other(
                    "domination_lalo_missing_null_server_link_proof",
                ));
            }
            Ok(slot)
        };
        let helos = map
            .helo_points
            .into_iter()
            .map(|row| verify(row, 4))
            .collect::<io::Result<BTreeSet<_>>>()?;
        let lalos = map
            .lalo_points
            .into_iter()
            .map(|row| verify(row, 5))
            .collect::<io::Result<BTreeSet<_>>>()?;
        if helos.len() != 4 || lalos.len() != 8 || !helos.is_disjoint(&lalos) {
            return Err(io::Error::other("duplicate_domination_aircraft_source"));
        }

        let poses: Poses = json_file(&core.qualified_poses, 512 * 1024)?;
        let controller = Controller::from_json(&read(&self.controller, 2 * 1024 * 1024)?)
            .map_err(io::Error::other)?;
        let map = controller
            .map(spawns.map_name())
            .ok_or_else(|| io::Error::other("missing_domination_controller_map"))?;
        if poses.schema != "mag_domination_qualified_live_poses_v1"
            || poses.map != spawns.map_name()
            || poses.retail_elf_sha256 != *native_elf
            || poses.metadata_sha256 != original.authored_catalog_sha256
            || poses.client_source_bindings_sha256 != original.binding_capture_sha256
            || poses.level_archive_sha256 != map.level_archive_sha256
            || poses.level_clv_sha256 != map.level_clv_sha256
            || poses.objective_poses.len() != objectives.object_count()
        {
            return Err(io::Error::other("domination_live_pose_source_mismatch"));
        }
        let mut root_poses = BTreeMap::new();
        for row in poses.objective_poses {
            row.pose.validate().map_err(io::Error::other)?;
            if objectives.binding(row.token).map(|(index, _)| index) != Some(row.index)
                || root_poses.insert(row.index, row.pose).is_some()
            {
                return Err(io::Error::other("invalid_domination_live_objective_pose"));
            }
        }
        let mut trigger_positions = BTreeMap::new();
        for row in poses.trigger_positions {
            if !row.position.iter().all(|value| value.is_finite())
                || trigger_positions
                    .insert((row.objective_index, row.trigger_index), row.position)
                    .is_some()
            {
                return Err(io::Error::other("invalid_domination_live_trigger_pose"));
            }
        }
        let expected_triggers = objectives
            .objects
            .iter()
            .flat_map(|object| {
                let count = object
                    .demolition
                    .as_ref()
                    .map(|rule| rule.trigger_count)
                    .or_else(|| object.interaction.as_ref().map(|rule| rule.trigger_count))
                    .unwrap_or(0);
                (0..count).map(move |trigger| (object.object_index, trigger))
            })
            .collect::<BTreeSet<_>>();
        if !trigger_positions.keys().copied().eq(expected_triggers) {
            return Err(io::Error::other(
                "domination_live_trigger_pose_coverage_mismatch",
            ));
        }

        let filter: FilterProof = json_file(&core.initial_filters, 1024 * 1024)?;
        if filter.schema != "mag_acquisition_volume_native_boundary_v1"
            || filter.retail_sha256 != *native_elf
        {
            return Err(io::Error::other(
                "domination_initial_filters_source_mismatch",
            ));
        }
        let mut filter_rows = [1_u32; 32];
        filter_rows[0] = u32::MAX;
        for [left, right] in filter.initial_filter_enable_pairs {
            if left >= 32 || right >= 32 {
                return Err(io::Error::other("invalid_domination_initial_filter_layer"));
            }
            filter_rows[usize::from(left)] |= 1 << right;
            filter_rows[usize::from(right)] |= 1 << left;
        }
        if !(0..32)
            .filter(|slot| filter_rows[6] & (1 << slot) != 0)
            .eq(filter.initial_filter6_allowed_layers)
        {
            return Err(io::Error::other(
                "domination_initial_filter_layer6_mismatch",
            ));
        }

        let hospital_raw = read(&core.hospital_hulks, 4 * 1024 * 1024)?;
        let hospital_assets = HospitalHulkAssets::from_json(
            &hospital_raw,
            spawns.map_name(),
            native_elf,
            bunkers
                .domination_structure_sha256()
                .ok_or_else(|| io::Error::other("domination_hospital_structure_source_missing"))?,
        )
        .map_err(io::Error::other)?;
        let hospital_doc: serde_json::Value =
            serde_json::from_slice(&hospital_raw).map_err(io::Error::other)?;
        let mut yaws = BTreeMap::new();
        let mut initial_collisions = SequenceCollisionState::default();
        for index in 0..16 {
            let bunker = bunkers
                .domination_bunker(index)
                .ok_or_else(|| io::Error::other("domination_hospital_index_missing"))?;
            let yaw =
                hospital_doc["factions"][bunker.faction.to_string()]["initial_turret_yaw_rad"]
                    .as_f64()
                    .filter(|&yaw| yaw == 0.0)
                    .ok_or_else(|| {
                        io::Error::other("domination_hospital_turret_yaw_source_missing")
                    })?;
            yaws.insert(index, yaw as f32);
            if !bunker.operational {
                return Err(io::Error::other(
                    "domination_hospital_initial_body_unavailable",
                ));
            }
            initial_collisions
                .install_operational_hospital(bunker, &hospital_assets)
                .map_err(io::Error::other)?;
        }
        let mut assignments = [None; 32];
        for (squad, assignment) in assignments.iter_mut().enumerate() {
            *assignment = Some(
                OwnedAssignment::initial(&objectives, squad as u16).map_err(io::Error::other)?,
            );
        }
        let (helo, helo_collision, helo_damage, disabled_helos, hospital_rocket_helo_areas) =
            if let Some(input) = &core.helo {
                let source = crate::domination_helo::FleetSource::load(
                    &read(&input.fleet, 256 * 1024)?,
                    spawns,
                )
                .map_err(io::Error::other)?;
                let models = read(&input.models, 4 * 1024 * 1024)?;
                let paths = read(&input.paths, 256 * 1024)?;
                for (raw, expected) in [
                    (&aircraft_links_raw, &source.source_receipt_sha256[0]),
                    (&paths, &source.source_receipt_sha256[1]),
                    (&models, &source.source_receipt_sha256[2]),
                ] {
                    if format!("{:x}", Sha256::digest(raw)) != *expected {
                        return Err(io::Error::other("domination_helo_source_pairing_mismatch"));
                    }
                }
                if source
                    .carriers
                    .iter()
                    .map(|carrier| carrier.spawn_index)
                    .collect::<BTreeSet<_>>()
                    != helos
                {
                    return Err(io::Error::other("domination_helo_spawn_coverage_mismatch"));
                }
                let damage_raw = read(&input.damage, 256 * 1024)?;
                let damage = crate::domination_helo_damage::HeloDamageRules::from_json(
                    &damage_raw,
                    native_elf,
                    &source,
                )
                .map_err(io::Error::other)?;
                let collision = crate::domination_structural_world::HeloCollisionAssets::from_json(
                    &models,
                    spawns.map_name(),
                    native_elf,
                    &source.source_receipt_sha256[2],
                )
                .map_err(io::Error::other)?;
                let rocket_areas = input
                    .hospital_rocket_damage_areas
                    .as_ref()
                    .map(|path| {
                        let areas = crate::hospital_rocket_helo::Catalog::from_bytes(&read(
                            path,
                            16 * 1024,
                        )?)
                        .map_err(io::Error::other)?;
                        areas
                            .validate(
                                native_elf,
                                hospital_doc["main_archive_sha256"]
                                    .as_str()
                                    .ok_or_else(|| {
                                        io::Error::other("hospital_rocket_helo_main_source_missing")
                                    })?,
                                &damage_raw,
                                &collision,
                            )
                            .map_err(io::Error::other)?;
                        Ok::<_, io::Error>(Arc::new(areas))
                    })
                    .transpose()?;
                let fleet = crate::domination_helo::FleetRuntime::from_source(
                    source.clone(),
                    input.random_seed,
                )
                .map_err(io::Error::other)?;
                // Promote the four source points only after every paired source validates.
                spawns
                    .qualify_domination_helo_points(&source)
                    .map_err(io::Error::other)?;
                (
                    Some(fleet),
                    Some(Arc::new(collision)),
                    Some(Arc::new(damage)),
                    BTreeSet::new(),
                    rocket_areas,
                )
            } else {
                spawns
                    .configure_domination_helo_scope(
                        helos.clone(),
                        sources
                            .runtime
                            .category_zero_indices()
                            .map_err(io::Error::other)?,
                    )
                    .map_err(io::Error::other)?;
                (None, None, None, helos, None)
            };
        let projectile_contacts = core
            .projectile_contacts
            .as_ref()
            .map(|path| {
                let bindings = crate::domination_projectile_collision::ContactBindings::from_json(
                    &read(path, 512 * 1024)?,
                )
                .map_err(io::Error::other)?;
                bindings
                    .validate(
                        spawns.map_name(),
                        native_elf,
                        &sources.sequence_assets,
                        &hospital_assets,
                        helo_collision.as_deref(),
                    )
                    .map_err(io::Error::other)?;
                Ok::<_, io::Error>(Arc::new(bindings))
            })
            .transpose()?;
        let hospital_rocket_ground = core
            .hospital_rocket_ground
            .as_ref()
            .map(|path| {
                let rules =
                    crate::hospital_rocket_ground::Catalog::from_bytes(&read(path, 16 * 1024)?)
                        .map_err(io::Error::other)?;
                rules
                    .validate(
                        native_elf,
                        hospital_doc["main_archive_sha256"]
                            .as_str()
                            .ok_or_else(|| {
                                io::Error::other("hospital_rocket_ground_main_source_missing")
                            })?,
                        &core.ground_damage_sha256,
                        grounds,
                    )
                    .map_err(io::Error::other)?;
                Ok::<_, io::Error>(Arc::new(rules))
            })
            .transpose()?;
        Ok(World {
            runtime: sources.runtime,
            state: State {
                objectives,
                structural: sources.structural,
                assets: Arc::new(sources.sequence_assets),
                hospital_hulks: Some(Arc::new(hospital_assets)),
                hospital_turret_yaws: Arc::new(yaws),
                hospital_rocket_ground,
                hospital_rocket_helo_areas,
                frag_character_blast: core.frag_character_blast,
                projectile_contacts,
                helo,
                helo_collision,
                helo_damage,
                root_poses: Arc::new(root_poses),
                trigger_positions: Arc::new(trigger_positions),
                filter_rows,
                terrain_materials: Arc::new(terrain_materials),
                ground_respawn_physics: Some(Default::default()),
                ground_damage: Arc::new(ground_damage),
                ground_capabilities: Arc::from(capabilities),
                ground_material_penetration: lifecycle.material_penetration,
                vehicle_waves: None,
                last_ground_update: None,
                known_empty_vehicle_spawns: Arc::new(lalos),
                disabled_helo_spawns: Arc::new(disabled_helos),
                platoons: Arc::new(sources.platoons),
                assignments,
                ability_enabled: [u16::MAX; 8],
                collisions: initial_collisions.clone(),
                initial_collisions: Arc::new(initial_collisions),
                area: Default::default(),
            },
        })
    }
}

impl<W: Write> Engine<W> {
    /// Explicit fixed-profile configuration for the source-bound core scope.
    /// Normal retail mode2 gameplay remains disabled without all listed files.
    pub fn with_domination_gameplay(mut self, config: &Configuration) -> io::Result<Self> {
        if self.runtime_application_id != Some(20231)
            || self.active.is_some()
            || !self.peers.is_empty()
            || self.match_epoch.is_some()
            || self.domination_gameplay.is_some()
            || self.domination_round.is_none()
            || self.ground_runtime.is_none()
            || self.collision_world.is_none()
            || self.dynamic_body_resources.is_none()
            || self.character_damage_rules.is_none()
            || !self
                .acquisition_static_partitions
                .contains_key(self.map_name())
        {
            return Err(io::Error::other(
                "domination_gameplay_requires_idle_source_resources",
            ));
        }
        let objectives =
            Arc::new(self.objective_catalog.clone().ok_or_else(|| {
                io::Error::other("domination_gameplay_requires_objective_catalog")
            })?);
        let spawns = self
            .spawn_catalog
            .as_mut()
            .ok_or_else(|| io::Error::other("domination_gameplay_requires_spawn_catalog"))?;
        let bunkers = self
            .bunker_world
            .as_ref()
            .ok_or_else(|| io::Error::other("domination_gameplay_requires_hospital_world"))?;
        let ground = self
            .ground_vehicles
            .as_ref()
            .ok_or_else(|| io::Error::other("domination_gameplay_requires_ground_catalog"))?;
        let native_elf = objectives.source().source_image_sha256.clone();
        let world = config.build_for(objectives, spawns, bunkers, ground)?;
        if world.state.frag_character_blast {
            if self.dynamic_body_resources.is_none() {
                return Err(io::Error::other(
                    "frag_character_blast_requires_dynamic_bodies",
                ));
            }
            validate_frag_character_inputs(
                &self.item_catalog,
                self.collision_world.as_ref(),
                spawns.map_name(),
            )
            .map_err(io::Error::other)?;
        }
        let hospital_source = config.hospital_turret_source()?;
        let hospital_rockets = config
            .hospital_rocket_source()?
            .map(|source| {
                crate::hospital_rockets::Catalog::from_bytes(
                    &source,
                    &self.item_catalog,
                    hospital_source
                        .as_deref()
                        .ok_or_else(|| io::Error::other("hospital_rockets_require_turrets"))?,
                    &native_elf,
                )
                .map(Arc::new)
                .map_err(io::Error::other)
            })
            .transpose()?;
        if let Some(contacts) = &world.state.projectile_contacts {
            contacts
                .validate_static(
                    self.collision_world
                        .as_ref()
                        .ok_or_else(|| io::Error::other("hospital_rockets_require_static_world"))?,
                )
                .map_err(io::Error::other)?;
            contacts
                .validate_dynamic(
                    &self
                        .dynamic_body_resources
                        .as_ref()
                        .ok_or_else(|| io::Error::other("hospital_rockets_require_dynamic_bodies"))?
                        .catalog,
                )
                .map_err(io::Error::other)?;
        }
        if let Some(rules) = &world.state.hospital_rocket_ground {
            rules
                .validate_dynamic(
                    self.dynamic_body_resources.as_ref().ok_or_else(|| {
                        io::Error::other("hospital_rocket_ground_requires_dynamic_bodies")
                    })?,
                    self.ground_vehicles.as_ref().unwrap(),
                )
                .map_err(io::Error::other)?;
        }
        if let Some(source) = hospital_source {
            let rules = crate::hospital_turrets::qualify(
                &source,
                bunkers,
                spawns,
                &self.item_catalog,
                &native_elf,
            )?;
            self.bunker_world.as_mut().unwrap().hospital_turrets = rules;
        }
        if let Some(source) = config.ground_turret_source()? {
            let grounds = self.ground_vehicles.as_mut().unwrap();
            let policies = crate::ground_turrets::qualify(
                &source,
                grounds,
                spawns,
                &self.item_catalog,
                &native_elf,
            )?;
            grounds.set_turret_policies(policies);
            self.ground_runtime
                .as_mut()
                .unwrap()
                .configure_turrets(grounds);
        }
        world
            .configure_ground(self.ground_runtime.as_mut().unwrap())
            .map_err(io::Error::other)?;
        self.projectile_contacts = world.state.projectile_contacts.clone();
        self.hospital_rockets = hospital_rockets;
        self.hospital_rocket_ground = world.state.hospital_rocket_ground.clone();
        self.frag_character_blast = world.state.frag_character_blast;
        self.domination_gameplay = Some(world);
        Ok(self)
    }
}

/// The source profile already owns the filter rows, material/terrain mapping
/// and Sequence resources. Require the remaining explicit Character inputs
/// before advertising or installing its opt-in.
pub(crate) fn validate_frag_character_inputs(
    items: &crate::item_catalog::ItemCatalog,
    collision: Option<&crate::collision::CollisionWorld>,
    map: &str,
) -> Result<(), &'static str> {
    collision
        .filter(|world| world.metadata().map_name == map && world.stats().triangles != 0)
        .ok_or("frag_character_blast_requires_static_world")?;
    if items.application_id() != 20231 {
        return Err("frag_character_blast_requires_retail_items");
    }
    items
        .retail_combat()?
        .validate_frag_character_blast()
        .map_err(|_| "frag_character_blast_metadata_required")
}

fn json_file<T: serde::de::DeserializeOwned>(path: &Path, limit: usize) -> io::Result<T> {
    serde_json::from_slice(&read(path, limit)?).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn core() -> serde_json::Value {
        json!({"scope":"core_without_helo_insertion", "aircraft_links":"links.json",
            "hospital_hulks":"hospitals.json", "initial_filters":"filters.json",
            "qualified_poses":"poses.json", "ground_damage":"ground.json",
            "ground_damage_sha256":"1".repeat(64), "ground_lifecycle":"lifecycle.json",
            "ground_lifecycle_sha256":"2".repeat(64), "terrain_material_index":"terrain.json",
            "terrain_material_pixels":"terrain.bin"})
    }

    fn helo() -> serde_json::Value {
        json!({"fleet":"fleet.json", "models":"models.json", "paths":"paths.json",
            "damage":"damage.json", "random_seed":19})
    }

    #[test]
    fn helo_scope_requires_paired_opt_in_without_ignored_inputs() {
        for (scope, fleet, valid) in [
            ("core_without_helo_insertion", None, true),
            ("core_without_helo_insertion", Some(helo()), false),
            ("core_with_helo_insertion", None, false),
            ("core_with_helo_insertion", Some(helo()), true),
            ("unknown", Some(helo()), false),
        ] {
            let mut value = core();
            value["scope"] = scope.into();
            value["helo"] = fleet.unwrap_or(serde_json::Value::Null);
            let parsed: Core = serde_json::from_value(value).unwrap();
            assert_eq!(parsed.validate_scope().is_ok(), valid, "{scope}");
        }
        let mut incomplete = core();
        incomplete["scope"] = "core_with_helo_insertion".into();
        incomplete["helo"] = helo();
        incomplete["helo"].as_object_mut().unwrap().remove("damage");
        assert!(serde_json::from_value::<Core>(incomplete).is_err());
    }

    #[test]
    fn helo_configuration_resolves_nested_paths_and_rejects_empty_sources() {
        let directory = std::env::temp_dir().join(format!(
            "mag-domination-helo-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let file = directory.join("config.json");
        let mut value = json!({"format":"mag_domination_gameplay_configuration_v1",
            "schema_version":1, "controller":"controller.json", "mission_start":"start.json",
            "sim_schedule":"schedule.json", "forward_spawns":"fsp.json",
            "structural_effects":"effects.json", "sequence_assets":"assets",
            "initial_assignments":"assignments.json", "core":core()});
        value["core"]["scope"] = "core_with_helo_insertion".into();
        value["core"]["helo"] = helo();
        std::fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
        let loaded = Configuration::load(&file).unwrap();
        let input = loaded.core.unwrap().helo.unwrap();
        assert_eq!(input.fleet, directory.join("fleet.json"));
        assert_eq!(input.models, directory.join("models.json"));
        assert_eq!(input.paths, directory.join("paths.json"));
        assert_eq!(input.damage, directory.join("damage.json"));
        assert_eq!(input.random_seed, 19);
        value["core"]["helo"]["models"] = "".into();
        std::fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            Configuration::load(&file).err().unwrap().to_string(),
            "empty_domination_helo_source_path"
        );
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn rocket_sources_require_both_the_turret_role_and_contact_provider() {
        for (rocket, contacts, turrets, valid) in [
            (false, false, false, true),
            (true, false, true, false),
            (true, true, false, false),
            (true, true, true, true),
            (false, true, false, true),
        ] {
            let mut value = core();
            if rocket {
                value["hospital_rockets"] = "rockets.json".into();
            }
            if contacts {
                value["projectile_contacts"] = "contacts.json".into();
            }
            if turrets {
                value["hospital_turrets"] = "turrets.json".into();
            }
            let parsed: Core = serde_json::from_value(value).unwrap();
            assert_eq!(parsed.validate_scope().is_ok(), valid);
        }
    }

    #[test]
    fn frag_character_scope_is_explicit_and_independent_of_rockets() {
        let base = core();
        let legacy: Core = serde_json::from_value(base.clone()).unwrap();
        assert!(!legacy.frag_character_blast);
        let mut enabled = base;
        enabled["frag_character_blast"] = true.into();
        let parsed: Core = serde_json::from_value(enabled).unwrap();
        assert!(parsed.frag_character_blast);
        assert!(parsed.hospital_rockets.is_none());
        assert!(parsed.projectile_contacts.is_none());
        assert!(parsed.validate_scope().is_ok());
    }

    #[test]
    fn helo_rocket_areas_require_the_equipped_rocket_provider() {
        for (areas, rocket, valid) in [
            (false, false, true),
            (false, true, true),
            (true, false, false),
            (true, true, true),
        ] {
            let mut value = core();
            value["scope"] = "core_with_helo_insertion".into();
            value["helo"] = helo();
            if areas {
                value["helo"]["hospital_rocket_damage_areas"] = "areas.json".into();
            }
            if rocket {
                value["hospital_rockets"] = "rockets.json".into();
                value["hospital_turrets"] = "turrets.json".into();
                value["projectile_contacts"] = "contacts.json".into();
            }
            let parsed: Core = serde_json::from_value(value).unwrap();
            assert_eq!(parsed.validate_scope().is_ok(), valid);
        }
    }

    #[test]
    fn ground_rocket_rules_require_the_equipped_rocket_provider() {
        for (ground, rocket, valid) in [
            (false, false, true),
            (true, false, false),
            (true, true, true),
        ] {
            let mut value = core();
            if ground {
                value["hospital_rocket_ground"] = "rocket-ground.json".into();
            }
            if rocket {
                value["hospital_rockets"] = "rockets.json".into();
                value["hospital_turrets"] = "turrets.json".into();
                value["projectile_contacts"] = "contacts.json".into();
            }
            let parsed: Core = serde_json::from_value(value).unwrap();
            assert_eq!(parsed.validate_scope().is_ok(), valid);
        }
    }
}
