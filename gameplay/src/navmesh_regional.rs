//! Explicit fixed-triangle walking authority for bounded runtime regions.
use super::*;
use crate::collision::CollisionWorld;

const COVERAGE_SCHEMA: &str = "mag_bot_regional_walk_coverage_v1";
pub(super) const INPUT_SCHEMA: &str = "mag_navmesh_regional_walk_input_v1";
const ARTIFACT_SCHEMA: &str = "mag_navmesh_regional_walk_v1";
pub(super) const MESH_FORMAT: &str = "mag_navmesh_regional_walk_native_v1";
const POLICY: &str = "represented_fixed_triangles_walk";
const MAX_COVERAGE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    level_archive_sha256: String,
    level_clv_sha256: String,
    assembly_sha256: String,
    root_inventory_sha256: String,
    absence_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    policy: String,
    map_name: String,
    world_sha256: String,
    geometry_sha256: String,
    bounds: Bounds,
    source: Evidence,
}

#[derive(Clone, Debug)]
pub struct Coverage {
    raw_sha256: String,
    map_name: String,
    world_sha256: String,
    geometry_sha256: String,
    bounds: Bounds,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionalBakeInput {
    schema: String,
    coverage_sha256: String,
    mesh: BakeInput,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionalArtifact {
    schema: String,
    coverage_sha256: String,
    mesh: Artifact,
}
impl DetourPayload for RegionalArtifact {
    fn detour(&self) -> &Artifact {
        &self.mesh
    }
}
impl RegionalArtifact {
    /// Read the artifact at `path` once per attempt, checking its Detour
    /// digest over those bytes and reading again when it does not verify
    /// (see `VerifiedArtifact::read`).
    pub fn read(path: &std::path::Path) -> std::io::Result<Self> {
        Ok(crate::verified_read::read(
            path,
            MAX_ARTIFACT_JSON_BYTES,
            "navmesh_artifact",
            decode_artifact::<Self>,
        )?)
    }
}

impl Coverage {
    pub fn from_bytes(raw: &[u8]) -> Result<Self, Error> {
        if raw.len() > MAX_COVERAGE_BYTES {
            return err("navmesh_regional_coverage_size");
        }
        let document: Document =
            serde_json::from_slice(raw).map_err(|_| Error("navmesh_regional_invalid_coverage"))?;
        if document.schema != COVERAGE_SCHEMA
            || document.policy != POLICY
            || document.map_name.is_empty()
            || document.map_name.len() > 128
            || !valid_bounds(&document.bounds)
            || (0..3).any(|i| document.bounds.max[i] - document.bounds.min[i] > 128.)
            || ![
                &document.world_sha256,
                &document.geometry_sha256,
                &document.source.level_archive_sha256,
                &document.source.level_clv_sha256,
                &document.source.assembly_sha256,
                &document.source.root_inventory_sha256,
                &document.source.absence_sha256,
            ]
            .into_iter()
            .all(|digest| valid_sha(digest))
        {
            return err("navmesh_regional_invalid_coverage");
        }
        Ok(Self {
            raw_sha256: sha256(raw),
            map_name: document.map_name,
            world_sha256: document.world_sha256,
            geometry_sha256: document.geometry_sha256,
            bounds: document.bounds,
        })
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        use std::io::Read;
        let mut raw = Vec::new();
        std::fs::File::open(path)?
            .take((MAX_COVERAGE_BYTES + 1) as u64)
            .read_to_end(&mut raw)?;
        Self::from_bytes(&raw).map_err(std::io::Error::other)
    }

    pub fn source_sha256(&self) -> &str {
        &self.raw_sha256
    }

    pub(crate) fn bounds(&self) -> &Bounds {
        &self.bounds
    }

    pub(super) fn validate_world(&self, world: &CollisionWorld) -> Result<(), Error> {
        if world.source_sha256() != self.world_sha256 || world.metadata().map_name != self.map_name
        {
            return err("navmesh_regional_world_mismatch");
        }
        Ok(())
    }

    pub(super) fn validate_mesh(
        &self,
        map: &str,
        world: &str,
        geometry: &str,
        bounds: &Bounds,
    ) -> Result<(), Error> {
        if self.map_name != map
            || self.world_sha256 != world
            || self.geometry_sha256 != geometry
            || &self.bounds != bounds
        {
            return err("navmesh_regional_mesh_mismatch");
        }
        Ok(())
    }
}

pub fn bake(input: RegionalBakeInput, coverage: &Coverage) -> Result<RegionalArtifact, Error> {
    if input.schema != INPUT_SCHEMA || input.coverage_sha256 != coverage.raw_sha256 {
        return err("navmesh_regional_coverage_mismatch");
    }
    let mesh = bake_artifact(input.mesh, false, None, Some(coverage))?;
    Ok(RegionalArtifact {
        schema: ARTIFACT_SCHEMA.to_owned(),
        coverage_sha256: input.coverage_sha256,
        mesh,
    })
}

pub fn load(
    artifact: RegionalArtifact,
    world: &CollisionWorld,
    coverage: Coverage,
) -> Result<NavMesh, Error> {
    coverage.validate_world(world)?;
    if artifact.schema != ARTIFACT_SCHEMA
        || artifact.coverage_sha256 != coverage.raw_sha256
        || !artifact.mesh.drop_links.is_empty()
    {
        return err("navmesh_regional_coverage_mismatch");
    }
    let mesh = artifact.mesh;
    coverage.validate_mesh(
        &mesh.map_name,
        &mesh.world_sha256,
        &mesh.geometry_sha256,
        &mesh.bounds,
    )?;
    let mut loaded = NavMesh::load_artifact(
        mesh,
        world.source_sha256(),
        &world.metadata().map_name,
        false,
        QUERY_NODE_LIMIT,
        None,
        true,
    )?;
    loaded.regional_coverage = Some(coverage);
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document() -> serde_json::Value {
        let digest = "a".repeat(64);
        json!({
            "schema":COVERAGE_SCHEMA,"policy":POLICY,"map_name":"invented",
            "world_sha256":digest,"geometry_sha256":digest,
            "bounds":{"min":[0.,-4.,0.],"max":[10.,8.,10.]},
            "source":{"level_archive_sha256":digest,"level_clv_sha256":digest,
                "assembly_sha256":digest,"root_inventory_sha256":digest,"absence_sha256":digest}
        })
    }

    #[test]
    fn regional_capability_requires_exact_scope_and_bounded_region() {
        let value = document();
        assert!(Coverage::from_bytes(&serde_json::to_vec(&value).unwrap()).is_ok());
        for (key, changed) in [
            ("policy", json!("projectile")),
            ("world_sha256", json!("bad")),
            ("bounds", json!({"min":[0,0,0],"max":[129,8,8]})),
            ("source", json!({})),
        ] {
            let mut invalid = value.clone();
            invalid[key] = changed;
            assert!(Coverage::from_bytes(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
    }

    #[test]
    fn regional_mesh_and_terrain_require_walking_only_authority() {
        let raw = serde_json::to_vec(&json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"invented","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":[],"physics_regions":[]},
            "instances":[{"id":"floor","vertices_xyz":[[0.,0.,0.],[0.,0.,10.],[10.,0.,0.],[10.,0.,10.]],
                "triangles":[[0,1,2],[2,1,3]]}]
        })).unwrap();
        let world = CollisionWorld::from_bytes(&raw, Default::default()).unwrap();
        let vertices: Vec<[f32; 3]> =
            vec![[0., 0., 0.], [0., 0., 10.], [10., 0., 0.], [10., 0., 10.]];
        let triangles: Vec<[u32; 3]> = vec![[0, 1, 2], [2, 1, 3]];
        let mut sha = Sha256::new();
        sha.update(b"MAG-NAV-GEOMETRY-v1\0");
        sha.update(4u32.to_le_bytes());
        sha.update(2u32.to_le_bytes());
        for v in vertices.iter().flatten() {
            sha.update(v.to_le_bytes());
        }
        for i in triangles.iter().flatten() {
            sha.update(i.to_le_bytes());
        }
        let mut document = document();
        document["world_sha256"] = json!(world.source_sha256());
        document["geometry_sha256"] = json!(format!("{:x}", sha.finalize()));
        let coverage = Coverage::from_bytes(&serde_json::to_vec(&document).unwrap()).unwrap();
        let input = |bounds: Bounds| RegionalBakeInput {
            schema: INPUT_SCHEMA.into(),
            coverage_sha256: coverage.raw_sha256.clone(),
            mesh: BakeInput {
                format: INPUT_SCHEMA.into(),
                map_name: "invented".into(),
                world_sha256: coverage.world_sha256.clone(),
                geometry_sha256: coverage.geometry_sha256.clone(),
                bounds,
                physics_regions: Vec::new(),
                vertices: vertices.clone(),
                triangles: triangles.clone(),
            },
        };
        let mut shifted = coverage.bounds.clone();
        shifted.min[0] += 1.;
        assert!(bake(input(shifted), &coverage).is_err());
        let artifact = bake(input(coverage.bounds.clone()), &coverage).unwrap();
        assert!(NavMesh::load(artifact.mesh.clone(), world.source_sha256(), "invented").is_err());
        let mesh = load(artifact, &world, coverage.clone()).unwrap();
        assert_eq!(
            mesh.route([2., 0., 2.], [6., 0., 6.], 1.).unwrap().status,
            Status::Complete
        );
        assert!(mesh.route([2., 0., 2.], [20., 0., 6.], 1.).is_err());
        assert!(world.metadata().physics_regions.is_empty());

        use crate::{actor_collision, bot_physics::*};
        let original = WorldTerrain::new(Some(&world), "invented").unwrap();
        let body = Body::default();
        assert!(
            original
                .require_horizontal_coverage([2., 0., 2.], [6., 0., 6.], body)
                .is_err()
        );
        let guarded = world.with_bot_regional_guard(
            RegionalGuard::new(
                actor_collision::Bounds {
                    min: coverage.bounds.min,
                    max: coverage.bounds.max,
                },
                &[],
            )
            .with_walking_coverage(),
        );
        let terrain = WorldTerrain::new(Some(&guarded), "invented").unwrap();
        assert!(
            terrain
                .require_horizontal_coverage([2., 0., 2.], [6., 0., 6.], body)
                .is_ok()
        );
        assert!(
            terrain
                .require_horizontal_coverage([2., 0., 2.], [9.9, 0., 6.], body)
                .is_err()
        );
        assert!(matches!(
            terrain.validate_descent([2., 0., 2.], [6., -2., 6.], 4., body, 1.),
            Err(PhysicsError::CoverageUnknown),
        ));
    }
}
