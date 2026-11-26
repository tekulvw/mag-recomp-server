// SYNTHETIC: all inputs in this suite are authored in code; no game catalogs or captures.
use mag_gameplay::{collision::PhysicsRegion, navmesh::*};
use sha2::{Digest, Sha256};
use std::{fs, process::Command};

#[test]
fn reference_cli_bakes_full_grid_without_physics_authority() {
    let root = std::env::temp_dir().join(format!("mag-nav-reference-suite-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let input_path = root.join("input.json");
    let bundle = root.join("mesh");
    let mut input = wide_fixture(false);
    input.bounds.max[0] = 870.;
    input.bounds.max[2] = 820.;
    fs::write(
        &input_path,
        serde_json::to_vec(&serde_json::json!({
            "format":"mag_navmesh_reference_bake_input_v1","map_name":input.map_name,
            "world_sha256":input.world_sha256,"geometry_sha256":input.geometry_sha256,
            "bounds":input.bounds,"physics_regions":[],"vertices":input.vertices,"triangles":input.triangles
        }))
        .unwrap(),
    ).unwrap();
    let cli = || Command::new(env!("CARGO_BIN_EXE_mag-navmesh"));
    let output = cli()
        .arg("bake-reference")
        .arg("--input")
        .arg(&input_path)
        .arg("--output")
        .arg(&bundle)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inspect = cli()
        .arg("inspect-reference")
        .arg("--mesh")
        .arg(&bundle)
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(report["format"], "mag_navmesh_reference_v1");
    assert_eq!(report["tiles"], 3);

    let query = || {
        cli()
            .arg("query-reference")
            .arg("--mesh")
            .arg(&bundle)
            .args([
                "--start",
                "3,0,10",
                "--goal",
                "147,0,10",
                "--goal-radius",
                "0.2",
            ])
            .output()
            .unwrap()
    };
    let output = query();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let route: Route = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(route.format, "mag_navmesh_reference_route_v1");
    assert_eq!(route.status, Status::Complete);
    assert!(route.corridor_refs.len() >= 3);

    let manifest: ReferenceManifest =
        serde_json::from_slice(&fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
    let data_path = bundle.join("detour.bin");
    let mut data = fs::read(&data_path).unwrap();
    let mut runtime_artifact = Artifact {
        format: manifest.format,
        map_name: manifest.map_name,
        world_sha256: manifest.world_sha256,
        geometry_sha256: manifest.geometry_sha256,
        profile: manifest.profile,
        movement_profile_sha256: manifest.movement_profile_sha256,
        bounds: manifest.bounds,
        upstream_commit: manifest.upstream_commit,
        bake_config: manifest.bake_config,
        detour_sha256: manifest.detour_sha256,
        detour_data: data.clone(),
        drop_links: Vec::new(),
        character_movement_layers: None,
        body_shape: None,
    };
    assert!(matches!(
        NavMesh::load(runtime_artifact.clone(), &"0".repeat(64), "synthetic"),
        Err(Error("navmesh_identity_mismatch"))
    ));
    runtime_artifact.format = "mag_navmesh_world_v2".into();
    runtime_artifact.bake_config = BAKE_CONFIG.into();
    assert!(matches!(
        NavMesh::load(runtime_artifact, &"0".repeat(64), "synthetic"),
        Err(Error("navmesh_invalid_binary"))
    ));
    data[99] ^= 1;
    fs::write(data_path, data).unwrap();
    let corrupt = query();
    assert!(!corrupt.status.success());
    assert!(String::from_utf8_lossy(&corrupt.stderr).contains("navmesh_binary_hash_mismatch"));
}

#[test]
fn reference_bake_checks_envelope_grid_and_scope() {
    let mut input = fixture(false, false, false);
    input.physics_regions.clear();
    assert_eq!(
        bake(input).unwrap_err(),
        Error("navmesh_geometry_coverage_unknown")
    );
    let mut input = fixture(false, false, false);
    input.format = "mag_navmesh_reference_bake_input_v1".into();
    input.bounds.max[0] = 1500.;
    input.bounds.max[2] = 1500.;
    assert_eq!(
        bake_reference(input).unwrap_err(),
        Error("navmesh_reference_grid_limit")
    );
    let mut input = fixture(false, false, false);
    input.format = "mag_navmesh_reference_bake_input_v1".into();
    input.bounds.max[0] = 19.;
    assert_eq!(
        bake_reference(input).unwrap_err(),
        Error("navmesh_reference_envelope_invalid")
    );
}

#[test]
fn reference_face_culling_preserves_legacy_tile_payloads() {
    // The separate floor islands include faces entirely outside some tiles.
    let verified = bake(wide_fixture(true)).unwrap();
    let mut input = wide_fixture(true);
    input.format = "mag_navmesh_reference_bake_input_v1".into();
    input.physics_regions.clear();
    let (_, reference) = bake_reference(input).unwrap();
    assert_ne!(verified.detour_data[4..8], reference[4..8]);
    assert_eq!(verified.detour_data[8..], reference[8..]);
}

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

#[test]
fn long_supported_ground_route_is_not_rejected_as_a_query_length_limit() {
    let mut input = fixture(false, false, false);
    for v in &mut input.vertices {
        v[0] *= 20.;
    }
    input.bounds.max[0] = 400.;
    input.physics_regions[0].max[0] = 400.;
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
    let m = load(input);
    let rules = mag_gameplay::fall_damage::Rules {
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
    let result = m.route_with_drops([3., 0., 5.], [380., 0., 5.], 0.2, &rules, 400);
    assert!(
        result.is_ok(),
        "a complete bounded corridor must not fail only because it exceeds256m: {result:?}"
    );
    assert!(result.unwrap().length_m > 300.);
}

#[test]
fn corridor_prefix_is_bounded_continues_and_preserves_wall_detours() {
    let mesh = load(fixture(true, false, false));
    let full = mesh.route([3., 0., 5.], [16., 0., 5.], 0.2).unwrap();
    let segment = full.bounded_prefix(12.).unwrap();
    assert_eq!(segment.status, Status::Complete);
    assert!(segment.length_m <= 12.000001);
    assert!(segment.projected_goal[2] > 14.5);
    assert!(mesh.contains(segment.projected_goal, &segment));
    let next = mesh
        .route(segment.projected_goal, full.requested_goal, 0.2)
        .unwrap();
    assert_eq!(next.status, Status::Complete);
    assert!(next.length_m < full.length_m);
    assert!(next.waypoints.iter().any(|p| p[2] > 14.5));
    assert!(full.bounded_prefix(f64::NAN).is_err());
    let mut invalid = full.clone();
    invalid.status = Status::Partial;
    assert!(invalid.bounded_prefix(12.).is_err());
}

#[test]
fn corridor_prefix_never_cuts_an_airborne_drop() {
    let mesh = load(fixture(false, false, false));
    let mut full = mesh.route([3., 0., 5.], [16., 0., 5.], 0.2).unwrap();
    full.waypoints = vec![
        [3., 0., 5.],
        [5., 0., 5.],
        [6., 0., 5.],
        [10., -3., 5.],
        [10., -3., 5.],
        [16., -3., 5.],
    ];
    full.waypoint_kinds = vec![
        WaypointKind::Walk,
        WaypointKind::Walk,
        WaypointKind::DropStart,
        WaypointKind::DropEnd,
        WaypointKind::Walk,
        WaypointKind::Walk,
    ];
    full.waypoint_drop_ids = vec![None, None, Some(7), Some(7), None, None];
    full.length_m = 13.;
    let before = full.bounded_prefix(5.).unwrap();
    assert_eq!(before.projected_goal, [5., 0., 5.]);
    assert!(before.waypoint_drop_ids.iter().all(Option::is_none));
    let after = full.bounded_prefix(9.).unwrap();
    assert!(after.waypoint_kinds.contains(&WaypointKind::DropStart));
    assert!(after.waypoint_kinds.contains(&WaypointKind::DropEnd));
    assert_eq!(after.waypoint_kinds.last(), Some(&WaypointKind::Walk));
    assert!(after.length_m <= 9.000001);
}

#[test]
fn goal_candidates_filter_by_radius_and_validate_input() {
    let mesh = load(fixture(false, true, false));
    let candidates = mesh.goal_candidates([10., 0., 10.], 4.).unwrap();
    assert_eq!(candidates.len(), 2);
    assert_ne!(candidates[0].0, candidates[1].0);
    assert!(
        candidates
            .iter()
            .all(|(_, point)| point[0] <= 8. || point[0] >= 12.)
    );
    assert!(
        mesh.goal_candidates([10., 0., 10.], 0.5)
            .unwrap()
            .is_empty()
    );
    assert!(mesh.goal_candidates([f64::NAN, 0., 10.], 1.).is_err());
    assert!(mesh.goal_candidates([10., 0., 10.], 5.1).is_err());
}

fn geometry_sha256(vertices: &[[f32; 3]], triangles: &[[u32; 3]]) -> String {
    let mut h = Sha256::new();
    h.update(b"MAG-NAV-GEOMETRY-v1\0");
    h.update((vertices.len() as u32).to_le_bytes());
    h.update((triangles.len() as u32).to_le_bytes());
    for x in vertices.iter().flatten() {
        h.update(x.to_le_bytes());
    }
    for x in triangles.iter().flatten() {
        h.update(x.to_le_bytes());
    }
    format!("{:x}", h.finalize())
}

fn bundle_version(artifact: &Artifact) -> u32 {
    u32::from_le_bytes(artifact.detour_data[4..8].try_into().unwrap())
}

fn bundle_tiles(artifact: &Artifact) -> u32 {
    u32::from_le_bytes(artifact.detour_data[8..12].try_into().unwrap())
}

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "mag-nav-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, value: &serde_json::Value) -> std::path::PathBuf {
        let path = self.0.join(name);
        fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        path
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const WIDE_SIDE: f64 = 870.;

/// A synthetic square floor wider than the 256-tile grid (819.2 m), as a
/// collision world without its own physics regions plus a hash-bound sidecar.
struct WideWorld {
    _dir: TempDir,
    world: std::path::PathBuf,
    sidecar: std::path::PathBuf,
    world_sha256: String,
    region: PhysicsRegion,
}

fn wide_world() -> WideWorld {
    let dir = TempDir::new("wide-world");
    let side = WIDE_SIDE;
    let world = dir.write(
        "world.json",
        &serde_json::json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic_wide","build":"synthetic","coverage":"provisional",
                "provenance":["SYNTHETIC wide floor"],"unknowns":["Not a game map"]},
            "instances":[{"id":"floor","vertices_xyz":[[0.,0.,0.],[side,0.,0.],[side,0.,side],[0.,0.,side]],
                "triangles":[[0,2,1],[0,3,2]]}]
        }),
    );
    let world_sha256 = sha256(&fs::read(&world).unwrap());
    let region = PhysicsRegion {
        min: [0., -3., 0.],
        max: [side, 3., side],
        ground_verified: true,
        obstacles_verified: true,
        provenance: vec!["SYNTHETIC operator assertion".into()],
    };
    let sidecar = dir.write(
        "coverage.json",
        &serde_json::json!({
            "format":"mag_collision_coverage_v1","map_name":"synthetic_wide",
            "world_sha256":world_sha256,"basis":"operator_assertion",
            "provenance":["SYNTHETIC whole-world assertion"],"physics_regions":[region]
        }),
    );
    WideWorld {
        _dir: dir,
        world,
        sidecar,
        world_sha256,
        region,
    }
}

fn wide_input(world: &WideWorld) -> BakeInput {
    let side = WIDE_SIDE as f32;
    let vertices = vec![
        [0., 0., 0.],
        [side, 0., 0.],
        [side, 0., side],
        [0., 0., side],
    ];
    let triangles = vec![[0, 2, 1], [0, 3, 2]];
    BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: "synthetic_wide".into(),
        world_sha256: world.world_sha256.clone(),
        geometry_sha256: geometry_sha256(&vertices, &triangles),
        bounds: Bounds {
            min: world.region.min,
            max: world.region.max,
        },
        // prepare.py appends the sidecar's regions to the world's (here none).
        physics_regions: vec![world.region.clone()],
        vertices,
        triangles,
    }
}

