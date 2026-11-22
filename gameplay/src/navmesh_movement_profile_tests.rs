use super::*;

fn ramp(reference: bool) -> BakeInput {
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for (start, end) in [
        ([0.0_f32, 0.], [4., 0.]),
        ([4., 0.], [8., 4.4]),
        ([8., 4.4], [12., 4.4]),
    ] {
        let n = vertices.len() as u32;
        vertices.extend([
            [start[0], start[1], 0.],
            [end[0], end[1], 0.],
            [end[0], end[1], 10.],
            [start[0], start[1], 10.],
        ]);
        triangles.extend([[n, n + 2, n + 1], [n, n + 3, n + 2]]);
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
    let min = [0., -2., 0.];
    let max = [20., 8., 20.];
    BakeInput {
        format: if reference {
            REFERENCE_INPUT
        } else {
            "mag_navmesh_bake_input_v1"
        }
        .into(),
        map_name: "synthetic".into(),
        world_sha256: "0".repeat(64),
        geometry_sha256: format!("{:x}", digest.finalize()),
        bounds: Bounds { min, max },
        physics_regions: if reference {
            Vec::new()
        } else {
            vec![crate::collision::PhysicsRegion {
                min,
                max,
                ground_verified: true,
                obstacles_verified: true,
                provenance: vec!["synthetic ramp".into()],
            }]
        },
        vertices,
        triangles,
    }
}

#[test]
fn authored_slope_profile_connects_a_synthetic_reference_ramp() {
    let start = [2., 0., 5.];
    let goal = [10., 4.4, 5.];
    let (manifest, data) = bake_reference(ramp(true)).unwrap();
    let mesh =
        ReferenceNavMesh::load(&manifest, data, &manifest.world_sha256, "synthetic").unwrap();
    assert_eq!(
        mesh.route(start, goal, 0.3).unwrap().status,
        Status::Disconnected
    );
    let rules = MovementProfile::synthetic(54.);
    let (manifest, data) = bake_reference_with_movement_profile(ramp(true), &rules).unwrap();
    assert_eq!(manifest.format, PROFILED_REFERENCE_FORMAT);
    assert_eq!(data[4..8], 4u32.to_le_bytes());
    assert!(
        ReferenceNavMesh::load(&manifest, data.clone(), &manifest.world_sha256, "synthetic")
            .is_err()
    );
    let mesh = ReferenceNavMesh::load_with_movement_profile(
        &manifest,
        data,
        &manifest.world_sha256,
        "synthetic",
        &rules,
    )
    .unwrap();
    let route = mesh.route(start, goal, 0.3).unwrap();
    assert_eq!(route.status, Status::Complete);
    assert_eq!(route.profile.max_slope_degrees, 54.);
    assert_eq!(route.format, "mag_navmesh_reference_route_v2");
}

#[test]
fn movement_profile_source_rejects_invented_or_inconsistent_rules() {
    let mut doc = serde_json::json!({
        "schema_version": 1, "application_id": 20231,
        "source": {
            "retail_elf_sha256": "1".repeat(64),
            "game_archive_sha256": "2".repeat(64),
            "character_zpr_sha256": "3".repeat(64),
        },
        "max_ascent_degrees": 53.0, "max_no_slide_degrees": 53.0
    });
    let parse =
        |doc: &serde_json::Value| MovementProfile::from_bytes(&serde_json::to_vec(doc).unwrap());
    assert!(matches!(
        parse(&doc),
        Err(Error("movement_profile_source_mismatch"))
    ));
    doc["max_no_slide_degrees"] = 52.0.into();
    assert!(matches!(
        parse(&doc),
        Err(Error("movement_profile_invalid_rules"))
    ));
    doc["max_ascent_degrees"] = 52.0.into();
    doc["unknown"] = 0.into();
    assert!(matches!(
        parse(&doc),
        Err(Error("movement_profile_invalid_document"))
    ));
}

#[test]
fn profiled_runtime_artifact_requires_rules_and_refuses_legacy_drop_certificates() {
    let rules = MovementProfile::synthetic(54.);
    let legacy = bake(ramp(false)).unwrap();
    let authored = bake_with_movement_profile(ramp(false), &rules).unwrap();
    let world = authored.world_sha256.clone();
    let map = authored.map_name.clone();
    assert_eq!(authored.format, PROFILED_RUNTIME_FORMAT);
    assert_eq!(
        authored.movement_profile_sha256.as_deref(),
        Some(rules.source_sha256())
    );
    assert_eq!(authored.detour_data[4..8], 3u32.to_le_bytes());
    assert!(matches!(
        NavMesh::load(authored.clone(), &world, &map),
        Err(Error("navmesh_identity_mismatch"))
    ));
    let load = |artifact| NavMesh::load_with_movement_profile(artifact, &world, &map, &rules);
    assert_eq!(
        load(authored.clone()).unwrap().body().max_slope_degrees,
        54.
    );
    let mut tampered = authored.clone();
    tampered.profile.max_slope_degrees = 45.;
    assert!(matches!(
        load(tampered),
        Err(Error("navmesh_identity_mismatch"))
    ));
    let mut relabeled = authored.clone();
    relabeled.detour_sha256 = legacy.detour_sha256;
    relabeled.detour_data = legacy.detour_data;
    assert!(matches!(
        load(relabeled),
        Err(Error("navmesh_invalid_binary"))
    ));
    let mut with_drop = authored;
    with_drop.drop_links.push(DropLink {
        id: 1,
        world_sha256: world.clone(),
        launch: [3., 1., 3.],
        landing: [4., 0., 3.],
        speed: 1.,
        max_drop_height: 1.,
        trajectory: Vec::new(),
        trajectory_sha256: "0".repeat(64),
    });
    assert!(matches!(
        load(with_drop),
        Err(Error("navmesh_profiled_drop_links_unsupported"))
    ));
}
