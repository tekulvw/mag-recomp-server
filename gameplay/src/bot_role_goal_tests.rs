//! SYNTHETIC role-destination regressions: a raised slab ("tower top") over
//! open ground, authored in code. No game catalogs, meshes or captures.
use super::*;
use sha2::{Digest, Sha256};

const TOWER_Y: f64 = 10.;

/// Open ground 30 m square with a 6 m slab 10 m above its middle, the
/// collision world and the navmesh baked from the same two quads.
fn tower() -> (collision::CollisionWorld, navmesh::NavMesh) {
    let quads: [[f64; 5]; 2] = [
        // x0, x1, y, z0, z1
        [0., 30., 0., 0., 30.],
        [10., 16., TOWER_Y, 10., 16.],
    ];
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for [x0, x1, y, z0, z1] in quads {
        let n = vertices.len() as u32;
        vertices.extend([[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]]);
        triangles.extend([[n, n + 2, n + 1], [n, n + 3, n + 2]]);
    }
    let (min, max) = ([0., -5., 0.], [30., 16., 30.]);
    let raw = serde_json::to_vec(&json!({
        "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_tower","build":"synthetic","coverage":"provisional",
            "provenance":["synthetic tower"],"unknowns":["Not a game map"],"physics_regions":[
                {"min":min,"max":max,"ground_verified":true,"obstacles_verified":true,
                 "provenance":["synthetic slab over open ground"]}]},
        "instances":[{"id":"tower","vertices_xyz":vertices,"triangles":triangles}]
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
        map_name: "synthetic_tower".into(),
        world_sha256: world.source_sha256().into(),
        geometry_sha256: format!("{:x}", hash.finalize()),
        bounds: navmesh::Bounds {
            min: [0., -1., 0.],
            max: [30., 14., 30.],
        },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices,
        triangles,
    })
    .unwrap();
    let mesh = navmesh::NavMesh::load(artifact, world.source_sha256(), "synthetic_tower").unwrap();
    (world, mesh)
}

fn on_mesh(mesh: &navmesh::NavMesh, goal: replication::CharacterPosition) -> bool {
    !mesh
        .goal_candidates(point(goal), ROLE_GOAL_TOLERANCE_M)
        .unwrap()
        .is_empty()
}

fn role(
    world: &collision::CollisionWorld,
    mesh: &navmesh::NavMesh,
    raw: [f64; 3],
) -> (
    Result<replication::CharacterPosition, &'static str>,
    Result<replication::CharacterPosition, &'static str>,
) {
    let policy = ObjectivePolicy::bundled();
    let body = bot_physics::Body::default();
    let raw = position(raw);
    (
        supported(Some(world), "synthetic_tower", raw, &policy, body),
        supported_role(
            Some(world),
            Some(mesh),
            "synthetic_tower",
            raw,
            &policy,
            body,
        ),
    )
}

#[test]
fn a_role_spot_at_tower_height_over_open_ground_falls_to_the_floor_below() {
    let (world, mesh) = tower();
    // An objective on the slab puts its role spots at the slab's height; this
    // one is beyond the slab, 10 m above the ground and out of search reach.
    let (direct, fallback) = role(&world, &mesh, [13., TOWER_Y + 0.5, 24.]);
    assert_eq!(direct, Err("bot_objective_unsupported"));
    let goal = fallback.unwrap();
    assert!(f64::from(goal.y).abs() < 0.2, "{goal:?}");
    assert!((f64::from(goal.x) - 13.).abs() < 0.2 && (f64::from(goal.z) - 24.).abs() < 0.2);
    assert!(on_mesh(&mesh, goal));
}

#[test]
fn a_supported_spot_beside_the_eroded_navmesh_moves_onto_it() {
    let (world, mesh) = tower();
    // 0.4 m inside the slab's edge: collision can hold a body there, but the
    // navmesh is eroded 0.6 m from the edge, so no route could end there.
    let raw = [15.6, TOWER_Y + 0.5, 13.];
    let (direct, fallback) = role(&world, &mesh, raw);
    if let Ok(direct) = direct {
        assert!(!on_mesh(&mesh, direct), "{direct:?}");
    }
    let goal = fallback.unwrap();
    assert!(
        (f64::from(goal.y) - TOWER_Y).abs() < 0.2,
        "stays on the slab: {goal:?}"
    );
    assert!(on_mesh(&mesh, goal));
    assert!(
        (f64::from(goal.x) - raw[0]).hypot(f64::from(goal.z) - raw[2]) < 1.,
        "{goal:?}"
    );
}

#[test]
fn a_spot_already_on_the_navmesh_is_unchanged() {
    let (world, mesh) = tower();
    for raw in [[5., 0.5, 5.], [13., TOWER_Y + 0.5, 13.]] {
        let (direct, fallback) = role(&world, &mesh, raw);
        assert!(on_mesh(&mesh, direct.unwrap()));
        assert_eq!(direct, fallback);
    }
}

#[test]
fn without_a_navmesh_the_collision_search_alone_decides() {
    let (world, _) = tower();
    let policy = ObjectivePolicy::bundled();
    let body = bot_physics::Body::default();
    let raw = position([13., TOWER_Y + 0.5, 24.]);
    assert_eq!(
        supported_role(Some(&world), None, "synthetic_tower", raw, &policy, body),
        Err("bot_objective_unsupported")
    );
}
