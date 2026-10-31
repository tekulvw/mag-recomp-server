// SYNTHETIC: all inputs in this suite are authored in code; no game catalogs or captures.
use mag_gameplay::{collision::PhysicsRegion, navmesh::*};
use sha2::{Digest, Sha256};
fn quad(
    vertices: &mut Vec<[f32; 3]>,
    triangles: &mut Vec<[u32; 3]>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    d: [f32; 3],
) {
    let n = vertices.len() as u32;
    vertices.extend([a, b, c, d]);
    triangles.extend([[n, n + 2, n + 1], [n, n + 3, n + 2]]);
}
fn fixture(wall: bool, gap: bool, upper: bool) -> BakeInput {
    let mut v = Vec::new();
    let mut t = Vec::new();
    quad(
        &mut v,
        &mut t,
        [0., 0., 0.],
        [if gap { 8. } else { 20. }, 0., 0.],
        [if gap { 8. } else { 20. }, 0., 20.],
        [0., 0., 20.],
    );
    if gap {
        quad(
            &mut v,
            &mut t,
            [12., 0., 0.],
            [20., 0., 0.],
            [20., 0., 20.],
            [12., 0., 20.],
        );
    }
    if wall {
        quad(
            &mut v,
            &mut t,
            [9., 0., 0.],
            [9., 4., 0.],
            [9., 4., 14.],
            [9., 0., 14.],
        );
        quad(
            &mut v,
            &mut t,
            [10., 0., 0.],
            [10., 0., 14.],
            [10., 4., 14.],
            [10., 4., 0.],
        );
        quad(
            &mut v,
            &mut t,
            [9., 4., 0.],
            [10., 4., 0.],
            [10., 4., 14.],
            [9., 4., 14.],
        );
    }
    if upper {
        quad(
            &mut v,
            &mut t,
            [1., 4., 1.],
            [19., 4., 1.],
            [19., 4., 19.],
            [1., 4., 19.],
        );
    }
    let mut h = Sha256::new();
    h.update(b"MAG-NAV-GEOMETRY-v1\0");
    h.update((v.len() as u32).to_le_bytes());
    h.update((t.len() as u32).to_le_bytes());
    for x in v.iter().flatten() {
        h.update(x.to_le_bytes());
    }
    for x in t.iter().flatten() {
        h.update(x.to_le_bytes());
    }
    BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: "synthetic".into(),
        world_sha256: "0".repeat(64),
        geometry_sha256: format!("{:x}", h.finalize()),
        bounds: Bounds {
            min: [0., -2., 0.],
            max: [20., 10., 20.],
        },
        physics_regions: vec![PhysicsRegion {
            min: [0., -2., 0.],
            max: [20., 10., 20.],
            ground_verified: true,
            obstacles_verified: true,
            provenance: vec!["complete synthetic fixture".into()],
        }],
        vertices: v,
        triangles: t,
    }
}
fn load(input: BakeInput) -> NavMesh {
    let a = bake(input).unwrap();
    NavMesh::load(a, &"0".repeat(64), "synthetic").unwrap()
}
#[test]
fn wall_routes_around_end_and_queries_are_deterministic() {
    let m = load(fixture(true, false, false));
    let a = m.route([3., 0., 5.], [16., 0., 5.], 0.2).unwrap();
    let b = m.route([3., 0., 5.], [16., 0., 5.], 0.2).unwrap();
    assert_eq!(a.status, Status::Complete);
    assert!(a.length_m > 20.);
    assert!(a.waypoints.iter().any(|p| p[2] > 14.5));
    assert_eq!(a.waypoints, b.waypoints);
    assert_eq!(a.corridor_refs, b.corridor_refs);
    assert!(m.contains([3., 0., 5.], &a));
    assert!(!m.contains([9.5, 0., 5.], &a));
}
#[test]
fn gap_is_explicitly_partial_or_disconnected() {
    let m = load(fixture(false, true, false));
    let r = m.route([3., 0., 5.], [16., 0., 5.], 0.2).unwrap();
    assert!(matches!(r.status, Status::Partial | Status::Disconnected));
    assert!(r.waypoints.last().unwrap()[0] < 8.);
    assert!(!m.contains([3., 0., 5.], &r));
}
#[test]
fn stacked_floors_remain_separate_layers() {
    let m = load(fixture(false, false, true));
    let r = m.route([3., 4., 5.], [16., 4., 5.], 0.2).unwrap();
    assert_eq!(r.status, Status::Complete);
    assert!(r.waypoints.iter().all(|p| (p[1] - 4.).abs() < 0.1));
    let wrong = m.route([3., 0., 5.], [16., 4., 5.], 0.2).unwrap();
    assert_ne!(wrong.status, Status::Complete);
}
#[test]
fn unknown_coverage_geometry_and_identity_are_rejected() {
    let mut input = fixture(false, false, false);
    input.physics_regions[0].obstacles_verified = false;
    assert_eq!(
        bake(input).unwrap_err(),
        Error("navmesh_geometry_coverage_unknown")
    );
    let mut input = fixture(false, false, false);
    input.vertices[0][0] = 1.;
    assert_eq!(
        bake(input).unwrap_err(),
        Error("navmesh_geometry_hash_mismatch")
    );
    let a = bake(fixture(false, false, false)).unwrap();
    assert!(NavMesh::load(a.clone(), &"1".repeat(64), "synthetic").is_err());
    let mut bad = a;
    bad.detour_data[0] ^= 1;
    assert!(NavMesh::load(bad, &"0".repeat(64), "synthetic").is_err());
}
#[test]
fn invalid_requests_and_missing_start_goal_are_explicit() {
    let m = load(fixture(false, false, false));
    assert!(m.route([f64::NAN, 0., 0.], [3., 0., 3.], 1.).is_err());
    assert_eq!(
        m.route([100., 0., 100.], [3., 0., 3.], 1.).unwrap().status,
        Status::InvalidStart
    );
    assert_eq!(
        m.route([3., 0., 3.], [100., 0., 100.], 1.).unwrap().status,
        Status::InvalidGoal
    );
}
#[test]
fn malformed_detour_layout_rejected_even_with_recomputed_checksum() {
    let a = bake(fixture(false, false, false)).unwrap();
    for offset in [24usize, 28, 32, 36, 40, 44, 48, 52, 56] {
        let mut bad = a.clone();
        // Skip the bounded bundle header and first tile length.
        bad.detour_data[44 + offset..48 + offset].copy_from_slice(&i32::MAX.to_le_bytes());
        bad.detour_sha256 = sha256(&bad.detour_data);
        assert!(
            NavMesh::load(bad, &"0".repeat(64), "synthetic").is_err(),
            "offset {offset}"
        );
    }
    let mut bad = a;
    bad.detour_data.truncate(bad.detour_data.len() - 4);
    bad.detour_sha256 = sha256(&bad.detour_data);
    assert!(NavMesh::load(bad, &"0".repeat(64), "synthetic").is_err());
}
#[test]
fn reachable_ground_within_goal_radius_is_chosen_over_disconnected_roof() {
    let m = load(fixture(false, false, true));
    let r = m.route([3., 0., 5.], [16., 4., 5.], 5.).unwrap();
    assert_eq!(r.status, Status::Complete);
    assert!(r.projected_goal[1] < 0.1);
    assert!((r.projected_goal[1] - 4.).abs() < 5.);
}

