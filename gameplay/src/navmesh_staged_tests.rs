// SYNTHETIC: every mesh here is authored in code; no game catalogs or captures.
use super::*;

const START: Point = [3., 0., 3.];
const GOAL: Point = [295., 0., 17.];

/// A 300 m by 24 m floor of 2 m cells with a regular pattern of holes. It
/// spans six tiles and yields many small polygons, so a search across it needs
/// more nodes than a deliberately small query pool holds.
fn perforated_strip() -> BakeInput {
    floor(12, |x, z| matches!(x % 5, 2 | 3) && z % 4 == 2)
}

/// A perforated 300 m by 60 m floor split along z = 30 m by a cut that is open
/// only past x = 250 m, so crossing it means walking to the far end and back.
fn split_field() -> BakeInput {
    floor(30, |x, z| {
        (z == 15 && x < 125) || (matches!(x % 5, 2 | 3) && z % 4 == 2)
    })
}

fn floor(rows: i32, hole: impl Fn(i32, i32) -> bool) -> BakeInput {
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for x in 0..150 {
        for z in 0..rows {
            if hole(x, z) {
                continue;
            }
            let (x, z) = ((2 * x) as f32, (2 * z) as f32);
            let n = vertices.len() as u32;
            vertices.extend([
                [x, 0., z],
                [x + 2., 0., z],
                [x + 2., 0., z + 2.],
                [x, 0., z + 2.],
            ]);
            triangles.extend([[n, n + 2, n + 1], [n, n + 3, n + 2]]);
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
    let (min, max) = ([0., -2., 0.], [300., 6., 2. * rows as f64]);
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
            provenance: vec!["synthetic floor".into()],
        }],
        vertices,
        triangles,
    }
}

fn strip(node_limit: c_int) -> NavMesh {
    static ARTIFACT: std::sync::OnceLock<Artifact> = std::sync::OnceLock::new();
    load(
        ARTIFACT.get_or_init(|| bake(perforated_strip()).unwrap()),
        node_limit,
    )
}

fn field(node_limit: c_int) -> NavMesh {
    static ARTIFACT: std::sync::OnceLock<Artifact> = std::sync::OnceLock::new();
    load(
        ARTIFACT.get_or_init(|| bake(split_field()).unwrap()),
        node_limit,
    )
}

fn load(artifact: &Artifact, node_limit: c_int) -> NavMesh {
    NavMesh::load_artifact(
        artifact.clone(),
        &"0".repeat(64),
        "synthetic",
        false,
        node_limit,
        None,
        false,
    )
    .unwrap()
}

/// A pool that a direct search across the strip exhausts while each staged
/// piece still fits.
const SMALL_POOL: c_int = 1024;

#[test]
fn a_route_needing_more_nodes_than_the_pool_completes_in_stages() {
    let full = strip(QUERY_NODE_LIMIT).route(START, GOAL, 0.5).unwrap();
    assert_eq!(full.status, Status::Complete);
    assert_eq!(full.stages, 0, "the default pool routes the strip directly");
    assert!(
        full.search_nodes > SMALL_POOL as u32,
        "the fixture must need more nodes than the small pool: {}",
        full.search_nodes
    );

    let mesh = strip(SMALL_POOL);
    let staged = mesh.route(START, GOAL, 0.5).unwrap();
    assert_eq!(staged.status, Status::Complete);
    assert!(staged.stages > 1, "{staged:?}");
    assert_eq!(staged.waypoints.first(), Some(&staged.projected_start));
    assert_eq!(staged.waypoints.last(), Some(&staged.projected_goal));
    assert_eq!(staged.projected_goal, full.projected_goal);
    // Pieces join inside locally shortest paths, not at fixed crossing points.
    assert!(
        staged.length_m <= full.length_m * 1.05,
        "staged {} m against direct {} m",
        staged.length_m,
        full.length_m
    );
    let mut unique = staged.corridor_refs.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        staged.corridor_refs.len(),
        "no corridor loops"
    );
    for waypoint in &staged.waypoints {
        assert!(mesh.contains(*waypoint, &staged), "{waypoint:?}");
    }
    let again = mesh.route(START, GOAL, 0.5).unwrap();
    assert_eq!(again.waypoints, staged.waypoints);
    assert_eq!(again.corridor_refs, staged.corridor_refs);
    // The bot follows a bounded prefix of a complete staged route.
    let prefix = staged.bounded_prefix(192.).unwrap();
    assert!(prefix.length_m <= 192.000001);
    assert!(mesh.contains(prefix.projected_goal, &staged));
}

