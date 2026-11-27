//! SYNTHETIC fixtures composed from schema constraints. No captured map data.
use super::*;
use crate::bot_registry::BotPopulation;
use crate::spawn_catalog::{ForwardSpawnBinding, SpawnCatalog};
use std::time::Duration;

const SQUAD: u16 = 0;
const HOME: u16 = 0;
const ALTERNATE: u16 = 1;
const DROP: u16 = 2;
const VEHICLE: u16 = 3;
const CAPTURED: u16 = 4;
const UNROUTABLE: u16 = 5;

fn spawns() -> SpawnCatalog {
    serde_json::from_value(spawns_value()).unwrap()
}

/// The same pool with point 10 as squad 16's hospital (bunker passenger)
/// point, installed after construction like a Domination world's catalog.
fn spawns_with_hospital() -> SpawnCatalog {
    let mut value = spawns_value();
    value["match_profile"]["has_bunkers"] = json!(true);
    value["active"]["available"] = json!([6, 7, 8, 9, 10]);
    let map = &mut value["maps"][0];
    map["locations"][10]["kind"] = json!("bunker_passenger");
    map["locations"][10]["active"] = json!(true);
    map["squads"][1]["available"] = json!([6, 7, 8, 9, 10]);
    serde_json::from_value(value).unwrap()
}

fn spawns_value() -> Value {
    json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC bot spawn selection",
        "active":{"map_name":"synthetic_arena","squad_index":16,"default_spawn":6,
            "available":[6,7,8,9]},
        "match_profile":{"has_bunkers":false,"has_objectives":false},
        "maps":[{"name":"synthetic_arena","owner_faction":2,"spawn_count":12,
            "runtime_indices_verified":true,"runtime_indices_basis":"SYNTHETIC pool identity",
            "locations":[
                {"index":0,"token":100,"kind":"insertion","position":{"x":10.,"y":0.,"z":10.}},
                {"index":1,"token":101,"kind":"alternate","position":{"x":14.,"y":0.,"z":10.}},
                {"index":2,"token":102,"kind":"parachute","active":false,
                    "position":{"x":20.,"y":40.,"z":30.},
                    "parachute":{"forward":[1.,0.,0.],"spacing_m":2.,"yaw":256}},
                {"index":3,"token":103,"kind":"vehicle_passenger","position":{"x":18.,"y":0.,"z":10.}},
                {"index":4,"token":104,"kind":"on_foot","active":false,"position":{"x":30.,"y":0.,"z":10.}},
                {"index":5,"token":105,"kind":"on_foot","active":false,"position":{"x":55.,"y":0.,"z":55.}},
                {"index":6,"token":106,"kind":"insertion","position":{"x":10.,"y":0.,"z":50.}},
                {"index":7,"token":107,"kind":"alternate","position":{"x":40.,"y":0.,"z":46.}},
                {"index":8,"token":108,"kind":"alternate","position":{"x":40.,"y":0.,"z":54.}},
                {"index":9,"token":109,"kind":"alternate","position":{"x":56.,"y":0.,"z":50.}},
                {"index":10,"token":110,"kind":"on_foot","active":false,"position":{"x":30.,"y":0.,"z":30.}},
                {"index":11,"token":111,"kind":"unsupported","position":{"x":30.,"y":0.,"z":40.}}],
            "squads":[
                {"index":0,"team":0,"platoon":0,"squad":0,"faction":null,"available":[0,1,3,4,5],"default_spawn":0},
                {"index":16,"team":1,"platoon":0,"squad":0,"faction":2,
                    "available":[6,7,8,9],"default_spawn":6}]}]})
}

fn geometry_hash(vertices: &[[f64; 3]], triangles: &[[u32; 3]]) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"MAG-NAV-GEOMETRY-v1\0");
    hash.update((vertices.len() as u32).to_le_bytes());
    hash.update((triangles.len() as u32).to_le_bytes());
    for x in vertices.iter().flatten() {
        hash.update((*x as f32).to_le_bytes());
    }
    for index in triangles.iter().flatten() {
        hash.update(index.to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}

/// A 60 m floor; the walking navmesh covers only x <= 48, so points 5 and 9
/// have collision support but no route to or from the walking network.
fn engine() -> Engine<Vec<u8>> {
    let items = crate::bot_registry::identity_tests::bootstrap_items();
    let names = crate::bot_names::Catalog::from_json(
        &serde_json::to_vec(
            &json!({"entries":[{"name":"Synthetic Alpha"},{"name":"Synthetic Bravo"}]}),
        )
        .unwrap(),
    )
    .unwrap();
    let resources = EngineResources::for_application(
        std::sync::Arc::new(items),
        spawns(),
        None,
        crate::resources::tests::wire_profiles(),
        20231,
    )
    .unwrap()
    .with_bot_names(names);
    let mut engine = Engine::from_resources(Vec::new(), 100_000, resources);
    engine.maximum_players = 2;
    let floor = [[0., 0., 0.], [60., 0., 0.], [60., 0., 60.], [0., 0., 60.]];
    let world = collision::CollisionWorld::from_bytes(
        &serde_json::to_vec(&json!({"format":"mag_collision_world","schema_version":1,
            "coordinate_space":"world",
            "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
                "provenance":["Synthetic spawn selection floor"],"unknowns":["Not a game map"],
                "physics_regions":[{"min":[-1.,-5.,-1.],"max":[61.,60.,61.],"ground_verified":true,
                    "obstacles_verified":true,"provenance":["Synthetic floor"]}]},
            "instances":[{"id":"floor","vertices_xyz":floor,"triangles":[[0,2,1],[0,3,2]]}]}))
        .unwrap(),
        collision::CollisionLimits::default(),
    )
    .unwrap();
    let walk = vec![[0., 0., 0.], [48., 0., 0.], [48., 0., 60.], [0., 0., 60.]];
    let triangles = vec![[0, 2, 1], [0, 3, 2]];
    let artifact = navmesh::bake(navmesh::BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: "synthetic_arena".into(),
        world_sha256: world.source_sha256().into(),
        geometry_sha256: geometry_hash(&walk, &triangles),
        bounds: navmesh::Bounds {
            min: [-1., -1., -1.],
            max: [49., 4., 61.],
        },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices: walk.iter().map(|p| p.map(|x| x as f32)).collect(),
        triangles,
    })
    .unwrap();
    let mesh = navmesh::NavMesh::load(artifact, world.source_sha256(), "synthetic_arena").unwrap();
    engine = engine.with_collision_world(world).with_navmesh_world(mesh);
    let config: BotPopulation = serde_json::from_value(json!({"format":"mag_bot_population_v1",
        "map_name":"synthetic_arena","firearms":{"enabled":false},"slots":[
            {"team":0,"squad":0,"source_spawn":0,"position":{"x":10.,"y":0.,"z":10.}},
            {"team":0,"squad":0,"source_spawn":0,"position":{"x":11.,"y":0.,"z":10.}},
            {"team":1,"squad":16,"source_spawn":6,"position":{"x":10.,"y":0.,"z":50.}},
            {"team":1,"squad":16,"source_spawn":6,"position":{"x":11.,"y":0.,"z":50.}}]}))
    .unwrap();
    engine
        .with_bot_population(config, 2)
        .unwrap_or_else(|reason| panic!("{reason}"))
}