#[test]
fn whole_map_world_bakes_more_than_256_tiles_and_loads_with_sidecar_coverage() {
    use mag_gameplay::{
        bot_physics::{Body, PhysicsError, WorldTerrain},
        collision::{CollisionLimits, CollisionWorld},
        collision_coverage::CoverageSidecar,
    };
    let fixture = wide_world();
    let input = wide_input(&fixture);
    assert_eq!(grid_tiles(&input.bounds), 17 * 17);
    let mut uncovered = wide_input(&fixture);
    uncovered.physics_regions.clear();
    assert_eq!(
        bake(uncovered).unwrap_err(),
        Error("navmesh_geometry_coverage_unknown")
    );
    let artifact = bake(input).unwrap();
    assert_eq!(artifact.format, "mag_navmesh_world_v4");
    assert_eq!(artifact.bake_config, WIDE_BAKE_CONFIG);
    assert_eq!(bundle_version(&artifact), 5);
    assert!(bundle_tiles(&artifact) > MAX_RUNTIME_TILES);
    // The runtime reads the serialized JSON artifact.
    let artifact: Artifact =
        serde_json::from_slice(&serde_json::to_vec(&artifact).unwrap()).unwrap();

    let world = CollisionWorld::load(&fixture.world, CollisionLimits::default()).unwrap();
    let mesh = NavMesh::load(artifact.clone(), world.source_sha256(), "synthetic_wide").unwrap();
    let route = mesh.route([3., 0., 3.], [866., 0., 866.], 0.5).unwrap();
    assert_eq!(route.status, Status::Complete);
    assert!(route.length_m > 1200.);
    assert!(mesh.contains([435., 0., 435.], &route));
    // Coverage at runtime comes from the loaded world plus its sidecar only.
    assert_eq!(
        mesh.validate_coverage(&world).unwrap_err(),
        Error("navmesh_geometry_coverage_unknown")
    );
    let body = Body::default();
    let terrain = WorldTerrain::new(Some(&world), "synthetic_wide").unwrap();
    assert!(matches!(
        terrain.require_horizontal_coverage([10., 0., 10.], [12., 0., 10.], body),
        Err(PhysicsError::CoverageUnknown)
    ));
    let covered = world
        .with_coverage_sidecar(CoverageSidecar::load(&fixture.sidecar).unwrap())
        .unwrap();
    assert_eq!(covered.source_sha256(), fixture.world_sha256);
    mesh.validate_coverage(&covered).unwrap();
    let terrain = WorldTerrain::new(Some(&covered), "synthetic_wide").unwrap();
    terrain
        .require_horizontal_coverage([10., 0., 10.], [12., 0., 10.], body)
        .unwrap();
    // The body corridor still has to fit inside the asserted region.
    assert!(matches!(
        terrain.require_horizontal_coverage([0.1, 0., 10.], [2., 0., 10.], body),
        Err(PhysicsError::CoverageUnknown)
    ));

    // A wide artifact cannot be relabelled as a 256-tile artifact or vice versa.
    let mut relabelled = artifact;
    relabelled.format = "mag_navmesh_world_v2".into();
    relabelled.bake_config = BAKE_CONFIG.into();
    assert_eq!(
        NavMesh::load(relabelled, world.source_sha256(), "synthetic_wide")
            .err()
            .unwrap(),
        Error("navmesh_invalid_binary")
    );
}

