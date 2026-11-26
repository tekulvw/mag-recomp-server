//! SYNTHETIC regressions for spots beside a body standing under a roof: open
//! ground with a wide slab 7 m above its middle (a canopy over a spawn),
//! authored in code. No game catalogs, meshes or captures.
use super::*;
use sha2::{Digest, Sha256};

const ROOF_Y: f64 = 7.;
const HERE: [f64; 3] = [15., 0., 15.];

/// Ground 30 m square and a roof slab over x/z 5..25, the collision world and
/// the navmesh baked from the same quads, with the population's first bot
/// settled on the ground under the middle of the roof. The roof top is its
/// own navmesh floor that no walk or certified drop reaches.
fn canopy_engine() -> (Engine<Vec<u8>>, u16) {
    let quads: [[f64; 5]; 2] = [
        // x0, x1, y, z0, z1
        [0., 30., 0., 0., 30.],
        [5., 25., ROOF_Y, 5., 25.],
    ];
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for [x0, x1, y, z0, z1] in quads {
        let n = vertices.len() as u32;
        vertices.extend([[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]]);
        triangles.extend([[n, n + 2, n + 1], [n, n + 3, n + 2]]);
    }
    let (min, max) = ([0., -5., 0.], [30., 12., 30.]);
    let raw = serde_json::to_vec(&json!({
        "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
            "provenance":["synthetic canopy"],"unknowns":["Not a game map"],"physics_regions":[
                {"min":min,"max":max,"ground_verified":true,"obstacles_verified":true,
                 "provenance":["synthetic roof slab over open ground"]}]},
        "instances":[{"id":"canopy","vertices_xyz":vertices,"triangles":triangles}]
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
            max: [30., 10., 30.],
        },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices,
        triangles,
    })
    .unwrap();
    let mesh = navmesh::NavMesh::load(artifact, world.source_sha256(), "synthetic_arena").unwrap();
    let mut engine = crate::bot_registry::identity_tests::engine("Synthetic Canopy");
    let mut config = engine.bot_registry.config.take().unwrap();
    config.slots[0].position = position(HERE);
    config.slots[1].position = position([HERE[0] + 3., 0., HERE[2]]);
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
        .find(|(_, b)| distance(b.target.runtime.unwrap().position, position(HERE)) < 0.5)
        .unwrap()
        .0;
    let body = bot_physics::Body::default();
    let terrain =
        bot_physics::WorldTerrain::new(engine.collision_world.as_ref(), engine.map_name()).unwrap();
    let settled = terrain
        .advance(
            terrain.start_motion([HERE[0], 0.5, HERE[2]], body).unwrap(),
            [0.; 3],
            1.,
            body,
        )
        .unwrap();
    assert!(
        settled.grounded && settled.position[1].abs() < 0.1,
        "{settled:?}"
    );
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
    runtime.position = position(settled.position);
    runtime.phase = BotPhase::Complete;
    runtime.navmesh_state.navigation_aborted = false;
    (engine, id)
}

/// The bundled policy with a ground search reaching the roof, as deployed
/// populations use.
fn deployed_policy() -> ObjectivePolicy {
    let mut policy = ObjectivePolicy::bundled();
    policy.ground_search_m = 8.;
    // The widest planning slice, so a slow test machine does not end a
    // planning pass before its route query.
    policy.planning_slice_us = 10_000;
    assert_eq!(policy.validate(), Ok(()));
    policy
}

#[test]
fn a_spot_beside_a_body_under_a_roof_is_on_its_floor_not_the_roof() {
    let (engine, id) = canopy_engine();
    let mesh = engine.navmesh_world.as_ref().unwrap();
    let here = engine.bot_registry.bots[&id]
        .target
        .runtime
        .unwrap()
        .physical_position();
    let floor = mesh.component_id(here).unwrap();
    let policy = deployed_policy();
    let body = bot_physics::Body::default();
    let raw = position([here[0] + 2., here[1] + 0.5, here[2]]);
    // The plain search takes the highest surface in reach: the roof, a floor
    // no route from here reaches.
    let roof = supported(
        engine.collision_world.as_ref(),
        engine.map_name(),
        raw,
        &policy,
        body,
    )
    .unwrap();
    assert!((f64::from(roof.y) - ROOF_Y).abs() < 0.2, "{roof:?}");
    assert_ne!(mesh.component_id(point(roof)).ok(), Some(floor));
    assert!(
        mesh.route(here, point(roof), 0.3)
            .map_or(true, |r| r.status != navmesh::Status::Complete)
    );
    // A spot beside the body stays on the body's own floor and routes.
    let spot = supported_local(
        engine.collision_world.as_ref(),
        engine.map_name(),
        raw,
        &policy,
        body,
    )
    .unwrap();
    assert!(f64::from(spot.y).abs() < 0.2, "{spot:?}");
    assert_eq!(mesh.component_id(point(spot)).ok(), Some(floor));
    assert_eq!(
        mesh.route(here, point(spot), 0.3).unwrap().status,
        navmesh::Status::Complete
    );
}

#[test]
fn a_bot_asked_to_yield_under_a_roof_stages_beside_itself() {
    let (mut engine, id) = canopy_engine();
    let requester = *engine.bot_registry.bots.keys().find(|&&k| k != id).unwrap();
    engine.bot_registry.bots.get_mut(&id).unwrap().traffic_yield =
        Some(crate::bot_traffic_flow::Yield {
            requester,
            generation: 1,
            objective: None,
            role_slot: 0,
            lane: Vec::new(),
            started: Duration::ZERO,
            goal: None,
            attempt: 0,
            retry_at: Duration::ZERO,
            resume: false,
        });
    let policy = deployed_policy();
    let actors = crate::actor_collision::ActorGrid::default();
    let mut receipts = Vec::new();
    let mut queries = 0;
    // Every pad tried is on the bot's floor; before, each was on the roof and
    // failed to route, so the bot never moved aside.
    for attempt in 0..4 {
        engine
            .plan_traffic_yield(
                id,
                Duration::from_secs(1 + attempt),
                &policy,
                &actors,
                &Instant::now(),
                &mut queries,
                &mut receipts,
            )
            .unwrap();
        let receipt = receipts.last().unwrap();
        assert!(receipt["result"]["Ok"].is_object(), "{receipt}");
        let goal = engine.bot_registry.bots[&id]
            .traffic_yield
            .as_ref()
            .unwrap()
            .goal
            .unwrap();
        assert!(f64::from(goal.y).abs() < 0.2, "{goal:?}");
        assert!(distance(goal, position(HERE)) < 3., "{goal:?}");
        // Release the staged move so the next pad is planned from the spot.
        let b = engine.bot_registry.bots.get_mut(&id).unwrap();
        b.target.pending_move = None;
        b.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
        let y = b.traffic_yield.as_mut().unwrap();
        y.goal = None;
        y.resume = false;
    }
    assert_eq!(queries, 4);
}