fn choose(
    engine: &mut Engine<Vec<u8>>,
    actors: &crate::actor_collision::ActorGrid,
) -> (LifeSpawnChoice, Vec<Value>) {
    let mut receipts = Vec::new();
    let choice = engine
        .choose_bot_life_spawn(200, SQUAD, 0, actors, &mut receipts)
        .unwrap();
    (choice, receipts)
}

fn spawned(choice: LifeSpawnChoice) -> LifeSpawn {
    match choice {
        LifeSpawnChoice::Spawn(spawn) => spawn,
        LifeSpawnChoice::Wait => panic!("expected a spawn"),
    }
}

fn activate_forward(engine: &mut Engine<Vec<u8>>) {
    engine
        .spawn_catalog
        .as_mut()
        .unwrap()
        .activate_forward(ForwardSpawnBinding {
            objective_index: 20,
            squad_index: SQUAD,
            spawn_point_index: DROP,
        })
        .unwrap();
}

fn set_runtime_default(engine: &mut Engine<Vec<u8>>, index: u16) {
    let catalog = engine.spawn_catalog.as_mut().unwrap();
    let token = catalog.point(index).unwrap().token;
    catalog
        .set_runtime_point_active(index, token, true)
        .unwrap();
    catalog.set_runtime_squad_default(SQUAD, index).unwrap();
}

fn occupy(
    actors: &mut crate::actor_collision::ActorGrid,
    id: u16,
    at: replication::CharacterPosition,
) {
    actors
        .insert(
            id,
            crate::actor_collision::Bounds::body(point(at), bot_physics::Body::default()),
        )
        .unwrap();
}

#[test]
fn authored_default_keeps_the_population_slot() {
    let mut engine = engine();
    assert_eq!(engine.bot_spawn_order(SQUAD, HOME), vec![HOME, ALTERNATE]);
    let (choice, receipts) = choose(&mut engine, &Default::default());
    let spawn = spawned(choice);
    assert_eq!(spawn.arrival, Arrival::PopulationSlot);
    assert_eq!(spawn.spawn_point, Some(HOME));
    assert_eq!(point(spawn.position), [10., 0., 10.]);
    assert!(receipts.is_empty());
}

#[test]
fn forward_activation_moves_bot_lives_to_the_landed_drop_line() {
    let mut engine = engine();
    activate_forward(&mut engine);
    assert_eq!(
        engine.bot_spawn_order(SQUAD, HOME),
        vec![DROP, HOME, ALTERNATE]
    );
    let mut actors = crate::actor_collision::ActorGrid::default();
    let (choice, receipts) = choose(&mut engine, &actors);
    let first = spawned(choice);
    assert_eq!(first.spawn_point, Some(DROP));
    assert_eq!(first.arrival, Arrival::ParachuteLanding);
    // Landed on the floor below the first authored drop position.
    assert!((first.position.x - 20.).abs() < 0.01 && (first.position.z - 30.).abs() < 0.01);
    assert!(first.position.y.abs() < 0.05, "{:?}", first.position);
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0]["op"], "bot_spawn_point_qualified");
    // The next body lands at the next drop-line position.
    occupy(&mut actors, 300, first.position);
    let (choice, receipts) = choose(&mut engine, &actors);
    let second = spawned(choice);
    assert_eq!(second.spawn_point, Some(DROP));
    assert!((second.position.x - 22.).abs() < 0.01 && (second.position.z - 30.).abs() < 0.01);
    assert!(receipts.is_empty(), "qualification is cached");
}

#[test]
fn choice_is_deterministic_for_identical_world_state() {
    let mut a = engine();
    let mut b = engine();
    for engine in [&mut a, &mut b] {
        activate_forward(engine);
    }
    let actors = crate::actor_collision::ActorGrid::default();
    assert_eq!(choose(&mut a, &actors).0, choose(&mut b, &actors).0);
    assert_eq!(choose(&mut a, &actors).0, choose(&mut b, &actors).0);
}

#[test]
fn inactive_and_vehicle_points_are_never_used() {
    let mut engine = engine();
    let actors = crate::actor_collision::ActorGrid::default();
    for index in [DROP, CAPTURED, UNROUTABLE, VEHICLE] {
        assert!(!engine.bot_spawn_order(SQUAD, HOME).contains(&index));
    }
    // A vehicle point as the squad's runtime default is still skipped.
    engine
        .spawn_catalog
        .as_mut()
        .unwrap()
        .set_runtime_squad_default(SQUAD, VEHICLE)
        .unwrap();
    assert!(!engine.bot_spawn_order(SQUAD, HOME).contains(&VEHICLE));
    assert_eq!(
        spawned(choose(&mut engine, &actors).0).spawn_point,
        Some(HOME)
    );
    // A point deactivated at runtime is dropped even as the squad default.
    set_runtime_default(&mut engine, CAPTURED);
    assert_eq!(engine.bot_spawn_order(SQUAD, HOME)[0], CAPTURED);
    let catalog = engine.spawn_catalog.as_mut().unwrap();
    catalog
        .set_runtime_point_active(CAPTURED, 104, false)
        .unwrap();
    assert!(!engine.bot_spawn_order(SQUAD, HOME).contains(&CAPTURED));
}

