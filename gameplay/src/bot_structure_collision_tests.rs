// SYNTHETIC: every world, structure and navmesh here is authored in code; no
// game catalogs or captures.
use super::*;
use crate::bot_physics::{Body, Motion, WorldTerrain};
use crate::collision::{CollisionLimits, CollisionWorld};
use crate::domination_structural_world::SequenceCollisionState;
use crate::navmesh::{self, NavMesh, Status};
use serde_json::json;
use sha2::{Digest, Sha256};

/// Pack layer of the synthetic structures, and the character movement row:
/// it collides with that layer (and child layer 0), not with layer 11.
const PACK_LAYER: u8 = 9;
const VEHICLE_ONLY: u8 = 11;
const ROW: u32 = 1 | 1 << PACK_LAYER;

const FLOOR: [[f64; 3]; 4] = [[0., 0., 0.], [40., 0., 0.], [40., 0., 20.], [0., 0., 20.]];

fn floor_world() -> CollisionWorld {
    CollisionWorld::from_bytes(
        json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":["Not a game map"],"physics_regions":[
                    {"min":[-1.,-3.,-1.],"max":[41.,7.,21.],"ground_verified":true,
                     "obstacles_verified":true,"provenance":["synthetic floor"]}]},
            "instances":[{"id":"floor","vertices_xyz":FLOOR,"triangles":[[0,2,1],[0,3,2]]}]
        })
        .to_string()
        .as_bytes(),
        CollisionLimits::default(),
    )
    .unwrap()
}

fn floor_mesh(world: &CollisionWorld) -> NavMesh {
    static ARTIFACT: std::sync::OnceLock<navmesh::Artifact> = std::sync::OnceLock::new();
    let artifact = ARTIFACT.get_or_init(|| {
        let triangles = [[0u32, 2, 1], [0, 3, 2]];
        let mut hash = Sha256::new();
        hash.update(b"MAG-NAV-GEOMETRY-v1\0");
        hash.update(4u32.to_le_bytes());
        hash.update(2u32.to_le_bytes());
        for x in FLOOR.into_iter().flatten() {
            hash.update((x as f32).to_le_bytes());
        }
        for i in triangles.into_iter().flatten() {
            hash.update(i.to_le_bytes());
        }
        navmesh::bake(navmesh::BakeInput {
            format: "mag_navmesh_bake_input_v1".into(),
            map_name: "synthetic".into(),
            world_sha256: world.source_sha256().into(),
            geometry_sha256: format!("{:x}", hash.finalize()),
            bounds: navmesh::Bounds {
                min: [-1., -3., -1.],
                max: [41., 7., 21.],
            },
            physics_regions: world.metadata().physics_regions.clone(),
            vertices: FLOOR.map(|p| p.map(|x| x as f32)).to_vec(),
            triangles: triangles.to_vec(),
        })
        .unwrap()
    });
    NavMesh::load(artifact.clone(), world.source_sha256(), "synthetic").unwrap()
}

/// Axis-aligned solid boxes (min, max), local to the structure's pose.
fn boxes(parts: &[([f64; 3], [f64; 3])]) -> CollisionWorld {
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for (lo, hi) in parts {
        let n = vertices.len() as u32;
        for k in 0..8 {
            vertices.push([
                if k & 1 == 0 { lo[0] } else { hi[0] },
                if k & 2 == 0 { lo[1] } else { hi[1] },
                if k & 4 == 0 { lo[2] } else { hi[2] },
            ]);
        }
        for f in [
            [0, 1, 3, 2],
            [4, 6, 7, 5],
            [0, 4, 5, 1],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 5, 7, 3],
        ] {
            triangles.push([n + f[0], n + f[1], n + f[2]]);
            triangles.push([n + f[0], n + f[2], n + f[3]]);
        }
    }
    CollisionWorld::from_bytes(
        json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":[]},
            "instances":[{"id":"structure","vertices_xyz":vertices,"triangles":triangles}]
        })
        .to_string()
        .as_bytes(),
        CollisionLimits::default(),
    )
    .unwrap()
}

/// A closed gate across the whole floor at x = 20, 0.4 m thick, 3 m high,
/// optionally with an opening of `gap` metres centred on z = 10.
fn gate(gap: f64) -> CollisionWorld {
    if gap <= 0. {
        boxes(&[([-0.2, 0., -10.5], [0.2, 3., 10.5])])
    } else {
        boxes(&[
            ([-0.2, 0., -10.5], [0.2, 3., -gap * 0.5]),
            ([-0.2, 0., gap * 0.5], [0.2, 3., 10.5]),
        ])
    }
}

