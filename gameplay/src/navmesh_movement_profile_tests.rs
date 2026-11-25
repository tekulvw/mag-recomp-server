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
        body_slope_degrees: None,
    });
    assert!(matches!(
        load(with_drop),
        Err(Error("navmesh_profiled_drop_links_unsupported"))
    ));
}

#[test]
fn profiled_wide_layout_is_versioned_separately_and_requires_rules() {
    let rules = MovementProfile::synthetic(54.);
    let tiled = bake_with_movement_profile(ramp(false), &rules).unwrap();
    let wide = bake_with_layout(ramp(false), Some(&rules), Layout::Wide).unwrap();
    assert_eq!(wide.format, WIDE_PROFILED_RUNTIME_FORMAT);
    assert_eq!(wide.bake_config, WIDE_PROFILED_RUNTIME_CONFIG);
    assert_eq!(wide.detour_data[4..8], 7u32.to_le_bytes());
    assert_eq!(wide.detour_data[8..], tiled.detour_data[8..]);
    let world = wide.world_sha256.clone();
    let map = wide.map_name.clone();
    assert!(matches!(
        NavMesh::load(wide.clone(), &world, &map),
        Err(Error("navmesh_identity_mismatch"))
    ));
    assert_eq!(
        NavMesh::load_with_movement_profile(wide.clone(), &world, &map, &rules)
            .unwrap()
            .body()
            .max_slope_degrees,
        54.
    );
    let mut relabeled = wide;
    relabeled.detour_sha256 = tiled.detour_sha256;
    relabeled.detour_data = tiled.detour_data;
    assert!(matches!(
        NavMesh::load_with_movement_profile(relabeled, &world, &map, &rules),
        Err(Error("navmesh_invalid_binary"))
    ));
}

/// A 2 m ledge above a floor (synthetic), with a 47.7 degree ramp beside it
/// so the profiled and legacy bakes differ. Returns the world and the
/// matching bake input.
fn ledge() -> (crate::collision::CollisionWorld, BakeInput) {
    let quads: [[[f64; 3]; 4]; 3] = [
        [[0., 2., 0.], [8., 2., 0.], [8., 2., 10.], [0., 2., 10.]],
        [[8., 0., 0.], [30., 0., 0.], [30., 0., 10.], [8., 0., 10.]],
        [
            [8., 0., 10.],
            [12., 0., 10.],
            [12., 4.4, 14.],
            [8., 4.4, 14.],
        ],
    ];
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for quad in quads {
        let n = vertices.len() as u32;
        vertices.extend(quad);
        triangles.extend([[n, n + 2, n + 1], [n, n + 3, n + 2]]);
    }
    let (min, max) = ([-1., -2., -1.], [31., 8., 15.]);
    let raw = serde_json::to_vec(&serde_json::json!({
        "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
            "provenance":["synthetic ledge"],"unknowns":["Not a game map"],"physics_regions":[
                {"min":min,"max":max,"ground_verified":true,"obstacles_verified":true,
                 "provenance":["synthetic ledge over a floor"]}]},
        "instances":[{"id":"ledge","vertices_xyz":vertices,"triangles":triangles}]
    }))
    .unwrap();
    let world = crate::collision::CollisionWorld::from_bytes(&raw, Default::default()).unwrap();
    let vertices: Vec<[f32; 3]> = vertices.iter().map(|p| p.map(|v| v as f32)).collect();
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
    let input = BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: "synthetic".into(),
        world_sha256: world.source_sha256().into(),
        geometry_sha256: format!("{:x}", digest.finalize()),
        bounds: Bounds {
            min: [0., -1., 0.],
            max: [30., 6., 14.],
        },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices,
        triangles,
    };
    (world, input)
}

#[test]
fn a_profiled_mesh_accepts_drops_certified_with_its_own_body() {
    let (world, input) = ledge();
    let sha = world.source_sha256().to_owned();
    let rules = MovementProfile::synthetic(54.);
    let (launch, landing) = ([7.3, 2., 5.], [10.5, 0., 5.]);
    let legacy = bake(input).unwrap();
    let profiled = bake_with_movement_profile(ledge().1, &rules).unwrap();
    let legacy_body = NavMesh::load(legacy.clone(), &sha, "synthetic")
        .unwrap()
        .body();
    let profiled_body =
        NavMesh::load_with_movement_profile(profiled.clone(), &sha, "synthetic", &rules)
            .unwrap()
            .body();
    let default_link = DropLink::certify(&world, &sha, 1, launch, landing, 4.5).unwrap();
    let profiled_link =
        DropLink::certify_with_body(&world, &sha, 1, launch, landing, 4.5, profiled_body).unwrap();
    // The default body records nothing, so legacy certificates serialize
    // exactly as before; a profiled body records its slope.
    assert_eq!(default_link.body_slope_degrees, None);
    assert!(
        !serde_json::to_string(&default_link)
            .unwrap()
            .contains("body_slope")
    );
    assert_eq!(
        DropLink::certify_with_body(&world, &sha, 1, launch, landing, 4.5, legacy_body)
            .unwrap()
            .body_slope_degrees,
        None
    );
    assert_eq!(profiled_link.body_slope_degrees, Some(54.));

    let with = |mut artifact: Artifact, link: &DropLink| {
        artifact.drop_links.push(link.clone());
        artifact
    };
    let load_legacy = |artifact| NavMesh::load(artifact, &sha, "synthetic");
    let load_profiled =
        |artifact| NavMesh::load_with_movement_profile(artifact, &sha, "synthetic", &rules);
    assert!(load_legacy(with(legacy.clone(), &default_link)).is_ok());
    assert!(load_profiled(with(profiled.clone(), &profiled_link)).is_ok());
    assert!(matches!(
        load_profiled(with(profiled.clone(), &default_link)),
        Err(Error("navmesh_profiled_drop_links_unsupported"))
    ));
    assert!(matches!(
        load_legacy(with(legacy, &profiled_link)),
        Err(Error("navmesh_drop_body_mismatch"))
    ));
    let mut other_body = profiled_link;
    other_body.body_slope_degrees = Some(50.);
    assert!(matches!(
        load_profiled(with(profiled, &other_body)),
        Err(Error("navmesh_drop_body_mismatch"))
    ));
}
