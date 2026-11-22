use super::*;
use sha2::{Digest, Sha256};

fn flat_corridor() -> (
    collision::CollisionWorld,
    Arc<navmesh::NavMesh>,
    navmesh::Route,
) {
    let vertices = [[0., 0., 0.], [20., 0., 0.], [20., 0., 10.], [0., 0., 10.]];
    let triangles = [[0u32, 2, 1], [0, 3, 2]];
    let min = [-1., -3., -1.];
    let max = [21., 7., 11.];
    let raw = serde_json::to_vec(&json!({
        "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_corridor","build":"synthetic","coverage":"provisional",
            "provenance":["synthetic flat floor"],"unknowns":[],"physics_regions":[
                {"min":min,"max":max,"ground_verified":true,"obstacles_verified":true,
                 "provenance":["synthetic isolated region"]}]},
        "instances":[{"id":"synthetic-floor","vertices_xyz":vertices,"triangles":triangles}]
    }))
    .unwrap();
    let world = collision::CollisionWorld::from_bytes(&raw, Default::default()).unwrap();
    let mut hash = Sha256::new();
    hash.update(b"MAG-NAV-GEOMETRY-v1\0");
    hash.update(4u32.to_le_bytes());
    hash.update(2u32.to_le_bytes());
    for x in vertices.into_iter().flatten() {
        hash.update((x as f32).to_le_bytes());
    }
    for i in triangles.into_iter().flatten() {
        hash.update(i.to_le_bytes());
    }
    let input = navmesh::BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: "synthetic_corridor".into(),
        world_sha256: world.source_sha256().into(),
        geometry_sha256: format!("{:x}", hash.finalize()),
        bounds: navmesh::Bounds { min, max },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices: vertices.map(|p| p.map(|x| x as f32)).to_vec(),
        triangles: triangles.to_vec(),
    };
    let profile = navmesh::MovementProfile::synthetic(54.);
    let artifact = navmesh::bake_with_movement_profile(input, &profile).unwrap();
    let mesh = Arc::new(
        navmesh::NavMesh::load_with_movement_profile(
            artifact,
            world.source_sha256(),
            "synthetic_corridor",
            &profile,
        )
        .unwrap(),
    );
    mesh.validate_coverage(&world).unwrap();
    let route = mesh.route([2., 0., 5.], [17., 0., 5.], 0.3).unwrap();
    assert_eq!(route.status, navmesh::Status::Complete);
    (world, mesh, route)
}

fn context<'a>(body: bot_physics::Body) -> NavigationContext<'a> {
    NavigationContext {
        state: NavmeshRuntime::default(),
        body,
        hp: 400,
        rules: None,
        generation: 1,
        hits: vec![],
        ground_events: vec![],
        sprint: false,
        facing: None,
        recovered_contacts: 0,
        actors: None,
        waiting_on: None,
        avoiding: false,
        swept_bounds: None,
        admission: DropAdmission::Permit {
            drop_id: None,
            entry_buffer_m: 1.5,
        },
        waiting_drop: None,
        waited_seconds: 0.,
    }
}

fn airborne_on(route: &navmesh::Route, height: f64, velocity: f64) -> bot_physics::Motion {
    let mut point = route.projected_start;
    point[0] += 0.5;
    point[1] += height;
    bot_physics::Motion {
        position: point,
        vertical_velocity: velocity,
        slide_velocity: [0.; 2],
        grounded: false,
        blocked: false,
    }
}

fn tick(
    navigation: &NavigationMove,
    world: &collision::CollisionWorld,
    motion: bot_physics::Motion,
    seconds: f64,
    context: &mut NavigationContext<'_>,
) -> Result<NavigationStep, &'static str> {
    navigation.advance(
        Some(world),
        "synthetic_corridor",
        motion,
        1,
        seconds,
        seconds,
        Some(context),
    )
}

#[test]
fn airborne_corridor_height_miss_settles_before_following_again() {
    let (world, mesh, route) = flat_corridor();
    let mut motion = airborne_on(&route, 1.125, -2.5);
    let waypoint = 1;
    let mut context = context(mesh.body());
    assert!(!mesh.contains(motion.position, &route));
    let mut surface = motion.position;
    surface[1] = route.projected_start[1];
    assert!(mesh.contains(surface, &route));
    let navigation = NavigationMove::from_complete_navmesh_route(mesh, route).unwrap();
    let start_xz = [motion.position[0], motion.position[2]];
    for _ in 0..10 {
        let step = tick(&navigation, &world, motion, 0.05, &mut context)
            .expect("airborne same-column miss must settle under actual gravity");
        assert_eq!(step.stopped, None);
        assert_eq!(step.waypoint, waypoint);
        assert_eq!([step.motion.position[0], step.motion.position[2]], start_xz);
        assert!(step.motion.position[1] < motion.position[1]);
        assert!(!step.reached);
        motion = step.motion;
        if motion.grounded {
            break;
        }
    }
    assert!(motion.grounded);
    assert_eq!(context.state.airborne_settle_seconds, 0.);
    let next = tick(&navigation, &world, motion, 0.05, &mut context).unwrap();
    assert!(next.motion.position[0] > motion.position[0]);
    assert_eq!(next.stopped, None);
}

