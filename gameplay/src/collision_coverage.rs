//! Verified-coverage sidecar for one collision world.
//!
//! The sidecar declares physics regions for a world without changing the
//! reviewed world bytes, so the world hash and every receipt bound to it stay
//! valid. It names the world by SHA-256 and map, and a world refuses any other
//! sidecar. Bot navigation (navmesh bake and load, bot body coverage and bot
//! perception) treats its regions like the world's own regions. Static world
//! casts for projectiles and abilities keep using only the world's own metadata.
use crate::collision::PhysicsRegion;
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path};

pub const FORMAT: &str = "mag_collision_coverage_v1";
pub const MAX_BYTES: u64 = 64 * 1024;
pub const MAX_REGIONS: usize = 64;
const MAX_ABS_COORDINATE: f64 = 100_000.;

/// How the regions were established. Both bases authorize bot navigation; the
/// basis is recorded so an operator assertion stays visible in logs and docs.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    OperatorAssertion,
    SourceVerified,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    map_name: String,
    world_sha256: String,
    basis: Basis,
    provenance: Vec<String>,
    physics_regions: Vec<PhysicsRegion>,
}

#[derive(Clone, Debug)]
pub struct CoverageSidecar {
    raw_sha256: String,
    map_name: String,
    world_sha256: String,
    basis: Basis,
    regions: Vec<PhysicsRegion>,
}

fn valid_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn valid_text(values: &[String]) -> bool {
    !values.is_empty() && values.iter().all(|v| !v.trim().is_empty())
}

impl CoverageSidecar {
    pub fn from_bytes(raw: &[u8]) -> Result<Self, &'static str> {
        if raw.len() as u64 > MAX_BYTES {
            return Err("collision_coverage_size_limit");
        }
        let document: Document =
            serde_json::from_slice(raw).map_err(|_| "collision_coverage_invalid")?;
        if document.format != FORMAT
            || document.map_name.trim().is_empty()
            || document.map_name.len() > 128
            || !valid_sha(&document.world_sha256)
            || !valid_text(&document.provenance)
            || document.physics_regions.is_empty()
            || document.physics_regions.len() > MAX_REGIONS
            || document.physics_regions.iter().any(|r| {
                !valid_text(&r.provenance)
                    || (0..3).any(|i| {
                        !r.min[i].is_finite()
                            || !r.max[i].is_finite()
                            || r.min[i].abs() > MAX_ABS_COORDINATE
                            || r.max[i].abs() > MAX_ABS_COORDINATE
                            || r.min[i] >= r.max[i]
                    })
            })
        {
            return Err("collision_coverage_invalid");
        }
        use sha2::Digest;
        Ok(Self {
            raw_sha256: format!("{:x}", sha2::Sha256::digest(raw)),
            map_name: document.map_name,
            world_sha256: document.world_sha256,
            basis: document.basis,
            regions: document.physics_regions,
        })
    }

    /// Read a bounded regular file; any failure is an error, never empty coverage.
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let info = std::fs::metadata(path)?;
        if !info.is_file() || info.len() > MAX_BYTES {
            return Err(std::io::Error::other("collision_coverage_size_limit"));
        }
        let mut raw = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut raw)?;
        Self::from_bytes(&raw).map_err(std::io::Error::other)
    }

    /// SHA-256 of the raw sidecar file.
    pub fn source_sha256(&self) -> &str {
        &self.raw_sha256
    }

    pub fn world_sha256(&self) -> &str {
        &self.world_sha256
    }

    pub fn map_name(&self) -> &str {
        &self.map_name
    }

    pub fn basis(&self) -> Basis {
        self.basis
    }

    pub fn regions(&self) -> &[PhysicsRegion] {
        &self.regions
    }
}