/// A 6 m square bunker shell centred on its pose: 0.3 m walls, 2.5 m high,
/// a roof, and a 1.6 m door in the west (-x) wall centred on z = 0.
fn bunker() -> CollisionWorld {
    boxes(&[
        ([-3., 0., -3.], [3., 2.5, -2.7]),
        ([-3., 0., 2.7], [3., 2.5, 3.]),
        ([2.7, 0., -3.], [3., 2.5, 3.]),
        ([-3., 0., -3.], [-2.7, 2.5, -0.8]),
        ([-3., 0., 0.8], [-2.7, 2.5, 3.]),
        ([-3., 2.5, -3.], [3., 2.8, 3.]),
    ])
}

fn snapshot(sequences: &SequenceCollisionState) -> Arc<StructureObstacles> {
    Arc::new(StructureObstacles::new(sequences, ROW))
}

fn with(world: &CollisionWorld, structures: &Arc<StructureObstacles>) -> CollisionWorld {
    let mut world = world.clone();
    world.set_bot_structures(Some(structures.clone()));
    world
}

/// Walk east at 4.5 m/s for three seconds from `start`; final position.
fn walk(world: &CollisionWorld, start: [f64; 3], velocity: [f64; 3]) -> [f64; 3] {
    let terrain = WorldTerrain::new(Some(world), "synthetic").unwrap();
    let body = Body::default();
    // Not `start_motion`: a structure may have appeared on an existing body.
    let mut motion = Motion::new([start[0], start[1] + 0.01, start[2]]);
    motion = terrain.advance(motion, [0.; 3], 0.5, body).unwrap();
    for _ in 0..60 {
        motion = terrain.advance(motion, velocity, 0.05, body).unwrap();
    }
    motion.position
}

#[test]
fn a_closed_gate_blocks_bot_movement_and_a_destroyed_one_does_not() {
    let floor = floor_world();
    let mut sequences = SequenceCollisionState::default();
    sequences.synthetic_structure_mesh(3, Pose::at([20., 0., 10.]), gate(0.), Some(PACK_LAYER));
    let closed = with(&floor, &snapshot(&sequences));
    let east = [4.5, 0., 0.];
    let end = walk(&closed, [15., 0., 10.], east);
    assert!(end[0] < 20. - 0.2 - 0.3, "walked into the gate: {end:?}");
    let terrain = WorldTerrain::new(Some(&closed), "synthetic").unwrap();
    assert!(
        !terrain
            .can_traverse([18., 0., 10.], [22., 0., 10.], Body::default())
            .unwrap()
    );

    // Destroying the gate stops its sequence: the road is open again.
    sequences.synthetic_remove_structure(3);
    let open = with(&floor, &snapshot(&sequences));
    let end = walk(&open, [15., 0., 10.], east);
    assert!(end[0] > 25., "{end:?}");
    let terrain = WorldTerrain::new(Some(&open), "synthetic").unwrap();
    assert!(
        terrain
            .can_traverse([18., 0., 10.], [22., 0., 10.], Body::default())
            .unwrap()
    );

    // A structure on a layer the movement row passes (vehicle-only) does not
    // stop a bot; one without a qualified binding is solid.
    for (tag, blocks) in [(Some(VEHICLE_ONLY), false), (None, true)] {
        let mut sequences = SequenceCollisionState::default();
        sequences.synthetic_structure_mesh(3, Pose::at([20., 0., 10.]), gate(0.), tag);
        let world = with(&floor, &snapshot(&sequences));
        let end = walk(&world, [15., 0., 10.], east);
        assert_eq!(end[0] < 20., blocks, "{tag:?}: {end:?}");
    }
}

#[test]
fn a_structure_placed_on_a_bot_does_not_trap_it() {
    let floor = floor_world();
    let mut sequences = SequenceCollisionState::default();
    // The gate appears where the bot already stands; it can step back out.
    sequences.synthetic_structure_mesh(3, Pose::at([20., 0., 10.]), gate(0.), Some(PACK_LAYER));
    let world = with(&floor, &snapshot(&sequences));
    let end = walk(&world, [20.1, 0., 10.], [-4.5, 0., 0.]);
    assert!(end[0] < 18., "{end:?}");
}