#[test]
fn runtime_squad_default_change_moves_bots_like_domination_and_acquisition_effects() {
    // Domination FSP and Acquisition spawn effects both stage point activation
    // plus a squad default on the world catalog, as done here.
    let mut engine = engine();
    set_runtime_default(&mut engine, CAPTURED);
    let (choice, _) = choose(&mut engine, &Default::default());
    let spawn = spawned(choice);
    assert_eq!(spawn.spawn_point, Some(CAPTURED));
    assert_eq!(spawn.arrival, Arrival::AuthoredPoint);
    assert!((spawn.position.x - 30.).abs() < 0.01 && (spawn.position.z - 10.).abs() < 0.01);
}

#[test]
fn unroutable_point_is_skipped_with_its_reason_and_the_slot_is_the_fallback() {
    let mut engine = engine();
    set_runtime_default(&mut engine, UNROUTABLE);
    let (choice, receipts) = choose(&mut engine, &Default::default());
    let spawn = spawned(choice);
    assert_eq!(spawn.arrival, Arrival::PopulationSlot);
    assert_eq!(spawn.spawn_point, Some(HOME));
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0]["op"], "bot_spawn_point_unusable");
    assert_eq!(receipts[0]["spawn_point_index"], UNROUTABLE);
    assert_eq!(receipts[0]["reason"], "bot_spawn_navmesh_uncovered");
}

#[test]
fn an_occupied_population_slot_still_waits() {
    let mut engine = engine();
    let mut actors = crate::actor_collision::ActorGrid::default();
    occupy(
        &mut actors,
        300,
        replication::CharacterPosition {
            x: 10.,
            y: 0.,
            z: 10.,
        },
    );
    assert_eq!(choose(&mut engine, &actors).0, LifeSpawnChoice::Wait);
}

#[test]
fn a_full_forward_drop_zone_falls_back_to_the_population_slot() {
    let mut engine = engine();
    activate_forward(&mut engine);
    let mut actors = crate::actor_collision::ActorGrid::default();
    let mut next = 300;
    loop {
        let spawn = spawned(choose(&mut engine, &actors).0);
        if spawn.arrival == Arrival::PopulationSlot {
            assert_eq!(spawn.spawn_point, Some(HOME));
            break;
        }
        assert_eq!(spawn.spawn_point, Some(DROP));
        occupy(&mut actors, next, spawn.position);
        next += 1;
        assert!(next < 400, "the drop zone must be bounded");
    }
}

#[test]
fn a_dead_bot_respawns_at_the_forward_point_after_activation() {
    let mut engine = engine();
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let reserve = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: "synthetic-spawn-selection-reserve".into(),
            reservation_id: "synthetic-spawn-selection-observer".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic Observer",
                "faction":2,"head":0,"voice":0,"universe_id":26,"character_id":1}))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(reserve.ok, "{:?}", reserve.error);
    crate::spawn_progression::ready(engine.active.as_mut().unwrap());
    engine.tick_bots(Duration::from_millis(250), 1).unwrap();
    let (&id, bot) = engine
        .bot_registry
        .bots
        .iter()
        .find(|(_, bot)| bot.target.squad.squad_index() == SQUAD)
        .expect("a squad 0 bot");
    assert_eq!(point(bot.target.position), [10., 0., 10.]);
    let generation = bot.target.runtime.unwrap().generation;
    activate_forward(&mut engine);
    engine
        .commit_bot_damage(
            id,
            generation,
            replication::CharacterDamage {
                health_quarter_points: 0,
                ..bot_registry_initial_damage()
            },
            Duration::from_millis(500),
        )
        .unwrap();
    engine.tick_bots(Duration::from_secs(120), 2).unwrap();
    let bot = &engine.bot_registry.bots[&id];
    let runtime = bot.target.runtime.unwrap();
    assert_eq!(runtime.generation, generation + 1);
    assert!((runtime.position.x - 20.).abs() < 0.01 && (runtime.position.z - 30.).abs() < 0.01);
    let log = String::from_utf8(engine.writer.clone()).unwrap();
    let respawn = log
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|entry| entry["op"] == "bot_respawn")
        .expect("a respawn receipt");
    assert_eq!(respawn["spawn_point_index"], DROP);
    assert_eq!(respawn["arrival"], "parachute_landing");
}

fn bot_registry_initial_damage() -> replication::CharacterDamage {
    replication::CharacterDamage {
        last_location_and_type: 0,
        health_quarter_points: 400,
        conditions: 0,
        last_attacker: u32::MAX,
        last_source_object: u32::MAX,
        last_user_data: 0,
    }
}

// Defending squad 16 (team 1) holds objective 5 at (44, 0, 50). Its population
// slot (point 6) is 34 m away; alternates 7 and 8 sit beside the objective,
// 9 is off the walking network, 10 is a hospital and 11 a field hospital.
const DEFENDERS: u16 = 16;
const DEFENDER_SLOT: usize = 2;
const DEFENDER_HOME: u16 = 6;
const NORTH: u16 = 7;
const SOUTH: u16 = 8;
const OFF_MESH: u16 = 9;
const HOSPITAL: u16 = 10;
const FIELD_HOSPITAL: u16 = 11;