#[test]
fn settlement_query_requires_current_corridor_below_feet_and_nearby_in_xz() {
    let (_, mesh, mut route) = flat_corridor();
    let mut point = airborne_on(&route, 1.125, -2.5).position;
    assert!(mesh.contains_settlement_column(point, &route));
    assert!(!mesh.contains(point, &route));
    assert!(!mesh.contains_settlement_column(airborne_on(&route, 1.6, -2.5).position, &route));
    assert!(!mesh.contains_settlement_column(airborne_on(&route, -0.4, -2.5).position, &route));
    point[2] = 10.;
    assert!(!mesh.contains_settlement_column(point, &route));
    point[2] = 5.;
    route.corridor_refs.fill(u64::MAX);
    assert!(!mesh.contains_settlement_column(point, &route));
}

#[test]
fn settlement_preserves_airborne_grounded_and_drop_admission_fences() {
    let (world, mesh, route) = flat_corridor();
    let motion = airborne_on(&route, 1.125, -2.5);
    let navigation =
        NavigationMove::from_complete_navmesh_route(mesh.clone(), route.clone()).unwrap();
    let mut cases = [
        motion,
        bot_physics::Motion {
            grounded: true,
            ..motion
        },
        bot_physics::Motion {
            slide_velocity: [0.2, 0.],
            ..motion
        },
        bot_physics::Motion {
            vertical_velocity: 0.2,
            ..motion
        },
    ];
    cases[0].position[2] = 10.;
    for case in cases {
        let mut context = context(mesh.body());
        assert!(matches!(
            tick(&navigation, &world, case, 0.05, &mut context),
            Err("bot_navmesh_off_corridor")
        ));
        assert_eq!(context.state.airborne_settle_seconds, 0.);
        assert!(context.ground_events.is_empty());
    }

    // A route with a future certified pair stays in its original admission lane.
    let mut route_with_drop = route;
    let goal = route_with_drop.projected_goal;
    route_with_drop.waypoints.extend([goal; 2]);
    route_with_drop.waypoint_kinds.extend([
        navmesh::WaypointKind::DropStart,
        navmesh::WaypointKind::DropEnd,
    ]);
    route_with_drop.waypoint_drop_ids.extend([Some(1); 2]);
    let navigation =
        NavigationMove::from_complete_navmesh_route(mesh.clone(), route_with_drop).unwrap();
    let mut context = context(mesh.body());
    assert!(matches!(
        tick(&navigation, &world, motion, 0.05, &mut context),
        Err("bot_navmesh_off_corridor")
    ));
    assert_eq!(context.state.airborne_settle_seconds, 0.);
    assert_eq!(context.state.drop_wait, Duration::ZERO);
    assert!(context.ground_events.is_empty());
}

#[test]
fn settlement_refuses_current_region_obstacles_even_without_horizontal_input() {
    let (world, mesh, route) = flat_corridor();
    let motion = airborne_on(&route, 1.125, -2.5);
    let navigation = NavigationMove::from_complete_navmesh_route(mesh.clone(), route).unwrap();
    let obstacle = crate::actor_collision::Bounds {
        min: [motion.position[0] - 0.2, 0., 4.8],
        max: [motion.position[0] + 0.2, 2., 5.2],
    };
    let unverified = collision::CollisionWorld::from_bytes(
        &serde_json::to_vec(&json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic_corridor","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic region unavailable"],"unknowns":[],"physics_regions":[]},
            "instances":[]
        }))
        .unwrap(),
        Default::default(),
    )
    .unwrap();
    let mut unverified_context = context(mesh.body());
    assert!(matches!(
        tick(
            &navigation,
            &unverified,
            motion,
            0.05,
            &mut unverified_context
        ),
        Err("bot_navigation_geometry_unavailable")
    ));
    assert_eq!(unverified_context.state.airborne_settle_seconds, 0.);
    let guarded = world.with_bot_regional_guard(bot_physics::RegionalGuard::new(
        crate::actor_collision::Bounds {
            min: [-1., -3., -1.],
            max: [21., 7., 11.],
        },
        &[obstacle],
    ));
    let mut context = context(mesh.body());
    assert!(matches!(
        tick(&navigation, &guarded, motion, 0.05, &mut context),
        Err("bot_regional_collision_blocked")
    ));
    assert_eq!(context.state.airborne_settle_seconds, 0.);
    assert!(context.ground_events.is_empty());
}

