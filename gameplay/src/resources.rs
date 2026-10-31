//! Explicit gameplay dependencies and development-only disk loading.
use super::*;
use std::sync::Arc;

/// Loaded dependencies for engine construction. Loading and validation happen
/// before supplying this value; constructing the engine performs no file I/O.
pub struct EngineResources {
    bot_names: bot_names::Catalog,
    wire_profiles: wire_profile::WireProfileCatalog,
    initial_wire_profile: wire_profile::WireProfile,
    runtime_application_id: Option<u32>,
    item_catalog: Arc<item_catalog::ItemCatalog>,
    spawn_catalog: spawn_catalog::SpawnCatalog,
    bunker_world: Option<structures::BunkerWorld>,
    bunker_exit_policy: Option<vehicle_exit::BunkerExitPolicy>,
}
impl EngineResources {
    pub fn with_bot_names(mut self, names: bot_names::Catalog) -> Self {
        self.bot_names = names;
        self
    }
    pub fn new(
        item_catalog: Arc<item_catalog::ItemCatalog>,
        spawn_catalog: spawn_catalog::SpawnCatalog,
        bunker_world: structures::BunkerWorld,
    ) -> Self {
        let wire_profiles = wire_profile::WireProfileCatalog::embedded();
        Self {
            initial_wire_profile: wire_profiles
                .profile(20230)
                .expect("embedded prototype profile"),
            wire_profiles,
            bot_names: Default::default(),
            runtime_application_id: None,
            item_catalog,
            spawn_catalog,
            bunker_world: Some(bunker_world),
            bunker_exit_policy: None,
        }
    }

