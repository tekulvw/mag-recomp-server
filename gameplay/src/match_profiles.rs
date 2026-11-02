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
    collision_world: Option<PathBuf>,
    navmesh_world: Option<PathBuf>,
    ground_vehicles: Option<PathBuf>,
    character_damage_rules: Option<PathBuf>,
}
#[derive(Clone)]
struct Loaded {
    summary: Summary,
    spawns: spawn_catalog::SpawnCatalog,
    objectives: Option<objective_catalog::ObjectiveCatalog>,
    collision: Option<collision::CollisionWorld>,
    navmesh: Option<Arc<navmesh::NavMesh>>,
    vehicles: Option<ground_vehicles::Catalog>,
    damage: Option<character_damage_rules::Rules>,
}
pub struct Registry {
    profiles: BTreeMap<String, Loaded>,
}
impl Registry {
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
                source.collision_world.as_ref(),
                source.navmesh_world.as_ref(),
                source.ground_vehicles.as_ref(),
                source.character_damage_rules.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                absolute(path)?;
            }
            let spawns = spawn_catalog::SpawnCatalog::load(&source.spawn_catalog)?;
            let policy = spawns.match_profile();
            if !matches!(policy.game_type, 0 | 1)
                || !policy.auto_assign_teams
                || policy.has_bunkers
                || policy.attacking_faction == policy.defending_faction
                || spawns.map_pool().owner_faction != Some(policy.defending_faction)
            {
                return Err(io::Error::other("unsupported_match_profile_policy"));
            }
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
            let damage = source
                .character_damage_rules
                .as_ref()
                .map(character_damage_rules::Rules::load)
                .transpose()?;
            if let Some(rules) = &damage {
                rules
                    .validate_match(20231, &spawns)
                    .map_err(io::Error::other)?;
            }
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
                    collision,
                    navmesh,
                    vehicles,
                    damage,
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
        {
            return Err("match_profiles_require_idle_explicit_retail_engine");
        }
        if self.bot_registry.config.is_some()
            || self.bunker_world.is_some()
            || self.bunker_exit_policy.is_some()
            || self.leadership.is_some()
            || self.development_dummy
        {
            return Err("match_profiles_unsupported_world_extras");
        }
        registry.validate_runtime(&self.item_catalog, self.wire_profile)?;
        self.match_profiles = Some(registry);
        Ok(self)
    }
    pub(crate) fn retire_match_selection(&mut self) {
        if let Some(selected) = self.selected_match.take() {
            self.retired_formations.insert(selected.formation_id);
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
        if self.active.is_some()
            || !self.peers.is_empty()
            || !self.pending_rewards.is_empty()
            || !self.pending_loadout_selections().is_empty()
        {
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
        self.record(
            json!({"op":"match_profile_selected","selected_match":selected}),
            wall,
        )?;
        self.retire_match_selection();
        self.reset_objective_world();
        self.bot_registry = Default::default();
        self.spawn_catalog = Some(loaded.spawns);
        self.objective_catalog = loaded.objectives;
        self.objective_world = world;
        self.collision_world = loaded.collision;
        self.navmesh_world = loaded.navmesh;
        self.ground_vehicles = loaded.vehicles;
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
    fn engine<W: Write>(files: &Files, writer: W, maximum_log: u64) -> Engine<W> {
        let resources = EngineResources::for_application(
            crate::resources::tests::retail_items(),
            spawn_catalog::SpawnCatalog::load(files.spawns(0, 0, 1)).unwrap(),
            None,
            crate::resources::tests::wire_profiles(),
            20231,
        )
        .unwrap();
        Engine::from_resources(writer, maximum_log, resources)
            .with_match_profiles(files.registry())
            .unwrap()
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
    fn pending_selection_survives_actor_release_and_blocks_switch_until_settled() {
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
        assert_eq!(
            select(&mut engine, "formation-2", "p-1-0-2")
                .error
                .as_deref(),
            Some("match_profile_selection_locked")
        );
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
    fn pending_rewards_block_idle_switch_without_erasing_outbox() {
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
        };
        engine.reward_order.push_back(reward.reward_id.clone());
        engine
            .pending_rewards
            .insert(reward.reward_id.clone(), reward.clone());
        assert_eq!(
            select(&mut engine, "formation-2", "p-1-0-2")
                .error
                .as_deref(),
            Some("match_profile_selection_locked")
        );
        assert_eq!(engine.pending_reward_batch(), [reward]);
        assert_eq!(
            engine.selected_match.as_ref().unwrap().formation_id,
            "formation-1"
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
        assert_eq!(
            select(&mut engine, "formation-2", "p-1-0-2")
                .error
                .as_deref(),
            Some("match_profile_selection_locked")
        );
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
