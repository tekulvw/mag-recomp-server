//! Explicit gameplay dependencies and development-only disk loading.
use super::*;
use std::sync::Arc;

/// Loaded dependencies for engine construction. Loading and validation happen
/// before supplying this value; constructing the engine performs no file I/O.
pub struct EngineResources {
    wire_profiles: wire_profile::WireProfileCatalog,
    item_catalog: Arc<item_catalog::ItemCatalog>,
    spawn_catalog: spawn_catalog::SpawnCatalog,
    bunker_world: structures::BunkerWorld,
    bunker_exit_policy: Option<vehicle_exit::BunkerExitPolicy>,
}
impl EngineResources {
    pub fn new(
        item_catalog: Arc<item_catalog::ItemCatalog>,
        spawn_catalog: spawn_catalog::SpawnCatalog,
        bunker_world: structures::BunkerWorld,
    ) -> Self {
        Self {
            wire_profiles: wire_profile::WireProfileCatalog::embedded(),
            item_catalog,
            spawn_catalog,
            bunker_world,
            bunker_exit_policy: None,
        }
    }
    /// Operator-owned transform and model offsets; absent policy denies bunker seats.
    pub fn with_bunker_exit_policy(
        mut self,
        policy: vehicle_exit::BunkerExitPolicy,
    ) -> Result<Self, &'static str> {
        policy.validate()?;
        self.bunker_exit_policy = Some(policy);
        Ok(self)
    }

    pub fn with_wire_profiles(mut self, catalog: wire_profile::WireProfileCatalog) -> Self {
        self.wire_profiles = catalog;
        self
    }

    /// Load the crate's authored development defaults. Errors are returned,
    /// never converted into missing gameplay features or origin coordinates.
    pub fn development_defaults() -> io::Result<Self> {
        let items = Arc::clone(item_catalog::bundled().map_err(io::Error::other)?);
        Self::development_defaults_with_items(items)
    }

    pub fn development_defaults_with_items(
        item_catalog: Arc<item_catalog::ItemCatalog>,
    ) -> io::Result<Self> {
        let spawn_catalog = spawn_catalog::SpawnCatalog::load(spawn_catalog::default_path())?;
        let bunker_world = structures::BunkerWorld::load_default()?;
        Ok(Self::new(item_catalog, spawn_catalog, bunker_world))
    }
}

impl<W: Write> Engine<W> {
    /// Development convenience constructor. Production startup should load
    /// resources explicitly and use `from_resources`.
    ///
    /// # Panics
    /// Panics if required authored development resources cannot be loaded.
    /// Use `try_new` for fallible development initialization.
    pub fn new(writer: W, max_log: u64) -> Self {
        Self::try_new(writer, max_log)
            .expect("authored gameplay development resources are required")
    }

    pub fn try_new(writer: W, max_log: u64) -> io::Result<Self> {
        Ok(Self::from_resources(
            writer,
            max_log,
            EngineResources::development_defaults()?,
        ))
    }

    /// Development defaults with an explicit item catalog.
    ///
    /// # Panics
    /// Panics if required development spawn/bunker resources cannot be loaded.
    /// For fallible loading use `EngineResources::development_defaults_with_items`.
    pub fn new_with_item_catalog(
        writer: W,
        max_log: u64,
        item_catalog: Arc<item_catalog::ItemCatalog>,
    ) -> Self {
        let resources = EngineResources::development_defaults_with_items(item_catalog)
            .expect("authored gameplay development spawn/bunker resources are required");
        Self::from_resources(writer, max_log, resources)
    }

    pub fn from_resources(writer: W, max_log: u64, resources: EngineResources) -> Self {
        Self {
            wire_profile: resources
                .wire_profiles
                .profile(20230)
                .expect("validated prototype profile"),
            wire_profiles: resources.wire_profiles,
            writer,
            item_catalog: resources.item_catalog,
            skill_catalog: None,
            leadership: None,
            fall_damage_rules: None,
            pending_rewards: Default::default(),
            reward_order: Default::default(),
            committed_rewards: Default::default(),
            committed_reward_order: Default::default(),
            active: None,
            peers: std::collections::BTreeMap::new(),
            minimum_players: 1,
            maximum_players: 1,
            match_started: true,
            last_wall_secs: 0,
            world_epoch: None,
            bot_registry: Default::default(),
            world_tick_enabled: false,
            last_world_tick: None,
            match_epoch: None,
            alpha_turret: turret::ClientTurretFields::default(),
            alpha_turret_reporter: None,
            team_waves: [None; 2],
            shared_drain_cursor: 0,
            last_published_rosters: Default::default(),
            sequence: 0,
            written: 0,
            max_log,
            failed: false,
            development_control: false,
            development_grenade_radius: false,
            development_dummy: false,
            development_dummy_position: resources.spawn_catalog.alternate_position(),
            collision_world: None,
            navmesh_world: None,
            spawn_catalog: Some(resources.spawn_catalog),
            objective_catalog: None,
            objective_world: None,
            score_clock: score_clock::Clock::default(),
            round: Default::default(),
            round_number: 0,
            reward_match_id: None,
            round_participants: Default::default(),
            match_terminal: None,
            released_reservations: Default::default(),
            release_order: Default::default(),
            bunker_world: Some(resources.bunker_world),
            bunker_exit_policy: resources.bunker_exit_policy,
            pending_bunker_damage: Default::default(),
            next_repair: Default::default(),
        }
    }
}
