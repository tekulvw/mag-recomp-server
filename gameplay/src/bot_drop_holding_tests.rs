//! SYNTHETIC drop-holding regressions: a narrow raised walkway above open
//! ground, authored in code. No game catalogs, meshes or captures.
use super::*;
use sha2::{Digest, Sha256};

/// Walkway centre line; the walkway is 1.6 m wide, so the walkable strip
/// left after the 0.6 m navmesh erosion is only 0.4 m wide.
const CENTRE_X: f64 = 15.;
const WALKWAY_Y: f64 = 3.;

fn walkway_engine() -> (Engine<Vec<u8>>, u16) {
    let quads: [[f64; 5]; 2] = [
        // x0, x1, y, z0, z1
        [0., 30., 0., 0., 30.],
        [CENTRE_X - 0.8, CENTRE_X + 0.8, WALKWAY_Y, 6., 15.],
    ];
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for [x0, x1, y, z0, z1] in quads {
        let n = vertices.len() as u32;
        vertices.extend([[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]]);
        triangles.extend([[n, n + 2, n + 1], [n, n + 3, n + 2]]);
    }
    let (min, max) = ([0., -5., 0.], [30., 8., 30.]);
    let raw = serde_json::to_vec(&json!({
        "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
            "provenance":["synthetic walkway"],"unknowns":["Not a game map"],"physics_regions":[
                {"min":min,"max":max,"ground_verified":true,"obstacles_verified":true,
                 "provenance":["synthetic walkway over open ground"]}]},
        "instances":[{"id":"walkway","vertices_xyz":vertices,"triangles":triangles}]
    }))
    .unwrap();
    let world = collision::CollisionWorld::from_bytes(&raw, Default::default()).unwrap();
    let vertices: Vec<[f32; 3]> = vertices.iter().map(|p| p.map(|v| v as f32)).collect();
    let mut hash = Sha256::new();
    hash.update(b"MAG-NAV-GEOMETRY-v1\0");
    hash.update((vertices.len() as u32).to_le_bytes());
    hash.update((triangles.len() as u32).to_le_bytes());
    for v in vertices.iter().flatten() {
        hash.update(v.to_le_bytes());
    }
    for i in triangles.iter().flatten() {
        hash.update(i.to_le_bytes());
    }
    let artifact = navmesh::bake(navmesh::BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: "synthetic_arena".into(),
        world_sha256: world.source_sha256().into(),
        geometry_sha256: format!("{:x}", hash.finalize()),
        bounds: navmesh::Bounds {
            min: [0., -1., 0.],
            max: [30., 6., 30.],
        },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices,
        triangles,
    })
    .unwrap();
    let mesh = navmesh::NavMesh::load(artifact, world.source_sha256(), "synthetic_arena").unwrap();
    let mut engine = crate::bot_registry::identity_tests::engine("Synthetic Walkway");
    let mut config = engine.bot_registry.config.take().unwrap();
    config.slots[0].position = replication::CharacterPosition {
        x: CENTRE_X as f32,
        y: WALKWAY_Y as f32,
        z: 9.,
    };
    let mut engine = engine
        .with_collision_world(world)
        .with_navmesh_world(mesh)
        .with_bot_population(config, 2)
        .unwrap();
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let id = *engine
        .bot_registry
        .bots
        .iter()
        .find(|(_, b)| b.target.runtime.unwrap().position.y > 2.)
        .unwrap()
        .0;
    // Settle the walker on the strip, as the queue leaves it: frozen, grounded.
    let body = bot_physics::Body::default();
    let terrain =
        bot_physics::WorldTerrain::new(engine.collision_world.as_ref(), engine.map_name()).unwrap();
    let [x, y, z] = engine.bot_registry.bots[&id]
        .target
        .runtime
        .unwrap()
        .physical_position();
    let settled = terrain
        .advance(
            terrain.start_motion([x, y + 0.5, z], body).unwrap(),
            [0.; 3],
            1.,
            body,
        )
        .unwrap();
    assert!(settled.grounded && (settled.position[1] - WALKWAY_Y).abs() < 0.1);
    let runtime = engine
        .bot_registry
        .bots
        .get_mut(&id)
        .unwrap()
        .target
        .runtime
        .as_mut()
        .unwrap();
    runtime.physics_motion = Some(settled);
    runtime.position.y = settled.position[1] as f32;
    runtime.phase = BotPhase::Complete;
    let mesh = engine.navmesh_world.as_ref().unwrap();
    assert!(mesh.component_id(settled.position).is_ok());
    (engine, id)
}

