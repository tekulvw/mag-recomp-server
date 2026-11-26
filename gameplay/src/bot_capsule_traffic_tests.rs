//! SYNTHETIC traffic regressions for capsule bots following navmesh routes
//! past each other: worlds, meshes and the capsule's dimensions are authored
//! here. No game geometry. Bots step in object-index order each 0.25 s tick
//! and reserve the path they moved, as the world registry does.
use super::*;
use crate::actor_collision::{ActorGrid, Bounds, Query};
use sha2::{Digest, Sha256};

const MAP: &str = "synthetic_capsule_traffic";

fn capsule() -> bot_physics::BodyShape {
    bot_physics::BodyShape::Capsule {
        radius: 0.4,
        half_height: 0.5,
        centre_height: 0.9,
    }
}

/// An axis-aligned box's top and four sides (12 m tall walls need no bottom).
fn block(
    min: [f64; 3],
    max: [f64; 3],
    vertices: &mut Vec<[f64; 3]>,
    triangles: &mut Vec<[u32; 3]>,
) {
    let n = vertices.len() as u32;
    for y in [min[1], max[1]] {
        vertices.extend([
            [min[0], y, min[2]],
            [max[0], y, min[2]],
            [max[0], y, max[2]],
            [min[0], y, max[2]],
        ]);
    }
    // Top, facing up.
    triangles.extend([[n + 4, n + 6, n + 5], [n + 4, n + 7, n + 6]]);
    for i in 0..4u32 {
        let j = (i + 1) % 4;
        triangles.extend([[n + i, n + j, n + 4 + j], [n + i, n + 4 + j, n + 4 + i]]);
    }
}

/// A 20 m by 10 m floor at y = 0 and, with `door`, a 3 m wall across x = 10
/// with a gap of that width centred on z = 5.
fn geometry(door: Option<f64>) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let mut vertices = vec![[0., 0., 0.], [20., 0., 0.], [20., 0., 10.], [0., 0., 10.]];
    let mut triangles = vec![[0u32, 2, 1], [0, 3, 2]];
    if let Some(width) = door {
        let half = width / 2.;
        block(
            [9.9, 0., 0.],
            [10.1, 3., 5. - half],
            &mut vertices,
            &mut triangles,
        );
        block(
            [9.9, 0., 5. + half],
            [10.1, 3., 10.],
            &mut vertices,
            &mut triangles,
        );
    }
    (vertices, triangles)
}

fn arena(door: Option<f64>) -> (collision::CollisionWorld, Arc<navmesh::NavMesh>) {
    arena_with_mesh_door(door, door)
}

/// The world has `door`; the mesh is baked from the same walls with
/// `mesh_door`. The mesh's conservative 0.6 m bake radius closes a gap of
/// 1.2 m or less, so a narrower physical door needs a wider baked one to be
/// routed through at all; the bots still squeeze through the physical door.
fn arena_with_mesh_door(
    door: Option<f64>,
    mesh_door: Option<f64>,
) -> (collision::CollisionWorld, Arc<navmesh::NavMesh>) {
    let (vertices, triangles) = geometry(door);
    let min = [-1., -3., -1.];
    let max = [21., 7., 11.];
    let raw = serde_json::to_vec(&json!({
        "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":MAP,"build":"synthetic","coverage":"provisional",
            "provenance":["synthetic floor and walls"],"unknowns":[],"physics_regions":[
                {"min":min,"max":max,"ground_verified":true,"obstacles_verified":true,
                 "provenance":["synthetic isolated region"]}]},
        "instances":[{"id":"synthetic-arena","vertices_xyz":vertices,"triangles":triangles}]
    }))
    .unwrap();
    let world = collision::CollisionWorld::from_bytes(&raw, Default::default()).unwrap();
    let (vertices, triangles) = geometry(mesh_door);
    let mut hash = Sha256::new();
    hash.update(b"MAG-NAV-GEOMETRY-v1\0");
    hash.update((vertices.len() as u32).to_le_bytes());
    hash.update((triangles.len() as u32).to_le_bytes());
    for x in vertices.iter().flatten() {
        hash.update((*x as f32).to_le_bytes());
    }
    for i in triangles.iter().flatten() {
        hash.update(i.to_le_bytes());
    }
    let input = navmesh::BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: MAP.into(),
        world_sha256: world.source_sha256().into(),
        geometry_sha256: format!("{:x}", hash.finalize()),
        bounds: navmesh::Bounds { min, max },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices: vertices.iter().map(|p| p.map(|x| x as f32)).collect(),
        triangles,
    };
    let profile = navmesh::MovementProfile::synthetic(54.);
    let artifact = navmesh::bake_with_movement_profile(input, &profile).unwrap();
    let mesh = Arc::new(
        navmesh::NavMesh::load_with_movement_profile(
            artifact,
            world.source_sha256(),
            MAP,
            &profile,
        )
        .unwrap(),
    );
    mesh.validate_coverage(&world).unwrap();
    (world, mesh)
}

