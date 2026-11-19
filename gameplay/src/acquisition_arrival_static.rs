//! Compiled static positive witnesses for the retail Acquisition Arrival ray.
use crate::{
    collision::{CollisionLimits, CollisionWorld, Coverage},
    replication::CharacterPosition,
};
use serde::Deserialize;
use std::path::{Path, PathBuf};

const ARRIVAL_LAYER: usize = 24;
const STATIC_WORLD_LAYER: u32 = 9;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub path: PathBuf,
    pub sha256: String,
}

impl Binding {
    pub fn load(&self, base: &Path, map: &str) -> Result<ArrivalStatic, &'static str> {
        let world = CollisionWorld::load(
            base.join(&self.path),
            CollisionLimits {
                max_file_bytes: 2 * 1024 * 1024,
                max_instances: 512,
                max_vertices: 20_000,
                max_triangles: 20_000,
                ..Default::default()
            },
        )
        .map_err(|_| "acquisition_arrival_static_load_failed")?;
        if world.metadata().map_name != map
            || world.metadata().coverage != Coverage::Provisional
            || world.source_sha256() != self.sha256
            || world.stats().triangles == 0
        {
            return Err("acquisition_arrival_static_binding_mismatch");
        }
        Ok(ArrivalStatic { world })
    }
}

/// Contains only fixed, unparented layer-9 triangles. This partition supplies
/// a positive witness only. An absent witness leaves other eligible roots for
/// the caller to test.
#[derive(Clone, Debug)]
pub struct ArrivalStatic {
    world: CollisionWorld,
}

impl ArrivalStatic {
    pub fn positive_hit(
        &self,
        position: CharacterPosition,
        filter_rows: [u32; 32],
        budget: &mut usize,
    ) -> Result<bool, &'static str> {
        position.validate()?;
        if filter_rows[ARRIVAL_LAYER] & (1 << STATIC_WORLD_LAYER) == 0 {
            return Ok(false);
        }
        let start = [
            f64::from(position.x),
            f64::from(position.y),
            f64::from(position.z),
        ];
        let end = [start[0], f64::from(position.y + 5.), start[2]];
        let result = self
            .world
            .raycast_segment_bounded(start, end, budget)
            .map_err(|_| "acquisition_arrival_static_query_failed")?;
        Ok(result
            .nearest
            .is_some_and(|hit| hit.fraction > 0. && hit.fraction < 1.))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrival_static_requires_bound_hash_map_current_root_filter_and_upward_hit() {
        let data = br#"{"format":"mag_collision_world","schema_version":1,"coordinate_space":"world","metadata":{"map_name":"invented","build":"synthetic","coverage":"provisional","provenance":["synthetic"],"unknowns":[]},"instances":[{"id":"invented-floor","vertices_xyz":[[0,8,0],[9,8,0],[0,8,9]],"triangles":[[0,1,2]]}]}"#;
        let path = std::env::temp_dir().join(format!(
            "mag-arrival-witness-test-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, data).unwrap();
        let world = crate::collision::CollisionWorld::load(&path, Default::default()).unwrap();
        let binding = Binding {
            path: path.clone(),
            sha256: world.source_sha256().to_owned(),
        };
        let static_world = binding.load(std::path::Path::new("."), "invented").unwrap();
        assert!(binding.load(std::path::Path::new("."), "other").is_err());
        let wrong_hash = Binding {
            path: path.clone(),
            sha256: "0".repeat(64),
        };
        assert!(
            wrong_hash
                .load(std::path::Path::new("."), "invented")
                .is_err()
        );
        let mut rows = [0; 32];
        let mut budget = 1000;
        let point = crate::replication::CharacterPosition {
            x: 2.,
            y: 5.,
            z: 2.,
        };
        assert!(!static_world.positive_hit(point, rows, &mut budget).unwrap());
        rows[24] = 1 << 9;
        assert!(static_world.positive_hit(point, rows, &mut budget).unwrap());
        assert!(
            !static_world
                .positive_hit(
                    crate::replication::CharacterPosition { y: 9., ..point },
                    rows,
                    &mut budget
                )
                .unwrap()
        );
        assert!(static_world.positive_hit(point, rows, &mut 0).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