#[test]
fn staged_search_work_is_bounded_by_four_pools() {
    let staged = strip(SMALL_POOL).route(START, GOAL, 0.5).unwrap();
    assert!(staged.search_nodes <= 4 * SMALL_POOL as u32);
    // With a pool too small for any piece the route keeps the direct partial
    // result and reports exhaustion, still within the same hard bound.
    for pool in [8, 24] {
        let route = strip(pool).route(START, GOAL, 0.5).unwrap();
        assert_eq!(route.status, Status::OutOfNodes, "pool {pool}");
        assert!(
            route.search_nodes <= 4 * pool as u32,
            "pool {pool}: {route:?}"
        );
        assert!(!route.corridor_refs.is_empty());
    }
}

#[test]
fn routes_within_the_pool_are_unchanged() {
    let default = strip(QUERY_NODE_LIMIT);
    let small = strip(SMALL_POOL);
    let goal = [30., 0., 21.];
    let a = default.route(START, goal, 0.5).unwrap();
    let b = small.route(START, goal, 0.5).unwrap();
    assert_eq!(a.status, Status::Complete);
    assert_eq!((a.stages, b.stages), (0, 0));
    assert!(b.search_nodes <= SMALL_POOL as u32);
    assert_eq!(a.waypoints, b.waypoints);
    assert_eq!(a.corridor_refs, b.corridor_refs);
    // Direct routes serialize exactly as before staged routing existed.
    let json = serde_json::to_value(&a).unwrap();
    assert!(json.get("stages").is_none() && json.get("search_nodes").is_none());
}

#[test]
fn drop_aware_routes_use_staged_ground_segments() {
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
    let route = strip(SMALL_POOL)
        .route_with_drops(START, GOAL, 0.5, &rules, 400)
        .unwrap();
    assert_eq!(route.status, Status::Complete);
    assert!(route.stages > 1);
    assert!(route.length_m > 290.);
}

#[test]
fn the_coarse_order_leads_staged_pieces_around_a_long_obstacle() {
    let (start, goal) = ([3., 0., 10.], [3., 0., 50.]);
    let full = field(QUERY_NODE_LIMIT).route(start, goal, 0.5).unwrap();
    assert_eq!(full.status, Status::Complete);
    assert_eq!(full.stages, 0);
    assert!(full.length_m > 490., "{}", full.length_m);
    let pool = 3072;
    assert!(full.search_nodes > pool as u32, "{}", full.search_nodes);
    let mesh = field(pool);
    let staged = mesh.route(start, goal, 0.5).unwrap();
    assert_eq!(staged.status, Status::Complete, "pool {pool}: {staged:?}");
    assert!(staged.stages > 1);
    assert!(staged.search_nodes <= 4 * pool as u32);
    assert!(staged.waypoints.iter().any(|p| p[0] > 250.));
    assert!(
        staged.length_m <= full.length_m * 1.05,
        "staged {} m against direct {} m",
        staged.length_m,
        full.length_m
    );
}

/// A perforated 300 m by 60 m floor cut into six 10 m lanes joined at
/// alternating ends, so a route from the first lane to the last snakes about
/// 1.8 km over far more polygons than a direct search's 1,024-polygon path.
fn serpentine() -> BakeInput {
    floor(30, |x, z| {
        let cut = z % 5 == 4 && z < 29;
        let open = if (z / 5) % 2 == 0 { x >= 145 } else { x < 5 };
        (cut && !open) || (matches!(x % 5, 2 | 3) && z % 4 == 2)
    })
}

#[test]
fn a_staged_route_carries_a_corridor_longer_than_a_direct_search() {
    static ARTIFACT: std::sync::OnceLock<Artifact> = std::sync::OnceLock::new();
    let mesh = load(
        ARTIFACT.get_or_init(|| bake(serpentine()).unwrap()),
        QUERY_NODE_LIMIT,
    );
    let (start, goal) = ([3., 0., 3.], [3., 0., 57.]);
    let route = mesh.route(start, goal, 0.5).unwrap();
    assert_eq!(route.status, Status::Complete, "{route:?}");
    assert!(route.stages > 1, "{}", route.stages);
    assert!(
        route.corridor_refs.len() > 1024,
        "the fixture must need a corridor over 1,024 polygons: {}",
        route.corridor_refs.len()
    );
    assert!(route.corridor_refs.len() <= MAX_ROUTE_CORRIDOR);
    assert!(route.length_m > 1500., "{}", route.length_m);
    for waypoint in &route.waypoints {
        assert!(mesh.contains(*waypoint, &route), "{waypoint:?}");
    }
}
