//! Explicit preloaded worlds for one idle, formation-selected retail worker.
//! Catalogs stay external; selecting never performs disk I/O.
use crate::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const CAPABILITY: &str = "match_profiles_v1";
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
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SelectedMatch {
    pub formation_id: String,
    pub profile_id: String,
    pub game_type: i8,
    pub attacking_faction: u8,
    pub defending_faction: u8,
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
    navmesh_world: Option<PathBuf>,
    ground_vehicles: Option<PathBuf>,
    character_damage_rules: Option<PathBuf>,
    acquisition_gameplay: Option<PathBuf>,
    domination_round: Option<PathBuf>,
    domination_gameplay: Option<PathBuf>,
    bunker_placement: Option<PathBuf>,
    bunker_structure: Option<PathBuf>,
    bunker_allocation: Option<PathBuf>,
}
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
}
pub struct Registry {
    profiles: BTreeMap<String, Loaded>,
}
impl Registry {
    pub fn validate_dynamic_bodies(
        &self,
        catalog: &crate::dynamic_collision::Catalog,
        bindings: &crate::dynamic_collision_runtime::RuntimeBindings,
    ) -> Result<(), &'static str> {
        for profile in self.profiles.values() {
            bindings.validate_for(catalog, profile.vehicles.as_ref())?;
            if let Some(setup) = &profile.acquisition {
                setup.validate_ground_hulk_templates(catalog)?;
            }
        }
        Ok(())
    }
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        absolute(path)?;
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
        for source in manifest.profiles {
            if !identifier(&source.profile_id, 1) || profiles.contains_key(&source.profile_id) {
                return Err(io::Error::other("invalid_or_duplicate_match_profile_id"));
            }
            for path in [
                Some(&source.spawn_catalog),
                source.objective_catalog.as_ref(),
                source.active_abilities.as_ref(),
                source.collision_world.as_ref(),
                source.navmesh_world.as_ref(),
                source.ground_vehicles.as_ref(),
                source.character_damage_rules.as_ref(),
                source.acquisition_gameplay.as_ref(),
                source.domination_round.as_ref(),
                source.domination_gameplay.as_ref(),
                source.bunker_placement.as_ref(),
                source.bunker_structure.as_ref(),
                source.bunker_allocation.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                absolute(path)?;
            }
            let mut spawns = spawn_catalog::SpawnCatalog::load(&source.spawn_catalog)?;
            if source.acquisition_gameplay.is_some() && source.domination_gameplay.is_some() {
                return Err(io::Error::other("conflicting_match_gameplay_configuration"));
            }
            let policy = spawns.match_profile();
            if !matches!(policy.game_type, 0..=2)
                || !policy.auto_assign_teams
                || (policy.has_bunkers && policy.game_type != 2)
                || (policy.game_type == 2
                    && (policy.squad_capacity > 4 || spawns.map_pool().squads.len() != 32))
                || policy.attacking_faction == policy.defending_faction
                || spawns.map_pool().owner_faction != Some(policy.defending_faction)
            {
                return Err(io::Error::other("unsupported_match_profile_policy"));
            }
            let bunkers = match (
                policy.has_bunkers,
                source.bunker_placement.as_ref(),
                source.bunker_structure.as_ref(),
                source.bunker_allocation.as_ref(),
            ) {
                (true, Some(placement), Some(structure), Some(allocation)) => {
                    Some(structures::BunkerWorld::load_domination(
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
                || policy.team_capacity * 2 > MAX_MATCH_PLAYERS
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
                    let mesh =
                        navmesh::NavMesh::load(artifact, world.source_sha256(), spawns.map_name())
                            .map_err(io::Error::other)?;
                    mesh.validate_coverage(world).map_err(io::Error::other)?;
                    Ok::<_, io::Error>(Arc::new(mesh))
                })
                .transpose()?;
            let vehicles = source
                .ground_vehicles
                .as_ref()
                .map(|p| ground_vehicles::Catalog::load(p, &spawns))
                .transpose()?;
            let acquisition = if let Some(path) = &source.acquisition_gameplay {
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
                let setup = acquisition_gameplay_config::Configuration::load(path)?
                    .build(&spawns, catalog, &runtime)?;
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
                Some(Arc::new(setup))
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
                None
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
            let domination = if let Some(path) = &source.domination_gameplay {
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
                Some(
                    domination_gameplay_config::Configuration::load(path)?.build_for(
                        objectives,
                        &mut spawns,
                        bunkers,
                        vehicles.as_ref().unwrap(),
                    )?,
                )
            } else {
                None
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
                },
            );
        }
        Ok(Self { profiles })
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

    pub fn summaries(&self) -> Vec<Summary> {
        self.profiles.values().map(|p| p.summary.clone()).collect()
    }
}
fn absolute(path: &Path) -> io::Result<()> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(io::Error::other("match_profiles_require_absolute_paths"))
    }
}
impl<W: Write> Engine<W> {
    pub fn with_match_profiles(mut self, registry: Registry) -> Result<Self, &'static str> {
        if self.runtime_application_id != Some(20231)
            || self.active.is_some()
            || !self.peers.is_empty()
            || self.match_profiles.is_some()
            || self.active_abilities.is_some()
            || self.acquisition_gameplay.is_some()
            || self.domination_round.is_some()
            || self.domination_gameplay.is_some()
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
        if registry.has_domination_gameplay() && self.dynamic_body_resources.is_none() {
            return Err("domination_profiles_require_dynamic_bodies");
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
        if let (Some(resources), Some(vehicles)) = (&self.dynamic_body_resources, &loaded.vehicles)
        {
            resources.validate_vehicles(vehicles)?;
        }
        // Allocate everything before the durable acknowledgement. A failed record
        // must leave the previously selected world, identity and outboxes intact.
        let selected = SelectedMatch {
            formation_id,
            profile_id,
            game_type: loaded.summary.game_type,
            attacking_faction: loaded.summary.attacking_faction,
            defending_faction: loaded.summary.defending_faction,
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
            json!({"op":"match_profile_selected","selected_match":selected}),
            wall,
        )?;
        if rollover {
            self.archive_terminal_cohort();
        }
        self.retire_match_selection();
        self.reset_objective_world();
        self.bot_registry = Default::default();
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
        self.minimum_players = 2;
        self.minimum_players_per_team = [1; 2];
        self.maximum_players = loaded.summary.maximum_players;
        self.match_started = self.starts_without_quorum();
        self.selected_match = Some(selected);
        Ok(Vec::new())
    }
}

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
                acquisition: Some(sample.acquisition_gameplay.unwrap().setup),
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
        spawn["match_profile"]["squad_capacity"] = json!(5);
        files.write("domination-spawns.json", spawn.clone());
        assert_eq!(
            Registry::load(&manifest).err().unwrap().to_string(),
            "unsupported_match_profile_policy"
        );

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
}