#[test]
fn a_bunker_shell_blocks_except_at_its_door() {
    let floor = floor_world();
    let mut sequences = SequenceCollisionState::default();
    sequences.synthetic_structure_mesh(5, Pose::at([30., 0., 10.]), bunker(), Some(PACK_LAYER));
    let world = with(&floor, &snapshot(&sequences));
    let terrain = WorldTerrain::new(Some(&world), "synthetic").unwrap();
    let body = Body::default();
    // Through the east, north and solid part of the west wall: blocked.
    for (from, to) in [
        ([30., 0., 10.], [35., 0., 10.]),
        ([30., 0., 10.], [30., 0., 15.]),
        ([29., 0., 8.], [25., 0., 8.]),
    ] {
        assert!(
            !terrain.can_traverse(from, to, body).unwrap(),
            "{from:?} -> {to:?}"
        );
    }
    // Out through the door.
    assert!(
        terrain
            .can_traverse([29., 0., 10.], [25., 0., 10.], body)
            .unwrap()
    );
    // The roof carries a body: ground under a point above the bunker is the roof.
    let ground = terrain
        .sample_ground([30., 3.5, 10.], 0.5, 4.)
        .unwrap()
        .unwrap();
    assert!((ground.position[1] - 2.8).abs() < 1e-6, "{ground:?}");
    assert!(ground.normal[1] > 0.99);
}

/// Every waypoint segment stays clear of the structures (a thin body swept
/// at knee height).
fn clear_of(route: &navmesh::Route, structures: &StructureObstacles) -> bool {
    route.waypoints.windows(2).all(|w| {
        let lift = |p: [f64; 3]| [p[0], p[1] + 0.9, p[2]];
        structures
            .sweep_aabb(lift(w[0]), lift(w[1]), [0.05, 0.4, 0.05], &mut 100_000)
            .unwrap()
            .is_none()
    })
}

#[test]
fn routes_stop_at_closed_gates_use_openings_and_reopen_when_destroyed() {
    let floor = floor_world();
    let mesh = floor_mesh(&floor);
    let (from, to) = ([5., 0., 10.], [35., 0., 10.]);
    assert_eq!(mesh.route(from, to, 0.5).unwrap().status, Status::Complete);

    let mut sequences = SequenceCollisionState::default();
    sequences.synthetic_structure_mesh(3, Pose::at([20., 0., 10.]), gate(0.), Some(PACK_LAYER));
    let closed = snapshot(&sequences);
    assert!(
        crate::navmesh_structure_gates::install_now(&mesh, Some(&closed))
            .unwrap()
            .unwrap()
            .0
            > 0
    );
    // Applying the same snapshot again is a no-op.
    assert_eq!(
        crate::navmesh_structure_gates::install_now(&mesh, Some(&closed)).unwrap(),
        None
    );
    let route = mesh.route(from, to, 0.5).unwrap();
    assert_eq!(route.status, Status::Partial, "{route:?}");
    let last = *route.waypoints.last().unwrap();
    assert!(last[0] < 20., "the route ends before the gate: {last:?}");
    assert!(clear_of(&route, &closed));

    // An opening in the gate: the route goes through it.
    let mut sequences = SequenceCollisionState::default();
    sequences.synthetic_structure_mesh(3, Pose::at([20., 0., 10.]), gate(2.), Some(PACK_LAYER));
    let opening = snapshot(&sequences);
    crate::navmesh_structure_gates::install_now(&mesh, Some(&opening)).unwrap();
    let around = mesh.route([5., 0., 3.], [35., 0., 3.], 0.5).unwrap();
    assert_eq!(around.status, Status::Complete, "{around:?}");
    let crossing = around
        .waypoints
        .windows(2)
        .find(|w| (w[0][0] - 20.) * (w[1][0] - 20.) <= 0.)
        .map(|w| {
            let t = (20. - w[0][0]) / (w[1][0] - w[0][0]);
            w[0][2] + (w[1][2] - w[0][2]) * t
        })
        .unwrap();
    assert!(
        (9.0..=11.0).contains(&crossing),
        "crossed the gate at z = {crossing}"
    );
    assert!(clear_of(&around, &opening), "{:?}", around.waypoints);

    // Destroyed: every gate is removed and the straight road is back.
    sequences.synthetic_remove_structure(3);
    crate::navmesh_structure_gates::install_now(&mesh, Some(&snapshot(&sequences))).unwrap();
    let open = mesh.route(from, to, 0.5).unwrap();
    assert_eq!(open.status, Status::Complete);
    assert_eq!(open.waypoints.len(), 2, "{open:?}");
}