#[test]
fn small_bakes_keep_the_256_tile_format_and_wide_layout_is_explicitly_versioned() {
    let legacy = bake(wide_fixture(false)).unwrap();
    assert_eq!(legacy.format, "mag_navmesh_world_v2");
    assert_eq!(legacy.bake_config, BAKE_CONFIG);
    assert_eq!(bundle_version(&legacy), 1);
    let wide = bake_with_layout(wide_fixture(false), None, Layout::Wide).unwrap();
    assert_eq!(wide.format, "mag_navmesh_world_v4");
    assert_eq!(bundle_version(&wide), 5);
    // Face culling and the smaller polygon cap change no tile payload.
    assert_eq!(legacy.detour_data[8..], wide.detour_data[8..]);
    for artifact in [legacy.clone(), wide.clone()] {
        let mesh = NavMesh::load(artifact, &"0".repeat(64), "synthetic").unwrap();
        assert_eq!(
            mesh.route([3., 0., 10.], [147., 0., 10.], 0.2)
                .unwrap()
                .status,
            Status::Complete
        );
    }
    let mut relabelled = legacy;
    relabelled.format = wide.format.clone();
    relabelled.bake_config = wide.bake_config.clone();
    assert_eq!(
        NavMesh::load(relabelled, &"0".repeat(64), "synthetic")
            .err()
            .unwrap(),
        Error("navmesh_invalid_binary")
    );
}