/// Queue the walker for a drop whose launch sits `launch_x` across the end of
/// the walkway, landing on the ground in front of it.
fn queue_for_drop(engine: &mut Engine<Vec<u8>>, id: u16, launch_x: f64) {
    let launch = [launch_x, WALKWAY_Y, 14.8];
    let landing = [launch_x, 0., 17.];
    let body = bot_physics::Body::default();
    let bot = engine.bot_registry.bots.get_mut(&id).unwrap();
    bot.drop_queue_passage = Some(development_bot_movement::DropPassage {
        id: 7,
        launch,
        landing,
        protected_bounds: crate::actor_collision::Bounds::body(launch, body)
            .union(crate::actor_collision::Bounds::body(landing, body)),
        phase: development_bot_movement::DropPhase::Approaching,
        distance_to_launch: 6.,
        route_distance_to_launch: 6.,
    });
    bot.drop_waiting = Some(7);
    bot.drop_queue_since = Some(Duration::ZERO);
}

/// Stage the walker until it has a holding spot or `passes` run out.
fn stage(engine: &mut Engine<Vec<u8>>, id: u16, passes: usize) -> Vec<Value> {
    let mut policy = ObjectivePolicy::bundled();
    // Generous slices: an unoptimised test build must not cut scans short.
    policy.planning_slice_us = 10_000;
    let actors = crate::actor_collision::ActorGrid::default();
    let mut receipts = Vec::new();
    for pass in 0..passes {
        let now = Duration::from_millis(100 * pass as u64);
        let started = Instant::now();
        engine
            .plan_drop_holding(
                id,
                now,
                1,
                &policy,
                &actors,
                &started,
                &mut 0,
                &mut receipts,
            )
            .unwrap();
        if engine.bot_registry.bots[&id].drop_holding_goal.is_some() {
            break;
        }
    }
    receipts
}

#[test]
fn a_walker_on_a_narrow_ledge_holds_where_it_stands_when_no_ring_spot_fits() {
    let (mut engine, id) = walkway_engine();
    // The launch is near the walkway's side, so every ring column around it
    // misses the 0.4 m walkable strip down the walkway's centre line.
    queue_for_drop(&mut engine, id, CENTRE_X + 0.75);
    let here = engine.bot_registry.bots[&id]
        .target
        .runtime
        .unwrap()
        .position;
    let receipts = stage(&mut engine, id, 12);
    let actions: Vec<_> = receipts
        .iter()
        .map(|r| (r["action"].as_str().unwrap(), r["result"].clone()))
        .collect();
    // The ring around the launch is scanned in full first and finds nothing.
    assert!(
        actions[..actions.len() - 1]
            .iter()
            .all(|(action, result)| *action == "stage" && result.get("Err").is_some()),
        "{actions:?}"
    );
    let bot = &engine.bot_registry.bots[&id];
    assert_eq!(bot.drop_holding_goal, Some(here), "{actions:?}");
    assert_eq!(actions.last().unwrap().0, "stage_in_place");
    assert!(!bot.drop_approach_ready);
    assert_eq!(bot.drop_holding_misses, 0);
    let policy = ObjectivePolicy::bundled();
    assert_eq!(
        receipts.len(),
        policy.drop_holding_candidate_limit / policy.drop_holding_scan,
        "the fallback comes on the pass that completes the ring"
    );
}

#[test]
fn a_ring_spot_on_the_walkway_is_still_preferred_to_holding_in_place() {
    let (mut engine, id) = walkway_engine();
    // A launch on the centre line puts a ring column on the strip.
    queue_for_drop(&mut engine, id, CENTRE_X);
    let here = engine.bot_registry.bots[&id]
        .target
        .runtime
        .unwrap()
        .position;
    let receipts = stage(&mut engine, id, 4);
    let bot = &engine.bot_registry.bots[&id];
    let goal = bot.drop_holding_goal.expect("a ring spot");
    assert_ne!(goal, here);
    assert!((f64::from(goal.x) - CENTRE_X).abs() < 0.2, "{goal:?}");
    assert_eq!(receipts.last().unwrap()["action"], "stage");
}
