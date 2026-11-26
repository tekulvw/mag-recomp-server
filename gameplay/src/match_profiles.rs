//! Explicit preloaded worlds for one idle, formation-selected retail worker.
//! Catalogs stay external; selecting never performs disk I/O.
use crate::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const CAPABILITY: &str = "match_profiles_v1";
/// Advertised when at least one profile fills its world with bots, so one
/// human can start that profile (see `Summary::bot_fill_total`).
pub const BOT_FILL_CAPABILITY: &str = "bot_fill_v1";
const MAX_MANIFEST: u64 = 1_048_576;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub profile_id: String,
    pub game_type: i8,
    pub attacking_faction: u8,
    pub defending_faction: u8,
    pub maximum_players: usize,
    pub team_capacity: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squad_capacity: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squad_counts: Option<[usize; 2]>,
    /// Total world population (humans plus bots) this profile fills to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_fill_total: Option<usize>,
    /// Humans the world admits, when it is larger than a legacy world; see
    /// `world_capacity`. Legacy rows omit it: every seat may be human.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_capacity: Option<usize>,
    /// Humans one side admits, present with `human_capacity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_team_capacity: Option<usize>,
}
impl Summary {
    /// Humans this profile's world admits.
    pub fn admitted_humans(&self) -> usize {
        self.human_capacity.unwrap_or(self.maximum_players)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SelectedMatch {
    pub formation_id: String,
    pub profile_id: String,
    pub game_type: i8,
    pub attacking_faction: u8,
    pub defending_faction: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission_epoch: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    profiles: Vec<Source>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    profile_id: String,
    spawn_catalog: PathBuf,
    objective_catalog: Option<PathBuf>,
    active_abilities: Option<PathBuf>,
    collision_world: Option<PathBuf>,
    collision_coverage: Option<PathBuf>,
    /// Retail collision filter rows; static rays then skip layers their
    /// query row passes through (see `collision::CollisionWorld::query_row`).
    #[serde(default)]
    collision_filter_rows: Option<PathBuf>,
    navmesh_world: Option<PathBuf>,
    movement_profile: Option<PathBuf>,
    /// Bot body profile (`bot_physics::BodyShape`); the box when absent.
    #[serde(default)]
    bot_body_profile: Option<PathBuf>,
    ground_vehicles: Option<PathBuf>,
    character_damage_rules: Option<PathBuf>,
    acquisition_gameplay: Option<PathBuf>,
    domination_round: Option<PathBuf>,
    domination_gameplay: Option<PathBuf>,
    bunker_placement: Option<PathBuf>,
    bunker_structure: Option<PathBuf>,
    bunker_allocation: Option<PathBuf>,
    bot_population: Option<PathBuf>,
    bot_fill_total: Option<usize>,
    bot_names: Option<PathBuf>,
    bot_resupply_catalog: Option<PathBuf>,
    bot_resupply_approach_policy: Option<PathBuf>,
}
/// One profile's bot inputs. A population is map-bound, so startup validates
/// it against this profile's own world before the worker advertises it.
#[derive(Clone)]
struct ProfileBots {
    population: bot_registry::BotPopulation,
    total: Option<usize>,
    names: Option<bot_names::Catalog>,
    resupply: Option<bot_resupply_catalog::Catalog>,
    approach: Option<bot_resupply_approach_policy::Policy>,
    validated: bool,
}
impl ProfileBots {
    fn load(source: &Source) -> io::Result<Option<Self>> {
        let Some(path) = &source.bot_population else {
            if source.bot_fill_total.is_some()
                || source.bot_names.is_some()
                || source.bot_resupply_catalog.is_some()
                || source.bot_resupply_approach_policy.is_some()
            {
                return Err(profile_error(
                    &source.profile_id,
                    &"match_profile_bot_settings_require_population",
                ));
            }
            return Ok(None);
        };
        let named = |code: &dyn std::fmt::Display| profile_error(&source.profile_id, code);
        if source.bot_resupply_approach_policy.is_some() && source.bot_resupply_catalog.is_none() {
            return Err(named(&"bot_resupply_approach_requires_provider_catalog"));
        }
        let population = bot_registry::BotPopulation::load(path).map_err(|e| named(&e))?;
        // Regional walking binds a coverage-qualified navmesh and live
        // Acquisition structure state that profile rows cannot supply.
        if population.regional_collision.is_some() || population.regional_team.is_some() {
            return Err(named(&"match_profile_bot_regional_collision_unsupported"));
        }
        Ok(Some(Self {
            population,
            total: source.bot_fill_total,
            names: source
                .bot_names
                .as_ref()
                .map(|path| bot_names::Catalog::load(path))
                .transpose()
                .map_err(|e| named(&e))?,
            resupply: source
                .bot_resupply_catalog
                .as_ref()
                .map(bot_resupply_catalog::Catalog::load)
                .transpose()
                .map_err(|e| named(&e))?,
            approach: source
                .bot_resupply_approach_policy
                .as_ref()
                .map(bot_resupply_approach_policy::Policy::load)
                .transpose()
                .map_err(|e| named(&e))?,
            validated: false,
        }))
    }
}
fn profile_error(profile_id: &str, code: &dyn std::fmt::Display) -> io::Error {
    io::Error::other(format!("match profile {profile_id}: {code}"))
}
/// Worker-wide inputs that bot validation shares with the served engine.
pub struct BotValidationInputs<'a> {
    pub items: &'a Arc<item_catalog::ItemCatalog>,
    pub wire_profiles: &'a wire_profile::WireProfileCatalog,
    pub names: &'a bot_names::Catalog,
    pub fall_damage_rules: Option<&'a fall_damage::Rules>,
}
/// A profile whose bot configuration does not fit its own world.
#[derive(Debug)]
pub struct BotValidationError {
    pub profile_id: String,
    pub code: &'static str,
}
impl std::fmt::Display for BotValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "match profile {}: {}", self.profile_id, self.code)
    }
}
impl std::error::Error for BotValidationError {}
#[derive(Clone)]
struct Loaded {
    summary: Summary,
    spawns: spawn_catalog::SpawnCatalog,
    objectives: Option<objective_catalog::ObjectiveCatalog>,
    active_abilities: Option<Arc<active_ability_engine::Setup>>,
    collision: Option<collision::CollisionWorld>,
    navmesh: Option<Arc<navmesh::NavMesh>>,
    vehicles: Option<ground_vehicles::Catalog>,
    damage: Option<character_damage_rules::Rules>,
    acquisition: Option<Arc<acquisition_gameplay_engine::Setup>>,
    domination_round: Option<acquisition_round::Round>,
    domination: Option<domination_gameplay_provider::World>,
    bunkers: Option<structures::BunkerWorld>,
    hospital_turret_source: Option<Arc<[u8]>>,
    hospital_rocket_source: Option<Arc<[u8]>>,
    hospital_rockets: Option<Arc<crate::hospital_rockets::Catalog>>,
    infantry_rocket_source: Option<Arc<[u8]>>,
    guided_launcher_source: Option<Arc<[u8]>>,
    infantry_rockets: Option<Arc<crate::infantry_rockets::Catalog>>,
    guided_launchers: Option<Arc<crate::guided_launchers::Catalog>>,
    gas_grenade_source: Option<Arc<[u8]>>,
    flash_grenade_source: Option<Arc<[u8]>>,
    smoke_grenades: Option<Arc<crate::smoke_grenades::Catalog>>,
    gas_grenades: Option<Arc<crate::gas_grenades::Catalog>>,
    flash_grenades: Option<Arc<crate::flash_grenades::Catalog>>,
    underbarrel_launcher_source: Option<Arc<[u8]>>,
    underbarrel_launchers: Option<Arc<crate::underbarrel_launchers::Catalog>>,
    ground_turret_source: Option<Arc<[u8]>>,
    acquisition_apc_turret_source: Option<Arc<[u8]>>,
    bots: Option<ProfileBots>,
}
pub struct Registry {
    profiles: BTreeMap<String, Loaded>,
    default_bot_fill_total: Option<usize>,
    /// The worker's `--smoke-grenade-rules`, bound to every profile.
    smoke_grenade_rules: Option<Arc<[u8]>>,
    /// Sabotage launcher and frag doses against each profile's static world.
    static_world: BTreeMap<String, static_world::Weapons>,
}
impl Registry {
    pub fn validate_dynamic_bodies(
        &self,
        catalog: &crate::dynamic_collision::Catalog,
        bindings: &crate::dynamic_collision_runtime::RuntimeBindings,
    ) -> Result<(), &'static str> {
        for profile in self.profiles.values() {
            bindings.validate_for(catalog, profile.vehicles.as_ref())?;
            if let Some(world) = &profile.domination
                && world.has_frag_ground_areas()
            {
                world.validate_frag_ground_dynamic(
                    catalog,
                    bindings,
                    profile
                        .vehicles
                        .as_ref()
                        .ok_or("domination_frag_ground_requires_vehicles")?,
                )?;
            }
            if let Some(rules) = profile
                .domination
                .as_ref()
                .and_then(|world| world.state.hospital_rocket_ground.as_ref())
            {
                rules.validate_dynamic_parts(
                    catalog,
                    bindings,
                    profile
                        .vehicles
                        .as_ref()
                        .ok_or("hospital_rocket_ground_requires_vehicles")?,
                )?;
            }
            if let Some(contacts) = profile
                .domination
                .as_ref()
                .and_then(|world| world.state.projectile_contacts.as_ref())
            {
                contacts.validate_dynamic(catalog)?;
            }
            if let Some(setup) = &profile.acquisition {
                if let Some(contacts) = &setup.projectile_contacts {
                    contacts.validate_dynamic(catalog)?;
                }
                setup.validate_ground_hulk_templates(catalog)?;
                if setup.has_frag_ground_areas() {
                    setup.validate_frag_ground_dynamic(
                        catalog,
                        bindings,
                        profile
                            .vehicles
                            .as_ref()
                            .ok_or("acquisition_frag_ground_requires_vehicles")?,
                    )?;
                }
            }
        }
        Ok(())
    }
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        absolute(path)?;
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other("match_profiles_manifest_directory_missing"))?;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_MANIFEST + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_MANIFEST {
            return Err(io::Error::other("match_profiles_manifest_too_large"));
        }
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if manifest.format != "mag_match_profiles_v1"
            || manifest.profiles.is_empty()
            || manifest.profiles.len() > 64
        {
            return Err(io::Error::other("invalid_match_profiles_manifest"));
        }
        let mut profiles = BTreeMap::new();
        let mut collision_cache: BTreeMap<PathBuf, collision::CollisionWorld> = BTreeMap::new();
        for mut source in manifest.profiles {
            if !identifier(&source.profile_id, 1) || profiles.contains_key(&source.profile_id) {
                return Err(io::Error::other("invalid_or_duplicate_match_profile_id"));
            }
            for path in [
                Some(&mut source.spawn_catalog),
                source.objective_catalog.as_mut(),
                source.active_abilities.as_mut(),
                source.collision_world.as_mut(),
                source.collision_coverage.as_mut(),
                source.navmesh_world.as_mut(),
                source.movement_profile.as_mut(),
                source.bot_body_profile.as_mut(),
                source.ground_vehicles.as_mut(),
                source.character_damage_rules.as_mut(),
                source.acquisition_gameplay.as_mut(),
                source.domination_round.as_mut(),
                source.domination_gameplay.as_mut(),
                source.bunker_placement.as_mut(),
                source.bunker_structure.as_mut(),
                source.bunker_allocation.as_mut(),
                source.bot_population.as_mut(),
                source.bot_names.as_mut(),
                source.bot_resupply_catalog.as_mut(),
                source.bot_resupply_approach_policy.as_mut(),
            ]
            .into_iter()
            .flatten()
            {
                crate::resources::resolve_manifest_path(directory, path)?;
            }
            let bots = ProfileBots::load(&source)?;
            let mut spawns = spawn_catalog::SpawnCatalog::load(&source.spawn_catalog)?;
            if source.acquisition_gameplay.is_some() && source.domination_gameplay.is_some() {
                return Err(io::Error::other("conflicting_match_gameplay_configuration"));
            }
            let policy = spawns.match_profile();
            if !matches!(policy.game_type, 0..=2)
                || !policy.auto_assign_teams
                || (policy.has_bunkers && !matches!(policy.game_type, 1 | 2))
                || (policy.game_type == 2 && spawns.map_pool().squads.len() != 32)
                || policy.attacking_faction == policy.defending_faction
                || spawns.map_pool().owner_faction != Some(policy.defending_faction)
            {
                return Err(io::Error::other("unsupported_match_profile_policy"));
            }
            if policy.game_type == 1 && policy.has_bunkers && source.acquisition_gameplay.is_none()
            {
                // Field hospitals follow their Structural objectives' forward spawns.
                return Err(io::Error::other("acquisition_hospitals_require_gameplay"));
            }
            let bunkers = match (
                policy.has_bunkers,
                source.bunker_placement.as_ref(),
                source.bunker_structure.as_ref(),
                source.bunker_allocation.as_ref(),
            ) {
                (true, Some(placement), Some(structure), Some(allocation)) => {
                    Some(structures::BunkerWorld::load_hospitals(
                        placement, structure, allocation, &spawns,
                    )?)
                }
                (false, None, None, None) => None,
                (true, ..) => return Err(io::Error::other("incomplete_match_profile_bunkers")),
                (false, ..) => return Err(io::Error::other("unexpected_match_profile_bunkers")),
            };
            // Every advertised seat must have an authored squad binding on both sides.
            let capacities = [0, 1].map(|team| {
                (spawns
                    .map_pool()
                    .squads
                    .iter()
                    .filter(|s| s.team == team)
                    .count()
                    * policy.squad_capacity)
                    .min(policy.team_capacity)
            });
            if capacities != [policy.team_capacity; 2]
                || policy.team_capacity * 2 > world_capacity::MAX_WORLD_CHARACTERS
            {
                return Err(io::Error::other("match_profile_capacity_mismatch"));
            }
            let objectives = source
                .objective_catalog
                .as_ref()
                .map(objective_catalog::ObjectiveCatalog::load)
                .transpose()?;
            spawns
                .validate_objectives(objectives.as_ref())
                .map_err(io::Error::other)?;
            if let Some(catalog) = &objectives {
                catalog
                    .validate_match(spawns.map_name(), policy)
                    .map_err(io::Error::other)?;
                for squad in &spawns.map_pool().squads {
                    catalog
                        .initial_assignment(squad.index)
                        .map_err(io::Error::other)?;
                }
            }
            if policy.has_objectives && objectives.is_none() {
                return Err(io::Error::other("retail_objective_catalog_required"));
            }
            let domination_round = source
                .domination_round
                .map(domination_match_flow::Setup::load)
                .transpose()?
                .map(|setup| {
                    setup
                        .round_for(&spawns, objectives.as_ref())
                        .map_err(io::Error::other)
                })
                .transpose()?;
            let collision = source
                .collision_world
                .as_ref()
                .map(|p| {
                    if let Some(world) = collision_cache.get(p) {
                        return Ok(world.clone());
                    }
                    let world =
                        collision::CollisionWorld::load(p, collision::CollisionLimits::default())
                            .map_err(io::Error::other)?;
                    collision_cache.insert(p.clone(), world.clone());
                    Ok::<_, io::Error>(world)
                })
                .transpose()?;
            if collision
                .as_ref()
                .is_some_and(|w| w.metadata().map_name != spawns.map_name())
            {
                return Err(io::Error::other("match_profile_collision_map_mismatch"));
            }
            // The sidecar binds to the world's exact hash; the cached world stays unchanged.
            let collision = match (source.collision_coverage.as_ref(), collision) {
                (Some(_), None) => {
                    return Err(io::Error::other(
                        "match_profile_collision_coverage_requires_collision",
                    ));
                }
                (Some(path), Some(world)) => Some(
                    world
                        .with_coverage_sidecar(crate::collision_coverage::CoverageSidecar::load(
                            path,
                        )?)
                        .map_err(io::Error::other)?,
                ),
                (None, world) => world,
            };
            let collision = match (source.collision_filter_rows.as_ref(), collision) {
                (Some(_), None) => {
                    return Err(io::Error::other(
                        "match_profile_collision_filter_rows_require_collision",
                    ));
                }
                (Some(path), Some(world)) => Some(
                    world
                        .with_filter_rows(Some(collision::CollisionWorld::load_filter_rows(path)?)),
                ),
                (None, world) => world,
            };
            if source.movement_profile.is_some() && source.navmesh_world.is_none() {
                return Err(io::Error::other(
                    "match_profile_movement_profile_requires_navmesh",
                ));
            }
            if source.bot_body_profile.is_some() && source.navmesh_world.is_none() {
                return Err(io::Error::other(
                    "match_profile_bot_body_profile_requires_navmesh",
                ));
            }
            let navmesh = source
                .navmesh_world
                .as_ref()
                .map(|path| {
                    let world = collision.as_ref().ok_or_else(|| {
                        io::Error::other("match_profile_navmesh_requires_collision")
                    })?;
                    let mut bytes = Vec::new();
                    std::fs::File::open(path)?
                        .take(navmesh::MAX_ARTIFACT_JSON_BYTES + 1)
                        .read_to_end(&mut bytes)?;
                    if bytes.len() as u64 > navmesh::MAX_ARTIFACT_JSON_BYTES {
                        return Err(io::Error::other("navmesh artifact size limit"));
                    }
                    let artifact = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                    let rules = source
                        .movement_profile
                        .as_ref()
                        .map(navmesh::MovementProfile::load)
                        .transpose()?;
                    let body = source
                        .bot_body_profile
                        .as_ref()
                        .map(navmesh::BodyShape::load_profile)
                        .transpose()?
                        .unwrap_or_default();
                    let mesh = navmesh::NavMesh::load_with_body(
                        artifact,
                        world.source_sha256(),
                        spawns.map_name(),
                        rules.as_ref(),
                        body,
                    )
                    .map_err(io::Error::other)?;
                    mesh.validate_coverage(world).map_err(io::Error::other)?;
                    Ok::<_, io::Error>(Arc::new(mesh))
                })
                .transpose()?;
            // Bots move against the same collision layers the mesh was baked from.
            let collision = match (collision, &navmesh) {
                (Some(world), Some(mesh)) => Some(
                    world.with_character_movement_layers(mesh.artifact().character_movement_layers),
                ),
                (world, _) => world,
            };
            let vehicles = source
                .ground_vehicles
                .as_ref()
                .map(|p| ground_vehicles::Catalog::load(p, &spawns))
                .transpose()?;
            let (
                acquisition,
                acquisition_hospital_turrets,
                acquisition_ground_turrets,
                acquisition_apc_turret_source,
                acquisition_rocket_source,
                acquisition_guided_source,
                acquisition_gas_source,
                acquisition_flash_source,
                acquisition_underbarrel_source,
            ) = if let Some(path) = &source.acquisition_gameplay {
                let world = collision.as_ref().ok_or_else(|| {
                    io::Error::other("acquisition_gameplay_requires_collision_world")
                })?;
                if world.stats().triangles == 0 {
                    return Err(io::Error::other(
                        "acquisition_gameplay_requires_nonempty_collision_world",
                    ));
                }
                let catalog = objectives
                    .as_ref()
                    .ok_or_else(|| io::Error::other("acquisition_gameplay_requires_objectives"))?;
                let ground = vehicles.as_ref().ok_or_else(|| {
                    io::Error::other("acquisition_gameplay_requires_ground_vehicles")
                })?;
                let runtime = acquisition_ground_runtime::Runtime::from_catalog(ground)
                    .map_err(io::Error::other)?;
                let config = acquisition_gameplay_config::Configuration::load(path)?;
                let setup = config.build(&spawns, catalog, &runtime)?;
                setup
                    .validate_projectile_contact_static(Some(world))
                    .map_err(io::Error::other)?;
                if let Some(hospitals) = &bunkers {
                    setup
                        .structural
                        .as_ref()
                        .ok_or_else(|| {
                            io::Error::other("acquisition_hospitals_require_structural_objectives")
                        })?
                        .validate_hospitals(hospitals)
                        .map_err(io::Error::other)?;
                }
                setup
                    .validate_ground_drivers(ground)
                    .map_err(io::Error::other)?;
                if let Some(policy) = &setup.ground_damage {
                    for creation in ground.creations() {
                        policy
                            .validate_vehicle(creation)
                            .map_err(io::Error::other)?;
                    }
                }
                let hospital_turrets = config.hospital_turret_source()?;
                if hospital_turrets.is_some() && bunkers.is_none() {
                    return Err(io::Error::other(
                        "acquisition_hospital_turrets_require_hospitals",
                    ));
                }
                (
                    Some(Arc::new(setup)),
                    hospital_turrets.map(Arc::from),
                    config.ground_turret_source()?.map(Arc::from),
                    config.apc_turret_source()?.map(Arc::from),
                    config.infantry_rocket_source()?.map(Arc::from),
                    config.guided_launcher_source()?.map(Arc::from),
                    config.gas_grenade_source()?.map(Arc::from),
                    config.flash_grenade_source()?.map(Arc::from),
                    config.underbarrel_launcher_source()?.map(Arc::from),
                )
            } else {
                if objectives.as_ref().is_some_and(|catalog| {
                    catalog.source().capture_qualification == "native_source_gameplay"
                        && !(policy.game_type == 2 && source.domination_gameplay.is_some())
                }) {
                    return Err(io::Error::other(if policy.game_type == 2 {
                        "domination_gameplay_configuration_required"
                    } else {
                        "acquisition_gameplay_configuration_required"
                    }));
                }
                (None, None, None, None, None, None, None, None, None)
            };
            let damage = source
                .character_damage_rules
                .as_ref()
                .map(character_damage_rules::Rules::load)
                .transpose()?;
            if acquisition.is_some() && damage.is_none() {
                return Err(io::Error::other(
                    "acquisition_gameplay_requires_character_damage_rules",
                ));
            }
            if let Some(rules) = &damage {
                rules
                    .validate_match(20231, &spawns)
                    .map_err(io::Error::other)?;
            }
            let (
                domination,
                hospital_turret_source,
                ground_turret_source,
                hospital_rocket_source,
                infantry_rocket_source,
                guided_launcher_source,
                gas_grenade_source,
                flash_grenade_source,
                underbarrel_launcher_source,
            ) = if let Some(path) = &source.domination_gameplay {
                if policy.game_type != 2
                    || domination_round.is_none()
                    || vehicles.is_none()
                    || damage.is_none()
                    || collision
                        .as_ref()
                        .is_none_or(|world| world.stats().triangles == 0)
                {
                    return Err(io::Error::other(
                        "domination_gameplay_requires_source_world_resources",
                    ));
                }
                let objectives =
                    Arc::new(objectives.clone().ok_or_else(|| {
                        io::Error::other("domination_gameplay_requires_objectives")
                    })?);
                let bunkers = bunkers
                    .as_ref()
                    .ok_or_else(|| io::Error::other("domination_gameplay_requires_hospitals"))?;
                let config = domination_gameplay_config::Configuration::load(path)?;
                let source = config.hospital_turret_source()?.map(Arc::from);
                let rocket_source = config.hospital_rocket_source()?.map(Arc::from);
                let ground_turret_source = config.ground_turret_source()?.map(Arc::from);
                let world = config.build_for(
                    objectives,
                    &mut spawns,
                    bunkers,
                    vehicles.as_ref().unwrap(),
                )?;
                if let Some(contacts) = &world.state.projectile_contacts {
                    contacts
                        .validate_static(collision.as_ref().ok_or_else(|| {
                            io::Error::other("hospital_rockets_require_static_world")
                        })?)
                        .map_err(io::Error::other)?;
                }
                (
                    Some(world),
                    source,
                    ground_turret_source,
                    rocket_source,
                    config.infantry_rocket_source()?.map(Arc::from),
                    config.guided_launcher_source()?.map(Arc::from),
                    config.gas_grenade_source()?.map(Arc::from),
                    config.flash_grenade_source()?.map(Arc::from),
                    config.underbarrel_launcher_source()?.map(Arc::from),
                )
            } else {
                (
                    None,
                    acquisition_hospital_turrets,
                    acquisition_ground_turrets,
                    None,
                    acquisition_rocket_source,
                    acquisition_guided_source,
                    acquisition_gas_source,
                    acquisition_flash_source,
                    acquisition_underbarrel_source,
                )
            };
            let ability_manifest = active_ability_config::manifest_path(
                source.active_abilities,
                source
                    .acquisition_gameplay
                    .as_deref()
                    .or(source.domination_gameplay.as_deref()),
                spawns.map_name(),
            )?;
            let active_abilities = ability_manifest
                .as_ref()
                .map(|path| {
                    if acquisition.is_none() && domination.is_none() {
                        return Err(io::Error::other(
                            "active_abilities_require_gameplay_provider",
                        ));
                    }
                    let world = collision.as_ref().ok_or_else(|| {
                        io::Error::other("active_abilities_require_collision_world")
                    })?;
                    let catalog = objectives.as_ref().ok_or_else(|| {
                        io::Error::other("active_abilities_require_objective_catalog")
                    })?;
                    let objective_path = source.objective_catalog.as_ref().ok_or_else(|| {
                        io::Error::other("active_abilities_require_objective_catalog")
                    })?;
                    active_ability_config::load_for_catalogs(
                        path,
                        &spawns,
                        world,
                        catalog,
                        objective_path,
                    )
                    .map(Arc::new)
                })
                .transpose()?;
            let summary = Summary {
                profile_id: source.profile_id.clone(),
                game_type: policy.game_type,
                attacking_faction: policy.attacking_faction,
                defending_faction: policy.defending_faction,
                maximum_players: policy.team_capacity * 2,
                team_capacity: policy.team_capacity,
                squad_capacity: None,
                squad_counts: None,
                bot_fill_total: None,
                human_capacity: world_capacity::is_large(policy.team_capacity)
                    .then(|| world_capacity::human_capacity(policy.team_capacity)),
                human_team_capacity: world_capacity::is_large(policy.team_capacity)
                    .then(|| world_capacity::human_team_capacity(policy.team_capacity)),
            };
            profiles.insert(
                source.profile_id,
                Loaded {
                    summary,
                    spawns,
                    objectives,
                    active_abilities,
                    collision,
                    navmesh,
                    vehicles,
                    damage,
                    acquisition,
                    domination_round,
                    domination,
                    bunkers,
                    hospital_turret_source,
                    hospital_rocket_source,
                    hospital_rockets: None,
                    infantry_rocket_source,
                    guided_launcher_source,
                    infantry_rockets: None,
                    guided_launchers: None,
                    gas_grenade_source,
                    flash_grenade_source,
                    smoke_grenades: None,
                    gas_grenades: None,
                    flash_grenades: None,
                    underbarrel_launcher_source,
                    underbarrel_launchers: None,
                    ground_turret_source,
                    acquisition_apc_turret_source,
                    bots,
                },
            );
        }
        Ok(Self {
            profiles,
            default_bot_fill_total: None,
            smoke_grenade_rules: None,
            static_world: BTreeMap::new(),
        })
    }

    /// Apply the worker's `--bot-fill-total` to every profile that has a bot
    /// population but no explicit `bot_fill_total` of its own.
    pub fn set_default_bot_fill_total(&mut self, total: usize) -> Result<(), &'static str> {
        if total == 0 {
            return Err("invalid_bot_fill_total");
        }
        if !self.profiles.values().any(|profile| profile.bots.is_some()) {
            return Err("bot_fill_total_requires_profile_bot_population");
        }
        self.default_bot_fill_total = Some(total);
        Ok(())
    }

    /// Whether any profile describes a world above the legacy size, which
    /// only a Horizon that understands `world_capacity_v1` may accept.
    pub fn has_large_world(&self) -> bool {
        self.profiles
            .values()
            .any(|profile| profile.summary.human_capacity.is_some())
    }

    pub fn has_bot_fill(&self) -> bool {
        self.profiles.values().any(|profile| profile.bots.is_some())
    }

    /// Validate every profile's bots against that profile's own spawn catalog,
    /// collision, navmesh and global items/names by running the ordinary bot
    /// population checks on a scratch engine that holds only that world. The
    /// settled population is kept for selection, which performs no I/O.
    pub fn validate_bots(
        &mut self,
        inputs: &BotValidationInputs<'_>,
    ) -> Result<(), BotValidationError> {
        let default_total = self.default_bot_fill_total;
        for (profile_id, loaded) in &mut self.profiles {
            let Some(mut bots) = loaded.bots.take() else {
                continue;
            };
            let result = validate_profile_bots(loaded, &mut bots, default_total, inputs);
            if result.is_ok() {
                loaded.summary.bot_fill_total = bots.total;
            }
            loaded.bots = Some(bots);
            result.map_err(|code| BotValidationError {
                profile_id: profile_id.clone(),
                code,
            })?;
        }
        Ok(())
    }
    /// Use the worker's `--smoke-grenade-rules` for every profile. Each
    /// profile binds them to its own objective catalog when bound.
    pub fn set_smoke_grenade_rules(&mut self, source: Arc<[u8]>) -> Result<(), &'static str> {
        if self.smoke_grenade_rules.is_some() {
            return Err("smoke_grenade_source_conflict");
        }
        self.smoke_grenade_rules = Some(source);
        Ok(())
    }

    pub fn bind_smoke_grenades(
        &mut self,
        items: &item_catalog::ItemCatalog,
    ) -> Result<(), &'static str> {
        let Some(source) = self.smoke_grenade_rules.as_ref() else {
            return Ok(());
        };
        for profile in self.profiles.values_mut() {
            if !matches!(
                (
                    profile.summary.game_type,
                    &profile.acquisition,
                    &profile.domination
                ),
                (0, None, None) | (1, Some(_), None) | (2, None, Some(_))
            ) {
                return Err("smoke_grenade_mode_required");
            }
            let objectives = profile
                .objectives
                .as_ref()
                .ok_or("smoke_grenade_objectives_required")?;
            profile.smoke_grenades = Some(Arc::new(crate::smoke_grenades::Catalog::from_bytes(
                source,
                items,
                &objectives.source().source_image_sha256,
            )?));
        }
        Ok(())
    }

    pub fn has_smoke_grenades(&self) -> bool {
        self.profiles
            .values()
            .any(|profile| profile.smoke_grenades.is_some())
    }

    pub fn bind_gas_grenades(
        &mut self,
        items: &item_catalog::ItemCatalog,
    ) -> Result<(), &'static str> {
        for profile in self.profiles.values_mut() {
            let Some(source) = profile.gas_grenade_source.as_ref() else {
                continue;
            };
            if !matches!(
                (
                    profile.summary.game_type,
                    &profile.acquisition,
                    &profile.domination
                ),
                (1, Some(_), None) | (2, None, Some(_))
            ) {
                return Err("gas_grenade_mode_required");
            }
            if profile.acquisition.as_ref().is_some_and(|setup| {
                setup.structural.is_none() || setup.filter_rows[6] & (1 << 21) == 0
            }) || profile
                .domination
                .as_ref()
                .is_some_and(|world| world.frag_blast_collision().filter_rows[6] & (1 << 21) == 0)
                || profile.damage.is_none()
            {
                return Err("gas_grenade_world_required");
            }
            let objectives = profile
                .objectives
                .as_ref()
                .ok_or("gas_grenade_objectives_required")?;
            profile.gas_grenades = Some(Arc::new(crate::gas_grenades::Catalog::from_bytes(
                source,
                items,
                &objectives.source().source_image_sha256,
            )?));
        }
        Ok(())
    }

    pub fn has_gas_grenades(&self) -> bool {
        self.profiles
            .values()
            .any(|profile| profile.gas_grenades.is_some())
    }

    pub fn bind_flash_grenades(
        &mut self,
        items: &item_catalog::ItemCatalog,
    ) -> Result<(), &'static str> {
        for profile in self.profiles.values_mut() {
            let Some(source) = profile.flash_grenade_source.as_ref() else {
                continue;
            };
            if !matches!(
                (
                    profile.summary.game_type,
                    &profile.acquisition,
                    &profile.domination
                ),
                (1, Some(_), None) | (2, None, Some(_))
            ) {
                return Err("flash_grenade_mode_required");
            }
            if profile.acquisition.as_ref().is_some_and(|setup| {
                setup.structural.is_none()
                    || !setup.has_projectile_contacts()
                    || setup.filter_rows[24] != 0x0040_0601
                    || setup.filter_rows[6] & (1 << 21) == 0
            }) || profile.domination.as_ref().is_some_and(|world| {
                world.state.projectile_contacts.is_none()
                    || world.frag_blast_collision().filter_rows[24] != 0x0040_0601
                    || world.frag_blast_collision().filter_rows[6] & (1 << 21) == 0
            }) || profile.damage.is_none()
            {
                return Err("flash_grenade_world_required");
            }
            let objectives = profile
                .objectives
                .as_ref()
                .ok_or("flash_grenade_objectives_required")?;
            profile.flash_grenades = Some(Arc::new(crate::flash_grenades::Catalog::from_bytes(
                source,
                items,
                &objectives.source().source_image_sha256,
            )?));
        }
        Ok(())
    }

    pub fn has_flash_grenades(&self) -> bool {
        self.profiles
            .values()
            .any(|profile| profile.flash_grenades.is_some())
    }

    pub fn bind_underbarrel_launchers(
        &mut self,
        items: &item_catalog::ItemCatalog,
    ) -> Result<(), &'static str> {
        for profile in self.profiles.values_mut() {
            let Some(source) = profile.underbarrel_launcher_source.as_ref() else {
                continue;
            };
            crate::underbarrel_launchers::map_support(
                match (
                    profile.summary.game_type,
                    &profile.acquisition,
                    &profile.domination,
                ) {
                    (1, Some(setup), None) => {
                        acquisition_gameplay_config::validate_infantry_rocket_support(setup)
                    }
                    (2, None, Some(world)) => {
                        domination_gameplay_config::validate_infantry_rocket_support(world)
                    }
                    _ => Err("infantry_rocket_mode_required"),
                },
            )?;
            if profile.damage.is_none() || profile.collision.is_none() || profile.vehicles.is_none()
            {
                return Err("underbarrel_launcher_world_resources_required");
            }
            let objectives = profile
                .objectives
                .as_ref()
                .ok_or("underbarrel_launcher_objectives_required")?;
            profile.underbarrel_launchers =
                Some(Arc::new(crate::underbarrel_launchers::Catalog::from_bytes(
                    source,
                    items,
                    &objectives.source().source_image_sha256,
                )?));
        }
        Ok(())
    }

    pub fn has_underbarrel_launchers(&self) -> bool {
        self.profiles
            .values()
            .any(|profile| profile.underbarrel_launchers.is_some())
    }

    /// Fit the optional launcher to each profile's current source ELF and receivers.
    pub fn bind_infantry_rockets(
        &mut self,
        items: &item_catalog::ItemCatalog,
    ) -> Result<(), &'static str> {
        for profile in self.profiles.values_mut() {
            let Some(source) = profile.infantry_rocket_source.as_ref() else {
                continue;
            };
            match (
                profile.summary.game_type,
                &profile.acquisition,
                &profile.domination,
            ) {
                (1, Some(setup), None) => {
                    acquisition_gameplay_config::validate_infantry_rocket_support(setup)?
                }
                (2, None, Some(world)) => {
                    domination_gameplay_config::validate_infantry_rocket_support(world)?
                }
                _ => return Err("infantry_rocket_mode_required"),
            }
            if profile.damage.is_none() || profile.collision.is_none() || profile.vehicles.is_none()
            {
                return Err("infantry_rocket_world_resources_required");
            }
            let objectives = profile
                .objectives
                .as_ref()
                .ok_or("infantry_rocket_objectives_required")?;
            let catalog = crate::infantry_rockets::Catalog::from_bytes(
                source,
                items,
                &objectives.source().source_image_sha256,
            )?;
            profile.infantry_rockets = Some(Arc::new(catalog));
        }
        Ok(())
    }

    pub fn has_infantry_rockets(&self) -> bool {
        self.profiles
            .values()
            .any(|profile| profile.infantry_rockets.is_some())
    }

    pub fn bind_guided_launchers(
        &mut self,
        items: &item_catalog::ItemCatalog,
    ) -> Result<(), &'static str> {
        for profile in self.profiles.values_mut() {
            let Some(source) = profile.guided_launcher_source.as_ref() else {
                continue;
            };
            match (
                profile.summary.game_type,
                &profile.acquisition,
                &profile.domination,
            ) {
                (1, Some(setup), None) => {
                    acquisition_gameplay_config::validate_infantry_rocket_support(setup)?
                }
                (2, None, Some(world)) => {
                    domination_gameplay_config::validate_infantry_rocket_support(world)?
                }
                _ => return Err("guided_launcher_mode_required"),
            }
            if profile.damage.is_none() || profile.collision.is_none() || profile.vehicles.is_none()
            {
                return Err("guided_launcher_world_resources_required");
            }
            let objectives = profile
                .objectives
                .as_ref()
                .ok_or("guided_launcher_objectives_required")?;
            let catalog = crate::guided_launchers::Catalog::from_bytes(
                source,
                items,
                &objectives.source().source_image_sha256,
            )?;
            profile.guided_launchers = Some(Arc::new(catalog));
        }
        Ok(())
    }

    pub fn has_guided_launchers(&self) -> bool {
        self.profiles
            .values()
            .any(|profile| profile.guided_launchers.is_some())
    }

    /// Bind source-qualified hospital weapons before advertising profile readiness.
    pub fn bind_hospital_turrets(
        &mut self,
        items: &item_catalog::ItemCatalog,
    ) -> Result<(), &'static str> {
        for profile in self.profiles.values_mut() {
            let Some(source) = profile.hospital_turret_source.take() else {
                continue;
            };
            let bunkers = profile
                .bunkers
                .as_mut()
                .ok_or("hospital_turret_world_required")?;
            let objectives = profile
                .objectives
                .as_ref()
                .ok_or("hospital_turret_objectives_required")?;
            if let Some(rocket_source) = profile.hospital_rocket_source.take() {
                if profile
                    .domination
                    .as_ref()
                    .and_then(|world| world.state.projectile_contacts.as_ref())
                    .is_none()
                {
                    return Err("hospital_rockets_require_contacts");
                }
                profile.hospital_rockets =
                    Some(Arc::new(crate::hospital_rockets::Catalog::from_bytes(
                        &rocket_source,
                        items,
                        &source,
                        &objectives.source().source_image_sha256,
                    )?));
            }
            bunkers.hospital_turrets = crate::hospital_turrets::qualify(
                &source,
                bunkers,
                &profile.spawns,
                items,
                &objectives.source().source_image_sha256,
            )
            .map_err(|_| "invalid_match_profile_hospital_turrets")?;
        }
        Ok(())
    }

    /// Bind optional indexed Ground gunner rules before creating a match.
    pub fn bind_ground_turrets(
        &mut self,
        items: &item_catalog::ItemCatalog,
    ) -> Result<(), &'static str> {
        for profile in self.profiles.values_mut() {
            let escort_or_domination = profile.ground_turret_source.as_deref();
            let acquisition_apc = profile.acquisition_apc_turret_source.as_deref();
            if escort_or_domination.is_none() && acquisition_apc.is_none() {
                if let Some(grounds) = &profile.vehicles {
                    grounds.validate_acquisition_apc_turrets()?;
                }
                continue;
            }
            let grounds = profile
                .vehicles
                .as_mut()
                .ok_or("ground_turret_catalog_required")?;
            let objectives = profile
                .objectives
                .as_ref()
                .ok_or("ground_turret_objectives_required")?;
            let native_elf = &objectives.source().source_image_sha256;
            if profile.acquisition.is_some() {
                crate::ground_turrets::bind_acquisition(
                    grounds,
                    &profile.spawns,
                    items,
                    native_elf,
                    escort_or_domination,
                    acquisition_apc,
                )
                .map_err(|_| "invalid_match_profile_ground_turrets")?;
            } else if let Some(source) = escort_or_domination {
                grounds.set_turret_policies(
                    crate::ground_turrets::qualify(
                        source,
                        grounds,
                        &profile.spawns,
                        items,
                        native_elf,
                    )
                    .map_err(|_| "invalid_match_profile_ground_turrets")?,
                );
            }
        }
        Ok(())
    }

    pub fn validate_runtime(
        &self,
        items: &item_catalog::ItemCatalog,
        wire: wire_profile::WireProfile,
    ) -> Result<(), &'static str> {
        if wire.application_id() != 20231 || items.application_id() != 20231 {
            return Err("match_profiles_require_explicit_retail_application");
        }
        if self.has_acquisition_gameplay() || self.has_domination_gameplay() {
            items.retail_combat()?;
        }
        for loaded in self.profiles.values() {
            if let Some(world) = &loaded.domination {
                world.validate_human_static_resupply(
                    &loaded.spawns,
                    loaded.collision.as_ref(),
                    items,
                )?;
                world.validate_frag_ground(
                    items,
                    loaded.collision.as_ref(),
                    loaded.spawns.map_name(),
                )?;
                world.validate_frag_helo(
                    items,
                    loaded.collision.as_ref(),
                    loaded.spawns.map_name(),
                )?;
                world.validate_hospital_blast(
                    items,
                    loaded.collision.as_ref(),
                    loaded.spawns.map_name(),
                )?;
            }
            if let Some(setup) = &loaded.acquisition {
                setup.validate_human_static_resupply(
                    &loaded.spawns,
                    loaded.collision.as_ref(),
                    loaded
                        .objectives
                        .as_ref()
                        .ok_or("acquisition_objectives_required")?,
                    items,
                )?;
                setup.validate_frag_character_blast(items, loaded.collision.as_ref())?;
                if setup.has_frag_ground_areas() {
                    setup.validate_frag_ground(
                        items,
                        loaded.collision.as_ref(),
                        loaded
                            .vehicles
                            .as_ref()
                            .ok_or("acquisition_frag_ground_requires_vehicles")?,
                        loaded
                            .objectives
                            .as_ref()
                            .ok_or("acquisition_frag_ground_requires_objectives")?,
                    )?;
                }
            }
            if loaded
                .domination
                .as_ref()
                .is_some_and(|world| world.state.frag_character_blast)
            {
                crate::domination_gameplay_config::validate_frag_character_inputs(
                    items,
                    loaded.collision.as_ref(),
                    loaded.spawns.map_name(),
                )?;
            }
            objective_catalog::validate_configuration(
                loaded.objectives.as_ref(),
                loaded.spawns.map_name(),
                loaded.spawns.match_profile(),
                wire,
            )?;
            items.validate_bootstrap_faction(loaded.summary.attacking_faction)?;
            items.validate_bootstrap_faction(loaded.summary.defending_faction)?;
        }
        Ok(())
    }
    /// Validate the map-specific static exclusion input for every enabled
    /// Acquisition world before a worker advertises the registry.
    pub fn validate_acquisition_static_partitions(
        &self,
        partitions: &[static_collision_partition::Partition],
    ) -> Result<(), &'static str> {
        for profile in self.profiles.values() {
            let Some(setup) = &profile.acquisition else {
                continue;
            };
            let partition = partitions
                .iter()
                .find(|p| p.map_name() == profile.spawns.map_name())
                .ok_or("match_profile_acquisition_static_partition_missing")?;
            partition.require_excluded(6, &setup.filter_rows)?;
        }
        Ok(())
    }

    pub fn has_platoon_abilities(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile
                .active_abilities
                .as_ref()
                .is_some_and(|setup| setup.has_platoon_effects())
        })
    }
    pub fn has_squad_only_abilities(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile
                .active_abilities
                .as_ref()
                .is_some_and(|setup| !setup.has_platoon_effects())
        })
    }

    pub fn has_legacy_company_abilities(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile.spawns.match_profile().game_type == 2
                && profile
                    .active_abilities
                    .as_ref()
                    .is_some_and(|setup| !setup.has_strategic_effects())
        })
    }

    pub fn has_active_abilities(&self) -> bool {
        self.profiles
            .values()
            .any(|profile| profile.active_abilities.is_some())
    }

    pub fn has_domination_gameplay(&self) -> bool {
        self.profiles
            .values()
            .any(|profile| profile.domination.is_some())
    }

    pub fn has_hospital_rockets(&self) -> bool {
        self.profiles
            .values()
            .any(|profile| profile.hospital_rockets.is_some())
    }

    pub fn has_human_static_resupply(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile
                .acquisition
                .as_ref()
                .is_some_and(|setup| setup.has_human_static_resupply())
                || profile
                    .domination
                    .as_ref()
                    .is_some_and(|world| world.state.human_static_resupply.is_some())
        })
    }

    /// Some Acquisition profile binds its field-hospital turrets (seat 0).
    pub fn has_acquisition_hospital_turrets(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile.acquisition.is_some()
                && profile
                    .bunkers
                    .as_ref()
                    .is_some_and(|world| !world.hospital_turrets.is_empty())
        })
    }

    pub fn has_acquisition_apc_crew(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile.vehicles.as_ref().is_some_and(|grounds| {
                grounds.has_acquisition_apc_crew()
                    && grounds.validate_acquisition_apc_turrets().is_ok()
            })
        })
    }

    pub fn has_frag_ground_areas(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile
                .acquisition
                .as_ref()
                .is_some_and(|setup| setup.has_frag_ground_areas())
                || profile
                    .domination
                    .as_ref()
                    .is_some_and(|world| world.has_frag_ground_areas())
        })
    }

    pub fn has_frag_helo_areas(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile
                .domination
                .as_ref()
                .is_some_and(|world| world.has_helo_frag_damage_areas())
        })
    }

    pub fn has_hospital_blast_areas(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile
                .domination
                .as_ref()
                .is_some_and(|world| world.has_hospital_blast_areas())
        })
    }

    pub fn has_frag_character_blast(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile
                .acquisition
                .as_ref()
                .is_some_and(|setup| setup.frag_character_blast)
                || profile
                    .domination
                    .as_ref()
                    .is_some_and(|world| world.state.frag_character_blast)
        })
    }

    pub fn has_hospital_rocket_helo(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile
                .domination
                .as_ref()
                .is_some_and(|world| world.state.hospital_rocket_helo_areas.is_some())
        })
    }

    pub fn has_hospital_rocket_ground(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile
                .domination
                .as_ref()
                .is_some_and(|world| world.state.hospital_rocket_ground.is_some())
        })
    }

    pub fn has_projectile_contacts(&self) -> bool {
        self.profiles.values().any(|profile| {
            profile
                .acquisition
                .as_ref()
                .is_some_and(|setup| setup.has_projectile_contacts())
                || profile
                    .domination
                    .as_ref()
                    .is_some_and(|world| world.state.projectile_contacts.is_some())
        })
    }

    /// Validate current immutable static partitions before advertising mode2.
    pub fn validate_domination_static_partitions<'a>(
        &self,
        partitions: impl IntoIterator<Item = &'a static_collision_partition::Partition>,
    ) -> Result<(), &'static str> {
        let partitions: Vec<_> = partitions.into_iter().collect();
        for profile in self.profiles.values() {
            let Some(world) = &profile.domination else {
                continue;
            };
            let partition = partitions
                .iter()
                .find(|p| p.map_name() == profile.spawns.map_name())
                .ok_or("match_profile_domination_static_partition_missing")?;
            partition.require_excluded(6, &world.state.filter_rows)?;
            partition.require_excluded(21, &world.state.filter_rows)?;
        }
        Ok(())
    }

    pub fn has_acquisition_gameplay(&self) -> bool {
        self.profiles
            .values()
            .any(|profile| profile.acquisition.is_some())
    }

    pub(crate) fn party_summaries(&self) -> Vec<Summary> {
        self.profiles
            .values()
            .map(|loaded| {
                let mut summary = loaded.summary.clone();
                let policy = loaded.spawns.match_profile();
                let counts = [0, 1].map(|team| {
                    loaded
                        .spawns
                        .map_pool()
                        .squads
                        .iter()
                        .filter(|squad| squad.team == team)
                        .count()
                });
                if policy.auto_assign_teams && counts.iter().all(|count| *count > 0) {
                    summary.squad_capacity = Some(policy.squad_capacity.min(policy.team_capacity));
                    summary.squad_counts = Some(counts);
                }
                summary
            })
            .collect()
    }

    pub fn summaries(&self) -> Vec<Summary> {
        self.profiles.values().map(|p| p.summary.clone()).collect()
    }
}
fn validate_profile_bots(
    loaded: &Loaded,
    bots: &mut ProfileBots,
    default_total: Option<usize>,
    inputs: &BotValidationInputs<'_>,
) -> Result<(), &'static str> {
    if bots.validated {
        return Ok(());
    }
    let total = bots
        .total
        .or(default_total)
        .ok_or("match_profile_bot_fill_total_required")?;
    // Bot resupply is qualified for Sabotage (0), Acquisition (1) and
    // Domination (2); the catalog itself pins each map to its one mode.
    if bots.resupply.is_some() && !(0..=2).contains(&loaded.summary.game_type) {
        return Err("bot_resupply_requires_qualified_mode");
    }
    let names = bots.names.clone().unwrap_or_else(|| inputs.names.clone());
    let resources = EngineResources::for_application(
        Arc::clone(inputs.items),
        loaded.spawns.clone(),
        loaded.bunkers.clone(),
        inputs.wire_profiles.clone(),
        20231,
    )?
    .with_bot_names(names.clone());
    let mut scratch = Engine::from_resources(io::sink(), u64::MAX, resources);
    scratch.collision_world = loaded.collision.clone();
    scratch.navmesh_world = loaded.navmesh.clone();
    scratch.fall_damage_rules = inputs.fall_damage_rules.cloned();
    let mut scratch = scratch.with_bot_population(bots.population.clone(), total)?;
    if let Some(catalog) = bots.resupply.clone() {
        scratch.configure_bot_resupply_catalog(catalog)?;
    }
    if let Some(policy) = bots.approach.clone() {
        scratch.configure_bot_resupply_approach_policy(policy)?;
    }
    bots.population = scratch
        .bot_registry
        .config
        .take()
        .ok_or("bot_population_unavailable")?;
    bots.total = Some(total);
    bots.names = Some(names);
    bots.validated = true;
    Ok(())
}
fn absolute(path: &Path) -> io::Result<()> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(io::Error::other("match_profiles_require_absolute_paths"))
    }
}
impl<W: Write> Engine<W> {
    pub fn with_match_profiles(mut self, mut registry: Registry) -> Result<Self, &'static str> {
        if self.runtime_application_id != Some(20231)
            || self.active.is_some()
            || !self.peers.is_empty()
            || self.match_profiles.is_some()
            || self.active_abilities.is_some()
            || self.acquisition_gameplay.is_some()
            || self.domination_round.is_some()
            || self.domination_gameplay.is_some()
            || self.smoke_grenades.is_some()
        {
            return Err("match_profiles_require_idle_explicit_retail_engine");
        }
        if self.bot_registry.config.is_some()
            || self.bunker_world.is_some()
            || self.bunker_exit_policy.is_some()
            || self
                .leadership
                .as_ref()
                .is_some_and(|leadership| !leadership.supports_match_profiles())
            || self.development_dummy
        {
            return Err("match_profiles_unsupported_world_extras");
        }
        let supports_platoon = self
            .leadership
            .as_ref()
            .is_some_and(|leadership| leadership.supports_platoon_election());
        if registry.has_platoon_abilities() && !supports_platoon {
            return Err("active_abilities_require_native_platoon_leadership");
        }
        if registry.has_squad_only_abilities() && supports_platoon {
            return Err("platoon_leadership_requires_complete_active_abilities");
        }
        if registry.has_active_abilities()
            && !self
                .leadership
                .as_ref()
                .is_some_and(leadership_runtime::LeadershipRuntime::supports_match_profiles)
        {
            return Err("active_abilities_require_native_leadership");
        }
        if registry.has_legacy_company_abilities()
            && self
                .leadership
                .as_ref()
                .is_some_and(|l| l.supports_oic_election())
        {
            return Err("oic_leadership_requires_strategic_effects");
        }
        registry.validate_runtime(&self.item_catalog, self.wire_profile)?;
        registry
            .validate_bots(&BotValidationInputs {
                items: &self.item_catalog,
                wire_profiles: &self.wire_profiles,
                names: &self.bot_names,
                fall_damage_rules: self.fall_damage_rules.as_ref(),
            })
            .map_err(|error| error.code)?;
        registry.bind_hospital_turrets(&self.item_catalog)?;
        registry.bind_ground_turrets(&self.item_catalog)?;
        registry.bind_infantry_rockets(&self.item_catalog)?;
        registry.bind_static_world_weapons(&self.item_catalog);
        registry.bind_guided_launchers(&self.item_catalog)?;
        registry.bind_smoke_grenades(&self.item_catalog)?;
        registry.bind_gas_grenades(&self.item_catalog)?;
        registry.bind_flash_grenades(&self.item_catalog)?;
        registry.bind_underbarrel_launchers(&self.item_catalog)?;
        if self.dynamic_body_resources.is_none() {
            if registry.has_guided_launchers() {
                return Err("guided_launchers_require_dynamic_bodies");
            }
            if registry.has_flash_grenades() {
                return Err("flash_grenades_require_dynamic_bodies");
            }
            if registry.has_gas_grenades() {
                return Err("gas_grenades_require_dynamic_bodies");
            }
            if registry.has_domination_gameplay() {
                return Err("domination_profiles_require_dynamic_bodies");
            }
            if registry.has_projectile_contacts() {
                return Err("acquisition_projectile_contacts_require_dynamic_bodies");
            }
        }
        registry.validate_domination_static_partitions(
            self.acquisition_static_partitions
                .values()
                .map(AsRef::as_ref),
        )?;
        if let Some(resources) = &self.dynamic_body_resources {
            resources.validate_profiles(&registry)?;
        }
        self.match_profiles = Some(registry);
        Ok(self)
    }
    pub(crate) fn retire_match_selection(&mut self) {
        if let Some(selected) = self.selected_match.take() {
            self.retired_formations.insert(selected.formation_id);
            self.active_abilities = None;
            self.hospital_rockets = None;
            self.infantry_rockets = None;
            self.guided_launchers = None;
            self.guided_warnings = Default::default();
            self.smoke_grenades = None;
            self.gas_grenades = None;
            self.flash_grenades = None;
            self.underbarrel_launchers = None;
            self.hospital_rocket_ground = None;
            self.frag_character_blast = false;
            self.static_world_frag_blast = false;
            self.projectile_contacts = None;
            if let Some(leadership) = &mut self.leadership {
                leadership.clear_held_roles();
            }
        }
    }
    pub(crate) fn select_match_profile(
        &mut self,
        formation_id: String,
        profile_id: String,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if self.failed {
            return Err("recorder_unavailable");
        }
        let registry = self
            .match_profiles
            .as_ref()
            .ok_or("match_profiles_disabled")?;
        if self.retired_formations.contains(&formation_id) {
            return Err("match_formation_retired");
        }
        if let Some(selected) = &self.selected_match {
            if selected.formation_id == formation_id && selected.profile_id == profile_id {
                return Ok(Vec::new());
            }
            if selected.formation_id == formation_id {
                return Err("match_formation_profile_conflict");
            }
        }
        let rollover = self.match_terminal.is_some();
        if rollover {
            self.check_cohort_rollover()?;
        } else if self.active.is_some() || !self.peers.is_empty() {
            return Err("match_profile_selection_locked");
        }
        if self.retired_formations.len() >= 4096 {
            return Err("match_formation_history_full");
        }
        let loaded = registry
            .profiles
            .get(&profile_id)
            .ok_or("unknown_match_profile")?
            .clone();
        let static_world = registry
            .static_world
            .get(&profile_id)
            .cloned()
            .unwrap_or_default();
        let character_weapons = registry.character_weapon_report(&profile_id);
        if let (Some(resources), Some(vehicles)) = (&self.dynamic_body_resources, &loaded.vehicles)
        {
            resources.validate_vehicles(vehicles)?;
        }
        if loaded
            .bots
            .as_ref()
            .is_some_and(|bots| !bots.validated || bots.total.is_none())
        {
            return Err("match_profile_bots_unvalidated");
        }
        if !self.bot_resupply_revisions_available() {
            return Err("bot_resupply_catalog_revision_exhausted");
        }
        // Allocate everything before the durable acknowledgement. A failed record
        // must leave the previously selected world, identity and outboxes intact.
        let selected = SelectedMatch {
            formation_id,
            profile_id,
            game_type: loaded.summary.game_type,
            attacking_faction: loaded.summary.attacking_faction,
            defending_faction: loaded.summary.defending_faction,
            admission_epoch: if self.party_reservations.enabled() {
                Some(
                    self.round_number
                        .checked_add(1)
                        .ok_or("round_number_exhausted")?,
                )
            } else {
                None
            },
        };
        let world = loaded
            .objectives
            .as_ref()
            .map(objective_control::ObjectiveWorld::new);
        let active_abilities = loaded
            .active_abilities
            .map(|setup| setup.runtime())
            .transpose()?;
        let acquisition_round = loaded
            .acquisition
            .as_ref()
            .map(|setup| acquisition_round::Round::new(setup.rules));
        let acquisition_gameplay = loaded
            .acquisition
            .map(acquisition_gameplay_engine::Runtime::from_setup);
        let mut ground_runtime = loaded
            .vehicles
            .as_ref()
            .map(acquisition_ground_runtime::Runtime::from_catalog)
            .transpose()?;
        if let Some(runtime) = &acquisition_gameplay {
            acquisition_gameplay_config::configure_ground(
                ground_runtime
                    .as_mut()
                    .ok_or("acquisition_gameplay_requires_ground_vehicles")?,
                &runtime.setup.ground_capabilities,
            )?;
        }
        if let Some(world) = &loaded.domination {
            world.configure_ground(
                ground_runtime
                    .as_mut()
                    .ok_or("domination_gameplay_requires_ground_vehicles")?,
            )?;
        }
        self.record(
            json!({"op":"match_profile_selected","selected_match":selected,
                "character_weapons":character_weapons}),
            wall,
        )?;
        if rollover {
            self.archive_terminal_cohort();
        }
        self.retire_match_selection();
        self.reset_objective_world();
        // Install this profile's own validated population (or none); bots never
        // carry another world's map-bound slots or providers into this one.
        let bot_fill = loaded.bots.is_some();
        let (bot_resupply, bot_approach) = match loaded.bots {
            Some(bots) => {
                self.bot_registry = bot_registry::BotRegistry::for_validated_population(
                    bots.population,
                    bots.total.unwrap_or_default(),
                );
                if let Some(names) = bots.names {
                    self.bot_names = names;
                }
                (bots.resupply, bots.approach)
            }
            None => {
                self.bot_registry = Default::default();
                (None, None)
            }
        };
        self.spawn_catalog = Some(loaded.spawns);
        self.bunker_world = loaded.bunkers;
        self.objective_catalog = loaded.objectives;
        self.objective_world = world;
        self.collision_world = loaded.collision;
        self.navmesh_world = loaded.navmesh;
        self.ground_vehicles = loaded.vehicles;
        self.ground_runtime = ground_runtime;
        self.acquisition_gameplay = acquisition_gameplay;
        self.active_abilities = active_abilities;
        self.acquisition_round = acquisition_round;
        self.domination_round = loaded.domination_round;
        self.projectile_contacts = match (&self.acquisition_gameplay, &loaded.domination) {
            (Some(runtime), None) => runtime.setup.projectile_contacts.clone(),
            (None, Some(world)) => world.state.projectile_contacts.clone(),
            _ => None,
        };
        self.hospital_rockets = loaded.hospital_rockets;
        // A profile has either represented launcher rows or, in Sabotage,
        // rows derived for its static world; never both.
        self.infantry_rockets = loaded.infantry_rockets.or(static_world.rockets());
        self.static_world_frag_blast = static_world.frags();
        self.guided_launchers = loaded.guided_launchers;
        self.guided_warnings = Default::default();
        self.smoke_grenades = loaded.smoke_grenades;
        self.gas_grenades = loaded.gas_grenades;
        self.flash_grenades = loaded.flash_grenades;
        self.underbarrel_launchers = loaded.underbarrel_launchers;
        self.hospital_rocket_ground = loaded
            .domination
            .as_ref()
            .and_then(|world| world.state.hospital_rocket_ground.clone());
        self.frag_character_blast = self
            .acquisition_gameplay
            .as_ref()
            .is_some_and(|runtime| runtime.setup.frag_character_blast)
            || loaded
                .domination
                .as_ref()
                .is_some_and(|world| world.state.frag_character_blast);
        self.domination_gameplay = loaded.domination;
        self.character_damage_rules = loaded.damage;
        self.development_dummy_position = self
            .spawn_catalog
            .as_ref()
            .and_then(|s| s.alternate_position());
        self.team_waves = [None; 2];
        self.parachute_waves.clear();
        self.next_repair.clear();
        self.pending_bunker_damage.clear();
        self.last_published_rosters.clear();
        self.shared_drain_cursor = 0;
        self.world_epoch = None;
        self.match_epoch = None;
        self.last_world_tick = None;
        self.automatic_repair = Default::default();
        self.automatic_heal = Default::default();
        self.alpha_turret = Default::default();
        self.alpha_turret_reporter = None;
        self.install_profile_bot_resupply(bot_resupply, bot_approach);
        // Bot-filled profiles default to a one-human start; others keep the
        // two-human, one-per-side formation default.
        if bot_fill {
            self.minimum_players = 1;
            self.minimum_players_per_team = [0; 2];
        } else {
            self.minimum_players = 2;
            self.minimum_players_per_team = [1; 2];
        }
        self.maximum_players = loaded.summary.admitted_humans();
        self.match_started = self.starts_without_quorum();
        self.party_reservations.clear();
        self.selected_match = Some(selected);
        Ok(Vec::new())
    }
}

