// SYNTHETIC: the mesh is baked here from a square authored in code.
use super::*;
use std::path::Path;

/// A 10 m square floor baked for the world digest `world`.
fn floor(world: &str) -> Artifact {
    let vertices: Vec<[f32; 3]> = vec![[0., 0., 0.], [10., 0., 0.], [10., 0., 10.], [0., 0., 10.]];
    let triangles: Vec<[u32; 3]> = vec![[0, 2, 1], [0, 3, 2]];
    let mut geometry = Sha256::new();
    geometry.update(b"MAG-NAV-GEOMETRY-v1\0");
    geometry.update((vertices.len() as u32).to_le_bytes());
    geometry.update((triangles.len() as u32).to_le_bytes());
    for value in vertices.iter().flatten() {
        geometry.update(value.to_le_bytes());
    }
    for value in triangles.iter().flatten() {
        geometry.update(value.to_le_bytes());
    }
    let (min, max) = ([0., -3., 0.], [10., 3., 10.]);
    bake(BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: "synthetic".into(),
        world_sha256: world.into(),
        geometry_sha256: format!("{:x}", geometry.finalize()),
        bounds: Bounds { min, max },
        physics_regions: vec![crate::collision::PhysicsRegion {
            min,
            max,
            ground_verified: true,
            obstacles_verified: true,
            provenance: vec!["SYNTHETIC floor".into()],
        }],
        vertices,
        triangles,
    })
    .unwrap()
}

/// The artifact's JSON, and the same JSON with one Detour byte changed but
/// the recorded digest kept: a read that came back damaged.
fn good_and_damaged() -> (Artifact, Vec<u8>, Vec<u8>) {
    let artifact = floor(&"0".repeat(64));
    let good = serde_json::to_vec(&artifact).unwrap();
    let mut damaged = artifact.clone();
    damaged.detour_data[150] ^= 0x10;
    (artifact, good, serde_json::to_vec(&damaged).unwrap())
}

#[test]
fn a_damaged_read_followed_by_a_good_one_loads_the_mesh() {
    let (artifact, good, damaged) = good_and_damaged();
    let mut reads = vec![good, damaged];
    let mut count = 0;
    let verified = VerifiedArtifact::read_with(Path::new("synthetic/mesh.json"), || {
        count += 1;
        Ok(reads.pop().unwrap())
    })
    .unwrap();
    assert_eq!(count, 2);
    assert_eq!(verified.artifact().detour_sha256, artifact.detour_sha256);
    let mesh = NavMesh::load_verified(verified, &"0".repeat(64), "synthetic", None, BodyShape::Box)
        .unwrap();
    assert_eq!(mesh.detour_bytes(), artifact.detour_data.len());
}

#[test]
fn a_persistent_mismatch_fails_naming_the_file_and_both_digests() {
    let (artifact, _, damaged) = good_and_damaged();
    let damaged_sha256 = {
        let parsed: Artifact = serde_json::from_slice(&damaged).unwrap();
        sha256(&parsed.detour_data)
    };
    let mut count = 0;
    let error = VerifiedArtifact::read_with(Path::new("synthetic/mesh.json"), || {
        count += 1;
        Ok(damaged.clone())
    })
    .err()
    .unwrap()
    .to_string();
    assert_eq!(count, 1 + crate::verified_read::RETRIES);
    assert!(error.starts_with("navmesh_binary_hash_mismatch: synthetic/mesh.json: "));
    assert!(error.contains(&format!("expected sha256 {}", artifact.detour_sha256)));
    assert!(error.contains(&format!("read sha256 {damaged_sha256}")));
    assert!(error.contains(&format!("over {} bytes", artifact.detour_data.len())));
}

#[test]
fn a_file_read_verifies_and_rejects_a_damaged_file_after_retrying() {
    let (artifact, good, damaged) = good_and_damaged();
    let dir = std::env::temp_dir().join(format!("mag-navmesh-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mesh.json");
    std::fs::write(&path, good).unwrap();
    let verified = VerifiedArtifact::read(&path).unwrap();
    assert_eq!(verified.artifact().detour_data, artifact.detour_data);
    std::fs::write(&path, damaged).unwrap();
    let error = VerifiedArtifact::read(&path).err().unwrap().to_string();
    assert!(error.starts_with(&format!(
        "navmesh_binary_hash_mismatch: {}: ",
        path.display()
    )));
    assert!(error.contains(&artifact.detour_sha256));
    std::fs::remove_dir_all(dir).unwrap();
}