#[test]
fn wide_grid_envelope_and_byte_caps_are_enforced() {
    let fixture = wide_world();
    let mut input = wide_input(&fixture);
    input.bounds.max[0] = 1200.;
    input.bounds.max[2] = 1200.;
    input.physics_regions[0].max = input.bounds.max;
    assert!(grid_tiles(&input.bounds) > u64::from(MAX_WIDE_RUNTIME_TILES));
    assert_eq!(bake(input).unwrap_err(), Error("navmesh_grid_limit"));
    let mut input = wide_input(&fixture);
    input.bounds.max[1] = 420.;
    input.physics_regions[0].max = input.bounds.max;
    assert_eq!(bake(input).unwrap_err(), Error("navmesh_envelope_invalid"));

    let small = bake_with_layout(wide_fixture(false), None, Layout::Wide).unwrap();
    let oversized = |mut artifact: Artifact, bytes: u64| {
        artifact.detour_data = vec![0; bytes as usize];
        artifact.detour_sha256 = sha256(&artifact.detour_data);
        NavMesh::load(artifact, &"0".repeat(64), "synthetic")
            .err()
            .unwrap()
    };
    // Over the wide cap: refused before the binary is parsed.
    assert_eq!(
        oversized(small.clone(), MAX_WIDE_ARTIFACT_BYTES + 1),
        Error("navmesh_binary_hash_mismatch")
    );
    // Over the 256-tile cap but within the wide cap: only the wide format parses it.
    assert_eq!(
        oversized(small, MAX_ARTIFACT_BYTES + 1),
        Error("navmesh_invalid_binary")
    );
    let legacy = bake(wide_fixture(false)).unwrap();
    assert_eq!(
        oversized(legacy, MAX_ARTIFACT_BYTES + 1),
        Error("navmesh_binary_hash_mismatch")
    );
    const { assert!(MAX_ARTIFACT_JSON_BYTES >= MAX_WIDE_ARTIFACT_BYTES * 4) };
}