#[test]
fn routes_enter_and_leave_a_bunker_only_through_its_door() {
    let floor = floor_world();
    let mesh = floor_mesh(&floor);
    let mut sequences = SequenceCollisionState::default();
    sequences.synthetic_structure_mesh(5, Pose::at([30., 0., 10.]), bunker(), Some(PACK_LAYER));
    let structures = snapshot(&sequences);
    crate::navmesh_structure_gates::install_now(&mesh, Some(&structures)).unwrap();
    let door_x = 30. - 2.85;
    let through_door = |route: &navmesh::Route| {
        route.waypoints.windows(2).any(|w| {
            (w[0][0] - door_x) * (w[1][0] - door_x) <= 0. && {
                let t = (door_x - w[0][0]) / (w[1][0] - w[0][0]);
                (w[0][2] + (w[1][2] - w[0][2]) * t - 10.).abs() <= 0.8
            }
        })
    };
    // Into the bunker from the west and from the east (around it).
    for start in [[5., 0., 10.], [38., 0., 10.]] {
        let route = mesh.route(start, [30., 0., 10.], 0.5).unwrap();
        assert_eq!(route.status, Status::Complete, "{start:?}: {route:?}");
        assert!(through_door(&route), "{start:?}: {:?}", route.waypoints);
        assert!(
            clear_of(&route, &structures),
            "{start:?}: {:?}",
            route.waypoints
        );
    }
    // Out of it to the east: through the door, then around.
    let out = mesh.route([30., 0., 10.], [38., 0., 10.], 0.5).unwrap();
    assert_eq!(out.status, Status::Complete, "{out:?}");
    assert!(through_door(&out), "{:?}", out.waypoints);
    assert!(clear_of(&out, &structures), "{:?}", out.waypoints);
    // Clearing the snapshot removes the gates.
    crate::navmesh_structure_gates::install_now(&mesh, None).unwrap();
    assert_eq!(mesh.structure_gate_key(), None);
}

/// The bunker without its door: a closed room.
fn closed_bunker() -> CollisionWorld {
    boxes(&[
        ([-3., 0., -3.], [3., 2.5, -2.7]),
        ([-3., 0., 2.7], [3., 2.5, 3.]),
        ([2.7, 0., -3.], [3., 2.5, 3.]),
        ([-3., 0., -3.], [-2.7, 2.5, 3.]),
        ([-3., 2.5, -3.], [3., 2.8, 3.]),
    ])
}

#[test]
fn a_life_started_in_a_closed_room_walks_out_and_nobody_walks_in() {
    let floor = floor_world();
    let mesh = floor_mesh(&floor);
    let mut sequences = SequenceCollisionState::default();
    sequences.synthetic_structure_mesh(
        6,
        Pose::at([30., 0., 10.]),
        closed_bunker(),
        Some(PACK_LAYER),
    );
    let structures = snapshot(&sequences);
    let world = with(&floor, &structures);
    // Before the room is analysed the shell holds the body.
    assert!(!structures.enclosed([30., 0., 10.]));
    assert!(analyse_interiors(&structures) >= 1);
    assert!(structures.enclosed([30., 0., 10.]));
    assert!(!structures.enclosed([24., 0., 10.]));
    // Inside: walks out west through the wall.
    let out = walk(&world, [30., 0., 10.], [-4.5, 0., 0.]);
    assert!(out[0] < 25., "{out:?}");
    // Outside: the wall still stops a body walking in.
    let blocked = walk(&world, [24., 0., 10.], [4.5, 0., 0.]);
    assert!(blocked[0] < 27. - 0.3, "{blocked:?}");
    // Routes from the room use the static mesh; routes into it end outside.
    crate::navmesh_structure_gates::install_now(&mesh, Some(&structures)).unwrap();
    let leave = mesh.route([30., 0., 10.], [5., 0., 10.], 0.5).unwrap();
    assert_eq!(leave.status, Status::Complete, "{leave:?}");
    let enter = mesh.route([5., 0., 10.], [30., 0., 10.], 0.5).unwrap();
    assert_eq!(enter.status, Status::Partial, "{enter:?}");
}

