// SYNTHETIC: every mesh here is authored in code; no game catalogs or captures.
use super::*;

/// Floors stacked 2.2 m apart, each a 14 m square perforated by a small hole
/// every 2 m. The holes break every floor into many small polygons, so a
/// 5 m goal box over the stack touches far more polygons than one fixed
/// 512-entry query buffer holds, while only a handful of walking components
/// (one per floor) are present.
fn perforated_stack(floors: &[f32]) -> BakeInput {
    const PITCH: f32 = 2.;
    const HOLE: f32 = 0.2;
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut quad = |x0: f32, x1: f32, y: f32, z0: f32, z1: f32| {
        let n = vertices.len() as u32;
        vertices.extend([[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]]);
        triangles.extend([[n, n + 2, n + 1], [n, n + 3, n + 2]]);
    };
    for &y in floors {
        for i in 0..7 {
            for j in 0..7 {
                let (x, z) = (-7. + PITCH * i as f32, -7. + PITCH * j as f32);
                // One tile minus a HOLE-sized square at its low corner.
                quad(x + HOLE, x + PITCH, y, z, z + PITCH);
                quad(x, x + HOLE, y, z + HOLE, z + PITCH);
            }
        }
    }
    let mut digest = Sha256::new();
    digest.update(b"MAG-NAV-GEOMETRY-v1\0");
    digest.update((vertices.len() as u32).to_le_bytes());
    digest.update((triangles.len() as u32).to_le_bytes());
    for x in vertices.iter().flatten() {
        digest.update(x.to_le_bytes());
    }
    for x in triangles.iter().flatten() {
        digest.update(x.to_le_bytes());
    }
    let (min, max) = ([-8., -10., -8.], [8., 12., 8.]);
    BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: "synthetic".into(),
        world_sha256: "0".repeat(64),
        geometry_sha256: format!("{:x}", digest.finalize()),
        bounds: Bounds { min, max },
        physics_regions: vec![crate::collision::PhysicsRegion {
            min,
            max,
            ground_verified: true,
            obstacles_verified: true,
            provenance: vec!["synthetic perforated floors".into()],
        }],
        vertices,
        triangles,
    }
}

fn stack() -> NavMesh {
    static ARTIFACT: std::sync::OnceLock<Artifact> = std::sync::OnceLock::new();
    let artifact = ARTIFACT
        .get_or_init(|| bake(perforated_stack(&[-6.6, -4.4, -2.2, 0., 2.2, 4.4, 6.6])).unwrap());
    NavMesh::load(artifact.clone(), &"0".repeat(64), "synthetic").unwrap()
}

#[test]
fn goal_candidates_cover_every_floor_of_a_dense_layered_stack() {
    let mesh = stack();
    let goal = [0.1, 0., 0.1];
    let candidates = mesh.goal_candidates(goal, 5.).unwrap();
    // Floors within 5 m of the goal: -4.4, -2.2, 0, 2.2 and 4.4.
    assert_eq!(candidates.len(), 5, "{candidates:?}");
    let mut heights: Vec<_> = candidates.iter().map(|(_, p)| p[1].round()).collect();
    heights.sort_by(f64::total_cmp);
    assert_eq!(heights, [-4., -2., 0., 2., 4.]);
    let own = mesh.component_id([0.7, 0., 0.7]).unwrap();
    let nearest = candidates.iter().find(|(c, _)| *c == own).unwrap().1;
    assert!(nearest[1].abs() < 0.2, "{nearest:?}");
}

#[test]
fn a_route_into_a_dense_layered_stack_projects_onto_the_nearest_polygon() {
    let mesh = stack();
    let start = [-4.6, 0., -4.6];
    let goal = [0.7, 0., 0.7];
    assert_eq!(mesh.component_id(start), mesh.component_id(goal));
    let exact = mesh.route(start, goal, 0.3).unwrap();
    assert_eq!(exact.status, Status::Complete);
    // A wide goal radius must still pick the nearest reachable polygon, not
    // whichever polygons happened to fit a fixed candidate buffer.
    let wide = mesh.route(start, goal, 5.).unwrap();
    assert_eq!(wide.status, Status::Complete);
    let error = (0..3)
        .map(|i| (wide.projected_goal[i] - exact.projected_goal[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    assert!(
        error < 0.05,
        "{:?} vs {:?}",
        wide.projected_goal,
        exact.projected_goal
    );
    // Drop-aware routing projects its goal the same way.
    let rules = crate::fall_damage::Rules {
        schema_version: 1,
        source_build: "synthetic".into(),
        application_id: 20231,
        min_height: 10.,
        damage_factor: 1.,
        minimum_damage_exclusive: 0.,
        jump_fall_state: 0,
        damage_type: 5,
        damage_source_type: 1,
    };
    let dropped = mesh.route_with_drops(start, goal, 5., &rules, 400).unwrap();
    assert_eq!(dropped.status, Status::Complete);
    assert_eq!(dropped.projected_goal, wide.projected_goal);
}

#[test]
fn column_candidates_reach_every_floor_of_a_tall_stack() {
    let mesh = stack();
    let goal = [0.7, 0., 0.7];
    // The 5 m sphere sees five floors; a 7 m column sees all seven, each
    // within the column's horizontal radius of the goal.
    let column = mesh.column_candidates(goal, 1., 7.).unwrap();
    let mut heights: Vec<_> = column.iter().map(|(_, p)| p[1].round()).collect();
    heights.sort_by(f64::total_cmp);
    assert_eq!(heights, [-7., -4., -2., 0., 2., 4., 7.]);
    assert!(
        column
            .iter()
            .all(|(_, p)| (p[0] - goal[0]).hypot(p[2] - goal[2]) <= 1.0001)
    );
    assert!(mesh.column_candidates(goal, 1., 40.).is_err());
}
