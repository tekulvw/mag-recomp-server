//! SYNTHETIC check that bot sight workers never change a match: two lines of
//! armed bots face each other across a generated floor, part of it behind a
//! wall, and fight with one, two and four workers. No game data.
use super::*;
use crate::resources::tests::wire_profiles;
use crate::spawn_progression::ready;

// One squad per team, which holds eight roles.
const PER_TEAM: usize = 8;

fn arena(prefix: &str, threads: usize, rays: usize) -> Engine<Vec<u8>> {
    let items = crate::bot_registry::identity_tests::bootstrap_items();
    let names = crate::bot_names::Catalog::from_json(
        &serde_json::to_vec(&json!({"entries":(0..=2 * PER_TEAM)
            .map(|i| json!({"name":format!("{prefix} {i}")})).collect::<Vec<_>>()}))
        .unwrap(),
    )
    .unwrap();
    let spawns: spawn_catalog::SpawnCatalog = serde_json::from_value(json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC sight workers test",
        "active":{"map_name":"synthetic_arena","squad_index":0,"default_spawn":0,"available":[0]},
        "match_profile":{"has_bunkers":false,"has_objectives":false,"auto_assign_teams":true,
            "team_capacity":16,"squad_capacity":16},
        "maps":[{"name":"synthetic_arena","owner_faction":2,"spawn_count":1,
            "locations":[{"index":0,"token":1,"kind":"on_foot","position":{"x":2.,"y":0.,"z":2.}}],
            "squads":[{"index":0,"team":0,"platoon":0,"squad":0,"faction":null,"available":[0],"default_spawn":0},
            {"index":16,"team":1,"platoon":0,"squad":0,"faction":2,"available":[0],"default_spawn":0}]}]}))
    .unwrap();
    let resources = EngineResources::for_application(
        std::sync::Arc::new(items),
        spawns,
        None,
        wire_profiles(),
        20231,
    )
    .unwrap()
    .with_bot_names(names);
    let mut engine = Engine::from_resources(Vec::new(), 64 << 20, resources);
    engine.maximum_players = 2 * PER_TEAM + 1;
    // A floor, and a wall across x = 30 that hides the first half of the lines.
    let world = json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
            "provenance":["Synthetic sight workers floor"],"unknowns":["Not a game map"],
            "physics_regions":[{"min":[-1.,-5.,-1.],"max":[61.,8.,81.],"ground_verified":true,
                "obstacles_verified":true,"provenance":["Synthetic floor"]}]},
        "instances":[
            {"id":"floor","vertices_xyz":[[0.,0.,0.],[60.,0.,0.],[60.,0.,80.],[0.,0.,80.]],
             "triangles":[[0,2,1],[0,3,2]]},
            {"id":"wall","vertices_xyz":[[30.,0.,0.],[30.,5.,0.],[30.,5.,36.],[30.,0.,36.]],
             "triangles":[[0,1,2],[0,2,3],[0,2,1],[0,3,2]]}]});
    engine = engine.with_collision_world(
        collision::CollisionWorld::from_bytes(
            &serde_json::to_vec(&world).unwrap(),
            collision::CollisionLimits::default(),
        )
        .unwrap(),
    );
    let slot = |team: usize, i: usize| json!({"team":team,"position":{"x":if team == 0 {10.} else {50.},"y":0.,"z":4. + 9. * i as f64}});
    let slots: Vec<_> = (0..PER_TEAM)
        .flat_map(|i| [slot(0, i), slot(1, i)])
        .collect();
    let config: crate::bot_registry::BotPopulation = serde_json::from_value(json!({
        "format":"mag_bot_population_v1","map_name":"synthetic_arena",
        // Armed below, once the synthetic rifle catalog is installed.
        "firearms":{"enabled":false},"slots":slots}))
    .unwrap();
    // The fill total counts the human, so one slot stays empty.
    engine = engine.with_bot_population(config, 2 * PER_TEAM).unwrap();
    // The human whose world the bots fight in, armed with the synthetic rifle,
    // standing well away from both lines.
    let response = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: "synthetic-workers-reserve".into(),
            reservation_id: "synthetic-workers-0000".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic Observer","faction":2,
                "head":0,"voice":0,"universe_id":26,"character_id":7}))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    let source = engine.active.as_mut().unwrap();
    ready(source);
    source.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let mut armed = crate::human_kill_reward_tests::fixture();
    let equipped = armed.active.take().unwrap();
    engine.item_catalog = armed.item_catalog;
    let source = engine.active.as_mut().unwrap();
    source.current_inventory_handle = equipped.current_inventory_handle;
    source.spawned_loadout = equipped.spawned_loadout;
    source.observed_primary = equipped.observed_primary;
    source.primary_equipped = true;
    source.tracking.server_position(
        replication::CharacterPosition {
            x: 58.,
            y: 0.,
            z: 79.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    let kit = source.equipped_loadout().unwrap();
    let faction = source.reservation.character.faction;
    engine.item_catalog = crate::human_kill_reward_tests::bot_firearm_items();
    engine.match_started = true;
    let config = engine.bot_registry.config.as_mut().unwrap();
    config.firearms = crate::bot_firearms::Policy {
        reaction_ms: 100,
        turn_degrees_per_second: 360.,
        aim_tolerance_degrees: 10.,
        accuracy_per_mille: 400,
        max_rays_per_tick: rays,
        max_candidates_per_bot: 4,
        ..Default::default()
    };
    let body = bot_physics::Body::default();
    let terrain =
        bot_physics::WorldTerrain::new(engine.collision_world.as_ref(), engine.map_name()).unwrap();
    let ids: Vec<u16> = engine.bot_registry.bots.keys().copied().collect();
    assert_eq!(ids.len(), 2 * PER_TEAM - 1);
    for id in ids {
        let b = engine.bot_registry.bots.get_mut(&id).unwrap();
        b.target.faction = faction;
        b.target.loadout = kit.clone();
        let runtime = b.target.runtime.as_mut().unwrap();
        let at = runtime.position;
        let supported = terrain
            .advance(
                terrain
                    .start_motion([at.x.into(), 0.5, at.z.into()], body)
                    .unwrap(),
                [0.; 3],
                1.,
                body,
            )
            .unwrap();
        assert!(supported.grounded && !supported.blocked);
        runtime.physics_motion = Some(supported);
        runtime.position.y = supported.position[1] as f32;
        // Face the other line: +X for team 0, -X for team 1.
        b.yaw = if b.target.squad.team() == 0 { 256 } else { 768 };
    }
    engine.set_bot_threads(threads);
    engine
}

/// Each bot's index, health, magazine, target and facing.
type BotState = Vec<(u16, u16, u32, Option<u16>, u16)>;

/// Everything the fight leaves behind: the evidence log, each bot's state
/// and how prefetched lines were used.
fn fight(threads: usize, rays: usize) -> (String, BotState, (u64, u64, u64)) {
    let mut engine = arena("SW", threads, rays);
    for t in (0..8000).step_by(50) {
        let now = Duration::from_millis(t);
        engine.tick_bot_perception(now, 1).unwrap();
        engine.tick_bot_firearms(now, 1).unwrap();
    }
    let bots = engine
        .bot_registry
        .bots
        .iter()
        .map(|(&id, b)| {
            (
                id,
                b.health,
                b.firearm.magazine,
                b.firearm.target_index(),
                b.yaw,
            )
        })
        .collect();
    let usage = engine.bot_sight_prefetch_usage();
    let log = String::from_utf8(engine.writer)
        .unwrap()
        .lines()
        .map(|line| {
            let mut row: Value = serde_json::from_str(line).unwrap();
            without_wall_time(&mut row);
            row.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    (log, bots, usage)
}

/// Receipts that measure the server's own wall time differ run to run.
fn without_wall_time(row: &mut Value) {
    match row {
        Value::Object(fields) => {
            fields.retain(|key, _| !key.ends_with("_us") && key != "slow_ticks_over_2ms");
            fields.values_mut().for_each(without_wall_time);
        }
        Value::Array(items) => items.iter_mut().for_each(without_wall_time),
        _ => {}
    }
}

#[test]
fn sight_workers_leave_every_shot_receipt_and_bot_state_identical() {
    // A roomy budget and one so small that bots queue for rays every tick.
    for rays in [64, 3] {
        let (log, bots, usage) = fight(1, rays);
        assert_eq!(usage, (0, 0, 0), "one thread prefetches nothing");
        let shots = log.matches("\"op\":\"bot_firearm_shot\"").count();
        assert!(shots > 20, "the lines must fight ({shots} shots)");
        assert!(
            log.contains("world_blocked"),
            "the wall must hide some lines"
        );
        for threads in [2, 4] {
            let (parallel_log, parallel_bots, usage) = fight(threads, rays);
            assert_eq!(parallel_bots, bots, "{threads} threads, {rays} rays");
            if let Some((a, b)) = log.lines().zip(parallel_log.lines()).find(|(a, b)| a != b) {
                panic!("{threads} threads, {rays} rays: logs differ\n{a}\n{b}");
            }
            assert_eq!(parallel_log.len(), log.len());
            assert!(usage.1 > 0, "prefetched lines were used: {usage:?}");
        }
    }
}