struct Walker {
    navigation: NavigationMove,
    motion: bot_physics::Motion,
    waypoint: usize,
    state: NavmeshRuntime,
    reached: bool,
}

/// Run capsule bots (object index = position in `trips`, plus one) along
/// their routes for at most `seconds`; returns the seconds each took to
/// arrive (None if it did not). Unless `replans` allows it, any stop fails
/// the test; allowed stops route the bot again from where it stands, as the
/// registry does, and are counted against `replans`.
fn run(
    world: &collision::CollisionWorld,
    mesh: &Arc<navmesh::NavMesh>,
    trips: &[([f64; 3], [f64; 3])],
    seconds: f64,
) -> Vec<Option<f64>> {
    run_replanning(world, mesh, trips, seconds, 0)
}

fn run_replanning(
    world: &collision::CollisionWorld,
    mesh: &Arc<navmesh::NavMesh>,
    trips: &[([f64; 3], [f64; 3])],
    seconds: f64,
    mut replans: usize,
) -> Vec<Option<f64>> {
    let body = mesh.body().with_shape(capsule());
    let grid_body = bot_physics::Body::default().with_shape(capsule());
    let terrain = bot_physics::WorldTerrain::new(Some(world), MAP).unwrap();
    let mut grid = ActorGrid::default();
    let mut walkers: Vec<Walker> = trips
        .iter()
        .enumerate()
        .map(|(i, &(start, goal))| {
            let route = mesh.route(start, goal, 0.3).unwrap();
            assert_eq!(route.status, navmesh::Status::Complete, "{i}");
            let motion = terrain.start_motion(route.projected_start, body).unwrap();
            let motion = terrain.advance(motion, [0.; 3], 0.5, body).unwrap();
            assert!(motion.grounded, "{i}: {motion:?}");
            grid.insert_reserved_body(i as u16 + 1, motion.position, grid_body)
                .unwrap();
            Walker {
                navigation: NavigationMove::from_complete_navmesh_route(mesh.clone(), route)
                    .unwrap(),
                motion,
                waypoint: 1,
                state: NavmeshRuntime::default(),
                reached: false,
            }
        })
        .collect();
    let mut arrived = vec![None; walkers.len()];
    let dt = 0.25;
    let mut t = 0.;
    while t < seconds && walkers.iter().any(|w| !w.reached) {
        t += dt;
        for (i, walker) in walkers.iter_mut().enumerate() {
            let id = i as u16 + 1;
            if walker.reached {
                continue;
            }
            let start = walker.motion.position;
            let mut context = NavigationContext {
                state: walker.state,
                body,
                hp: 400,
                rules: None,
                generation: 1,
                hits: vec![],
                ground_events: vec![],
                sprint: false,
                facing: None,
                recovered_contacts: 0,
                actors: Some(Query {
                    grid: &grid,
                    actor: id,
                }),
                waiting_on: None,
                avoiding: false,
                swept_bounds: Some(Bounds::body(start, bot_physics::Body::default())),
                admission: DropAdmission::Unrestricted,
                waiting_drop: None,
                waited_seconds: 0.,
            };
            let step = walker
                .navigation
                .advance(
                    Some(world),
                    MAP,
                    walker.motion,
                    walker.waypoint,
                    dt,
                    dt,
                    Some(&mut context),
                )
                .unwrap_or_else(|e| panic!("bot {id} at {t} s: {e} {:?}", walker.motion));
            let swept = context.swept_bounds.unwrap();
            walker.state = context.state;
            walker.motion = step.motion;
            walker.waypoint = step.waypoint;
            if let Some(stop) = step.stopped {
                assert!(replans > 0, "bot {id} at {t} s: {stop} {:?}", step.motion);
                replans -= 1;
                let route = mesh.route(step.motion.position, trips[i].1, 0.3).unwrap();
                assert_eq!(route.status, navmesh::Status::Complete, "{id} {stop}");
                walker.navigation =
                    NavigationMove::from_complete_navmesh_route(mesh.clone(), route).unwrap();
                walker.waypoint = 1;
                walker.state = walker.state.for_new_move();
            }
            if step.reached {
                walker.reached = true;
                arrived[i] = Some(t);
            }
            grid.insert_reserved_path(id, &[start, step.motion.position], grid_body, swept)
                .unwrap();
        }
    }
    arrived
}