fn defend(engine: &mut Engine<Vec<u8>>) {
    let catalog = crate::objective_catalog::ObjectiveCatalog::from_json(
        &json!({"schema_version":1,"format":"mag_retail_objective_catalog","application_id":20231,
        "protocol":protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":0,"map_name":"synthetic_arena",
        "entity_family":12,"network_type":12,
        "source":{"authored_catalog_sha256":"0".repeat(64),"binding_capture_sha256":"1".repeat(64),
            "startup_proof_sha256":"2".repeat(64),"source_image_sha256":"3".repeat(64),
            "capture_qualification":"bounded_concurrent_token_slot_capture"},
        "objects":[{"token":"0x00000005","object_index":5,"initial_state":1,
            "interaction":{"owner_team":1,"activate_ms":100,"complete_ms":20_000,"deactivate_ms":100,
                "can_deactivate":true,"position":[44.,0.,50.],"trigger_count":1,
                "maximum_distance_m":3.,"distance_policy":"development_point_distance"}},
            // FRAGO targets outside every lane: one near the defenders'
            // population slot, one off the walking network.
            {"token":"0x00000006","object_index":NEAR_HOME_FRAGO,"initial_state":1,
            "interaction":{"owner_team":0,"activate_ms":100,"complete_ms":20_000,"deactivate_ms":100,
                "can_deactivate":true,"position":[4.,0.,58.],"trigger_count":1,
                "maximum_distance_m":3.,"distance_policy":"development_point_distance"}},
            {"token":"0x00000007","object_index":OFF_MESH_FRAGO,"initial_state":1,
            "interaction":{"owner_team":0,"activate_ms":100,"complete_ms":20_000,"deactivate_ms":100,
                "can_deactivate":true,"position":[58.,0.,10.],"trigger_count":1,
                "maximum_distance_m":3.,"distance_policy":"development_point_distance"}}],
        "squads":(0..2u16).flat_map(|team| (0..4u16).map(move |slot| json!({
            "team":team,"squad_index":team*16+slot,"initial_objective_index":5,
            "authored_objectives":[{"object_index":5,"order":1}]}))).collect::<Vec<_>>()})
        .to_string(),
    )
    .unwrap();
    engine.objective_world = Some(crate::objective_control::ObjectiveWorld::new(&catalog));
    engine.objective_catalog = Some(catalog);
}

fn choose_defender(
    engine: &mut Engine<Vec<u8>>,
    actor: u16,
    actors: &crate::actor_collision::ActorGrid,
) -> (LifeSpawn, Vec<Value>) {
    let mut receipts = Vec::new();
    let choice = engine
        .choose_bot_life_spawn(actor, DEFENDERS, DEFENDER_SLOT, actors, &mut receipts)
        .unwrap();
    (spawned(choice), receipts)
}

fn deactivate(engine: &mut Engine<Vec<u8>>, index: u16) {
    let catalog = engine.spawn_catalog.as_mut().unwrap();
    let token = catalog.point(index).unwrap().token;
    catalog
        .set_runtime_point_active(index, token, false)
        .unwrap();
}

fn horizontal(a: replication::CharacterPosition, x: f32, z: f32) -> f32 {
    (a.x - x).hypot(a.z - z)
}

#[test]
fn defenders_spread_over_the_forward_points_nearest_their_objective() {
    let mut engine = engine();
    defend(&mut engine);
    let actors = crate::actor_collision::ActorGrid::default();
    let (first, receipts) = choose_defender(&mut engine, 200, &actors);
    let (second, _) = choose_defender(&mut engine, 201, &actors);
    let used = [first.spawn_point, second.spawn_point];
    assert!(
        used.contains(&Some(NORTH)) && used.contains(&Some(SOUTH)),
        "{used:?}"
    );
    for spawn in [first, second] {
        assert_eq!(spawn.arrival, Arrival::AuthoredPoint);
        assert!(
            horizontal(spawn.position, 44., 50.) < 8.,
            "{:?}",
            spawn.position
        );
    }
    let unusable: Vec<_> = receipts
        .iter()
        .filter(|r| r["op"] == "bot_lane_spawn_unusable")
        .map(|r| r["spawn_point_index"].as_u64().unwrap())
        .collect();
    assert_eq!(unusable, [u64::from(OFF_MESH)]);
    // Identical world state gives identical choices.
    let mut again = engine_with_defence();
    assert_eq!(choose_defender(&mut again, 200, &actors).0, first);
}

fn engine_with_defence() -> Engine<Vec<u8>> {
    let mut engine = engine();
    defend(&mut engine);
    engine
}

#[test]
fn defenders_without_a_routable_forward_point_use_their_population_slot() {
    let mut engine = engine_with_defence();
    deactivate(&mut engine, NORTH);
    deactivate(&mut engine, SOUTH);
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(spawn.arrival, Arrival::PopulationSlot);
    assert_eq!(spawn.spawn_point, Some(DEFENDER_HOME));
    assert_eq!(point(spawn.position), [10., 0., 50.]);
}

#[test]
fn a_full_forward_point_overflows_to_the_next_forward_point() {
    let mut engine = engine_with_defence();
    deactivate(&mut engine, SOUTH);
    let mut actors = crate::actor_collision::ActorGrid::default();
    let mut next = 300;
    loop {
        let (spawn, _) = choose_defender(&mut engine, 200, &actors);
        if spawn.spawn_point != Some(NORTH) {
            assert_eq!(spawn.spawn_point, Some(DEFENDER_HOME));
            break;
        }
        occupy(&mut actors, next, spawn.position);
        next += 1;
        assert!(next < 400, "the placements must be bounded");
    }
}

#[test]
fn attackers_on_their_lane_keep_the_existing_order() {
    // Squad 0's lane (objective 5) is owned by the other side: it attacks,
    // so its bots keep their population slot while the authored default
    // stands.
    let mut engine = engine_with_defence();
    let spawn = spawned(choose(&mut engine, &Default::default()).0);
    assert_eq!(spawn.arrival, Arrival::PopulationSlot);
    assert_eq!(spawn.spawn_point, Some(HOME));
    assert_eq!(spawn.objective, None);
    assert!(engine.bot_spawn_goals(SQUAD).is_empty());
}

