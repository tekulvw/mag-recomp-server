use mag_gameplay::{
    acquisition_atomic::WorldKey,
    acquisition_volume_damage::{Target, TargetKey, TargetKind},
    dynamic_collision::*,
};
fn target(id: u64, generation: u32) -> Target {
    Target {
        key: TargetKey { id, generation },
        kind: TargetKind::GroundVehicle,
        team: 0,
    }
}
#[path = "support/dynamic_collision_fixture.rs"]
mod support;
use support::fixture;

fn catalog() -> Catalog {
    Catalog::from_json(&fixture().to_string()).unwrap()
}
fn world() -> WorldKey {
    WorldKey {
        world_id: 1,
        incarnation: 2,
    }
}
fn query(center: [f64; 3], half_extents: [f64; 3]) -> QueryBox {
    QueryBox {
        pose: Pose::at(center),
        half_extents,
        radius: 0.05,
    }
}
fn snapshot(c: &Catalog, t: Target, p: Pose) -> Snapshot {
    Snapshot::capture(
        world(),
        4,
        &[t],
        vec![c.instantiate("invented-vehicle", t, p, false).unwrap()],
        u32::MAX,
    )
    .unwrap()
}
#[test]
fn real_convex_query_retains_both_registered_vehicle_roots() {
    let c = catalog();
    let t = target(1, 1);
    let s = snapshot(&c, t, Pose::at([0.; 3]));
    let hits = s
        .penetrations(query([0.; 3], [0.2, 1., 0.2]), QueryLimits::default())
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].root, "chassis");
    assert_eq!(hits[1].root, "wheel");
    assert_eq!(hits[0].target, hits[1].target);
    assert!(
        s.penetrations(query([10., 0., 0.], [1.; 3]), QueryLimits::default())
            .unwrap()
            .is_empty()
    );
}
#[test]
fn coverage_is_independent_of_geometry_rows_and_rejects_missing_extra_or_reused_entities() {
    let c = catalog();
    let t = target(1, 1);
    let make = || {
        c.instantiate("invented-vehicle", t, Pose::at([0.; 3]), false)
            .unwrap()
    };
    assert!(Snapshot::capture(world(), 4, &[t], vec![], u32::MAX).is_err());
    assert!(Snapshot::capture(world(), 4, &[], vec![make()], u32::MAX).is_err());
    assert!(Snapshot::capture(world(), 4, &[target(1, 2)], vec![make()], u32::MAX).is_err());
    assert!(Snapshot::capture(world(), 4, &[t], vec![make(), make()], u32::MAX).is_err());
    let newer = target(t.key.id, 2);
    assert!(
        Snapshot::capture(
            world(),
            4,
            &[t, newer],
            vec![
                make(),
                c.instantiate("invented-vehicle", newer, Pose::at([0.; 3]), false)
                    .unwrap()
            ],
            u32::MAX
        )
        .is_err(),
        "one live identity cannot carry two simultaneous generations"
    );
    assert!(
        c.instantiate("missing", t, Pose::at([0.; 3]), false)
            .is_err()
    );
}
#[test]
fn capsule_sphere_and_filters_use_current_state_without_dropping_dead_bodies() {
    let c = catalog();
    let character = Target {
        kind: TargetKind::Character,
        ..target(3, 1)
    };
    let mine = Target {
        kind: TargetKind::Mine,
        ..target(4, 1)
    };
    let rows = vec![
        c.instantiate("invented-character", character, Pose::at([0.; 3]), true)
            .unwrap(),
        c.instantiate("invented-mine", mine, Pose::at([3., 0., 0.]), false)
            .unwrap(),
    ];
    let s = Snapshot::capture(world(), 1, &[character, mine], rows, 1 << 6).unwrap();
    let hits = s
        .penetrations(query([0., 0.5, 0.], [4., 1., 1.]), QueryLimits::default())
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].target, character);
}
#[test]
fn rounded_margin_strict_boundary_and_work_exhaustion_do_not_return_partial_results() {
    let c = catalog();
    let s = snapshot(&c, target(1, 1), Pose::at([0.; 3]));
    assert!(
        !s.penetrations(query([1.14, 0., 0.], [0.; 3]), QueryLimits::default())
            .unwrap()
            .is_empty()
    );
    assert!(
        s.penetrations(query([1.16, 0., 0.], [0.; 3]), QueryLimits::default())
            .unwrap()
            .is_empty()
    );
    assert!(
        s.penetrations(query([1.15, 0., 0.], [0.; 3]), QueryLimits::default())
            .is_err()
    );
    assert!(
        s.penetrations(
            query([0.; 3], [1.; 3]),
            QueryLimits {
                max_support_points: 1,
                ..QueryLimits::default()
            }
        )
        .is_err()
    );
}
#[test]
fn current_pose_and_rotated_local_wheel_offset_change_the_query() {
    let c = catalog();
    let mut pose = Pose::at([5., 0., 0.]);
    pose.rotation = [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]];
    let s = snapshot(&c, target(1, 1), pose);
    let hits = s
        .penetrations(query([6.25, 0., 0.], [0.01; 3]), QueryLimits::default())
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].root, "wheel");
    assert!(
        s.penetrations(query([5., -1.25, 0.], [0.01; 3]), QueryLimits::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn convex_distance_matches_independent_sphere_and_axis_capsule_box_distance() {
    let c = catalog();
    let half = [0.6, 0.7, 0.5];
    let mut seed = 239u64;
    for kind in [TargetKind::Mine, TargetKind::Character] {
        for _ in 0..300 {
            let mut position = [0.; 3];
            for p in &mut position {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                *p = ((seed >> 32) % 6001) as f64 / 1000. - 3.;
            }
            let (template, radius, segment) = if kind == TargetKind::Mine {
                ("invented-mine", 0.2, 0.)
            } else {
                ("invented-character", 0.4, 1.)
            };
            let t = Target {
                kind,
                ..target(9, 1)
            };
            let row = c
                .instantiate(template, t, Pose::at(position), false)
                .unwrap();
            let s = Snapshot::capture(world(), 1, &[t], vec![row], u32::MAX).unwrap();
            let dx = (position[0].abs() - half[0]).max(0.);
            let dy = (position[1] - half[1])
                .max(-half[1] - (position[1] + segment))
                .max(0.);
            let dz = (position[2].abs() - half[2]).max(0.);
            let distance = (dx * dx + dy * dy + dz * dz).sqrt();
            let threshold = radius + 0.05;
            if (distance - threshold).abs() < 1e-7 {
                continue;
            }
            let hits = s
                .penetrations(query([0.; 3], half), QueryLimits::default())
                .unwrap();
            assert_eq!(
                !hits.is_empty(),
                distance < threshold,
                "{kind:?} {position:?} distance={distance}"
            );
        }
    }
}