#[test]
fn unchanged_structures_keep_their_polygon_gates() {
    let floor = floor_world();
    let mesh = floor_mesh(&floor);
    let mut sequences = SequenceCollisionState::default();
    sequences.synthetic_structure_mesh(3, Pose::at([10., 0., 10.]), gate(0.), Some(PACK_LAYER));
    sequences.synthetic_structure_mesh(5, Pose::at([30., 0., 10.]), bunker(), Some(PACK_LAYER));
    let (gated, reused) =
        crate::navmesh_structure_gates::install_now(&mesh, Some(&snapshot(&sequences)))
            .unwrap()
            .unwrap();
    assert!(gated > 0);
    assert_eq!(reused, 0);
    // Destroying the gate re-gates only the polygons it touched.
    sequences.synthetic_remove_structure(3);
    let (after, reused) =
        crate::navmesh_structure_gates::install_now(&mesh, Some(&snapshot(&sequences)))
            .unwrap()
            .unwrap();
    assert!(
        after < gated && reused > 0 && reused <= after,
        "{gated} {after} {reused}"
    );
}

#[test]
fn a_wall_crossing_a_polygon_edge_does_not_seal_the_route_beside_it() {
    // The floor's polygons meet along x = 20. A wall along z = 10 from x = 14
    // to the east edge crosses that edge, so both of the east polygon's
    // regions (north and south of the wall) touch it. A route that stays
    // south of the wall passes; one that has to cross the wall does not.
    let floor = floor_world();
    let mesh = floor_mesh(&floor);
    let mut sequences = SequenceCollisionState::default();
    sequences.synthetic_structure_mesh(
        7,
        Pose::at([27., 0., 10.]),
        boxes(&[([-13., 0., -0.2], [13.5, 3., 0.2])]),
        Some(PACK_LAYER),
    );
    let structures = snapshot(&sequences);
    crate::navmesh_structure_gates::install_now(&mesh, Some(&structures)).unwrap();
    for (from, to) in [
        ([16., 0., 5.], [24., 0., 5.]),
        ([16., 0., 15.], [36., 0., 15.]),
    ] {
        let beside = mesh.route(from, to, 0.5).unwrap();
        assert_eq!(beside.status, Status::Complete, "{from:?}: {beside:?}");
        assert!(clear_of(&beside, &structures), "{:?}", beside.waypoints);
    }
    // Around the wall's west end is the only way to the other side.
    let around = mesh.route([16., 0., 5.], [36., 0., 15.], 0.5).unwrap();
    assert_eq!(around.status, Status::Complete, "{around:?}");
    assert!(
        around.waypoints.iter().any(|p| p[0] < 14.),
        "{:?}",
        around.waypoints
    );
    assert!(clear_of(&around, &structures), "{:?}", around.waypoints);
}

#[test]
fn only_a_beside_route_reaches_a_walled_off_goal_beside_it() {
    // A goal inside a closed shell (a console a charge was planted on, a
    // sealed room): a beside route (a spawn's lane check) with a goal radius
    // that reaches past the shell ends at the nearest reachable free spot
    // within that radius. An ordinary route (a bot walking to defuse) stays
    // partial, so its caller keeps its own approach to the trigger.
    let floor = floor_world();
    let mesh = floor_mesh(&floor);
    let mut sequences = SequenceCollisionState::default();
    sequences.synthetic_structure_mesh(
        8,
        Pose::at([30., 0., 10.]),
        closed_bunker(),
        Some(PACK_LAYER),
    );
    let structures = snapshot(&sequences);
    crate::navmesh_structure_gates::install_now(&mesh, Some(&structures)).unwrap();
    let goal = [30., 0., 10.];
    // An approach that must stand at the goal (an interaction range) is not
    // told it arrived outside the shell.
    let approach = mesh.route([5., 0., 10.], goal, 5.).unwrap();
    assert_eq!(approach.status, Status::Partial, "{approach:?}");
    let route = mesh.route_beside([5., 0., 10.], goal, 5.).unwrap();
    assert_eq!(route.status, Status::Complete, "{route:?}");
    let end = *route.waypoints.last().unwrap();
    let away = (end[0] - goal[0]).hypot(end[2] - goal[2]);
    assert!(away <= 5. && away > 3., "{end:?}");
    assert!(clear_of(&route, &structures), "{:?}", route.waypoints);
    // A goal radius that ends inside the shell still stops outside it.
    let short = mesh.route_beside([5., 0., 10.], goal, 0.5).unwrap();
    assert_eq!(short.status, Status::Partial, "{short:?}");
}