#[test]
fn attackers_with_a_frago_rank_their_points_toward_it() {
    // Both points route to the FRAGO within the spread, so the squad's bots
    // spread over them by route.
    let mut engine = engine_with_defence();
    order(&mut engine, SQUAD, Some(NEAR_HOME_FRAGO));
    let mut used = std::collections::BTreeSet::new();
    for actor in [200, 201] {
        let mut receipts = Vec::new();
        let spawn = spawned(
            engine
                .choose_bot_life_spawn(actor, SQUAD, 0, &Default::default(), &mut receipts)
                .unwrap(),
        );
        assert_eq!(spawn.objective.map(|o| o.0), Some(NEAR_HOME_FRAGO));
        used.insert(spawn.spawn_point);
    }
    assert_eq!(used, [Some(HOME), Some(ALTERNATE)].into());
    // An attacking squad's lane is no fallback: it keeps the earlier order.
    assert_eq!(
        engine
            .bot_spawn_goals(SQUAD)
            .into_iter()
            .map(|g| g.0)
            .collect::<Vec<_>>(),
        [NEAR_HOME_FRAGO]
    );
}

#[test]
fn attackers_keep_the_existing_order_with_the_switch_off() {
    let mut engine = engine_with_defence();
    engine
        .bot_registry
        .config
        .as_mut()
        .unwrap()
        .squad_orders
        .spawn_at_current_objective = false;
    let spawn = spawned(choose(&mut engine, &Default::default()).0);
    assert_eq!(spawn.arrival, Arrival::PopulationSlot);
    assert_eq!(spawn.spawn_point, Some(HOME));
    assert_eq!(spawn.objective, None);
    // Defenders still rank toward their lane objective, ignoring a FRAGO.
    order(&mut engine, DEFENDERS, Some(NEAR_HOME_FRAGO));
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert!(
        matches!(spawn.spawn_point, Some(NORTH | SOUTH)),
        "{spawn:?}"
    );
    assert_eq!(spawn.objective.map(|o| o.0), Some(5));
}

const NEAR_HOME_FRAGO: u16 = 6;
const OFF_MESH_FRAGO: u16 = 7;

/// The objective the squad worked on at the last bot tick (its FRAGO here).
fn order(engine: &mut Engine<Vec<u8>>, squad: u16, objective: Option<u16>) {
    let current = &mut engine.bot_registry.director.squad_orders.current;
    match objective {
        Some(_) => current.insert(squad, objective),
        None => current.remove(&squad),
    };
}

#[test]
fn a_frago_far_from_the_lane_moves_lives_to_the_point_nearest_it() {
    let mut engine = engine_with_defence();
    order(&mut engine, DEFENDERS, Some(NEAR_HOME_FRAGO));
    for actor in [200, 201] {
        let (spawn, receipts) = choose_defender(&mut engine, actor, &Default::default());
        assert_eq!(spawn.spawn_point, Some(DEFENDER_HOME), "{receipts:?}");
        assert_eq!(spawn.arrival, Arrival::PopulationSlot);
        let (objective, route_m) = spawn.objective.unwrap();
        assert_eq!(objective, NEAR_HOME_FRAGO);
        assert!(route_m < 13., "{route_m}");
    }
    // Clearing the FRAGO returns the squad to its lane objective.
    order(&mut engine, DEFENDERS, None);
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert!(
        matches!(spawn.spawn_point, Some(NORTH | SOUTH)),
        "{spawn:?}"
    );
    assert_eq!(spawn.objective.map(|o| o.0), Some(5));
}

#[test]
fn a_frago_no_point_routes_to_falls_back_to_the_lane_objective() {
    let mut engine = engine_with_defence();
    order(&mut engine, DEFENDERS, Some(OFF_MESH_FRAGO));
    let (spawn, receipts) = choose_defender(&mut engine, 200, &Default::default());
    assert!(
        matches!(spawn.spawn_point, Some(NORTH | SOUTH)),
        "{spawn:?}"
    );
    assert_eq!(spawn.objective.map(|o| o.0), Some(5));
    assert!(
        receipts
            .iter()
            .any(|r| r["op"] == "bot_lane_spawn_unusable" && r["objective"] == OFF_MESH_FRAGO)
    );
    // An attacking squad has no defended lane to fall back on: with nothing
    // routable it keeps the earlier active-spawn order.
    order(&mut engine, SQUAD, Some(OFF_MESH_FRAGO));
    let spawn = spawned(choose(&mut engine, &Default::default()).0);
    assert_eq!(spawn.spawn_point, Some(HOME));
    assert_eq!(spawn.objective, None);
}

#[test]
fn the_forward_tier_still_comes_first_toward_a_frago() {
    // The hospital (~38 m from the FRAGO) is forward; the population slot
    // (~10 m) is not.
    let mut engine = engine_with_defence();
    engine.spawn_catalog = Some(spawns_with_hospital());
    engine.bunker_world = Some(crate::structures::BunkerWorld::synthetic_hospital(
        HOSPITAL,
        1,
        2,
        replication::CharacterPosition {
            x: 30.,
            y: 0.,
            z: 30.,
        },
    ));
    order(&mut engine, DEFENDERS, Some(NEAR_HOME_FRAGO));
    let (spawn, receipts) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(spawn.spawn_point, Some(HOSPITAL), "{receipts:?}");
    assert_eq!(spawn.objective.map(|o| o.0), Some(NEAR_HOME_FRAGO));
    // Once the hospital falls, the point nearest the FRAGO takes over.
    deactivate(&mut engine, HOSPITAL);
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(spawn.spawn_point, Some(DEFENDER_HOME));
}

