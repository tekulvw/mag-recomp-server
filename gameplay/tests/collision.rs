// SYNTHETIC: all inputs in this suite are authored in code; no game catalogs or captures.
use mag_gameplay::collision::{CollisionLimits, CollisionWorld};
use serde_json::{Value, json};
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

fn fixture() -> Value {
    json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic","build":"any-build","coverage":"provisional",
          "provenance":["Synthetic test geometry"],"unknowns":["Not a game map"]},
        "instances":(0..20).map(|i| { let x=2.0+3.0*i as f64;
          json!({"id":format!("wall-{i}"),"vertices_xyz":[[x,-2.0,-2.0],[x,2.0,-2.0],[x,0.0,2.0]],"triangles":[[0,1,2]]})
        }).collect::<Vec<_>>()})
}
struct DiskFixture(std::path::PathBuf);
impl DiskFixture {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "mag-collision-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&p, bytes).unwrap();
        Self(p)
    }
    fn world(value: &Value) -> Self {
        Self::new(&serde_json::to_vec(value).unwrap())
    }
    fn load(&self) -> Result<CollisionWorld, mag_gameplay::collision::CollisionError> {
        CollisionWorld::load(&self.0, CollisionLimits::default())
    }
}
impl Drop for DiskFixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn malformed_world_and_resource_limits_never_become_empty_collision() {
    assert_eq!(
        DiskFixture::world(&fixture())
            .load()
            .unwrap()
            .stats()
            .triangles,
        20
    );
    let bad_values: Vec<Value> = vec![
        {
            let mut v = fixture();
            v["format"] = json!("mag_collision_geometry");
            v
        },
        {
            let mut v = fixture();
            v["schema_version"] = json!(2);
            v
        },
        {
            let mut v = fixture();
            v["coordinate_space"] = json!("collision_shape_local");
            v
        },
        {
            let mut v = fixture();
            v["instances"][0]["triangles"][0][2] = json!(3);
            v
        },
        {
            let mut v = fixture();
            v["instances"][0]["vertices_xyz"][0][0] = json!(1e20);
            v
        },
        {
            let mut v = fixture();
            v["instances"][0]["triangles"][0] = json!([0, 0, 2]);
            v
        },
        {
            let mut v = fixture();
            v["instances"][1]["id"] = json!("wall-0");
            v
        },
        {
            let mut v = fixture();
            v["metadata"]["coverage"] = json!("complete");
            v
        },
        {
            let mut v = fixture();
            v["instances"][0]["transform"] = json!([1, 0, 0]);
            v
        },
        {
            let mut v = fixture();
            v["metadata"]["provenance"] = json!([]);
            v
        },
    ];
    for bad in bad_values {
        assert!(DiskFixture::world(&bad).load().is_err(), "accepted {bad}");
    }
    for bytes in [b"hkx\0".as_slice(), b"{}", b"{\"schema_version\":NaN}"] {
        assert!(DiskFixture::new(bytes).load().is_err());
    }
    let file = DiskFixture::world(&fixture());
    for limits in [
        CollisionLimits {
            max_file_bytes: 10,
            ..Default::default()
        },
        CollisionLimits {
            max_instances: 1,
            ..Default::default()
        },
        CollisionLimits {
            max_vertices: 1,
            ..Default::default()
        },
        CollisionLimits {
            max_triangles: 1,
            ..Default::default()
        },
        CollisionLimits {
            max_abs_coordinate: 1.0,
            ..Default::default()
        },
    ] {
        assert!(CollisionWorld::load(&file.0, limits).is_err());
    }
}

#[test]
fn bvh_returns_nearest_double_sided_finite_segment_hit_including_endpoints() {
    let file = DiskFixture::world(&fixture());
    let world = file.load().unwrap();
    let query = world
        .raycast_segment([0.0, 0.0, 0.0], [10.0, 0.0, 0.0])
        .unwrap();
    assert_eq!(query.metadata.unknowns, vec!["Not a game map"]);
    let hit = query.nearest.unwrap();
    assert_eq!(hit.instance_id, "wall-0");
    assert_eq!(hit.triangle_index, 0);
    assert!((hit.fraction - 0.2).abs() < 1e-12);
    assert_eq!(hit.position, [2.0, 0.0, 0.0]);
    let reverse = world
        .raycast_segment([6.0, 0.0, 0.0], [0.0, 0.0, 0.0])
        .unwrap()
        .nearest
        .unwrap();
    assert_eq!(reverse.instance_id, "wall-1");
    assert!((reverse.fraction - 1.0 / 6.0).abs() < 1e-12);
    for (start, end, fraction) in [
        ([0.0, 0.0, 0.0], [2.0, 0.0, 0.0], 1.0),
        ([2.0, 0.0, 0.0], [3.0, 0.0, 0.0], 0.0),
    ] {
        assert_eq!(
            world
                .raycast_segment(start, end)
                .unwrap()
                .nearest
                .unwrap()
                .fraction,
            fraction
        );
    }
    for (start, end) in [
        ([0.0, 0.0, 0.0], [1.999, 0.0, 0.0]),
        ([0.0, 2.0, 2.0], [10.0, 2.0, 2.0]),
        ([0.0, 100.0, 0.0], [10.0, 100.0, 0.0]),
        ([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ] {
        assert!(world.raycast_segment(start, end).unwrap().nearest.is_none());
    }
    for end in [
        [f64::NAN, 0.0, 0.0],
        [f64::INFINITY, 0.0, 0.0],
        [1e20, 0.0, 0.0],
        [0.0, 0.0, 0.0],
    ] {
        assert!(world.raycast_segment([0.0; 3], end).is_err());
    }
}

#[test]
fn disk_loader_reads_current_file_and_preserves_explicit_empty_provisional_world() {
    let file = DiskFixture::world(&fixture());
    assert_eq!(file.load().unwrap().stats().instances, 20);
    let mut empty = fixture();
    empty["instances"] = json!([]);
    fs::write(&file.0, serde_json::to_vec(&empty).unwrap()).unwrap();
    let world = file.load().unwrap();
    assert_eq!(world.stats().triangles, 0);
    let query = world.raycast_segment([0.0; 3], [1.0; 3]).unwrap();
    assert!(query.nearest.is_none());
    assert!(!query.metadata.unknowns.is_empty());
    fs::write(&file.0, b"broken").unwrap();
    assert!(file.load().is_err());
    fs::remove_file(&file.0).unwrap();
    assert!(file.load().is_err());
}