/// One region with both ground and obstacles verified contains the whole box.
/// Regions are not merged: a box spanning two adjacent regions is not covered.
pub fn regions_cover<'a>(
    regions: impl IntoIterator<Item = &'a PhysicsRegion>,
    min: [f64; 3],
    max: [f64; 3],
) -> bool {
    regions.into_iter().any(|r| {
        r.ground_verified
            && r.obstacles_verified
            && !r.provenance.is_empty()
            && (0..3).all(|i| r.min[i] <= min[i] && r.max[i] >= max[i])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{CollisionLimits, CollisionWorld};
    use serde_json::json;

    fn world(regions: serde_json::Value) -> CollisionWorld {
        let raw = serde_json::to_vec(&json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"invented","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":[],"physics_regions":regions},
            "instances":[{"id":"floor","vertices_xyz":[[0.,0.,0.],[0.,0.,10.],[10.,0.,0.]],
                "triangles":[[0,1,2]]}]
        }))
        .unwrap();
        CollisionWorld::from_bytes(&raw, CollisionLimits::default()).unwrap()
    }

    fn document(world_sha256: &str) -> serde_json::Value {
        json!({
            "format":FORMAT,"map_name":"invented","world_sha256":world_sha256,
            "basis":"operator_assertion","provenance":["synthetic assertion"],
            "physics_regions":[{"min":[-1.,-3.,-1.],"max":[11.,3.,11.],
                "ground_verified":true,"obstacles_verified":true,"provenance":["synthetic"]}]
        })
    }

    fn parse(value: &serde_json::Value) -> Result<CoverageSidecar, &'static str> {
        CoverageSidecar::from_bytes(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn sidecar_parse_refuses_malformed_documents() {
        let valid = document(&"a".repeat(64));
        let sidecar = parse(&valid).unwrap();
        assert_eq!(sidecar.basis(), Basis::OperatorAssertion);
        assert_eq!(sidecar.regions().len(), 1);
        assert_eq!(sidecar.source_sha256().len(), 64);
        let region = valid["physics_regions"][0].clone();
        let mut inverted = region.clone();
        inverted["min"][0] = json!(20.);
        let mut unnamed = region.clone();
        unnamed["provenance"] = json!([" "]);
        let mut far = region.clone();
        far["max"][2] = json!(1e6);
        for (key, value) in [
            ("format", json!("mag_collision_coverage_v2")),
            ("map_name", json!("")),
            ("world_sha256", json!("A".repeat(64))),
            ("world_sha256", json!("a".repeat(63))),
            ("basis", json!("guess")),
            ("provenance", json!([])),
            ("physics_regions", json!([])),
            ("physics_regions", json!([inverted])),
            ("physics_regions", json!([unnamed])),
            ("physics_regions", json!([far])),
            ("physics_regions", json!(vec![region; MAX_REGIONS + 1])),
            ("unexpected", json!(true)),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert_eq!(
                parse(&invalid).unwrap_err(),
                "collision_coverage_invalid",
                "{key}"
            );
        }
        let oversized = vec![b' '; MAX_BYTES as usize + 1];
        assert_eq!(
            CoverageSidecar::from_bytes(&oversized).unwrap_err(),
            "collision_coverage_size_limit"
        );
    }

    #[test]
    fn sidecar_binds_to_the_world_hash_and_map_and_unions_regions() {
        let base = world(json!([]));
        assert!(!base.covers_box([0., 0., 0.], [10., 1., 10.]));
        let other = parse(&document(&"b".repeat(64))).unwrap();
        assert_eq!(
            base.with_coverage_sidecar(other).unwrap_err(),
            "collision_coverage_world_mismatch"
        );
        let mut renamed = document(base.source_sha256());
        renamed["map_name"] = json!("elsewhere");
        assert_eq!(
            base.with_coverage_sidecar(parse(&renamed).unwrap())
                .unwrap_err(),
            "collision_coverage_world_mismatch"
        );

        let sidecar = parse(&document(base.source_sha256())).unwrap();
        let covered = base.with_coverage_sidecar(sidecar.clone()).unwrap();
        assert_eq!(covered.source_sha256(), base.source_sha256());
        assert!(covered.metadata().physics_regions.is_empty());
        assert!(covered.covers_box([0., 0., 0.], [10., 1., 10.]));
        assert!(!covered.covers_box([0., 0., 0.], [12., 1., 10.]));
        assert_eq!(
            covered.coverage_sidecar().unwrap().source_sha256(),
            sidecar.source_sha256()
        );
        assert_eq!(
            covered.with_coverage_sidecar(sidecar).unwrap_err(),
            "collision_coverage_already_attached"
        );

        // The world's own regions and the sidecar's regions are one set.
        let own = world(json!([{"min":[20.,-1.,20.],"max":[30.,3.,30.],
            "ground_verified":true,"obstacles_verified":true,"provenance":["synthetic"]}]));
        let both = own
            .with_coverage_sidecar(parse(&document(own.source_sha256())).unwrap())
            .unwrap();
        assert_eq!(both.verified_physics_regions().count(), 2);
        assert!(both.covers_box([21., 0., 21.], [29., 1., 29.]));
        assert!(both.covers_box([1., 0., 1.], [9., 1., 9.]));
        // A box spanning both regions is not covered by either alone.
        assert!(!both.covers_box([1., 0., 1.], [29., 1., 29.]));
    }

    #[test]
    fn unverified_sidecar_regions_do_not_authorize_navigation() {
        let base = world(json!([]));
        let mut value = document(base.source_sha256());
        value["physics_regions"][0]["obstacles_verified"] = json!(false);
        let covered = base.with_coverage_sidecar(parse(&value).unwrap()).unwrap();
        assert!(!covered.covers_box([0., 0., 0.], [10., 1., 10.]));
    }
}