#[test]
fn a_moved_squad_default_is_forward_for_attackers() {
    // An Acquisition LALO zone becomes the squad's default while its
    // platoon's anti-aircraft is down; here a point the squad already had.
    let mut engine = engine_with_defence();
    order(&mut engine, SQUAD, Some(NEAR_HOME_FRAGO));
    set_runtime_default(&mut engine, CAPTURED);
    let spawn = spawned(choose(&mut engine, &Default::default()).0);
    assert_eq!(spawn.spawn_point, Some(CAPTURED));
    assert_eq!(spawn.objective.map(|o| o.0), Some(NEAR_HOME_FRAGO));
    // Closing it (the anti-aircraft repaired) returns to the nearest points.
    deactivate(&mut engine, CAPTURED);
    let spawn = spawned(choose(&mut engine, &Default::default()).0);
    assert!(
        matches!(spawn.spawn_point, Some(HOME | ALTERNATE)),
        "{spawn:?}"
    );
}

#[test]
fn the_current_objective_comes_from_the_last_bot_tick_else_the_lane() {
    let mut engine = engine_with_defence();
    assert_eq!(engine.bot_squad_current_objective(DEFENDERS), Some(5));
    order(&mut engine, DEFENDERS, Some(NEAR_HOME_FRAGO));
    assert_eq!(
        engine.bot_squad_current_objective(DEFENDERS),
        Some(NEAR_HOME_FRAGO)
    );
    let goals: Vec<u16> = engine
        .bot_spawn_goals(DEFENDERS)
        .into_iter()
        .map(|g| g.0)
        .collect();
    assert_eq!(goals, [NEAR_HOME_FRAGO, 5]);
}

#[test]
fn defenders_arrive_outside_an_operational_hospital_of_their_side() {
    let mut engine = engine_with_defence();
    engine.spawn_catalog = Some(spawns_with_hospital());
    deactivate(&mut engine, NORTH);
    deactivate(&mut engine, SOUTH);
    let at = replication::CharacterPosition {
        x: 30.,
        y: 0.,
        z: 30.,
    };
    // A hospital of the other side is never used.
    engine.bunker_world = Some(crate::structures::BunkerWorld::synthetic_hospital(
        HOSPITAL, 0, 1, at,
    ));
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(spawn.spawn_point, Some(DEFENDER_HOME));
    engine.bot_registry.lane_spawns.clear();
    engine.bunker_world = Some(crate::structures::BunkerWorld::synthetic_hospital(
        HOSPITAL, 1, 2, at,
    ));
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(spawn.spawn_point, Some(HOSPITAL));
    assert_eq!(spawn.arrival, Arrival::HospitalExit);
    // At the authored exit, 2 m from the hospital.
    assert!(
        horizontal(spawn.position, 32., 30.) < 0.01,
        "{:?}",
        spawn.position
    );
    // A hospital point the mode deactivates (destroyed) is dropped.
    deactivate(&mut engine, HOSPITAL);
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(spawn.spawn_point, Some(DEFENDER_HOME));
}

#[test]
fn defenders_use_a_natively_routed_field_hospital_their_catalog_leaves_unsupported() {
    let mut engine = engine_with_defence();
    deactivate(&mut engine, NORTH);
    deactivate(&mut engine, SOUTH);
    let catalog = engine.spawn_catalog.as_mut().unwrap();
    let mut fallbacks = [u16::MAX; 32];
    fallbacks[usize::from(DEFENDERS)] = DEFENDER_HOME;
    fallbacks[usize::from(SQUAD)] = HOME;
    catalog.set_unselectable_native_spawns_for_test([FIELD_HOSPITAL].into(), fallbacks);
    catalog
        .set_runtime_point_active(FIELD_HOSPITAL, 111, true)
        .unwrap();
    catalog
        .set_runtime_squad_default(DEFENDERS, FIELD_HOSPITAL)
        .unwrap();
    // Players still fall back to the category-zero point.
    assert_eq!(
        catalog.locally_selectable(DEFENDERS, FIELD_HOSPITAL),
        DEFENDER_HOME
    );
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(spawn.spawn_point, Some(FIELD_HOSPITAL));
    assert_eq!(spawn.arrival, Arrival::FieldHospital);
    assert!(horizontal(spawn.position, 30., 40.) <= 6. * 2f32.sqrt() + 0.01);
    // Once the structure falls the mode deactivates the point.
    deactivate(&mut engine, FIELD_HOSPITAL);
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(spawn.spawn_point, Some(DEFENDER_HOME));
}

#[test]
fn a_dead_defender_respawns_forward() {
    let mut engine = engine_with_defence();
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let (&id, bot) = engine
        .bot_registry
        .bots
        .iter()
        .find(|(_, bot)| bot.target.squad.squad_index() == DEFENDERS)
        .expect("a defending bot");
    let generation = bot.target.runtime.unwrap().generation;
    engine
        .commit_bot_damage(
            id,
            generation,
            replication::CharacterDamage {
                health_quarter_points: 0,
                ..bot_registry_initial_damage()
            },
            Duration::from_millis(500),
        )
        .unwrap();
    let (spawn, _) = {
        let slot = engine.bot_registry.bots[&id].slot;
        let mut receipts = Vec::new();
        let choice = engine
            .choose_bot_life_spawn(id, DEFENDERS, slot, &Default::default(), &mut receipts)
            .unwrap();
        (spawned(choice), receipts)
    };
    assert!(
        matches!(spawn.spawn_point, Some(NORTH | SOUTH)),
        "{spawn:?}"
    );
}

#[test]
fn defenders_prefer_a_standing_hospital_to_nearer_rear_points() {
    // North and south sit beside the objective; the hospital is ~24 m out.
    let mut engine = engine_with_defence();
    engine.spawn_catalog = Some(spawns_with_hospital());
    engine.bunker_world = Some(crate::structures::BunkerWorld::synthetic_hospital(
        HOSPITAL,
        1,
        2,
        replication::CharacterPosition {
            x: 30.,
            y: 0.,
            z: 30.,
        },
    ));
    for actor in [200, 201] {
        let (spawn, receipts) = choose_defender(&mut engine, actor, &Default::default());
        assert_eq!(spawn.spawn_point, Some(HOSPITAL), "{receipts:?}");
        assert_eq!(spawn.arrival, Arrival::HospitalExit);
    }
    // Once the hospital falls, the points beside the objective take over.
    deactivate(&mut engine, HOSPITAL);
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert!(
        matches!(spawn.spawn_point, Some(NORTH | SOUTH)),
        "{spawn:?}"
    );
}