/// What a route found, without its timing.
fn found(route: &navmesh::Route) -> String {
    format!(
        "{:?} {:?} {:?}",
        route.status, route.waypoints, route.corridor_refs
    )
}

/// Requests whose routes differ between a closed gate and a gate with an
/// opening, routed with each start in and out of the structure's reach.
const GATE_REQUESTS: [([f64; 3], [f64; 3]); 4] = [
    ([5., 0., 10.], [35., 0., 10.]),
    ([5., 0., 3.], [35., 0., 3.]),
    ([35., 0., 17.], [5., 0., 12.]),
    ([18., 0., 2.], [22., 0., 18.]),
];

/// A closed gate and one with an opening, as structure snapshots.
fn closed_and_open_gates() -> [Arc<StructureObstacles>; 2] {
    [0., 2.].map(|gap| {
        let mut sequences = SequenceCollisionState::default();
        sequences.synthetic_structure_mesh(
            3,
            Pose::at([20., 0., 10.]),
            gate(gap),
            Some(PACK_LAYER),
        );
        snapshot(&sequences)
    })
}

/// Each request's route under the gates installed now.
fn routes_now(mesh: &NavMesh) -> Vec<String> {
    GATE_REQUESTS
        .iter()
        .map(|&(from, to)| found(&mesh.route(from, to, 0.5).unwrap()))
        .collect()
}

#[test]
fn routes_during_gate_changes_match_the_routes_of_one_gate_set() {
    let floor = floor_world();
    let mesh = Arc::new(floor_mesh(&floor));
    let gates = closed_and_open_gates();
    let serial = gates.clone().map(|structures| {
        crate::navmesh_structure_gates::install_now(&mesh, Some(&structures)).unwrap();
        routes_now(&mesh)
    });
    assert_ne!(serial[0], serial[1]);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let routers: Vec<_> = (0..4)
        .map(|_| {
            let (mesh, stop, serial) = (mesh.clone(), stop.clone(), serial.clone());
            std::thread::spawn(move || {
                let mut seen = [0usize; 2];
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    for (i, &(from, to)) in GATE_REQUESTS.iter().enumerate() {
                        let route = found(&mesh.route(from, to, 0.5).unwrap());
                        let set = serial.iter().position(|routes| routes[i] == route);
                        // A route uses one gate set throughout, never a mix.
                        seen[set.unwrap_or_else(|| panic!("request {i}: {route}"))] += 1;
                    }
                }
                seen
            })
        })
        .collect();
    for change in 0..60 {
        crate::navmesh_structure_gates::install_now(&mesh, Some(&gates[change % 2])).unwrap();
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for router in routers {
        router.join().expect("a concurrent route differed");
    }
    assert!(mesh.query_contexts().0 >= 2, "the routes ran at once");
}

#[test]
fn a_route_started_after_gates_change_on_another_thread_uses_the_new_gates() {
    let floor = floor_world();
    let mesh = Arc::new(floor_mesh(&floor));
    let gates = closed_and_open_gates();
    let serial = gates.clone().map(|structures| {
        crate::navmesh_structure_gates::install_now(&mesh, Some(&structures)).unwrap();
        routes_now(&mesh)
    });
    let (changed, inbox) = std::sync::mpsc::channel::<usize>();
    let (answer, answers) = std::sync::mpsc::channel();
    let router = {
        let mesh = mesh.clone();
        std::thread::spawn(move || {
            for set in inbox {
                answer.send((set, routes_now(&mesh))).unwrap();
            }
        })
    };
    // The installer runs on this thread, the routes on the other.
    for change in 0..20 {
        let set = change % 2;
        crate::navmesh_structure_gates::install_now(&mesh, Some(&gates[set])).unwrap();
        changed.send(set).unwrap();
        let (routed, routes) = answers.recv().unwrap();
        assert_eq!(routes, serial[routed], "change {change}");
    }
    drop(changed);
    router.join().unwrap();
}
