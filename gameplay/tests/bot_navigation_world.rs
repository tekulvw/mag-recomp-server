// SYNTHETIC: all inputs in this suite are authored in code; no game catalogs or captures.
use mag_gameplay::{
    bot_navigation::{Limits, PlanError, PlanningBounds, WorldNavigation, plan_route},
    bot_physics::Body,
    collision::{CollisionLimits, CollisionWorld},
};
use serde_json::json;
fn fixture_world(obstacles_verified: bool) -> CollisionWorld {
    let doc = json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
    "metadata":{"map_name":"synthetic-navigation","build":"synthetic","coverage":"provisional",
    "provenance":["Synthetic floor and wall fixture only"],"unknowns":["Not a game map"],
    "physics_regions":[{"min":[-10.,-5.,-10.],"max":[10.,8.,10.],"ground_verified":true,"obstacles_verified":obstacles_verified,"provenance":["Synthetic floor and wall bounded test region"]}]},
    "instances":[
        {"id":"floor","vertices_xyz":[[-10.,0.,-10.],[10.,0.,-10.],[10.,0.,10.],[-10.,0.,10.]],"triangles":[[0,2,1],[0,3,2]]},
        {"id":"wall","vertices_xyz":[[2.,0.,-1.],[2.,3.,-1.],[2.,3.,1.],[2.,0.,1.]],"triangles":[[0,1,2],[0,2,3]]}
    ]});
    let path = std::env::temp_dir().join(format!("mag-nav-world-{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    let world = CollisionWorld::load(&path, CollisionLimits::default()).unwrap();
    std::fs::remove_file(path).unwrap();
    world
}
fn bounds() -> PlanningBounds {
    PlanningBounds {
        min_xz: [-8., -8.],
        max_xz: [8., 8.],
    }
}
#[test]
fn supplied_world_adapter_detours_but_refuses_ground_only_or_uncovered_regions() {
    let world = fixture_world(true);
    let body = Body::default();
    let nav =
        WorldNavigation::new(Some(&world), "synthetic-navigation", body, bounds(), 2., 2.).unwrap();
    let limits = Limits {
        radius_cells: 6,
        ..Limits::default()
    };
    let route = plan_route(&nav, [0., 0., 0.], [4., 0., 0.], limits).unwrap();
    assert!(route.waypoints.iter().any(|p| p[2].abs() >= 2.));
    assert!(matches!(
        WorldNavigation::new(None, "synthetic-navigation", body, bounds(), 2., 2.),
        Err(PlanError::GeometryUnavailable)
    ));
    assert!(matches!(
        WorldNavigation::new(Some(&world), "wrong-map", body, bounds(), 2., 2.),
        Err(PlanError::GeometryUnavailable)
    ));
    assert_eq!(
        plan_route(&nav, [0., 0., 0.], [8., 0., 0.], Limits::default()),
        Err(PlanError::GeometryUnavailable)
    );
    assert!(matches!(
        WorldNavigation::new(
            Some(&world),
            "synthetic-navigation",
            body,
            bounds(),
            f64::NAN,
            2.
        ),
        Err(PlanError::InvalidRequest)
    ));
    let ground_only = fixture_world(false);
    let ground_nav = WorldNavigation::new(
        Some(&ground_only),
        "synthetic-navigation",
        body,
        bounds(),
        2.,
        2.,
    )
    .unwrap();
    assert_eq!(
        plan_route(&ground_nav, [0., 0., 0.], [4., 0., 0.], limits),
        Err(PlanError::GeometryUnavailable)
    );
    assert!(matches!(
        WorldNavigation::new(
            Some(&world),
            "synthetic-navigation",
            Body {
                radius: f64::NAN,
                ..body
            },
            bounds(),
            2.,
            2.
        ),
        Err(PlanError::InvalidRequest)
    ));
}