fn apart(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}

#[test]
fn a_head_on_capsule_pair_passes_each_other() {
    let (world, mesh) = arena(None);
    let arrived = run(
        &world,
        &mesh,
        &[([2., 0., 5.], [17., 0., 5.]), ([17., 0., 5.], [2., 0., 5.])],
        20.,
    );
    // Alone each walks 15 m at 4.5 m/s in about 3.5 s.
    for (i, t) in arrived.iter().enumerate() {
        assert!(t.is_some_and(|t| t < 8.), "bot {i}: {arrived:?}");
    }
}

#[test]
fn crossing_capsule_paths_both_get_through() {
    let (world, mesh) = arena(None);
    let arrived = run(
        &world,
        &mesh,
        &[
            ([5.8, 0., 5.], [17., 0., 5.]),
            ([10., 0., 0.8], [10., 0., 9.2]),
        ],
        20.,
    );
    for (i, t) in arrived.iter().enumerate() {
        assert!(t.is_some_and(|t| t < 8.), "bot {i}: {arrived:?}");
    }
}

#[test]
fn an_overlapping_spawn_pair_separates_and_both_proceed() {
    let (world, mesh) = arena(None);
    // Spawned 0.5 m apart: their 0.4 m capsules overlap by 0.3 m.
    let trips = [
        ([4., 0., 5.], [15., 0., 4.]),
        ([4.4, 0., 5.3], [15., 0., 6.]),
    ];
    assert!(apart(trips[0].0, trips[1].0) < 0.8);
    let arrived = run(&world, &mesh, &trips, 20.);
    for (i, t) in arrived.iter().enumerate() {
        assert!(t.is_some_and(|t| t < 8.), "bot {i}: {arrived:?}");
    }
}

#[test]
fn a_doorway_queue_of_capsule_bots_all_get_through() {
    let (world, mesh) = arena_with_mesh_door(Some(1.2), Some(1.6));
    let trips = [
        ([6., 0., 3.], [14., 0., 3.]),
        ([6., 0., 7.], [14., 0., 7.]),
        ([4., 0., 4.], [16., 0., 4.]),
        ([4., 0., 6.], [16., 0., 6.]),
        ([2., 0., 5.], [18., 0., 5.]),
    ];
    let arrived = run_replanning(&world, &mesh, &trips, 40., 20);
    for (i, t) in arrived.iter().enumerate() {
        assert!(t.is_some(), "bot {i} never got through: {arrived:?}");
    }
}

#[test]
fn opposing_capsule_bots_get_through_a_doorway_too_narrow_to_pass_in() {
    let (world, mesh) = arena_with_mesh_door(Some(1.2), Some(1.6));
    let arrived = run_replanning(
        &world,
        &mesh,
        &[([5., 0., 5.], [15., 0., 5.]), ([15., 0., 5.], [5., 0., 5.])],
        40.,
        20,
    );
    for (i, t) in arrived.iter().enumerate() {
        assert!(t.is_some(), "bot {i} never got through: {arrived:?}");
    }
}