#[test]
fn settlement_time_limit_spans_updates_and_does_not_grant_drive_by_containment() {
    let (world, mesh, route) = flat_corridor();
    let mut motion = airborne_on(&route, 1.49, 0.);
    let mut context = context(mesh.body());
    let navigation =
        NavigationMove::from_complete_navmesh_route(mesh.clone(), route.clone()).unwrap();
    let origin = motion.position;
    for (seconds, expired) in [(0.25, false), (0.24, false), (0.25, true)] {
        let step = tick(&navigation, &world, motion, seconds, &mut context).unwrap();
        assert_eq!(step.waypoint, 1);
        assert!(!step.motion.grounded && !step.reached);
        assert_eq!(step.motion.position[0], origin[0]);
        assert_eq!(step.motion.position[2], origin[2]);
        assert_eq!(
            step.stopped,
            expired.then_some("bot_navmesh_airborne_settle_limit")
        );
        motion = step.motion;
    }
    assert!((context.state.airborne_settle_seconds - 0.5).abs() < 1e-9);
    assert!((motion.vertical_velocity + 0.5 * 9.81).abs() < 1e-9);
    assert!(mesh.contains(motion.position, &route));
    let stopped = tick(&navigation, &world, motion, 0.05, &mut context).unwrap();
    assert_eq!(stopped.stopped, Some("bot_navmesh_airborne_settle_limit"));
    assert_eq!(stopped.motion.position, motion.position);
    assert_eq!(context.state.for_new_move().airborne_settle_seconds, 0.);
}

#[test]
fn settlement_carries_lethal_landing_through_the_normal_stopped_step() {
    let (world, mesh, route) = flat_corridor();
    let motion = airborne_on(&route, 1.125, -2.5);
    let navigation = NavigationMove::from_complete_navmesh_route(mesh.clone(), route).unwrap();
    let mut context = context(mesh.body());
    let rules = fall_damage::Rules {
        schema_version: 1,
        source_build: "synthetic".into(),
        application_id: 20231,
        min_height: 0.,
        damage_factor: 200.,
        minimum_damage_exclusive: 0.,
        jump_fall_state: 1,
        damage_type: 5,
        damage_source_type: 1,
    };
    context.rules = Some(&rules);
    let final_step = tick(&navigation, &world, motion, 0.5, &mut context).unwrap();
    assert_eq!(final_step.stopped, Some("bot_navmesh_fall_lethal"));
    assert!(!final_step.reached);
    assert_eq!(final_step.waypoint, 1);
    assert!(final_step.motion.grounded);
    assert_eq!(context.hp, 0);
    assert_eq!(context.hits.len(), 1);
    assert_eq!(context.ground_events.len(), 1);
}

#[test]
fn settlement_cannot_consume_a_waypoint_on_another_actor() {
    let (world, mesh, route) = flat_corridor();
    let motion = airborne_on(&route, 1.125, -2.5);
    let mut context = context(mesh.body());
    let mut actors = crate::actor_collision::ActorGrid::default();
    let [x, _, z] = motion.position;
    actors
        .insert(
            2,
            crate::actor_collision::Bounds {
                min: [x - 0.35, 0., z - 0.35],
                max: [x + 0.35, 0.7, z + 0.35],
            },
        )
        .unwrap();
    context.actors = Some(crate::actor_collision::Query {
        grid: &actors,
        actor: 1,
    });
    let navigation = NavigationMove::from_complete_navmesh_route(mesh, route).unwrap();
    let landing = tick(&navigation, &world, motion, 0.5, &mut context).unwrap();
    assert!(landing.motion.grounded && context.state.on_player);
    assert_eq!(context.waiting_on, Some(2));
    assert_eq!(landing.waypoint, 1);
    assert!(!landing.reached);
    let next = tick(&navigation, &world, landing.motion, 0.05, &mut context).unwrap();
    assert!(!next.reached && context.state.on_player);
    assert_eq!(next.waypoint, landing.waypoint);
    assert_eq!([next.motion.position[0], next.motion.position[2]], [x, z]);
}

#[test]
fn late_owner_update_only_settles_within_remaining_route_time() {
    let (world, mesh, route) = flat_corridor();
    let motion = airborne_on(&route, 1.125, -2.5);
    let mut context = context(mesh.body());
    let navigation = NavigationMove::from_complete_navmesh_route(mesh, route).unwrap();
    let step = navigation
        .advance(
            Some(&world),
            "synthetic_corridor",
            motion,
            1,
            0.5,
            0.02,
            Some(&mut context),
        )
        .unwrap();
    assert_eq!(context.state.airborne_settle_seconds, 0.02);
    assert_eq!(step.waypoint, 1);
    assert!(!step.reached && !step.motion.grounded);
    assert!(
        (step.motion.vertical_velocity - (motion.vertical_velocity - 9.81 * 0.02)).abs() < 1e-9
    );
    let expired = navigation
        .advance(
            Some(&world),
            "synthetic_corridor",
            step.motion,
            step.waypoint,
            0.5,
            0.,
            Some(&mut context),
        )
        .unwrap();
    assert_eq!(expired.waypoint, step.waypoint);
    assert_eq!(expired.motion.position, step.motion.position);
    assert!(!expired.reached);
}