#[path = "match_profile_static_world.rs"]
pub(crate) mod static_world;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Files(PathBuf);
    impl Files {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "mag-synthetic-profiles-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn write(&self, name: &str, data: Value) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
            path
        }
        fn spawns(&self, mode: i8, attacker: u8, defender: u8) -> PathBuf {
            self.write(&format!("{mode}-{attacker}-{defender}.json"), json!({
                "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC profile registry",
                "active":{"map_name":format!("synthetic_home_{defender}"),"squad_index":0,"default_spawn":0,"available":[0]},
                "match_profile":{"game_type":mode,"auto_assign_teams":true,"team_capacity":1,"squad_capacity":1,"attacking_faction":attacker,"has_bunkers":false,"has_objectives":false},
                "maps":[{"name":format!("synthetic_home_{defender}"),"owner_faction":defender,"spawn_count":1,
                    "locations":[{"index":0,"token":7,"kind":"on_foot","position":{"x":2.,"y":0.,"z":2.}}],
                    "squads":[{"index":0,"team":0,"platoon":0,"squad":0,"faction":null,"available":[0],"default_spawn":0},
                    {"index":16,"team":1,"platoon":0,"squad":0,"faction":defender,"available":[0],"default_spawn":0}]}]
            }))
        }
        fn registry(&self) -> Registry {
            let rows: Vec<_> = (0..2).flat_map(|mode| (0..3).flat_map(move |attacker| (0..3).filter(move |defender| *defender != attacker).map(move |defender| (mode, attacker, defender)))).map(|(mode, attacker, defender)| {
                json!({"profile_id":format!("p-{mode}-{attacker}-{defender}"),"spawn_catalog":self.spawns(mode, attacker, defender)})
            }).collect();
            Registry::load(self.write(
                "manifest.json",
                json!({"format":"mag_match_profiles_v1","profiles":rows}),
            ))
            .unwrap()
        }
    }
    impl Drop for Files {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn base_engine<W: Write>(files: &Files, writer: W, maximum_log: u64) -> Engine<W> {
        let resources = EngineResources::for_application(
            crate::resources::tests::retail_items(),
            spawn_catalog::SpawnCatalog::load(files.spawns(0, 0, 1)).unwrap(),
            None,
            crate::resources::tests::wire_profiles(),
            20231,
        )
        .unwrap();
        Engine::from_resources(writer, maximum_log, resources)
    }
    fn engine<W: Write>(files: &Files, writer: W, maximum_log: u64) -> Engine<W> {
        base_engine(files, writer, maximum_log)
            .with_match_profiles(files.registry())
            .unwrap()
    }

    fn party_wire() -> Value {
        let members = [0, 1].map(|index| {
            json!({
                "reservation_id": format!("synthetic-party-member-{index}"),
                "player_id": index, "account_id": index + 1,
                "session_key": "aaaaaaaaaaaaaaaa", "access_key": "bbbbbbbbbbbbbbbb",
                "character": {"name": format!("Synthetic Player {index}"), "faction": 0,
                    "head": 0, "voice": 0, "universe_id": 26}
            })
        });
        json!({
            "v": BRIDGE_VERSION, "op": "reserve_party", "request_id": "party-reserve",
            "formation_id": "party-formation", "profile_id": "party-profile",
            "cohort_id": "cohort-a", "world_id": 7, "admission_epoch": 1,
            "members": members
        })
    }

    #[test]
    fn party_bridge_accepts_bounded_group_request() {
        let candidate = serde_json::to_vec(&party_wire()).unwrap();
        parse_request(&candidate).expect("the worker understands the group reservation");
    }

    fn party_worker(files: &Files, enable: bool) -> Engine<Vec<u8>> {
        let path = files.spawns(0, 0, 1);
        let mut spawn: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        spawn["match_profile"]["team_capacity"] = json!(4);
        spawn["match_profile"]["squad_capacity"] = json!(2);
        let squads = spawn["maps"][0]["squads"].as_array_mut().unwrap();
        for index in [0, 1] {
            let mut second = squads[index].clone();
            second["index"] = json!(index * 16 + 1);
            second["squad"] = json!(1);
            squads.push(second);
        }
        let manifest = files.write("party-profiles.json", json!({
            "format":"mag_match_profiles_v1",
            "profiles":[{"profile_id":"party-profile","spawn_catalog":files.write("party-spawn.json",spawn)}]
        }));
        let mut engine = base_engine(files, Vec::new(), 100_000)
            .with_match_profiles(Registry::load(&manifest).unwrap())
            .unwrap();
        if enable {
            engine.configure_party_reservations().unwrap();
        }
        let selected = select(&mut engine, "party-formation", "party-profile");
        assert!(selected.ok, "{:?}", selected.error);
        assert_eq!(
            selected.selected_match.unwrap().admission_epoch,
            enable.then_some(1)
        );
        engine
    }

    fn party_order(engine: &mut Engine<Vec<u8>>, value: &Value) -> Response {
        let request = parse_request(&serde_json::to_vec(value).unwrap()).unwrap();
        engine.handle(request, Duration::from_millis(10), 1)
    }

    fn opposing_party_wire() -> Value {
        let mut reply = party_wire();
        reply["cohort_id"] = json!("cohort-b");
        for (offset, row) in reply["members"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            row["reservation_id"] = json!(format!("synthetic-opponent-{offset}"));
            row["player_id"] = json!(offset + 2);
            row["account_id"] = json!(offset + 3);
            row["character"]["faction"] = json!(1);
        }
        reply
    }

    fn party_release_wire(value: &Value) -> Value {
        let mut release = value.clone();
        release.as_object_mut().unwrap().remove("members");
        release["op"] = json!("release_party");
        release["request_id"] = json!("party-rollback");
        release
    }

    #[test]
    fn party_bridge_is_opt_in_and_reports_exact_selected_epoch() {
        let files = Files::new();
        for enable in [false, true] {
            let mut engine = party_worker(&files, enable);
            let hello =
                serde_json::to_value(engine.describe_worker("hello".into(), false)).unwrap();
            assert_eq!(
                hello["capabilities"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("party_reservation_v1")),
                enable
            );
            assert_eq!(
                hello["match_profiles"][0]["squad_capacity"],
                if enable { json!(2) } else { Value::Null }
            );
            assert_eq!(
                hello["match_profiles"][0]["squad_counts"],
                if enable { json!([2, 2]) } else { Value::Null }
            );
            let selected = select(&mut engine, "party-formation", "party-profile");
            assert!(selected.ok);
            assert_eq!(
                selected.selected_match.unwrap().admission_epoch,
                enable.then_some(1)
            );
            let response = party_order(&mut engine, &party_wire());
            assert_eq!(response.ok, enable);
            if !enable {
                assert_eq!(
                    response.error.as_deref(),
                    Some("party_reservations_disabled")
                );
                assert!(
                    !serde_json::to_value(response)
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .contains_key("party_reservation")
                );
            }
        }
    }

    #[test]
    fn party_bridge_reserves_two_opposing_parties_before_any_claim() {
        let files = Files::new();
        let mut engine = party_worker(&files, true);
        let mut attacking = party_wire();
        attacking["members"].as_array_mut().unwrap().reverse();
        let defending = opposing_party_wire();
        let start = engine.sequence;
        let first = party_order(&mut engine, &attacking);
        assert!(first.ok, "{:?}", first.error);
        let first = first.party_reservation.unwrap();
        assert_eq!(first.status, party_reservation::Status::Reserved);
        assert_eq!(
            first.reservation_ids,
            ["synthetic-party-member-1", "synthetic-party-member-0"]
        );
        assert_eq!(
            (first.team, first.squad, first.admission_epoch),
            (Some(0), Some(0), 1)
        );
        assert_eq!(engine.sequence, start + 1);
        assert_eq!(engine.round_number, 1);

        let second = party_order(&mut engine, &defending);
        assert!(second.ok, "{:?}", second.error);
        let second = second.party_reservation.unwrap();
        assert_eq!(
            (second.team, second.squad, second.admission_epoch),
            (Some(1), Some(16), 1)
        );
        assert_eq!(engine.sequence, start + 2);
        assert_eq!(engine.round_number, 1);
        assert_eq!(engine.peers.len(), 4);
        for (ids, squad) in [(&first.reservation_ids, 0), (&second.reservation_ids, 16)] {
            for id in ids {
                assert_eq!(engine.peers[id].squad_catalog.squad_index(), squad);
                assert!(engine.peers[id].claimed.is_none());
            }
        }
        assert!(!String::from_utf8_lossy(&engine.writer).contains("aaaaaaaaaaaaaaaa"));
        attacking["request_id"] = json!("retry");
        assert_eq!(
            party_order(&mut engine, &attacking).party_reservation,
            Some(first)
        );
        assert_eq!(engine.sequence, start + 2);
    }

    #[test]
    fn party_bridge_later_denial_and_refused_record_never_install_partial_members() {
        let files = Files::new();
        let mut engine = party_worker(&files, true);
        let first = party_order(&mut engine, &party_wire())
            .party_reservation
            .unwrap();
        let snapshot = engine.writer.clone();
        let mut second = opposing_party_wire();
        second["members"][1]["account_id"] = json!(1);
        let denied = party_order(&mut engine, &second);
        assert_eq!(denied.error.as_deref(), Some("party_member_conflict"));
        assert_eq!(engine.writer, snapshot);
        assert_eq!(
            engine.peers.keys().cloned().collect::<Vec<_>>(),
            first.reservation_ids
        );
        let mut divergent = party_wire();
        divergent["members"][0]["access_key"] = json!("cccccccccccccccc");
        assert_eq!(
            party_order(&mut engine, &divergent).error.as_deref(),
            Some("party_cohort_conflict")
        );
        engine.max_log = engine.written;
        let response = party_order(&mut engine, &opposing_party_wire());
        assert_eq!(response.error.as_deref(), Some("log_limit_reached"));
        assert_eq!(engine.writer, snapshot);
        assert_eq!(engine.peers.len(), 2);
        assert_eq!(engine.round_number, 1);
    }

    #[test]
    fn party_bridge_rollback_and_preemptive_tombstones_fence_empty_rounds() {
        let files = Files::new();
        let mut engine = party_worker(&files, true);
        let attacking = party_wire();
        let defending = opposing_party_wire();
        let early = party_order(&mut engine, &party_release_wire(&defending))
            .party_reservation
            .unwrap();
        assert_eq!(early.status, party_reservation::Status::Released);
        assert!(early.reservation_ids.is_empty());
        assert_eq!(
            party_order(&mut engine, &defending).error.as_deref(),
            Some("party_cohort_released")
        );
        let accepted = party_order(&mut engine, &attacking)
            .party_reservation
            .unwrap();
        let release = party_release_wire(&attacking);
        let removed = party_order(&mut engine, &release)
            .party_reservation
            .unwrap();
        assert_eq!(removed.status, party_reservation::Status::Released);
        assert_eq!(removed.reservation_ids, accepted.reservation_ids);
        assert_eq!((removed.team, removed.squad), (None, None));
        assert!(engine.peers.is_empty() && engine.active.is_none());
        assert!(engine.selected_match.is_none());
        let observed_sequence = engine.sequence;
        assert_eq!(
            party_order(&mut engine, &release).party_reservation,
            Some(removed)
        );
        assert_eq!(engine.sequence, observed_sequence);
        assert_eq!(
            party_order(&mut engine, &attacking).error.as_deref(),
            Some("party_cohort_released")
        );
        let mut fresh_group = opposing_party_wire();
        fresh_group["cohort_id"] = json!("new-cohort");
        assert_eq!(
            party_order(&mut engine, &fresh_group).error.as_deref(),
            Some("party_formation_stale")
        );
        let new = select(&mut engine, "next-formation", "party-profile");
        assert_eq!(new.selected_match.unwrap().admission_epoch, Some(2));
        assert_eq!(
            party_order(&mut engine, &release).error.as_deref(),
            Some("party_formation_stale")
        );
    }

    #[test]
    fn party_bridge_can_roll_back_two_accepted_cohorts_with_exact_retries() {
        let files = Files::new();
        for refuse_record in [false, true] {
            let mut engine = party_worker(&files, true);
            let a = party_wire();
            let b = opposing_party_wire();
            let first = party_order(&mut engine, &a).party_reservation.unwrap();
            let second = party_order(&mut engine, &b).party_reservation.unwrap();
            let mut alien = b.clone();
            alien["world_id"] = json!(8);
            assert_eq!(
                party_order(&mut engine, &alien).error.as_deref(),
                Some("party_world_conflict")
            );
            let a_release = party_release_wire(&a);
            let b_release = party_release_wire(&b);
            if refuse_record {
                let snapshot = engine.writer.clone();
                engine.max_log = engine.written;
                assert_eq!(
                    party_order(&mut engine, &b_release).error.as_deref(),
                    Some("log_limit_reached")
                );
                assert_eq!(engine.peers.len(), 4);
                assert_eq!(engine.writer, snapshot);
                continue;
            }
            let b_removed = party_order(&mut engine, &b_release)
                .party_reservation
                .unwrap();
            assert_eq!(b_removed.reservation_ids, second.reservation_ids);
            assert_eq!(
                engine.peers.keys().cloned().collect::<Vec<_>>(),
                first.reservation_ids
            );
            let a_removed = party_order(&mut engine, &a_release)
                .party_reservation
                .unwrap();
            assert!(engine.peers.is_empty());
            let sequence = engine.sequence;
            assert_eq!(
                party_order(&mut engine, &a_release).party_reservation,
                Some(a_removed)
            );
            assert_eq!(
                party_order(&mut engine, &b_release).party_reservation,
                Some(b_removed)
            );
            assert_eq!(engine.sequence, sequence);
        }
    }

    #[test]
    fn party_bridge_real_claim_protects_whole_group_and_admits_a_late_group() {
        let files = Files::new();
        let mut engine = party_worker(&files, true);
        let attacking = party_wire();
        let accepted = party_order(&mut engine, &attacking)
            .party_reservation
            .unwrap();
        let mut claim = engine
            .wire_profile
            .wire_message_type(3)
            .to_be_bytes()
            .to_vec();
        claim.extend([1; 36]);
        claim.extend([0; 12]); // Empty clan/keys, and the four-byte address.
        let event = json!({
            "v": BRIDGE_VERSION, "op":"event","request_id":"claim-group-player",
            "reservation_id":accepted.reservation_ids[0], "world_id":7, "player_id":0,
            "transport":"tcp","direction":"client_to_server","rt_type":61,"plugin_id":1,
            "payload_hex":protocol::encode_hex(&claim)
        });
        let claimed = party_order(&mut engine, &event);
        assert!(claimed.ok, "{:?}", claimed.error);
        assert!(!claimed.messages.is_empty());
        let before = engine.sequence;
        assert_eq!(
            party_order(&mut engine, &party_release_wire(&attacking))
                .error
                .as_deref(),
            Some("party_already_claimed")
        );
        assert_eq!(
            party_order(&mut engine, &attacking).party_reservation,
            Some(accepted)
        );
        assert_eq!(engine.sequence, before);
        // The running match still admits a whole new group, late, together.
        let late = party_order(&mut engine, &opposing_party_wire());
        assert!(late.ok, "{:?}", late.error);
        let late = late.party_reservation.unwrap();
        assert_eq!((late.team, late.squad), (Some(1), Some(16)));
        assert_eq!(engine.sequence, before + 1);
        assert_eq!(engine.active.iter().count() + engine.peers.len(), 4);
        assert!(String::from_utf8_lossy(&engine.writer).contains("\"late_entry\":true"));
    }

    #[test]
    fn party_bridge_requires_one_squad_even_when_the_side_has_room() {
        let files = Files::new();
        let mut engine = party_worker(&files, true);
        let mut attacking = party_wire();
        for row in attacking["members"].as_array_mut().unwrap() {
            let mut solo = row.clone();
            solo["op"] = json!("reserve");
            solo["v"] = json!(BRIDGE_VERSION);
            solo["world_id"] = json!(7);
            solo["request_id"] = json!("solo-before-party");
            let reserved = party_order(&mut engine, &solo);
            assert!(reserved.ok, "{:?}", reserved.error);
            row["reservation_id"] = json!(format!("synthetic-new-member-{}", row["player_id"]));
            row["player_id"] = json!(row["player_id"].as_u64().unwrap() + 2);
            row["account_id"] = json!(row["account_id"].as_u64().unwrap() + 2);
        }
        assert_eq!(engine.squad_reservation_count(0), 1);
        assert_eq!(engine.squad_reservation_count(1), 1);
        let before = engine.sequence;
        assert_eq!(
            party_order(&mut engine, &attacking).error.as_deref(),
            Some("party_no_room")
        );
        assert_eq!(engine.sequence, before);
        assert_eq!(engine.active.iter().count() + engine.peers.len(), 2);
    }

    fn leadership_catalog() -> leadership_passives::LeadershipCatalog {
        leadership_passives::LeadershipCatalog::from_json(
            &json!({
                "schema_version":1,"application_id":20231,
                "source":{"member":"scripts/character_leadership_abilities.zpr",
                    "member_sha256":"a".repeat(64),"archive_sha256":"b".repeat(64),
                    "extractor":"synthetic","projector":"synthetic"},
                "max_separation_range":15.0,"refresh_hold_ms":400,"removal_grace_ms":200,
                "levels":[
                    {"level":0,"name":"none","modifiers":[]},
                    {"level":1,"name":"squad","modifiers":[{"kind":"kCharModIncapacitatedTime","value":0.5}]},
                    {"level":2,"name":"platoon","modifiers":[{"kind":"kCharModReloadTime","value":0.1}]},
                    {"level":3,"name":"officer","modifiers":[{"kind":"kCharModAutoHeal","value":0.1}]}
                ]
            }).to_string(),
        ).unwrap()
    }

    fn native_leadership_policy() -> leadership_runtime::RolePolicy {
        native_policy_with_bindings(json!([]))
    }

    fn native_policy_with_bindings(bindings: Value) -> leadership_runtime::RolePolicy {
        leadership_runtime::RolePolicy::from_json(
            &json!({
                "schema_version":2,"application_id":20231,
                "squad_leader":"native","development_bindings":bindings,
                "native_election":{
                    "source_sha256":"a".repeat(64),"squad_level_weight":1.0,"squad_mslc_weight":5.0,
                    "default_record":{"leadership_level":1,"nominations":1,"matches_since_last_command":0}
                }
            }).to_string(),
        ).unwrap()
    }

    #[test]
    fn registry_accepts_native_leadership_in_either_builder_order() {
        let files = Files::new();
        assert!(
            base_engine(&files, Vec::new(), 100_000)
                .with_leadership_passives(leadership_catalog(), native_leadership_policy())
                .unwrap()
                .with_match_profiles(files.registry())
                .unwrap()
                .native_leader_election()
        );
        assert!(
            engine(&files, Vec::new(), 100_000)
                .with_leadership_passives(leadership_catalog(), native_leadership_policy())
                .unwrap()
                .native_leader_election()
        );
    }

    #[test]
    fn registry_legacy_active_abilities_reject_platoon_election_in_both_builder_orders() {
        const ERROR: &str = "platoon_leadership_requires_complete_active_abilities";
        let files = Files::new();
        let legacy_registry = || {
            let mut registry = files.registry();
            let profile = registry.profiles.get_mut("p-1-0-1").unwrap();
            let mut setup = crate::active_ability_engine_tests::configured()
                .active_abilities
                .unwrap()
                .setup;
            Arc::make_mut(&mut setup).map_name = profile.spawns.map_name().to_owned();
            profile.active_abilities = Some(setup);
            registry
        };
        assert_eq!(
            base_engine(&files, Vec::new(), 100_000)
                .with_leadership_passives(
                    leadership_catalog(),
                    platoon_leadership_policy().unwrap()
                )
                .unwrap()
                .with_match_profiles(legacy_registry())
                .err(),
            Some(ERROR)
        );
        assert_eq!(
            base_engine(&files, Vec::new(), 100_000)
                .with_leadership_passives(leadership_catalog(), native_leadership_policy())
                .unwrap()
                .with_match_profiles(legacy_registry())
                .unwrap()
                .with_leadership_passives(
                    leadership_catalog(),
                    platoon_leadership_policy().unwrap()
                )
                .err(),
            Some(ERROR)
        );
    }

    #[test]
    fn registry_still_rejects_first_member_leadership_in_either_builder_order() {
        let files = Files::new();
        let legacy = leadership_runtime::RolePolicy::from_json(
            r#"{"schema_version":1,"application_id":20231,"first_member_squad_leader":true,"development_bindings":[]}"#,
        ).unwrap();
        assert_eq!(
            base_engine(&files, Vec::new(), 100_000)
                .with_leadership_passives(leadership_catalog(), legacy.clone())
                .unwrap()
                .with_match_profiles(files.registry())
                .err(),
            Some("match_profiles_unsupported_world_extras")
        );
        assert_eq!(
            engine(&files, Vec::new(), 100_000)
                .with_leadership_passives(leadership_catalog(), legacy)
                .err(),
            Some("match_profiles_unsupported_world_extras")
        );
    }

    #[test]
    fn registry_rejects_native_leadership_with_development_bindings() {
        let files = Files::new();
        let policy = native_policy_with_bindings(json!([
            {"account_id":1,"character_id":1,"native_role":1}
        ]));
        assert_eq!(
            engine(&files, Vec::new(), 100_000)
                .with_development_control()
                .with_leadership_passives(leadership_catalog(), policy.clone())
                .err(),
            Some("match_profiles_unsupported_world_extras")
        );
        let mut fixed = base_engine(&files, Vec::new(), 100_000)
            .with_development_control()
            .with_leadership_passives(leadership_catalog(), policy)
            .unwrap();
        reserve_leader(&mut fixed, 0, 1, Value::Null);
        fixed.reconcile_squad_leaders().unwrap();
        assert_eq!(
            fixed.command_leadership_role(fixed.active.as_ref().unwrap()),
            None
        );
        // A registry is only installable before admission. Check the shared
        // predicate independently of the idle-engine guard.
        fixed.active = None;
        assert_eq!(
            fixed.with_match_profiles(files.registry()).err(),
            Some("match_profiles_unsupported_world_extras")
        );
    }

    fn reserve_leader(
        engine: &mut Engine<Vec<u8>>,
        player: u16,
        faction: u8,
        leadership: Value,
    ) -> String {
        let id = format!("synthetic-leader-reservation-{player}");
        let result = engine.handle(
            Request::Reserve {
                v: BRIDGE_VERSION,
                request_id: format!("reserve-{player}"),
                reservation_id: id.clone(),
                world_id: 7,
                player_id: player,
                account_id: u32::from(player) + 1,
                session_key: "aaaaaaaaaaaaaaaa".into(),
                access_key: "bbbbbbbbbbbbbbbb".into(),
                character: serde_json::from_value(json!({
                    "name": format!("Synthetic Leader {player}"),
                    "faction": faction,"head":0,"voice":0,"universe_id":26,
                    "character_id":u32::from(player)+1,
                    "leadership":leadership
                }))
                .unwrap(),
            },
            Duration::ZERO,
            1,
        );
        assert!(result.ok, "{:?}", result.error);
        let actor = engine
            .active
            .iter_mut()
            .chain(engine.peers.values_mut())
            .find(|actor| actor.reservation.id == id)
            .unwrap();
        crate::spawn_progression::tests::ready(actor);
        id
    }

    fn domination_spawns(files: &Files) -> Value {
        let path = files.spawns(2, 0, 1);
        let mut spawn: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        spawn["match_profile"]["team_capacity"] = json!(32);
        spawn["match_profile"]["squad_capacity"] = json!(4);
        let squads: Vec<_> = (0..32)
            .map(|index| {
                let team = index / 16;
                json!({"index":index,"team":team,"platoon":index % 16 / 4,"squad":index % 4,
                    "faction":if team == 0 { None } else { Some(1) },"available":[0],"default_spawn":0})
            })
            .collect();
        spawn["maps"][0]["squads"] = json!(squads);
        spawn
    }

    fn leadership_match(files: &Files) -> Engine<Vec<u8>> {
        leadership_match_with_policy(files, native_leadership_policy())
    }

    fn leadership_match_with_policy(
        files: &Files,
        policy: leadership_runtime::RolePolicy,
    ) -> Engine<Vec<u8>> {
        let path = files.write("leadership-spawns.json", domination_spawns(files));
        let manifest = files.write(
            "leadership-profiles.json",
            json!({"format":"mag_match_profiles_v1","profiles":[{
                "profile_id":"domination","spawn_catalog":path
            }]}),
        );
        let mut engine = base_engine(files, Vec::new(), 100_000)
            .with_match_profiles(Registry::load(manifest).unwrap())
            .unwrap()
            .with_leadership_passives(leadership_catalog(), policy)
            .unwrap();
        assert!(select(&mut engine, "leadership-1", "domination").ok);
        engine
    }

    #[test]
    fn registry_native_command_roles_follow_saved_nominations_and_held_squad_identity() {
        use crate::active_abilities::CommandRole;

        let files = Files::new();
        let mut engine = leadership_match(&files);
        let record = |leadership_level, nominations, matches_since_last_command| {
            json!({"leadership_level":leadership_level,"nominations":nominations,
                "matches_since_last_command":matches_since_last_command})
        };
        let fallback = reserve_leader(&mut engine, 0, 0, Value::Null);
        let masked = reserve_leader(&mut engine, 1, 0, record(0, 1, 100));
        let ranked = reserve_leader(&mut engine, 2, 0, record(1, 1, 10));
        let defender = reserve_leader(&mut engine, 3, 1, record(1, 1, 1));
        let squad = Arc::new(engine.spawn_catalog.as_ref().unwrap().for_squad(0).unwrap());
        for id in [&masked, &ranked] {
            engine.peers.get_mut(id).unwrap().squad_catalog = squad.clone();
        }
        assert_eq!(engine.peers[&masked].nominations, 0);
        assert_eq!(engine.peers[&fallback].nominations, 1);
        assert_eq!(
            engine.active.as_ref().unwrap().squad_catalog.squad_index(),
            16
        );
        assert!(!engine.match_started);
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(engine.command_leadership_role(&engine.peers[&ranked]), None);

        engine.match_started = true;
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(engine.command_leadership_role(&engine.peers[&masked]), None);
        assert_eq!(
            engine.command_leadership_role(&engine.peers[&fallback]),
            None
        );
        assert_eq!(
            engine.command_leadership_role(&engine.peers[&ranked]),
            Some(CommandRole::SquadLeader)
        );
        let defender_actor = engine.active.as_ref().unwrap();
        assert_eq!(defender_actor.reservation.id, defender);
        assert_eq!(
            engine.command_leadership_role(defender_actor),
            Some(CommandRole::SquadLeader)
        );

        let mut former = engine.peers.remove(&ranked).unwrap();
        let original_id = std::mem::replace(&mut former.reservation.id, "new-reservation".into());
        assert_eq!(engine.command_leadership_role(&former), None);
        former.reservation.id = original_id;
        let created = former.initial_objects.take();
        assert_eq!(engine.command_leadership_role(&former), None);
        former.initial_objects = created;
        former.squad_catalog = Arc::new(squad.for_squad(1).unwrap());
        assert_eq!(engine.command_leadership_role(&former), None);
        former.squad_catalog = squad;
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(engine.command_leadership_role(&former), None);
        assert_eq!(
            engine.command_leadership_role(&engine.peers[&fallback]),
            Some(CommandRole::SquadLeader)
        );
    }

    fn platoon_leadership_policy() -> Result<leadership_runtime::RolePolicy, io::Error> {
        leadership_runtime::RolePolicy::from_json(
            &json!({
                "schema_version":3,"application_id":20231,
                "squad_leader":"native","development_bindings":[],
                "native_election":{
                    "source_sha256":"a".repeat(64),"squad_level_weight":1.0,"squad_mslc_weight":5.0,
                    "default_record":{"leadership_level":1,"nominations":1,"matches_since_last_command":0}
                },
                "platoon_election":{"level_weight":2.0,"mslc_weight":1.0}
            }).to_string(),
        )
    }

    fn oic_policy() -> leadership_runtime::RolePolicy {
        leadership_runtime::RolePolicy::from_json(
            &json!({
                "schema_version":4,"application_id":20231,
                "squad_leader":"native","development_bindings":[],
                "native_election":{
                    "source_sha256":"a".repeat(64),"squad_level_weight":1.0,"squad_mslc_weight":5.0,
                    "default_record":{"leadership_level":1,"nominations":1,"matches_since_last_command":0}
                },
                "platoon_election":{"level_weight":2.0,"mslc_weight":1.0},
                "company_election":{"level_weight":2.0,"mslc_weight":1.0}
            }).to_string(),
        ).unwrap()
    }

    #[test]
    fn company_promotion_replaces_previous_authority_and_refills_it_atomically() {
        use crate::active_abilities::CommandRole::PlatoonLeader;

        let files = Files::new();
        let mut engine = leadership_match_with_policy(&files, oic_policy());
        let first = reserve_leader(
            &mut engine,
            0,
            0,
            json!({
                "leadership_level":3,"nominations":3,"matches_since_last_command":4
            }),
        );
        reserve_leader(
            &mut engine,
            1,
            0,
            json!({
                "leadership_level":3,"nominations":3,"matches_since_last_command":1
            }),
        );
        let squad = engine.active.as_ref().unwrap().squad_catalog.clone();
        engine.active.as_mut().unwrap().squad_catalog = squad.for_squad(0).unwrap().into();
        engine.match_started = true;
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(
            engine.command_leadership_role(&engine.peers[&first]),
            Some(PlatoonLeader)
        );

        let accepted = engine.requested_nominations(&engine.peers[&first], 0xff);
        assert_eq!(accepted, Some(7));
        engine.peers.get_mut(&first).unwrap().nominations = accepted.unwrap();
        let saved_limit = engine.max_log;
        engine.max_log = engine.written;
        assert_eq!(
            engine.reconcile_squad_leaders().err(),
            Some("log_limit_reached")
        );
        assert_eq!(engine.company_leader_index(0), u16::MAX);
        assert_eq!(engine.platoon_leader_index(0), 0);

        engine.max_log = saved_limit;
        engine.failed = false;
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(engine.company_leader_index(0), 0);
        assert_eq!(engine.company_leader_index(1), u16::MAX);
        assert_eq!(engine.platoon_leader_index(0), 1);
        assert_eq!(
            engine.strategic_leadership_team(&engine.peers[&first]),
            Some(0)
        );
        assert_eq!(engine.command_leadership_role(&engine.peers[&first]), None);
        assert_eq!(
            engine.strategic_leadership_team(engine.active.as_ref().unwrap()),
            None
        );

        engine.peers.get_mut(&first).unwrap().nominations = 3;
        assert_eq!(
            engine.strategic_leadership_team(&engine.peers[&first]),
            None
        );
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(engine.company_leader_index(0), u16::MAX);
        assert_eq!(engine.platoon_leader_index(0), 1);
    }

    #[test]
    fn company_nomination_requires_saved_tier_three_and_domination_policy() {
        let files = Files::new();
        let mut engine = leadership_match_with_policy(&files, oic_policy());
        let party = reserve_leader(
            &mut engine,
            0,
            0,
            json!({
                "leadership_level":2,"nominations":7,"matches_since_last_command":0
            }),
        );
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(actor.reservation.id, party);
        assert_eq!(actor.nominations, 3);
        assert_eq!(engine.requested_nominations(actor, 7), Some(3));

        let legacy = leadership_match_with_policy(&files, platoon_leadership_policy().unwrap());
        let mut character = actor.reservation.character.clone();
        character.leadership.as_mut().unwrap().leadership_level = 3;
        assert_eq!(legacy.initial_nominations(&character), 3);
        assert_eq!(engine.initial_nominations(&character), 7);
        let mut acquisition = base_engine(&files, Vec::new(), 100_000)
            .with_leadership_passives(leadership_catalog(), oic_policy())
            .unwrap();
        acquisition.spawn_catalog =
            Some(spawn_catalog::SpawnCatalog::load(files.spawns(1, 0, 1)).unwrap());
        assert_eq!(acquisition.initial_nominations(&character), 3);
        assert_eq!(character.leadership.unwrap().nominations, 7);
    }

    #[test]
    fn native_platoon_promotion_vacates_squad_and_refills_it_atomically() {
        use crate::active_abilities::CommandRole::{PlatoonLeader, SquadLeader};

        let files = Files::new();
        let mut engine = leadership_match_with_policy(&files, platoon_leadership_policy().unwrap());
        let first = reserve_leader(
            &mut engine,
            0,
            0,
            json!({
                "leadership_level":3,"nominations":5,"matches_since_last_command":2
            }),
        );
        let second = reserve_leader(
            &mut engine,
            1,
            0,
            json!({
                "leadership_level":2,"nominations":3,"matches_since_last_command":2
            }),
        );
        let squad = engine.active.as_ref().unwrap().squad_catalog.clone();
        engine.active.as_mut().unwrap().squad_catalog = squad.for_squad(0).unwrap().into();
        assert_eq!(engine.peers[&first].nominations, 1);
        engine.active.as_mut().unwrap().nominations = 1;
        engine.match_started = true;
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(
            engine.command_leadership_role(&engine.peers[&first]),
            Some(SquadLeader)
        );

        engine.peers.get_mut(&first).unwrap().nominations = 3;
        engine.active.as_mut().unwrap().nominations = 3;
        let saved_log = engine.max_log;
        engine.max_log = engine.written;
        assert!(engine.reconcile_squad_leaders().is_err());
        assert_eq!(
            engine.squad_leader_index(
                0,
                &[
                    0,
                    1,
                    u16::MAX,
                    u16::MAX,
                    u16::MAX,
                    u16::MAX,
                    u16::MAX,
                    u16::MAX
                ]
            ),
            0
        );
        engine.max_log = saved_log;
        engine.failed = false;
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(
            engine.command_leadership_role(&engine.peers[&first]),
            Some(PlatoonLeader)
        );
        assert_eq!(engine.active.as_ref().unwrap().reservation.id, second);
        assert_eq!(
            engine.command_leadership_role(engine.active.as_ref().unwrap()),
            Some(SquadLeader)
        );

        engine.peers.get_mut(&first).unwrap().nominations = 1;
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(
            engine.command_leadership_role(&engine.peers[&first]),
            Some(SquadLeader)
        );
        assert_eq!(
            engine.command_leadership_role(engine.active.as_ref().unwrap()),
            Some(PlatoonLeader)
        );

        engine.active.take();
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(
            engine.command_leadership_role(&engine.peers[&first]),
            Some(SquadLeader)
        );
    }

    #[test]
    fn native_platoon_election_scores_all_squads_with_platoon_weights() {
        use crate::active_abilities::CommandRole::{PlatoonLeader, SquadLeader};

        let files = Files::new();
        let policy = platoon_leadership_policy().unwrap();
        assert!(policy.supports_platoon_election());
        assert!(!native_leadership_policy().supports_platoon_election());
        let mut engine = leadership_match_with_policy(&files, policy);
        let first = reserve_leader(
            &mut engine,
            0,
            0,
            json!({
                "leadership_level":2,"nominations":3,"matches_since_last_command":3
            }),
        );
        reserve_leader(
            &mut engine,
            1,
            0,
            json!({
                "leadership_level":2,"nominations":3,"matches_since_last_command":0
            }),
        );
        let actor = engine.active.as_mut().unwrap();
        actor.reservation.character.progression = Some(progression::ProgressionSnapshot {
            level: 3,
            ..Default::default()
        });
        actor.squad_catalog = actor.squad_catalog.for_squad(1).unwrap().into();
        assert_eq!(engine.platoon_leader_index(0), u16::MAX);

        engine.match_started = true;
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(engine.platoon_leader_index(0), 1);
        assert_eq!(
            engine.command_leadership_role(engine.active.as_ref().unwrap()),
            Some(PlatoonLeader)
        );
        assert_eq!(
            engine.command_leadership_role(&engine.peers[&first]),
            Some(SquadLeader)
        );

        let actor = engine.active.as_mut().unwrap();
        actor.squad_catalog = actor.squad_catalog.for_squad(4).unwrap().into();
        assert_eq!(
            engine.command_leadership_role(engine.active.as_ref().unwrap()),
            None
        );
        engine.reconcile_squad_leaders().unwrap();
        assert_eq!(engine.platoon_leader_index(0), 0);
        assert_eq!(engine.platoon_leader_index(1), 1);
    }

    #[test]
    fn leadership_formation_retry_and_failed_selection_preserve_held_roles() {
        let files = Files::new();
        for fail_selection in [false, true] {
            let mut engine = leadership_match(&files);
            reserve_leader(&mut engine, 0, 0, Value::Null);
            engine.match_started = true;
            engine.reconcile_squad_leaders().unwrap();
            let mut members = [u16::MAX; 8];
            members[0] = 0;
            assert_eq!(engine.squad_leader_index(0, &members), 0);
            assert!(select(&mut engine, "leadership-1", "domination").ok);
            assert_eq!(engine.squad_leader_index(0, &members), 0);
            // Exercise the selection boundary with held roster state left over.
            engine.active.take().unwrap();
            if fail_selection {
                engine.max_log = engine.written;
            }
            let result = select(&mut engine, "leadership-2", "domination");
            assert_eq!(result.ok, !fail_selection, "{:?}", result.error);
            assert_eq!(
                engine.squad_leader_index(0, &members),
                if fail_selection { 0 } else { u16::MAX }
            );
        }
    }
    fn select<W: Write>(engine: &mut Engine<W>, formation: &str, profile: &str) -> Response {
        engine.handle(
            Request::SelectMatchProfile {
                v: BRIDGE_VERSION,
                request_id: "select".into(),
                formation_id: formation.into(),
                profile_id: profile.into(),
            },
            Duration::ZERO,
            1,
        )
    }
    fn reserve<W: Write>(engine: &mut Engine<W>) -> Response {
        reserve_id(engine, "synthetic-reservation-0001")
    }
    fn reserve_id<W: Write>(engine: &mut Engine<W>, reservation: &str) -> Response {
        engine.handle(Request::Reserve { v: BRIDGE_VERSION, request_id: "reserve".into(), reservation_id:reservation.into(), world_id: 7, player_id: 0, account_id: 3, session_key: "aaaaaaaaaaaaaaaa".into(), access_key: "bbbbbbbbbbbbbbbb".into(), character: serde_json::from_value(json!({"name":"Synthetic Player","faction":0,"head":0,"voice":0,"universe_id":26})).unwrap() }, Duration::ZERO, 1)
    }
    fn release<W: Write>(engine: &mut Engine<W>) -> Response {
        engine.handle(
            Request::Release {
                v: BRIDGE_VERSION,
                request_id: "release".into(),
                reservation_id: "synthetic-reservation-0001".into(),
            },
            Duration::ZERO,
            1,
        )
    }
    fn add_gameplay_profile<W: Write>(engine: &mut Engine<W>) {
        let sample = crate::spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::capture_fixture();
        let spawns = sample.spawn_catalog.unwrap();
        let policy = spawns.match_profile();
        engine.match_profiles.as_mut().unwrap().profiles.insert(
            "acquisition-world".into(),
            Loaded {
                active_abilities: None,
                summary: Summary {
                    profile_id: "acquisition-world".into(),
                    game_type: 1,
                    attacking_faction: policy.attacking_faction,
                    defending_faction: policy.defending_faction,
                    team_capacity: policy.team_capacity,
                    squad_capacity: None,
                    squad_counts: None,
                    bot_fill_total: None,
                    human_capacity: None,
                    human_team_capacity: None,
                    maximum_players: policy.team_capacity * 2,
                },
                spawns,
                objectives: sample.objective_catalog,
                collision: sample.collision_world,
                navmesh: sample.navmesh_world,
                vehicles: sample.ground_vehicles,
                damage: None,
                domination_round: None,
                domination: None,
                bunkers: None,
                hospital_turret_source: None,
                hospital_rocket_source: None,
                hospital_rockets: None,
                infantry_rocket_source: None,
                guided_launcher_source: None,
                infantry_rockets: None,
                guided_launchers: None,
                gas_grenade_source: None,
                flash_grenade_source: None,
                smoke_grenades: None,
                gas_grenades: None,
                flash_grenades: None,
                underbarrel_launcher_source: None,
                underbarrel_launchers: None,
                ground_turret_source: None,
                acquisition_apc_turret_source: None,
                acquisition: Some(sample.acquisition_gameplay.unwrap().setup),
                bots: None,
            },
        );
    }

    #[test]
    fn profile_bunkers_require_all_three_bound_inputs() {
        let files = Files::new();
        let plain = files.spawns(0, 0, 1);
        for fields in [
            json!({"bunker_placement": files.0.join("placements.json")}),
            json!({
                "bunker_placement": files.0.join("placements.json"),
                "bunker_structure": files.0.join("structures.json"),
                "bunker_allocation": files.0.join("allocation.json"),
            }),
        ] {
            let mut profile = json!({"profile_id":"fake","spawn_catalog":plain});
            profile
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let manifest = files.write(
                "manifest.json",
                json!({"format":"mag_match_profiles_v1","profiles":[profile]}),
            );
            assert_eq!(
                Registry::load(manifest).err().unwrap().to_string(),
                "unexpected_match_profile_bunkers"
            );
        }
    }

    #[test]
    fn acquisition_registry_requires_retail_combat_authority() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        assert!(engine.item_catalog.retail_combat().is_err());
        assert!(
            engine
                .match_profiles
                .as_ref()
                .unwrap()
                .validate_runtime(&engine.item_catalog, engine.wire_profile)
                .is_ok()
        );
        add_gameplay_profile(&mut engine);
        assert_eq!(
            engine
                .match_profiles
                .as_ref()
                .unwrap()
                .validate_runtime(&engine.item_catalog, engine.wire_profile),
            Err("retail_combat_unavailable")
        );
    }

    #[test]
    fn acquisition_selection_replays_current_formation_and_resets_a_fresh_formation() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        add_gameplay_profile(&mut engine);
        assert!(
            engine
                .match_profiles
                .as_ref()
                .unwrap()
                .has_acquisition_gameplay()
        );
        assert!(select(&mut engine, "acquisition-1", "acquisition-world").ok);
        let runtime = engine.acquisition_gameplay.as_mut().unwrap();
        runtime
            .state
            .request_state(100, replication::ObjectiveState::Locked)
            .unwrap();
        let round = engine.acquisition_round.as_mut().unwrap();
        let before = round.points();
        round
            .apply_score(acquisition_round::ScoreIntent {
                team: 1,
                delta: -200,
            })
            .unwrap();
        assert_eq!(round.points()[1], before[1] - 200);

        assert!(select(&mut engine, "acquisition-1", "acquisition-world").ok);
        assert_eq!(
            engine.acquisition_round.as_ref().unwrap().points()[1],
            before[1] - 200
        );
        assert_eq!(
            engine
                .acquisition_gameplay
                .as_ref()
                .unwrap()
                .state
                .pending_transitions()
                .count(),
            1
        );

        assert!(select(&mut engine, "acquisition-2", "acquisition-world").ok);
        assert_eq!(engine.acquisition_round.as_ref().unwrap().points(), before);
        assert_eq!(
            engine
                .acquisition_gameplay
                .as_ref()
                .unwrap()
                .state
                .pending_transitions()
                .count(),
            0
        );
        assert!(
            engine
                .ground_runtime
                .as_ref()
                .unwrap()
                .get(4)
                .is_some_and(|v| v.state == 0)
        );

        assert!(select(&mut engine, "sabotage-1", "p-0-0-1").ok);
        assert!(engine.acquisition_gameplay.is_none() && engine.acquisition_round.is_none());
    }

    #[test]
    fn infantry_rocket_registry_requires_mode_and_contacts_before_readiness() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        add_gameplay_profile(&mut engine);
        let items = Arc::clone(&engine.item_catalog);
        let registry = engine.match_profiles.as_mut().unwrap();
        assert!(!registry.has_infantry_rockets());
        registry
            .profiles
            .get_mut("acquisition-world")
            .unwrap()
            .infantry_rocket_source = Some(Arc::from(&b"{}"[..]));
        assert_eq!(
            registry.bind_infantry_rockets(&items),
            Err("infantry_rocket_contact_bindings_required")
        );
        let source = registry.profiles.get_mut("acquisition-world").unwrap();
        source.summary.game_type = 0;
        assert_eq!(
            registry.bind_infantry_rockets(&items),
            Err("infantry_rocket_mode_required")
        );
        assert!(!registry.has_infantry_rockets());
    }

    #[test]
    fn underbarrel_registry_requires_contact_support_and_clears_on_profile_switch() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        add_gameplay_profile(&mut engine);
        let items = Arc::clone(&engine.item_catalog);
        let registry = engine.match_profiles.as_mut().unwrap();
        let profile = registry.profiles.get_mut("acquisition-world").unwrap();
        profile.underbarrel_launcher_source = Some(Arc::from(&b"{}"[..]));
        assert_eq!(
            registry.bind_underbarrel_launchers(&items),
            Err("underbarrel_launcher_contact_bindings_required")
        );
        assert!(!registry.has_underbarrel_launchers());

        let profile = registry.profiles.get_mut("acquisition-world").unwrap();
        profile.underbarrel_launcher_source = None;
        profile.underbarrel_launchers = Some(Arc::new(
            crate::underbarrel_launchers::tests::synthetic_catalog(),
        ));
        assert!(select(&mut engine, "underbarrel-enabled", "acquisition-world").ok);
        assert!(engine.underbarrel_launchers.is_some());
        assert!(select(&mut engine, "underbarrel-disabled", "p-0-0-1").ok);
        assert!(engine.underbarrel_launchers.is_none());
    }

    #[test]
    fn acquisition_projectile_contacts_survive_replay_and_clear_on_profile_switch() {
        use crate::domination_projectile_collision::{ContactBindings, acquisition_tests};

        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        add_gameplay_profile(&mut engine);
        let profile = engine
            .match_profiles
            .as_mut()
            .unwrap()
            .profiles
            .get_mut("acquisition-world")
            .unwrap();
        let raw = serde_json::to_vec(&acquisition_tests::acquisition()).unwrap();
        let contacts = Arc::new(ContactBindings::from_acquisition_json(&raw).unwrap());
        Arc::get_mut(profile.acquisition.as_mut().unwrap())
            .unwrap()
            .projectile_contacts = Some(Arc::clone(&contacts));

        for _ in 0..2 {
            assert!(select(&mut engine, "acquisition-contacts", "acquisition-world").ok);
            assert!(Arc::ptr_eq(
                engine.projectile_contacts.as_ref().unwrap(),
                &contacts
            ));
            assert!(engine.domination_gameplay.is_none());
        }
        assert!(select(&mut engine, "switch-without-contacts", "p-0-0-1").ok);
        assert!(engine.projectile_contacts.is_none() && engine.acquisition_gameplay.is_none());
    }

    #[test]
    fn failed_profile_selection_preserves_existing_acquisition_authority() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        add_gameplay_profile(&mut engine);
        assert!(select(&mut engine, "acquisition-1", "acquisition-world").ok);
        let round = engine.acquisition_round.as_mut().unwrap();
        round
            .apply_score(acquisition_round::ScoreIntent {
                team: 1,
                delta: -400,
            })
            .unwrap();
        let committed = round.clone();
        let old_setup = Arc::clone(&engine.acquisition_gameplay.as_ref().unwrap().setup);
        let old_ground = engine.ground_runtime.clone();
        let old_selected = engine.selected_match.clone();
        engine.max_log = engine.written;

        let response = select(&mut engine, "failed-switch", "p-0-0-1");
        assert_eq!(response.error.as_deref(), Some("log_limit_reached"));
        assert_eq!(engine.selected_match, old_selected);
        assert_eq!(engine.acquisition_round.as_ref(), Some(&committed));
        assert_eq!(engine.ground_runtime, old_ground);
        assert!(Arc::ptr_eq(
            &engine.acquisition_gameplay.as_ref().unwrap().setup,
            &old_setup
        ));
    }

    #[test]
    fn domination_profiles_require_all_squads_and_bounded_admission() {
        let files = Files::new();
        let mut spawn = domination_spawns(&files);
        let path = files.write("domination-spawns.json", spawn.clone());
        let manifest = files.write(
            "domination-profiles.json",
            json!({"format":"mag_match_profiles_v1","profiles":[{
                "profile_id":"domination","spawn_catalog":path
            }]}),
        );
        let mut engine = engine(&files, Vec::new(), 100_000);
        engine.match_profiles = Some(Registry::load(&manifest).unwrap());
        let selected = select(&mut engine, "formation-domination", "domination");
        assert!(selected.ok, "{:?}", selected.error);
        assert_eq!(selected.selected_match.unwrap().game_type, 2);
        assert_eq!(engine.admission_capacity(), 64);
        assert_eq!(engine.allocate_squad(0).unwrap().squad_index(), 0);
        assert_eq!(engine.allocate_squad(1).unwrap().squad_index(), 16);
        assert_eq!(
            engine.allocate_squad(2).unwrap_err(),
            "unsupported_match_character"
        );

        spawn["match_profile"]["team_capacity"] = json!(8);
        spawn["match_profile"]["squad_capacity"] = json!(2);
        files.write("domination-spawns.json", spawn.clone());
        assert_eq!(
            Registry::load(&manifest).unwrap().summaries()[0].maximum_players,
            16
        );
        // Retail eight-member squads; the Squad object holds no more than eight.
        spawn["match_profile"]["squad_capacity"] = json!(8);
        files.write("domination-spawns.json", spawn.clone());
        assert_eq!(
            Registry::load(&manifest).unwrap().summaries()[0].maximum_players,
            16
        );
        spawn["match_profile"]["squad_capacity"] = json!(9);
        files.write("domination-spawns.json", spawn.clone());
        assert!(Registry::load(&manifest).is_err());

        spawn["match_profile"]["team_capacity"] = json!(32);
        spawn["match_profile"]["squad_capacity"] = json!(4);
        // Fifteen squads can still seat 32 players. The authored 16th squad
        // must be present independently of the local seat limit.
        spawn["maps"][0]["squads"].as_array_mut().unwrap().pop();
        files.write("domination-spawns.json", spawn);
        assert_eq!(
            Registry::load(&manifest).err().unwrap().to_string(),
            "unsupported_match_profile_policy"
        );
    }

    /// A world of `team` characters per side over `squads` eight-member
    /// squads on each side.
    fn eight_member_spawns(files: &Files, mode: i8, team: usize, squads: usize) -> PathBuf {
        let mut spawn: Value =
            serde_json::from_slice(&std::fs::read(files.spawns(mode, 0, 1)).unwrap()).unwrap();
        spawn["match_profile"]["team_capacity"] = json!(team);
        spawn["match_profile"]["squad_capacity"] = json!(8);
        let squads: Vec<_> = (0..2)
            .flat_map(|side| (0..squads).map(move |k| (side, k)))
            .map(|(side, k)| {
                json!({"index":side * 16 + k,"team":side,"platoon":k / 4,"squad":k % 4,
                    "faction":if side == 0 { None } else { Some(1) },"available":[0],"default_spawn":0})
            })
            .collect();
        spawn["maps"][0]["squads"] = json!(squads);
        files.write(&format!("eight-{mode}-{team}.json"), spawn)
    }

    fn hello(engine: &mut Engine<Vec<u8>>) -> Value {
        let response = engine.handle(
            Request::Hello {
                v: BRIDGE_VERSION,
                request_id: "hello".into(),
            },
            Duration::ZERO,
            1,
        );
        assert!(response.ok, "{:?}", response.error);
        serde_json::to_value(response).unwrap()
    }

    #[test]
    fn worlds_above_the_legacy_size_advertise_a_separate_human_cap() {
        let files = Files::new();
        let manifest = files.write(
            "large-profiles.json",
            json!({"format":"mag_match_profiles_v1","profiles":[
                {"profile_id":"acquisition-128","spawn_catalog":eight_member_spawns(&files, 1, 64, 8)},
                {"profile_id":"domination-256","spawn_catalog":eight_member_spawns(&files, 2, 128, 16)},
                {"profile_id":"domination-64","spawn_catalog":eight_member_spawns(&files, 2, 32, 16)},
            ]}),
        );
        let mut engine = engine(&files, Vec::new(), 100_000);
        engine.match_profiles = Some(Registry::load(&manifest).unwrap());
        let hello = hello(&mut engine);
        assert!(
            hello["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!(world_capacity::CAPABILITY))
        );
        let row = |id: &str| {
            hello["match_profiles"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["profile_id"] == id)
                .unwrap()
                .clone()
        };
        // maximum_players and team_capacity keep meaning the world's size.
        let acquisition = row("acquisition-128");
        assert_eq!(
            [
                &acquisition["maximum_players"],
                &acquisition["team_capacity"],
                &acquisition["human_capacity"],
                &acquisition["human_team_capacity"]
            ],
            [&json!(128), &json!(64), &json!(32), &json!(16)]
        );
        let domination = row("domination-256");
        assert_eq!(
            [
                &domination["maximum_players"],
                &domination["team_capacity"],
                &domination["human_capacity"],
                &domination["human_team_capacity"]
            ],
            [&json!(256), &json!(128), &json!(32), &json!(16)]
        );
        // A legacy-size world, here with retail eight-member Domination squads,
        // keeps today's row: every seat may be human.
        let legacy = row("domination-64");
        assert_eq!(legacy["maximum_players"], 64);
        assert!(legacy.get("human_capacity").is_none());
        assert!(legacy.get("human_team_capacity").is_none());

        // Selecting the large world admits humans against the human cap.
        assert!(select(&mut engine, "formation-large", "acquisition-128").ok);
        assert_eq!(engine.admission_capacity(), 128);
        assert_eq!(engine.maximum_players, 32);
        let configure = |engine: &mut Engine<Vec<u8>>, maximum, teams| {
            engine.handle(
                Request::ConfigureMatch {
                    v: BRIDGE_VERSION,
                    request_id: "configure".into(),
                    minimum_players: 2,
                    minimum_players_per_team: teams,
                    maximum_players: maximum,
                    application_id: 20231,
                },
                Duration::ZERO,
                1,
            )
        };
        assert_eq!(
            configure(&mut engine, 64, [1, 1]).error.as_deref(),
            Some("invalid_match_limits")
        );
        assert_eq!(
            configure(&mut engine, 32, [17, 1]).error.as_deref(),
            Some("invalid_team_quorum")
        );
        let configured = configure(&mut engine, 32, [1, 1]);
        assert!(configured.ok, "{:?}", configured.error);
        assert_eq!(engine.maximum_players, 32);
        // The legacy world still admits its whole capacity.
        assert!(select(&mut engine, "formation-legacy", "domination-64").ok);
        assert_eq!(engine.maximum_players, 64);
    }

    #[test]
    fn worlds_beyond_the_character_pool_are_refused() {
        let files = Files::new();
        let path = eight_member_spawns(&files, 2, 128, 16);
        let manifest = files.write(
            "pool-profiles.json",
            json!({"format":"mag_match_profiles_v1","profiles":[
                {"profile_id":"too-large","spawn_catalog":path}
            ]}),
        );
        assert!(Registry::load(&manifest).is_ok());
        let mut spawn: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        spawn["match_profile"]["team_capacity"] = json!(129);
        files.write("eight-2-128.json", spawn);
        assert!(Registry::load(&manifest).is_err());
    }

    #[test]
    fn all_six_ordered_pairings_per_mode_are_preloaded_and_advertised() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        let response = engine.handle(
            Request::Hello {
                v: BRIDGE_VERSION,
                request_id: "hello".into(),
            },
            Duration::ZERO,
            1,
        );
        assert!(response.ok);
        assert!(response.capabilities.contains(&CAPABILITY.to_owned()));
        // No world above the legacy size: no capability and no human caps.
        assert!(
            !response
                .capabilities
                .contains(&world_capacity::CAPABILITY.to_owned())
        );
        assert!(
            serde_json::to_value(&response.match_profiles)
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row.get("human_capacity").is_none())
        );
        assert_eq!(response.match_profiles.len(), 12);
        for summary in response.match_profiles {
            let selection = select(
                &mut engine,
                &format!("formation-{}", summary.profile_id),
                &summary.profile_id,
            );
            assert!(selection.ok, "{:?}", selection.error);
            let selected = selection.selected_match.unwrap();
            assert_eq!(
                (
                    selected.game_type,
                    selected.attacking_faction,
                    selected.defending_faction
                ),
                (
                    summary.game_type,
                    summary.attacking_faction,
                    summary.defending_faction
                )
            );
            assert_eq!(
                engine.map_name(),
                format!("synthetic_home_{}", selected.defending_faction)
            );
            assert_eq!(summary.maximum_players, 2);
            assert_eq!(summary.team_capacity, 1);
        }
    }
    #[test]
    fn reserve_gate_active_conflict_retry_and_retired_formation() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        assert_eq!(
            reserve(&mut engine).error.as_deref(),
            Some("match_profile_not_selected")
        );
        assert!(select(&mut engine, "formation-1", "p-0-0-1").ok);
        assert!(reserve(&mut engine).ok);
        let sequence = engine.sequence;
        assert!(select(&mut engine, "formation-1", "p-0-0-1").ok);
        assert_eq!(
            engine.sequence, sequence,
            "exact retry writes no duplicate selection"
        );
        assert_eq!(
            select(&mut engine, "formation-1", "p-1-0-2")
                .error
                .as_deref(),
            Some("match_formation_profile_conflict")
        );
        assert_eq!(
            select(&mut engine, "formation-2", "p-1-0-2")
                .error
                .as_deref(),
            Some("match_profile_selection_locked")
        );
        assert!(release(&mut engine).ok);
        assert!(engine.selected_match.is_none());
        assert_eq!(
            reserve(&mut engine).error.as_deref(),
            Some("match_profile_not_selected")
        );
        assert_eq!(
            select(&mut engine, "formation-1", "p-0-0-1")
                .error
                .as_deref(),
            Some("match_formation_retired")
        );
        assert!(select(&mut engine, "formation-2", "p-1-0-2").ok);
        assert_eq!(
            reserve(&mut engine).error.as_deref(),
            Some("reservation_id_retired")
        );
        assert!(reserve_id(&mut engine, "synthetic-reservation-0002").ok);
    }
    #[test]
    fn idle_switch_clears_world_state_and_keeps_global_catalogs() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        assert!(select(&mut engine, "formation-1", "p-0-0-1").ok);
        let items = Arc::clone(&engine.item_catalog);
        engine.match_epoch = Some(Duration::from_secs(1));
        engine.world_epoch = engine.match_epoch;
        engine.last_world_tick = engine.match_epoch;
        engine.parachute_waves.insert((0, Duration::ZERO), 1);
        engine
            .next_repair
            .insert("old-world".into(), Duration::ZERO);
        engine
            .pending_bunker_damage
            .push_back((0, 5., "old-world".into()));
        assert!(select(&mut engine, "formation-2", "p-1-0-2").ok);
        assert!(
            engine.match_epoch.is_none()
                && engine.world_epoch.is_none()
                && engine.last_world_tick.is_none()
        );
        assert!(
            engine.parachute_waves.is_empty()
                && engine.next_repair.is_empty()
                && engine.pending_bunker_damage.is_empty()
        );
        assert!(Arc::ptr_eq(&items, &engine.item_catalog));
        assert!(
            engine.collision_world.is_none()
                && engine.navmesh_world.is_none()
                && engine.ground_vehicles.is_none()
                && engine.character_damage_rules.is_none()
        );
        assert_eq!(engine.map_name(), "synthetic_home_2");
    }
    #[test]
    fn pending_selection_survives_actor_release_and_independent_switch() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        assert!(select(&mut engine, "formation-1", "p-0-0-1").ok);
        assert!(reserve(&mut engine).ok);
        let receipt = loadout_selection::CommittedLoadoutSelection {
            layout_revision: 0,
            selection_id: "synthetic-selection".into(),
            reservation_id: "synthetic-reservation-0001".into(),
            world_id: 7,
            player_id: 0,
            account_id: "synthetic-account".into(),
            character_id: 4,
            revision: 1,
            inventory_index: 0,
        };
        engine.active.as_mut().unwrap().pending_loadout_selection = Some(receipt.clone());
        assert!(release(&mut engine).ok);
        assert_eq!(
            engine.pending_loadout_selections().as_slice(),
            std::slice::from_ref(&receipt)
        );
        assert!(select(&mut engine, "formation-2", "p-1-0-2").ok);
        assert_eq!(engine.pending_loadout_selections(), vec![receipt.clone()]);
        let response = engine.handle(
            Request::LoadoutSelectionSettled {
                v: BRIDGE_VERSION,
                request_id: "settle".into(),
                reservation_id: receipt.reservation_id,
                world_id: 7,
                player_id: 0,
                selection_id: receipt.selection_id,
                revision: 1,
                status: loadout_selection::SelectionStatus::Persisted,
            },
            Duration::ZERO,
            1,
        );
        assert!(response.ok);
        assert!(engine.pending_loadout_selections().is_empty());
        assert!(select(&mut engine, "formation-2", "p-1-0-2").ok);
    }
    #[test]
    fn journal_failure_is_atomic_and_does_not_ack_a_different_world() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        assert!(select(&mut engine, "formation-1", "p-0-0-1").ok);
        engine.max_log = engine.written;
        let response = select(&mut engine, "formation-2", "p-1-0-2");
        assert_eq!(response.error.as_deref(), Some("log_limit_reached"));
        assert!(response.selected_match.is_none());
        assert_eq!(
            engine.selected_match.as_ref().unwrap().formation_id,
            "formation-1"
        );
        assert_eq!(engine.map_name(), "synthetic_home_1");
        assert!(!engine.retired_formations.contains("formation-1"));
    }
    #[test]
    fn registry_match_configuration_requires_two_human_sides() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        assert!(select(&mut engine, "formation-1", "p-0-0-1").ok);
        assert_eq!(engine.minimum_players, 2);
        assert_eq!(engine.minimum_players_per_team, [1; 2]);
        for (minimum, teams, expected) in
            [(1, [0, 0], false), (2, [1, 0], false), (2, [1, 1], true)]
        {
            let response = engine.handle(
                Request::ConfigureMatch {
                    v: BRIDGE_VERSION,
                    request_id: "configure".into(),
                    minimum_players: minimum,
                    minimum_players_per_team: teams,
                    maximum_players: 2,
                    application_id: 20231,
                },
                Duration::ZERO,
                1,
            );
            assert_eq!(response.ok, expected, "{:?}", response.error);
        }
    }

    // Synthetic bot-filled profiles: an invented flat floor and two slots per
    // world, validated against the profile's own map rather than the base.
    fn bot_floor(files: &Files, defender: u8) -> PathBuf {
        files.write(&format!("floor-{defender}.json"), json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":format!("synthetic_home_{defender}"),"build":"synthetic","coverage":"provisional",
                "provenance":["Synthetic profile bot floor"],"unknowns":["Not a game map"],
                "physics_regions":[{"min":[-10.,-5.,-10.],"max":[10.,8.,10.],"ground_verified":true,
                    "obstacles_verified":true,"provenance":["Synthetic floor"]}]},
            "instances":[{"id":"floor","vertices_xyz":[[-10.,0.,-10.],[10.,0.,-10.],[10.,0.,10.],[-10.,0.,10.]],
                "triangles":[[0,2,1],[0,3,2]]}]
        }))
    }
    fn bot_population(files: &Files, map: &str) -> PathBuf {
        files.write(&format!("population-{map}.json"), json!({
            "format":"mag_bot_population_v1","map_name":map,"firearms":{"enabled":false},
            "slots":[{"team":0,"position":{"x":2.,"y":0.,"z":2.}},{"team":1,"position":{"x":6.,"y":0.,"z":2.}}]
        }))
    }
    fn bot_names_file(files: &Files, names: &[&str]) -> PathBuf {
        let entries: Vec<_> = names.iter().map(|name| json!({"name": name})).collect();
        files.write(
            &format!("names-{}.json", names.len()),
            json!({"entries": entries}),
        )
    }
    /// Row "bots" (Sabotage 0 vs 1) fills with bots; row "plain" (1 vs 0) does not.
    fn bot_manifest(files: &Files, bots: Value) -> PathBuf {
        let mut row = json!({
            "profile_id":"bots","spawn_catalog":files.spawns(0, 0, 1),
            "collision_world":bot_floor(files, 1),
        });
        for (key, value) in bots.as_object().unwrap() {
            row[key] = value.clone();
        }
        files.write(
            "bot-profiles.json",
            json!({"format":"mag_match_profiles_v1","profiles":[
                row, {"profile_id":"plain","spawn_catalog":files.spawns(0, 1, 0)}
            ]}),
        )
    }
    fn bot_ready_row(files: &Files) -> Value {
        json!({
            "bot_population": bot_population(files, "synthetic_home_1"),
            "bot_fill_total": 2,
            "bot_names": bot_names_file(files, &["Profile Alpha", "Profile Bravo"]),
        })
    }
    fn bot_base_engine(files: &Files) -> Engine<Vec<u8>> {
        let resources = EngineResources::for_application(
            Arc::new(crate::bot_registry::identity_tests::bootstrap_items()),
            spawn_catalog::SpawnCatalog::load(files.spawns(0, 0, 1)).unwrap(),
            None,
            crate::resources::tests::wire_profiles(),
            20231,
        )
        .unwrap();
        Engine::from_resources(Vec::new(), 100_000, resources)
    }
    fn configure_quorum(
        engine: &mut Engine<Vec<u8>>,
        minimum: usize,
        teams: [usize; 2],
    ) -> Response {
        engine.handle(
            Request::ConfigureMatch {
                v: BRIDGE_VERSION,
                request_id: "configure".into(),
                minimum_players: minimum,
                minimum_players_per_team: teams,
                maximum_players: 2,
                application_id: 20231,
            },
            Duration::ZERO,
            1,
        )
    }

    #[test]
    fn bot_filled_profile_is_advertised_installed_and_starts_with_one_human() {
        let files = Files::new();
        let registry = Registry::load(bot_manifest(&files, bot_ready_row(&files))).unwrap();
        let mut engine = bot_base_engine(&files)
            .with_match_profiles(registry)
            .unwrap();
        // No bots run in the unselected base world.
        assert!(!engine.bot_registry.enabled());
        let hello = serde_json::to_value(engine.describe_worker("hello".into(), false)).unwrap();
        assert!(
            hello["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!(BOT_FILL_CAPABILITY))
        );
        let rows = hello["match_profiles"].as_array().unwrap();
        let row = |id: &str| rows.iter().find(|row| row["profile_id"] == id).unwrap();
        assert_eq!(row("bots")["bot_fill_total"], json!(2));
        assert!(row("plain").get("bot_fill_total").is_none());

        assert!(select(&mut engine, "formation-bots", "bots").ok);
        assert!(engine.bot_registry.enabled());
        assert_eq!(engine.bot_names.name_for(0), Some("Profile Alpha"));
        assert_eq!(
            (engine.minimum_players, engine.minimum_players_per_team),
            (1, [0, 0])
        );
        assert!(
            engine.match_started,
            "one bot-filled human starts the world"
        );
        let one = configure_quorum(&mut engine, 1, [0, 0]);
        assert!(one.ok, "{:?}", one.error);
        assert!(configure_quorum(&mut engine, 1, [1, 0]).ok);

        // The lone human's side keeps that seat; the bot fills the other side.
        assert!(reserve(&mut engine).ok);
        engine
            .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
            .unwrap();
        let teams: Vec<_> = engine
            .bot_registry
            .bots
            .values()
            .map(|bot| bot.target.squad.team())
            .collect();
        assert_eq!(teams, [1]);
    }

    #[test]
    fn selecting_a_profile_without_bots_clears_bots_and_keeps_two_human_sides() {
        let files = Files::new();
        let registry = Registry::load(bot_manifest(&files, bot_ready_row(&files))).unwrap();
        let mut worker = bot_base_engine(&files)
            .with_match_profiles(registry)
            .unwrap();
        let engine = &mut worker;
        assert!(select(engine, "formation-bots", "bots").ok);
        assert!(engine.bot_registry.enabled());
        assert!(select(engine, "formation-plain", "plain").ok);
        assert!(!engine.bot_registry.enabled());
        assert_eq!(
            (engine.minimum_players, engine.minimum_players_per_team),
            (2, [1, 1])
        );
        assert_eq!(
            configure_quorum(engine, 1, [0, 0]).error.as_deref(),
            Some("match_profiles_require_two_human_sides")
        );
        assert!(configure_quorum(engine, 2, [1, 1]).ok);
        // A registry without any bot rows advertises nothing new.
        let plain = self::engine(&files, Vec::new(), 100_000);
        let hello = serde_json::to_value(plain.describe_worker("hello".into(), false)).unwrap();
        assert!(
            !hello["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!(BOT_FILL_CAPABILITY))
        );
        assert!(
            hello["match_profiles"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row.get("bot_fill_total").is_none())
        );
    }

    #[test]
    fn profile_bots_are_refused_at_startup_against_their_own_world() {
        let files = Files::new();
        let refused = |row: Value| {
            let mut registry = Registry::load(bot_manifest(&files, row)).unwrap();
            let names = bot_names::Catalog::default();
            let items = Arc::new(crate::bot_registry::identity_tests::bootstrap_items());
            let error = registry
                .validate_bots(&BotValidationInputs {
                    items: &items,
                    wire_profiles: &crate::resources::tests::wire_profiles(),
                    names: &names,
                    fall_damage_rules: None,
                })
                .unwrap_err();
            assert_eq!(error.profile_id, "bots");
            assert!(error.to_string().starts_with("match profile bots: "));
            error.code
        };
        // A population authored for another map never runs in this world.
        let mut row = bot_ready_row(&files);
        row["bot_population"] = json!(bot_population(&files, "synthetic_home_0"));
        assert_eq!(refused(row), "bot_population_map_profile_mismatch");
        let mut row = bot_ready_row(&files);
        row["bot_names"] = json!(bot_names_file(&files, &["Only One"]));
        assert_eq!(refused(row), "bot_names_insufficient_capacity");
        let mut row = bot_ready_row(&files);
        row["bot_fill_total"] = json!(4);
        row["bot_names"] = json!(bot_names_file(&files, &["N1", "N2", "N3", "N4"]));
        assert_eq!(refused(row), "bot_population_capacity");
        let mut row = bot_ready_row(&files);
        row.as_object_mut().unwrap().remove("bot_fill_total");
        assert_eq!(refused(row), "match_profile_bot_fill_total_required");
        // A slot beyond this profile's verified floor has no supported ground.
        let mut row = bot_ready_row(&files);
        row["bot_population"] = json!(files.write("off-floor.json", json!({
            "format":"mag_bot_population_v1","map_name":"synthetic_home_1","firearms":{"enabled":false},
            "slots":[{"team":0,"position":{"x":2.,"y":0.,"z":2.}},{"team":1,"position":{"x":60.,"y":0.,"z":2.}}]
        })));
        assert_eq!(refused(row), "bot_physics_geometry_coverage_unknown");

        // The engine applies the same validation before installing a registry.
        let mut row = bot_ready_row(&files);
        row["bot_population"] = json!(bot_population(&files, "synthetic_home_0"));
        assert_eq!(
            bot_base_engine(&files)
                .with_match_profiles(Registry::load(bot_manifest(&files, row)).unwrap())
                .err(),
            Some("bot_population_map_profile_mismatch")
        );

        // A worker-wide default fills rows without their own total.
        let mut row = bot_ready_row(&files);
        row.as_object_mut().unwrap().remove("bot_fill_total");
        let mut registry = Registry::load(bot_manifest(&files, row)).unwrap();
        registry.set_default_bot_fill_total(2).unwrap();
        let mut engine = bot_base_engine(&files)
            .with_match_profiles(registry)
            .unwrap();
        assert!(select(&mut engine, "formation-default", "bots").ok);
        assert!(engine.bot_registry.enabled());
        assert_eq!(
            files.registry().set_default_bot_fill_total(2),
            Err("bot_fill_total_requires_profile_bot_population")
        );
    }

    #[test]
    fn profile_bot_settings_are_bound_to_a_population_and_supported_scope() {
        let files = Files::new();
        let load_error = |row: Value| {
            Registry::load(bot_manifest(&files, row))
                .err()
                .unwrap()
                .to_string()
        };
        assert_eq!(
            load_error(json!({"bot_fill_total": 2})),
            "match profile bots: match_profile_bot_settings_require_population"
        );
        let mut row = bot_ready_row(&files);
        row["bot_resupply_approach_policy"] = json!(files.0.join("approach.json"));
        assert_eq!(
            load_error(row),
            "match profile bots: bot_resupply_approach_requires_provider_catalog"
        );
        let regional = files.write(
            "regional.json",
            json!({
                "format":"mag_bot_population_v1","map_name":"synthetic_home_1","regional_team":0,
                "slots":[{"team":0,"position":{"x":2.,"y":0.,"z":2.}}]
            }),
        );
        let mut row = bot_ready_row(&files);
        row["bot_population"] = json!(regional);
        assert!(load_error(row).starts_with("match profile bots: "));
        // Global populations stay refused: they belong to one map.
        let global: crate::bot_registry::BotPopulation = serde_json::from_slice(
            &std::fs::read(bot_population(&files, "synthetic_home_1")).unwrap(),
        )
        .unwrap();
        let registry = Registry::load(bot_manifest(&files, bot_ready_row(&files))).unwrap();
        assert_eq!(
            bot_base_engine(&files)
                .with_match_profiles(registry)
                .unwrap()
                .with_bot_population(global, 2)
                .err(),
            Some("match_profiles_unsupported_world_extras")
        );
    }

    #[test]
    fn terminal_selection_archives_old_members_and_failure_leaves_them_live() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        assert!(select(&mut engine, "formation-1", "p-0-0-1").ok);
        assert!(reserve(&mut engine).ok);
        let old_match = engine.reward_match_id.clone().unwrap();
        let mut next = engine.round.clone();
        next.scores = Some([0, 1000]);
        next.outcome = Some(match_flow::Outcome {
            winner: 1,
            major: true,
            deadline: Duration::from_secs(5),
            finalized: false,
        });
        engine
            .commit_round_update(next, vec![], vec![], Duration::from_secs(2), 2)
            .unwrap();
        let max = engine.max_log;
        engine.max_log = engine.written;
        assert_eq!(
            select(&mut engine, "formation-2", "p-1-0-2")
                .error
                .as_deref(),
            Some("log_limit_reached")
        );
        assert!(engine.results_cohorts.is_empty());
        assert!(engine.active.is_some());
        assert_eq!(
            engine.selected_match.as_ref().unwrap().formation_id,
            "formation-1"
        );
        engine.max_log = max;
        engine.failed = false;
        assert!(select(&mut engine, "formation-2", "p-1-0-2").ok);
        assert!(engine.active.is_none() && engine.peers.is_empty());
        assert_eq!(engine.results_cohorts[&old_match].actors.len(), 1);
        assert!(engine.round.outcome.is_none());
        assert!(reserve_id(&mut engine, "synthetic-fresh-profile-reservation").ok);
        assert_eq!(engine.active.as_ref().unwrap().match_experience, 0);
        assert_eq!(
            select(&mut engine, "formation-1", "p-0-0-1")
                .error
                .as_deref(),
            Some("match_formation_retired")
        );
    }

    #[test]
    fn pending_rewards_survive_idle_switch_without_blocking_next_cohort() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        assert!(select(&mut engine, "formation-1", "p-0-0-1").ok);
        let reward = progression::RewardEvent {
            match_id: "synthetic-match".into(),
            reward_id: "synthetic-reward".into(),
            reservation_id: "synthetic-reservation-0001".into(),
            world_id: 7,
            player_id: 0,
            account_id: "synthetic-account".into(),
            character_id: 4,
            kind: progression::RewardKind::EnemyKilled,
            units: 1,
            leadership: None,
            frago: None,
        };
        engine.reward_order.push_back(reward.reward_id.clone());
        engine
            .pending_rewards
            .insert(reward.reward_id.clone(), reward.clone());
        assert!(select(&mut engine, "formation-2", "p-1-0-2").ok);
        assert_eq!(engine.pending_reward_batch(), [reward]);
        assert_eq!(
            engine.selected_match.as_ref().unwrap().formation_id,
            "formation-2"
        );
    }

    #[test]
    fn expiry_retires_selection_and_reservation_and_preserves_selection_receipt() {
        let files = Files::new();
        let mut engine = engine(&files, Vec::new(), 100_000);
        assert!(select(&mut engine, "formation-1", "p-0-0-1").ok);
        assert!(reserve(&mut engine).ok);
        engine.active.as_mut().unwrap().pending_loadout_selection =
            Some(loadout_selection::CommittedLoadoutSelection {
                layout_revision: 0,
                selection_id: "synthetic-selection".into(),
                reservation_id: "synthetic-reservation-0001".into(),
                world_id: 7,
                player_id: 0,
                account_id: "synthetic-account".into(),
                character_id: 4,
                revision: 1,
                inventory_index: 0,
            });
        engine
            .expire_world_peers(Duration::from_secs(121), 122)
            .unwrap();
        assert!(engine.active.is_none() && engine.peers.is_empty());
        assert!(engine.selected_match.is_none());
        assert!(engine.retired_formations.contains("formation-1"));
        assert!(
            engine
                .released_reservations
                .contains("synthetic-reservation-0001")
        );
        assert_eq!(engine.pending_loadout_selections().len(), 1);
        assert!(select(&mut engine, "formation-2", "p-1-0-2").ok);
        assert_eq!(engine.pending_loadout_selections().len(), 1);
    }

    #[test]
    fn loaded_world_resources_are_source_bound_and_removed_by_resource_less_switch() {
        let files = Files::new();
        let spawn = files.spawns(0, 0, 1);
        let catalog = spawn_catalog::SpawnCatalog::load(&spawn).unwrap();
        let collision = files.write("collision.json", json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic_home_1","build":"synthetic","coverage":"provisional","provenance":["SYNTHETIC floor"],"unknowns":["Not a game map"]},
            "instances":[{"id":"floor","vertices_xyz":[[0.,0.,0.],[4.,0.,0.],[0.,0.,4.]],"triangles":[[0,2,1]]}]}));
        let damage = files.write("damage-good.json", json!({"schema_version":1,"application_id":20231,"map_name":"synthetic_home_1","game_type":0,"friendly_fire_multiplier":0.5,"provenance":{"source":"synthetic","spawn_catalog_sha256":catalog.source_sha256().unwrap()}}));
        let registry = Registry::load(files.write("worlds.json", json!({"format":"mag_match_profiles_v1","profiles":[
            {"profile_id":"with-world","spawn_catalog":spawn,"collision_world":collision,"character_damage_rules":damage},
            {"profile_id":"without-world","spawn_catalog":files.spawns(1,0,2)}]}))).unwrap();
        let mut engine = engine(&files, Vec::new(), 100_000);
        engine.match_profiles = Some(registry);
        assert!(select(&mut engine, "formation-1", "with-world").ok);
        assert!(engine.collision_world.is_some());
        assert_eq!(
            engine
                .character_damage_rules
                .as_ref()
                .unwrap()
                .friendly_fire_multiplier(),
            0.5
        );
        assert!(select(&mut engine, "formation-2", "without-world").ok);
        assert!(engine.collision_world.is_none() && engine.character_damage_rules.is_none());
    }

    #[test]
    fn collision_coverage_sidecar_binds_to_the_profile_world_and_covers_its_navmesh() {
        use sha2::Digest;
        let files = Files::new();
        let spawn = files.spawns(0, 0, 1);
        let collision = files.write("collision.json", json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic_home_1","build":"synthetic","coverage":"provisional","provenance":["SYNTHETIC floor"],"unknowns":["Not a game map"]},
            "instances":[{"id":"floor","vertices_xyz":[[0.,0.,0.],[10.,0.,0.],[10.,0.,10.],[0.,0.,10.]],"triangles":[[0,2,1],[0,3,2]]}]}));
        let world_sha256 = navmesh::sha256(&std::fs::read(&collision).unwrap());
        let region = crate::collision::PhysicsRegion {
            min: [0., -3., 0.],
            max: [10., 3., 10.],
            ground_verified: true,
            obstacles_verified: true,
            provenance: vec!["SYNTHETIC assertion".into()],
        };
        let sidecar = |world: &str| {
            json!({"format":"mag_collision_coverage_v1","map_name":"synthetic_home_1",
                "world_sha256":world,"basis":"operator_assertion","provenance":["SYNTHETIC"],
                "physics_regions":[region]})
        };
        let coverage = files.write("coverage.json", sidecar(&world_sha256));
        let foreign = files.write("foreign.json", sidecar(&"f".repeat(64)));
        let vertices: Vec<[f32; 3]> =
            vec![[0., 0., 0.], [10., 0., 0.], [10., 0., 10.], [0., 0., 10.]];
        let triangles: Vec<[u32; 3]> = vec![[0, 2, 1], [0, 3, 2]];
        let mut geometry = sha2::Sha256::new();
        geometry.update(b"MAG-NAV-GEOMETRY-v1\0");
        geometry.update(4u32.to_le_bytes());
        geometry.update(2u32.to_le_bytes());
        for value in vertices.iter().flatten() {
            geometry.update(value.to_le_bytes());
        }
        for value in triangles.iter().flatten() {
            geometry.update(value.to_le_bytes());
        }
        let artifact = navmesh::bake(navmesh::BakeInput {
            format: "mag_navmesh_bake_input_v1".into(),
            map_name: "synthetic_home_1".into(),
            world_sha256: world_sha256.clone(),
            geometry_sha256: format!("{:x}", geometry.finalize()),
            bounds: navmesh::Bounds {
                min: region.min,
                max: region.max,
            },
            physics_regions: vec![region.clone()],
            vertices,
            triangles,
        })
        .unwrap();
        let mesh = files.write("mesh.json", serde_json::to_value(&artifact).unwrap());
        let load = |name: &str, row: Value| {
            Registry::load(files.write(
                name,
                json!({"format":"mag_match_profiles_v1","profiles":[row]}),
            ))
        };
        let row = json!({"profile_id":"covered","spawn_catalog":spawn,
            "collision_world":collision,"navmesh_world":mesh});
        assert_eq!(
            load("uncovered.json", row.clone())
                .err()
                .unwrap()
                .to_string(),
            "navmesh_geometry_coverage_unknown"
        );
        let mut wrong = row.clone();
        wrong["collision_coverage"] = json!(foreign);
        assert_eq!(
            load("foreign-profiles.json", wrong)
                .err()
                .unwrap()
                .to_string(),
            "collision_coverage_world_mismatch"
        );
        let orphan =
            json!({"profile_id":"covered","spawn_catalog":spawn,"collision_coverage":coverage});
        assert_eq!(
            load("orphan.json", orphan).err().unwrap().to_string(),
            "match_profile_collision_coverage_requires_collision"
        );
        let mut covered = row;
        covered["collision_coverage"] = json!(coverage);
        let registry = load("covered.json", covered).unwrap();
        let world = registry.profiles["covered"].collision.as_ref().unwrap();
        assert_eq!(world.source_sha256(), world_sha256);
        assert!(world.metadata().physics_regions.is_empty());
        assert_eq!(
            world.coverage_sidecar().unwrap().world_sha256(),
            world_sha256
        );
        assert!(registry.profiles["covered"].navmesh.is_some());
    }

    #[test]
    fn disk_writer_failure_never_installs_or_acknowledges_selection() {
        struct BrokenWriter;
        impl Write for BrokenWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("synthetic disk failure"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let files = Files::new();
        let mut engine = engine(&files, BrokenWriter, 100_000);
        let response = select(&mut engine, "formation-1", "p-1-0-2");
        assert_eq!(response.error.as_deref(), Some("recorder_unavailable"));
        assert!(response.selected_match.is_none() && engine.selected_match.is_none());
        assert_eq!(engine.map_name(), "synthetic_home_1");
        assert_eq!(
            reserve(&mut engine).error.as_deref(),
            Some("match_profile_not_selected")
        );
    }

    #[test]
    fn portable_registry_uses_moved_manifest_directory_for_relative_paths() {
        let files = Files::new();
        let original = files.0.join("prepared");
        let moved = files.0.join("installed");
        std::fs::create_dir_all(original.join("catalogs")).unwrap();
        std::fs::rename(files.spawns(0, 0, 1), original.join("catalogs/spawns.json")).unwrap();
        let save = |directory: &Path, spawn: &str| {
            let path = directory.join("profiles.json");
            std::fs::write(
                &path,
                json!({"format":"mag_match_profiles_v1","profiles":[
                {"profile_id":"invented","spawn_catalog":spawn}]})
                .to_string(),
            )
            .unwrap();
            path
        };
        save(&original, "catalogs/spawns.json");
        std::fs::rename(&original, &moved).unwrap();
        let loaded = Registry::load(moved.join("profiles.json")).unwrap();
        assert_eq!(loaded.summaries()[0].profile_id, "invented");

        std::fs::copy(
            moved.join("catalogs/spawns.json"),
            files.0.join("outside.json"),
        )
        .unwrap();
        for rejected in ["../outside.json", "", "catalogs/missing.json"] {
            assert!(
                Registry::load(save(&moved, rejected)).is_err(),
                "{rejected:?}"
            );
        }
    }

    #[test]
    fn manifest_rejects_duplicates_missing_sources_and_incoherent_worlds() {
        let files = Files::new();
        let spawn = files.spawns(0, 0, 1);
        let row = json!({"profile_id":"profile","spawn_catalog":spawn});
        assert!(
            Registry::load(files.write(
                "duplicate.json",
                json!({"format":"mag_match_profiles_v1","profiles":[row.clone(),row.clone()]})
            ))
            .is_err()
        );
        assert!(Registry::load(files.write("relative.json", json!({"format":"mag_match_profiles_v1","profiles":[{"profile_id":"profile","spawn_catalog":"relative.json"}]}))).is_err());
        assert!(Registry::load(files.write("missing.json", json!({"format":"mag_match_profiles_v1","profiles":[{"profile_id":"profile","spawn_catalog":files.0.join("missing-source.json")}]}))).is_err());
        let damage = files.write("damage.json", json!({"schema_version":1,"application_id":20231,"map_name":"synthetic_home_1","game_type":0,"friendly_fire_multiplier":1.,"provenance":{"source":"synthetic","spawn_catalog_sha256":"0".repeat(64)}}));
        let mut wrong = row.clone();
        wrong["character_damage_rules"] = json!(damage);
        assert!(
            Registry::load(files.write(
                "bad-damage.json",
                json!({"format":"mag_match_profiles_v1","profiles":[wrong]})
            ))
            .is_err()
        );
        let mut wrong = row;
        wrong["navmesh_world"] = json!(files.0.join("mesh.json"));
        assert!(
            Registry::load(files.write(
                "bad-navmesh.json",
                json!({"format":"mag_match_profiles_v1","profiles":[wrong]})
            ))
            .is_err()
        );
    }

    /// A Sabotage row whose objective catalog names `elf`, optionally beside a
    /// plain game-type-1 row that has no gameplay configuration.
    fn smoke_registry(files: &Files, elf: &str, plain_acquisition: bool) -> Registry {
        let spawns = files.write("smoke-sabotage-spawns.json", json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC smoke registry",
            "active":{"map_name":"synthetic_home_1","squad_index":0,"default_spawn":0,"available":[0]},
            "match_profile":{"game_type":0,"auto_assign_teams":true,"team_capacity":1,"squad_capacity":1,"attacking_faction":0,"has_bunkers":false,"has_objectives":true},
            "maps":[{"name":"synthetic_home_1","owner_faction":1,"spawn_count":1,
                "locations":[{"index":0,"token":7,"kind":"on_foot","position":{"x":2.,"y":0.,"z":2.}}],
                "squads":[{"index":0,"team":0,"platoon":0,"squad":0,"faction":null,"available":[0],"default_spawn":0},
                {"index":16,"team":1,"platoon":0,"squad":0,"faction":1,"available":[0],"default_spawn":0}]}]
        }));
        let squads: Vec<_> = (0..8)
            .map(|slot| {
                json!({
                    "team": slot / 4, "squad_index": slot / 4 * 16 + slot % 4,
                    "initial_objective_index": 117,
                    "authored_objectives": [{"object_index": 117, "order": 1}]
                })
            })
            .collect();
        let objectives = files.write("smoke-sabotage-objectives.json", json!({
            "schema_version": 1, "format": "mag_retail_objective_catalog",
            "application_id": 20231, "protocol": crate::protocol::ProtocolInfo::RETAIL_PROTOCOL,
            "game_type": 0, "map_name": "synthetic_home_1", "entity_family": 12, "network_type": 12,
            "source": {
                "authored_catalog_sha256": "0".repeat(64), "binding_capture_sha256": "1".repeat(64),
                "startup_proof_sha256": "2".repeat(64), "source_image_sha256": elf,
                "capture_qualification": "bounded_concurrent_token_slot_capture"
            },
            "objects": [{"token": "0x76543210", "object_index": 117, "initial_state": 1}],
            "squads": squads
        }));
        let mut rows = vec![
            json!({"profile_id":"sabotage","spawn_catalog":spawns,"objective_catalog":objectives}),
        ];
        if plain_acquisition {
            rows.push(json!({"profile_id":"acquisition","spawn_catalog":files.spawns(1, 0, 1)}));
        }
        Registry::load(files.write(
            "smoke-manifest.json",
            json!({"format":"mag_match_profiles_v1","profiles":rows}),
        ))
        .unwrap()
    }

    /// The Acquisition fixture row, plus a Domination row built from the
    /// synthetic Domination world, both bound to the retail executable.
    fn add_smoke_gameplay_profiles<W: Write>(engine: &mut Engine<W>, elf: &str) {
        add_gameplay_profile(engine);
        let fixture = crate::domination_gameplay_engine::tests::structural_repair::repair_fixture();
        let profiles = &mut engine.match_profiles.as_mut().unwrap().profiles;
        let mut domination = profiles.remove("acquisition-world").unwrap();
        domination.summary.profile_id = "domination-world".into();
        domination.summary.game_type = 2;
        domination.acquisition = None;
        domination.objectives = fixture.objective_catalog.clone();
        domination.domination = fixture.domination_gameplay.clone();
        profiles.insert("domination-world".into(), domination);
        add_gameplay_profile(engine);
        for profile in engine
            .match_profiles
            .as_mut()
            .unwrap()
            .profiles
            .values_mut()
        {
            profile.objectives = profile
                .objectives
                .take()
                .map(|catalog| catalog.with_source_image(elf));
        }
    }

    #[test]
    fn one_worker_smoke_flag_reaches_every_mode_on_selection() {
        use crate::smoke_grenades::tests::{
            SYNTHETIC_ELF, synthetic_item_id, synthetic_rules_and_items,
        };
        let files = Files::new();
        let (rules, items) = synthetic_rules_and_items();
        let mut engine = base_engine(&files, Vec::new(), 100_000)
            .with_match_profiles(smoke_registry(&files, SYNTHETIC_ELF, false))
            .unwrap();
        add_smoke_gameplay_profiles(&mut engine, SYNTHETIC_ELF);
        let registry = engine.match_profiles.as_mut().unwrap();
        registry.set_smoke_grenade_rules(Arc::from(rules)).unwrap();
        registry.bind_smoke_grenades(&items).unwrap();
        let bound: BTreeMap<_, _> = registry
            .profiles
            .iter()
            .map(|(id, profile)| {
                (
                    id.clone(),
                    Arc::clone(profile.smoke_grenades.as_ref().unwrap()),
                )
            })
            .collect();
        assert_eq!(
            bound.keys().map(String::as_str).collect::<Vec<_>>(),
            ["acquisition-world", "domination-world", "sabotage"]
        );

        for (formation, profile, mode) in [
            ("smoke-sabotage", "sabotage", 0),
            ("smoke-acquisition", "acquisition-world", 1),
            ("smoke-sabotage-again", "sabotage", 0),
        ] {
            let selected = select(&mut engine, formation, profile);
            assert!(selected.ok, "{profile}: {:?}", selected.error);
            assert_eq!(engine.match_profile().game_type, mode);
            assert!(Arc::ptr_eq(
                engine.smoke_grenades.as_ref().unwrap(),
                &bound[profile]
            ));
            for faction in 0..3 {
                let throw = engine
                    .grenade_parameters(faction, synthetic_item_id(faction))
                    .expect("every mode throws the stock smoke");
                assert_eq!(
                    (
                        throw.fuse_ms,
                        throw.authored_lifetime_ms,
                        throw.initial_count
                    ),
                    (2000, 9000, 2)
                );
                assert_eq!(throw.damage.character, 0.0);
            }
        }
        // The same install gate a fixed worker uses accepts each mode's catalog.
        let domination =
            crate::domination_gameplay_engine::tests::structural_repair::repair_fixture();
        assert_eq!(domination.match_profile().game_type, 2);
        assert!(
            domination
                .with_smoke_grenades(Arc::clone(&bound["domination-world"]))
                .is_ok()
        );
        assert!(
            base_engine(&files, Vec::new(), 100_000)
                .with_smoke_grenades(Arc::clone(&bound["sabotage"]))
                .is_ok()
        );
    }

    #[test]
    fn worker_smoke_flag_binds_through_the_registry_install() {
        use crate::smoke_grenades::tests::{SYNTHETIC_ELF, synthetic_rules_and_items};
        let files = Files::new();
        let (rules, items) = synthetic_rules_and_items();
        let mut registry = smoke_registry(&files, SYNTHETIC_ELF, false);
        registry.set_smoke_grenade_rules(Arc::from(rules)).unwrap();
        let mut engine = base_engine(&files, Vec::new(), 100_000);
        engine.item_catalog = Arc::new(items);
        let engine = engine.with_match_profiles(registry).unwrap();
        assert!(engine.match_profiles.as_ref().unwrap().has_smoke_grenades());
        // Without the flag no profile has smoke, whatever its mode.
        let mut engine = engine_without_smoke(&files);
        add_smoke_gameplay_profiles(&mut engine, SYNTHETIC_ELF);
        let registry = engine.match_profiles.as_mut().unwrap();
        registry
            .bind_smoke_grenades(&synthetic_rules_and_items().1)
            .unwrap();
        assert!(!registry.has_smoke_grenades());
    }

    fn engine_without_smoke(files: &Files) -> Engine<Vec<u8>> {
        base_engine(files, Vec::new(), 100_000)
            .with_match_profiles(smoke_registry(
                files,
                crate::smoke_grenades::tests::SYNTHETIC_ELF,
                false,
            ))
            .unwrap()
    }

    #[test]
    fn worker_smoke_flag_refuses_profiles_it_cannot_bind() {
        use crate::smoke_grenades::tests::{SYNTHETIC_ELF, synthetic_rules_and_items};
        let files = Files::new();
        let (rules, items) = synthetic_rules_and_items();
        let rules: Arc<[u8]> = Arc::from(rules);

        // One worker-wide source; a second is a conflict, not a replacement.
        let mut registry = smoke_registry(&files, SYNTHETIC_ELF, false);
        registry
            .set_smoke_grenade_rules(Arc::clone(&rules))
            .unwrap();
        assert_eq!(
            registry.set_smoke_grenade_rules(Arc::from(&b"other"[..])),
            Err("smoke_grenade_source_conflict")
        );
        assert!(Arc::ptr_eq(
            registry.smoke_grenade_rules.as_ref().unwrap(),
            &rules
        ));

        // Every profile binds to its own objective catalog's executable.
        let mut registry = files.registry();
        registry
            .set_smoke_grenade_rules(Arc::clone(&rules))
            .unwrap();
        assert_eq!(
            registry.bind_smoke_grenades(&items),
            Err("smoke_grenade_objectives_required")
        );
        // A game-type-1 row without Acquisition gameplay cannot carry smoke.
        let mut registry = smoke_registry(&files, SYNTHETIC_ELF, true);
        registry
            .set_smoke_grenade_rules(Arc::clone(&rules))
            .unwrap();
        assert_eq!(
            registry.bind_smoke_grenades(&items),
            Err("smoke_grenade_mode_required")
        );
        let mut registry = smoke_registry(&files, &"3".repeat(64), false);
        registry
            .set_smoke_grenade_rules(Arc::clone(&rules))
            .unwrap();
        assert_eq!(
            registry.bind_smoke_grenades(&items),
            Err("smoke_grenade_source_mismatch")
        );
        let mut registry = smoke_registry(&files, SYNTHETIC_ELF, false);
        registry
            .set_smoke_grenade_rules(Arc::from(&b"{}"[..]))
            .unwrap();
        assert_eq!(
            registry.bind_smoke_grenades(&items),
            Err("invalid_smoke_grenade_rules")
        );
        assert!(!registry.has_smoke_grenades());

        // A base world carrying fixed-profile smoke cannot become a registry.
        let catalog =
            crate::smoke_grenades::Catalog::from_bytes(&rules, &items, SYNTHETIC_ELF).unwrap();
        let mut registry = smoke_registry(&files, SYNTHETIC_ELF, false);
        registry.set_smoke_grenade_rules(rules).unwrap();
        let mut fixed = base_engine(&files, Vec::new(), 100_000);
        fixed.item_catalog = Arc::new(items);
        let fixed = fixed.with_smoke_grenades(Arc::new(catalog)).unwrap();
        assert_eq!(
            fixed.with_match_profiles(registry).err(),
            Some("match_profiles_require_idle_explicit_retail_engine")
        );
    }
}