#[test]
fn defenders_prefer_a_forward_point_their_squad_gained() {
    let mut engine = engine_with_defence();
    let catalog = engine.spawn_catalog.as_mut().unwrap();
    catalog
        .activate_forward(ForwardSpawnBinding {
            objective_index: 20,
            squad_index: DEFENDERS,
            spawn_point_index: HOSPITAL,
        })
        .unwrap();
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(spawn.spawn_point, Some(HOSPITAL));
    assert_eq!(spawn.arrival, Arrival::AuthoredPoint);
}

#[test]
fn spawn_routes_are_qualified_again_when_the_structure_gates_change() {
    let mut engine = engine_with_defence();
    let lane_receipts = |receipts: &[Value]| {
        receipts
            .iter()
            .filter(|r| r["op"] == "bot_lane_spawn_qualified")
            .count()
    };
    let (_, first) = choose_defender(&mut engine, 200, &Default::default());
    assert!(lane_receipts(&first) > 0);
    let (_, cached) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(lane_receipts(&cached), 0);
    // New structure gates (here: an empty snapshot) invalidate the routes.
    let structures = std::sync::Arc::new(crate::bot_structure_collision::StructureObstacles::new(
        &crate::domination_structural_world::SequenceCollisionState::default(),
        1,
    ));
    let mesh = engine.navmesh_world.clone().unwrap();
    crate::navmesh_structure_gates::install_now(&mesh, Some(&structures)).unwrap();
    let (_, again) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(lane_receipts(&again), lane_receipts(&first));
}

#[test]
fn each_side_starts_its_bots_at_its_own_first_wave() {
    // A human confirmed at 0 s, so their side's first wave is at its 35 s
    // initial delay. The bot-only side counts from the same moment with its
    // own 20 s delay, as retail defenders come in before attackers.
    let mut engine = engine();
    engine.bot_names = crate::bot_names::Catalog::from_json(
        &serde_json::to_vec(&json!({"entries":[{"name":"Synthetic One"},
            {"name":"Synthetic Two"},{"name":"Synthetic Three"},{"name":"Synthetic Four"}]}))
        .unwrap(),
    )
    .unwrap();
    let config = engine.bot_registry.config.clone().unwrap();
    let mut engine = engine.with_bot_population(config, 4).unwrap();
    let reserve = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: "synthetic-first-wave-reserve".into(),
            reservation_id: "synthetic-first-wave-human".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic Human",
                "faction":2,"head":0,"voice":0,"universe_id":26,"character_id":1}))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(reserve.ok, "{:?}", reserve.error);
    let human = engine.active.as_ref().unwrap().team();
    let bots_only = 1 - human;
    let mut delays = [0; 2];
    delays[usize::from(human)] = 35_000;
    delays[usize::from(bots_only)] = 20_000;
    let mut value = spawns_value();
    value["match_profile"]["initial_spawn_delay_ms"] = json!(delays);
    engine.spawn_catalog = Some(serde_json::from_value(value).unwrap());
    engine.team_waves[usize::from(human)] = Some(TeamWaveClock {
        first_deadline: Duration::from_secs(35),
        period: Duration::from_secs(20),
    });
    assert_eq!(
        engine.bot_team_first_wave(human),
        Some(Duration::from_secs(35))
    );
    assert_eq!(
        engine.bot_team_first_wave(bots_only),
        Some(Duration::from_secs(20))
    );
    let teams = |engine: &Engine<Vec<u8>>| {
        let mut teams: Vec<_> = engine
            .bot_registry
            .bots
            .values()
            .map(|b| b.target.squad.team())
            .collect();
        teams.sort_unstable();
        teams
    };
    let reconcile = |engine: &mut Engine<Vec<u8>>, secs| {
        engine
            .reconcile_bot_population(Duration::from_secs(secs), 1, &mut Default::default())
            .unwrap()
    };
    reconcile(&mut engine, 10);
    assert!(teams(&engine).is_empty());
    reconcile(&mut engine, 20);
    assert_eq!(teams(&engine), [bots_only; 2]);
    // The bot-only side's wave clock starts at its first wave.
    assert_eq!(
        engine.team_waves[usize::from(bots_only)].map(|c| c.first_deadline),
        Some(Duration::from_secs(20))
    );
    reconcile(&mut engine, 35);
    assert!(teams(&engine).contains(&human), "{:?}", teams(&engine));
}

fn lane_receipts(receipts: &[Value]) -> usize {
    receipts
        .iter()
        .filter(|r| r["op"] == "bot_lane_spawn_qualified" || r["op"] == "bot_lane_spawn_unusable")
        .count()
}