#[test]
fn cli_bakes_wide_layout_and_binds_coverage_sidecar_to_world_queries() {
    let fixture = wide_world();
    let dir = TempDir::new("wide-cli");
    let mut input = wide_fixture(false);
    input.map_name = "synthetic_wide".into();
    input.world_sha256 = fixture.world_sha256.clone();
    input.bounds.min[1] = -3.;
    input.bounds.max[1] = 3.;
    input.bounds.max[2] = 20.;
    input.physics_regions = vec![fixture.region.clone()];
    let input_path = dir.write(
        "input.json",
        &serde_json::json!({
            "format":input.format,"map_name":input.map_name,"world_sha256":input.world_sha256,
            "geometry_sha256":input.geometry_sha256,"bounds":input.bounds,
            "physics_regions":input.physics_regions,"vertices":input.vertices,"triangles":input.triangles
        }),
    );
    let mesh = dir.0.join("mesh.mag-nav");
    let cli = || Command::new(env!("CARGO_BIN_EXE_mag-navmesh"));
    let rejected = cli()
        .args(["bake", "--layout", "tall", "--input"])
        .arg(&input_path)
        .arg("--output")
        .arg(&mesh)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    let baked = cli()
        .args(["bake", "--layout", "wide", "--input"])
        .arg(&input_path)
        .arg("--output")
        .arg(&mesh)
        .output()
        .unwrap();
    assert!(
        baked.status.success(),
        "{}",
        String::from_utf8_lossy(&baked.stderr)
    );
    let artifact: Artifact = serde_json::from_slice(&fs::read(&mesh).unwrap()).unwrap();
    assert_eq!(artifact.format, "mag_navmesh_world_v4");

    let goals = |coverage: Option<&std::path::Path>| {
        let mut command = cli();
        command
            .args(["goals", "--goal", "10,0,10", "--goal-radius", "1", "--mesh"])
            .arg(&mesh)
            .arg("--collision-world")
            .arg(&fixture.world);
        if let Some(coverage) = coverage {
            command.arg("--collision-coverage").arg(coverage);
        }
        command.output().unwrap()
    };
    let without = goals(None);
    assert!(!without.status.success());
    assert!(String::from_utf8_lossy(&without.stderr).contains("navmesh_geometry_coverage_unknown"));
    let with = goals(Some(&fixture.sidecar));
    assert!(
        with.status.success(),
        "{}",
        String::from_utf8_lossy(&with.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&with.stdout).unwrap();
    assert_eq!(report["world_sha256"], fixture.world_sha256);
    assert_eq!(report["candidates"].as_array().unwrap().len(), 1);

    let mut foreign =
        serde_json::from_slice::<serde_json::Value>(&fs::read(&fixture.sidecar).unwrap()).unwrap();
    foreign["world_sha256"] = serde_json::json!("f".repeat(64));
    let foreign = dir.write("foreign.json", &foreign);
    let mismatched = goals(Some(&foreign));
    assert!(!mismatched.status.success());
    assert!(
        String::from_utf8_lossy(&mismatched.stderr).contains("collision_coverage_world_mismatch")
    );
}