fn wide_fixture(disconnected: bool) -> BakeInput {
    let mut input = fixture(false, disconnected, false);
    // Three 51.2m tiles; the disconnected fixture retains a gap in the middle.
    for v in &mut input.vertices {
        v[0] *= 7.5;
    }
    input.bounds.max[0] = 150.;
    input.physics_regions[0].max[0] = 150.;
    let mut h = Sha256::new();
    h.update(b"MAG-NAV-GEOMETRY-v1\0");
    h.update((input.vertices.len() as u32).to_le_bytes());
    h.update((input.triangles.len() as u32).to_le_bytes());
    for x in input.vertices.iter().flatten() {
        h.update(x.to_le_bytes());
    }
    for x in input.triangles.iter().flatten() {
        h.update(x.to_le_bytes());
    }
    input.geometry_sha256 = format!("{:x}", h.finalize());
    input
}
#[test]
fn tiled_roundtrip_routes_across_portals_and_contains_seams() {
    let artifact = bake(wide_fixture(false)).unwrap();
    assert_eq!(artifact.format, "mag_navmesh_world_v2");
    assert_eq!(
        u32::from_le_bytes(artifact.detour_data[8..12].try_into().unwrap()),
        3
    );
    let json = serde_json::to_vec(&artifact).unwrap();
    let artifact: Artifact = serde_json::from_slice(&json).unwrap();
    let mesh = NavMesh::load(artifact.clone(), &"0".repeat(64), "synthetic").unwrap();
    let route = mesh.route([3., 0., 10.], [147., 0., 10.], 0.2).unwrap();
    assert_eq!(route.status, Status::Complete);
    assert!(route.corridor_refs.len() >= 3);
    assert!(mesh.contains([51.2, 0., 10.], &route));
    assert!(mesh.contains([102.4, 0., 10.], &route));
    assert_eq!(
        mesh.component_id([3., 0., 10.]).unwrap(),
        mesh.component_id([147., 0., 10.]).unwrap()
    );
    let second = NavMesh::load(artifact, &"0".repeat(64), "synthetic").unwrap();
    assert_eq!(
        mesh.component_id([147., 0., 10.]).unwrap(),
        second.component_id([147., 0., 10.]).unwrap()
    );
    assert_eq!(
        mesh.component_diagnostic([3., 0., 10.], [147., 0., 10.])
            .unwrap()["component_polygons"],
        mesh.component_diagnostic([3., 0., 10.], [147., 0., 10.])
            .unwrap()["total_polygons"]
    );
}
#[test]
fn tiled_disconnected_components_do_not_merge_by_polygon_index() {
    let mesh = load(wide_fixture(true));
    assert_ne!(
        mesh.component_id([3., 0., 10.]).unwrap(),
        mesh.component_id([147., 0., 10.]).unwrap()
    );
    assert_ne!(
        mesh.route([3., 0., 10.], [147., 0., 10.], 0.2)
            .unwrap()
            .status,
        Status::Complete
    );
}
#[test]
fn tiled_malformed_container_and_tile_bounds_are_rejected_with_valid_hash() {
    let original = bake(wide_fixture(false)).unwrap();
    for (offset, bytes) in [
        (8, 257u32.to_le_bytes()),
        (12, 1f32.to_le_bytes()),
        (40, u32::MAX.to_le_bytes()),
        (44 + 12, 99i32.to_le_bytes()),
        (44 + 72, f32::NAN.to_le_bytes()),
    ] {
        let mut artifact = original.clone();
        artifact.detour_data[offset..offset + 4].copy_from_slice(&bytes);
        artifact.detour_sha256 = sha256(&artifact.detour_data);
        assert!(
            NavMesh::load(artifact, &"0".repeat(64), "synthetic").is_err(),
            "offset {offset}"
        );
    }
    let mut artifact = original;
    artifact.detour_data.push(0);
    artifact.detour_sha256 = sha256(&artifact.detour_data);
    assert!(NavMesh::load(artifact, &"0".repeat(64), "synthetic").is_err());
}

#[test]
fn legacy_single_tile_bytes_remain_loadable() {
    let mut input = fixture(false, false, false);
    input.bounds.max[0] = 51.2;
    input.bounds.max[2] = 51.2;
    input.physics_regions[0].max = input.bounds.max;
    let mut artifact = bake(input).unwrap();
    // A bundle with one tile uses its first 40 bytes for metadata and four
    // bytes for the tile length. The tile itself is the legacy Detour layout.
    assert_eq!(
        u32::from_le_bytes(artifact.detour_data[8..12].try_into().unwrap()),
        1
    );
    artifact.detour_data.drain(..44);
    artifact.detour_sha256 = sha256(&artifact.detour_data);
    artifact.format = "mag_navmesh_world_v1".into();
    artifact.bake_config = LEGACY_BAKE_CONFIG.into();
    let mesh = NavMesh::load(artifact, &"0".repeat(64), "synthetic").unwrap();
    assert_eq!(
        mesh.route([3., 0., 10.], [17., 0., 10.], 0.2)
            .unwrap()
            .status,
        Status::Complete
    );
}