#[test]
fn a_slow_defender_route_never_holds_the_spawn_choice() {
    // On a large map one failing lane qualification can search the whole
    // navmesh; a side's first wave asks for dozens and held the world past
    // the 10 s stream deadline. Queued, the choice returns at once with the
    // population slot and the forward points arrive on a later life.
    let mut engine = engine_with_defence();
    engine.bot_spawn_routes.inline = false;
    engine.bot_spawn_routes.test_delay = Some(Duration::from_millis(800));
    let started = std::time::Instant::now();
    let (spawn, receipts) = choose_defender(&mut engine, 200, &Default::default());
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "the choice waited for a route: {:?}",
        started.elapsed()
    );
    assert_eq!(spawn.arrival, Arrival::PopulationSlot);
    assert_eq!(spawn.spawn_point, Some(DEFENDER_HOME));
    assert_eq!(
        lane_receipts(&receipts),
        0,
        "pending points are not reported"
    );
    assert!(!engine.bot_spawn_routes.pending.is_empty());
    // Asking again while the routes run queues nothing new and stays fast.
    let queued = engine.bot_spawn_routes.pending.len();
    let started = std::time::Instant::now();
    let (again, _) = choose_defender(&mut engine, 201, &Default::default());
    assert!(started.elapsed() < Duration::from_millis(400));
    assert_eq!(again.arrival, Arrival::PopulationSlot);
    assert_eq!(engine.bot_spawn_routes.pending.len(), queued);
    // Once answered, the next life uses the forward points, reported once.
    engine.bot_spawn_routes.blocking = true;
    engine.wait_bot_spawn_routes(1).unwrap();
    let (forward, receipts) = choose_defender(&mut engine, 202, &Default::default());
    assert!(
        [Some(NORTH), Some(SOUTH)].contains(&forward.spawn_point),
        "{forward:?}"
    );
    assert!(lane_receipts(&receipts) > 0);
    assert!(engine.bot_spawn_routes.pending.is_empty());
    // The same choice as routing inline.
    let mut inline = engine_with_defence();
    let (expected, _) = choose_defender(&mut inline, 202, &Default::default());
    assert_eq!(forward, expected);
}

#[test]
fn queued_routes_through_replaced_structure_gates_are_routed_again() {
    let mut engine = engine_with_defence();
    engine.bot_spawn_routes.inline = false;
    engine.bot_spawn_routes.blocking = true;
    // Slow enough that no answer can arrive within one spawn choice.
    engine.bot_spawn_routes.test_delay = Some(Duration::from_millis(250));
    let (first, _) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(first.arrival, Arrival::PopulationSlot);
    // The gates change while the answers are in flight.
    let structures = std::sync::Arc::new(crate::bot_structure_collision::StructureObstacles::new(
        &crate::domination_structural_world::SequenceCollisionState::default(),
        1,
    ));
    let mesh = engine.navmesh_world.clone().unwrap();
    crate::navmesh_structure_gates::install_now(&mesh, Some(&structures)).unwrap();
    engine.wait_bot_spawn_routes(1).unwrap();
    let (stale, receipts) = choose_defender(&mut engine, 200, &Default::default());
    assert_eq!(
        stale.arrival,
        Arrival::PopulationSlot,
        "answers routed through the old gates are dropped"
    );
    assert_eq!(lane_receipts(&receipts), 0, "{receipts:?}");
    engine.wait_bot_spawn_routes(1).unwrap();
    let (current, receipts) = choose_defender(&mut engine, 200, &Default::default());
    assert!(
        [Some(NORTH), Some(SOUTH)].contains(&current.spawn_point),
        "{current:?}"
    );
    assert!(lane_receipts(&receipts) > 0);
}

#[test]
fn a_prefetch_pass_probes_at_most_its_budget_and_resumes_next_tick() {
    // Defenders' points 7, 8 and 9 need placements probed; the budget is two
    // per pass, so the third waits for the next tick instead of the interval.
    let mut engine = engine_with_defence();
    engine.bot_spawn_routes.inline = false;
    engine.bot_spawn_routes.blocking = true;
    let probes = |engine: &Engine<Vec<u8>>| engine.bot_registry.spawn_candidates.probes;
    engine.prefetch_bot_spawn_routes(Duration::ZERO, 1).unwrap();
    assert_eq!(probes(&engine), SPAWN_PREFETCH_PROBES_PER_TICK);
    assert!(engine.bot_spawn_routes.prefetch_resume);
    engine
        .prefetch_bot_spawn_routes(Duration::from_millis(50), 1)
        .unwrap();
    assert_eq!(probes(&engine), 3);
    assert!(!engine.bot_spawn_routes.prefetch_resume);
    // A finished pass waits for the interval.
    let queued = engine.bot_spawn_routes.pending.len();
    engine
        .prefetch_bot_spawn_routes(Duration::from_millis(100), 1)
        .unwrap();
    assert_eq!(engine.bot_spawn_routes.pending.len(), queued);
    engine.wait_bot_spawn_routes(1).unwrap();
    let (spawn, _) = choose_defender(&mut engine, 200, &Default::default());
    assert!(
        [Some(NORTH), Some(SOUTH)].contains(&spawn.spawn_point),
        "{spawn:?}"
    );
    assert_eq!(
        probes(&engine),
        3,
        "the choice reused the probed placements"
    );
}

#[test]
fn structure_gate_changes_route_again_without_probing_placements_again() {
    let mut engine = engine_with_defence();
    engine.bot_spawn_routes.inline = false;
    engine.bot_spawn_routes.blocking = true;
    for t in [0, 50] {
        engine
            .prefetch_bot_spawn_routes(Duration::from_millis(t), 1)
            .unwrap();
    }
    engine.wait_bot_spawn_routes(1).unwrap();
    choose_defender(&mut engine, 200, &Default::default());
    let probed = engine.bot_registry.spawn_candidates.probes;
    let structures = std::sync::Arc::new(crate::bot_structure_collision::StructureObstacles::new(
        &crate::domination_structural_world::SequenceCollisionState::default(),
        1,
    ));
    let mesh = engine.navmesh_world.clone().unwrap();
    crate::navmesh_structure_gates::install_now(&mesh, Some(&structures)).unwrap();
    engine
        .prefetch_bot_spawn_routes(Duration::from_secs(2), 1)
        .unwrap();
    assert!(
        !engine.bot_spawn_routes.pending.is_empty(),
        "routes are qualified again"
    );
    assert!(!engine.bot_spawn_routes.prefetch_resume);
    assert_eq!(engine.bot_registry.spawn_candidates.probes, probed);
    engine.wait_bot_spawn_routes(1).unwrap();
    let (spawn, receipts) = choose_defender(&mut engine, 200, &Default::default());
    assert!(
        [Some(NORTH), Some(SOUTH)].contains(&spawn.spawn_point),
        "{spawn:?}"
    );
    assert!(lane_receipts(&receipts) > 0);
    assert_eq!(engine.bot_registry.spawn_candidates.probes, probed);
}
