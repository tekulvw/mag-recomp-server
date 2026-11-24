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
    serde_json::from_value(json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC bot spawn selection",
        "active":{"map_name":"synthetic_arena","squad_index":16,"default_spawn":6,"available":[6]},
        "match_profile":{"has_bunkers":false,"has_objectives":false},
        "maps":[{"name":"synthetic_arena","owner_faction":2,"spawn_count":7,
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
                {"index":6,"token":106,"kind":"insertion","position":{"x":10.,"y":0.,"z":50.}}],
            "squads":[
                {"index":0,"team":0,"platoon":0,"squad":0,"faction":null,"available":[0,1,3,4,5],"default_spawn":0},
                {"index":16,"team":1,"platoon":0,"squad":0,"faction":2,"available":[6],"default_spawn":6}]}]}))
    .unwrap()
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

/// A 60 m floor; the walking navmesh covers only x <= 48, so point 5 has
/// collision support but no route back to the squad's population slot.
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