    /// Production resources for one application. All catalogs are supplied by
    /// the caller; this constructor performs no disk I/O or fallback loading.
    /// A disabled bunker policy needs no structure or placement catalog.
    pub fn for_application(
        item_catalog: Arc<item_catalog::ItemCatalog>,
        spawn_catalog: spawn_catalog::SpawnCatalog,
        bunker_world: Option<structures::BunkerWorld>,
        wire_profiles: wire_profile::WireProfileCatalog,
        application_id: u32,
    ) -> Result<Self, &'static str> {
        let initial_wire_profile = wire_profiles.profile(application_id)?;
        if item_catalog.application_id() != application_id {
            return Err("item_catalog_application_mismatch");
        }
        if spawn_catalog.match_profile().has_bunkers && bunker_world.is_none() {
            return Err("bunker_world_required");
        }
        if !spawn_catalog.match_profile().has_bunkers {
            if bunker_world.is_some() {
                return Err("bunker_world_disabled_by_match_policy");
            }
            if spawn_catalog.map_pool().locations.iter().any(|point| {
                point.active && matches!(point.kind, spawn_catalog::SpawnKind::BunkerPassenger)
            }) {
                return Err("bunker_spawn_requires_enabled_bunkers");
            }
        }
        Ok(Self {
            wire_profiles,
            bot_names: Default::default(),
            initial_wire_profile,
            runtime_application_id: Some(application_id),
            item_catalog,
            spawn_catalog,
            bunker_world,
            bunker_exit_policy: None,
        })
    }
    /// Operator-owned transform and model offsets; absent policy denies bunker seats.
    pub fn with_bunker_exit_policy(
        mut self,
        policy: vehicle_exit::BunkerExitPolicy,
    ) -> Result<Self, &'static str> {
        if self.bunker_world.is_none() {
            return Err("bunker_exit_policy_requires_bunkers");
        }
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
            wire_profile: resources.initial_wire_profile,
            runtime_application_id: resources.runtime_application_id,
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
            bot_names: resources.bot_names,
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
            bunker_world: resources.bunker_world,
            bunker_exit_policy: resources.bunker_exit_policy,
            pending_bunker_damage: Default::default(),
            next_repair: Default::default(),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn retail_items() -> Arc<item_catalog::ItemCatalog> {
        Arc::new(
            item_catalog::ItemCatalog::from_value_for_application(
                json!({"schema_version":2,"application_id":20231,
                "items":{"FactionItemList0:2":{"authored_ref":"FactionItemList0:2",
                    "faction_list":"FactionItemList0","local_ordinal":2,"wire_item_id":null,
                    "name":"synthetic_tool","item_type":"kActiveItem",
                    "definition":{"group":"active_items","name":"synthetic_tool"}}},
                "definitions":{"active_items":{"synthetic_tool":{"parameters":{}}}},
                "resources":{}}),
                blast_rules::CharacterBlastRules {
                    armor_rating_reduction: 0.25,
                    armor_prevention_reduction: 0.75,
                    hit_location: 3,
                    damage_type: 10,
                },
                20231,
            )
            .unwrap(),
        )
    }

    fn spawns(has_bunkers: bool) -> spawn_catalog::SpawnCatalog {
        spawns_with_kind(has_bunkers, "on_foot")
    }

    fn spawns_with_kind(has_bunkers: bool, kind: &str) -> spawn_catalog::SpawnCatalog {
        serde_json::from_value(json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC resource test",
            "active":{"map_name":"synthetic_arena","squad_index":0,"default_spawn":0,"available":[0]},
            "match_profile":{"has_bunkers":has_bunkers,"has_objectives":false},
            "maps":[{"name":"synthetic_arena","spawn_count":1,"locations":[
                {"index":0,"token":1,"kind":kind,"position":{"x":10.,"y":2.,"z":12.}}
            ]}]
        })).unwrap()
    }

    pub(crate) fn wire_profiles() -> wire_profile::WireProfileCatalog {
        wire_profile::WireProfileCatalog::from_json(include_str!(
            "../data/synthetic-wire-profiles.json"
        ))
        .unwrap()
    }

    fn retail_items_with_armory() -> Arc<item_catalog::ItemCatalog> {
        let mut raw = retail_items().raw().clone();
        raw["items"] = json!({
            "FactionItemList0:2":{"authored_ref":"FactionItemList0:2","faction_list":"FactionItemList0",
                "local_ordinal":2,"wire_item_id":null,"name":"synthetic_weapon","item_type":"kWeapon",
                "definition":{"group":"weapons","name":"synthetic_weapon"}},
            "FactionItemList0:3":{"authored_ref":"FactionItemList0:3","faction_list":"FactionItemList0",
                "local_ordinal":3,"wire_item_id":null,"name":"synthetic_unresolved_uniform",
                "item_type":"kUniform","definition":null}
        });
        raw["definitions"] =
            json!({"weapons":{"synthetic_weapon":{"parameters":{"magazine_capacity":7}}}});
        let catalog = item_catalog::ItemCatalog::from_value_for_application(
            raw,
            retail_items().character_blast_rules(),
            20231,
        )
        .unwrap();
        let armory = retail_armory::RetailArmory::from_json(&json!({
            "schema_version":1,"application_id":20231,"source_sha256":{"synthetic":"a".repeat(64)},
            "items":[
                {"wire_id":2,"name":"synthetic_weapon","authored_ref":"FactionItemList0:2",
                    "available_bit":2,"factions":[0,1,2],"primary_ammo":7},
                {"wire_id":3,"name":"synthetic_unresolved_uniform","authored_ref":"FactionItemList0:3",
                    "available_bit":3,"factions":[0,1,2],"primary_ammo":null}
            ],
            "factions":(0..3).map(|faction|json!({"faction":faction,"available_item_bits":[2,3],"skill_ids":[],
                "loadouts":(0..5).map(|_|json!({"update_id":1,"items":vec![None::<u16>;22]})).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        }).to_string()).unwrap();
        Arc::new(catalog.with_retail_armory(armory))
    }

    fn saved_equipment(unresolved_inventory: Option<usize>) -> loadouts::SavedLoadouts {
        let mut saved = loadouts::SavedLoadouts {
            selected_index: 0,
            update_ids: [1; 5],
            inventories: [[None; 22]; 5],
        };
        saved.inventories[0][0] = Some(2);
        if let Some(index) = unresolved_inventory {
            saved.inventories[index][0] = Some(2);
            saved.inventories[index][21] = Some(3);
        }
        saved
    }

    fn saved_character(saved: loadouts::SavedLoadouts) -> Character {
        serde_json::from_value(json!({
            "name":"Synthetic Player","faction":0,"head":0,"voice":0,"universe_id":1,
            "character_id":7,"loadouts":saved,
        }))
        .unwrap()
    }

    fn reserve_saved_character(engine: &mut Engine<Vec<u8>>, character: Character) -> Response {
        engine.handle(
            Request::Reserve {
                v: BRIDGE_VERSION,
                request_id: "synthetic-saved-reserve".into(),
                reservation_id: "synthetic-reservation-0001".into(),
                world_id: 1,
                player_id: 0,
                account_id: 3,
                session_key: "aaaaaaaaaaaaaaaa".into(),
                access_key: "bbbbbbbbbbbbbbbb".into(),
                character,
            },
            Duration::ZERO,
            1,
        )
    }

    #[test]
    fn retail_reserve_rejects_unresolved_raw_items_even_when_armory_accepts_every_inventory() {
        for inventory in 0..5 {
            let items = retail_items_with_armory();
            let saved = saved_equipment(Some(inventory));
            assert_eq!(items.validate_item(3), Err("unresolved_saved_item"));
            assert_eq!(
                items.retail_armory().unwrap().validate_saved(0, &saved),
                Ok(())
            );
            let resources = EngineResources::for_application(
                items,
                spawns(false),
                None,
                wire_profiles(),
                20231,
            )
            .unwrap();
            let mut engine = Engine::from_resources(Vec::new(), 100_000, resources);
            let response = reserve_saved_character(&mut engine, saved_character(saved));
            assert_eq!(
                response.error.as_deref(),
                Some("unresolved_saved_item"),
                "inventory {inventory}"
            );
            assert!(!response.ok);
            assert!(engine.active.is_none());
        }
    }

    #[test]
    fn retail_replication_rejects_unresolved_raw_items_in_unselected_inventories() {
        let items = retail_items_with_armory();
        for inventory in 0..5 {
            let character = saved_character(saved_equipment(Some(inventory)));
            assert!(
                character
                    .replication_loadout(&items, 0, wire_profiles().profile(20231).unwrap())
                    .is_none(),
                "inventory {inventory}"
            );
        }
    }

    #[test]
    fn retail_resolved_saved_equipment_still_reserves_and_replicates() {
        let items = retail_items_with_armory();
        let character = saved_character(saved_equipment(None));
        let kit = character
            .replication_loadout(&items, 0, wire_profiles().profile(20231).unwrap())
            .unwrap();
        assert_eq!(kit.inventories[0].slots[0], Some(2));
        let resources =
            EngineResources::for_application(items, spawns(false), None, wire_profiles(), 20231)
                .unwrap();
        let mut engine = Engine::from_resources(Vec::new(), 100_000, resources);
        let response = reserve_saved_character(&mut engine, character);
        assert!(response.ok, "{:?}", response.error);
        assert!(engine.active.is_some());
    }

    #[test]
    fn explicit_runtime_starts_retail_and_rejects_prototype_reconfiguration() {
        let resources = EngineResources::for_application(
            retail_items(),
            spawns(false),
            None,
            wire_profiles(),
            20231,
        )
        .unwrap();
        let mut engine = Engine::from_resources(Vec::new(), 100_000, resources);
        assert_eq!(engine.wire_profile.application_id(), 20231);
        assert!(engine.bunker_world.is_none());
        let response = engine.handle(
            Request::ConfigureMatch {
                v: BRIDGE_VERSION,
                request_id: "synthetic-configure".into(),
                minimum_players: 2,
                maximum_players: 2,
                application_id: 20230,
            },
            Duration::ZERO,
            1,
        );
        assert!(!response.ok);
        assert_eq!(
            response.error.as_deref(),
            Some("runtime_application_mismatch")
        );
        assert_eq!(engine.wire_profile.application_id(), 20231);
        assert_eq!(engine.minimum_players, 1);
        assert_eq!(engine.maximum_players, 1);
    }

    #[test]
    fn explicit_runtime_requires_enabled_bunker_resources() {
        let result = EngineResources::for_application(
            retail_items(),
            spawns(true),
            None,
            wire_profiles(),
            20231,
        );
        assert_eq!(result.err(), Some("bunker_world_required"));
    }

    #[test]
    fn explicit_runtime_rejects_item_application_mismatch() {
        let result = EngineResources::for_application(
            retail_items(),
            spawns(false),
            None,
            wire_profiles(),
            20230,
        );
        assert_eq!(result.err(), Some("item_catalog_application_mismatch"));
    }

    #[test]
    fn disabled_bunkers_reject_active_passenger_spawns() {
        let result = EngineResources::for_application(
            retail_items(),
            spawns_with_kind(false, "bunker_passenger"),
            None,
            wire_profiles(),
            20231,
        );
        assert_eq!(result.err(), Some("bunker_spawn_requires_enabled_bunkers"));
    }

    #[test]
    fn explicit_runtime_constructs_without_a_prototype_wire_profile() {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../data/synthetic-wire-profiles.json")).unwrap();
        raw["profiles"] = json!([raw["profiles"][1].clone()]);
        let resources = EngineResources::for_application(
            retail_items(),
            spawns(false),
            None,
            wire_profile::WireProfileCatalog::from_json(&raw.to_string()).unwrap(),
            20231,
        )
        .unwrap();
        let engine = Engine::from_resources(Vec::new(), 100_000, resources);
        assert_eq!(engine.wire_profile.application_id(), 20231);
        assert!(engine.wire_profiles.profile(20230).is_err());
    }

    #[test]
    fn explicit_runtime_requires_the_selected_wire_profile() {
        let mut raw: serde_json::Value =
            serde_json::from_str(include_str!("../data/synthetic-wire-profiles.json")).unwrap();
        raw["profiles"] = json!([raw["profiles"][0].clone()]);
        let result = EngineResources::for_application(
            retail_items(),
            spawns(false),
            None,
            wire_profile::WireProfileCatalog::from_json(&raw.to_string()).unwrap(),
            20231,
        );
        assert_eq!(result.err(), Some("unsupported_application_wire_profile"));
    }

    #[test]
    fn disabled_bunkers_still_publish_spawn_setup() {
        let resources = EngineResources::for_application(
            retail_items(),
            spawns(false),
            None,
            wire_profiles(),
            20231,
        )
        .unwrap();
        let mut engine = Engine::from_resources(Vec::new(), 100_000, resources);
        let response = engine.handle(
            Request::Reserve {
                v: BRIDGE_VERSION,
                request_id: "synthetic-reserve".into(),
                reservation_id: "synthetic-reservation-0001".into(),
                world_id: 1,
                player_id: 0,
                account_id: 3,
                session_key: "aaaaaaaaaaaaaaaa".into(),
                access_key: "bbbbbbbbbbbbbbbb".into(),
                character: serde_json::from_value(json!({
                    "name":"Synthetic Player","faction":0,"head":0,"voice":0,"universe_id":1,
                }))
                .unwrap(),
            },
            Duration::ZERO,
            1,
        );
        assert!(response.ok, "{:?}", response.error);
        let active = engine.active.as_mut().unwrap();
        active.ready_for_updates = true;
        active.initial_objects = Some(Vec::new());
        active.outbound_sequence = 1;
        let packet = replication::ObjectUpdate {
            header: replication::UpdateHeader {
                sequence: 1,
                nak_sequence: 0,
                client_index: 0,
                object_type_mask: 0,
                timestamp_ms: 1,
            },
            wire: &[],
        }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
        let response = engine.handle(
            Request::Event {
                v: BRIDGE_VERSION,
                request_id: "synthetic-update".into(),
                reservation_id: "synthetic-reservation-0001".into(),
                world_id: 1,
                player_id: 0,
                transport: Transport::Udp,
                direction: Direction::ClientToServer,
                rt_type: 0x3d,
                plugin_id: Some(1),
                payload_hex: protocol::encode_hex(&packet),
            },
            Duration::from_millis(1),
            1,
        );
        assert!(response.ok, "{:?}", response.error);
        assert_eq!(response.messages.len(), 1);
        let packet = protocol::decode_hex(&response.messages[0].payload_hex);
        let update =
            replication::ObjectUpdate::decode_with_profile(&packet, engine.wire_profile).unwrap();
        assert_eq!(
            update.header.object_type_mask,
            (1 << 3) | (1 << 7) | (1 << 8)
        );
        assert!(
            engine
                .active
                .as_ref()
                .unwrap()
                .emplacement_creation
                .is_some()
        );
    }
}
