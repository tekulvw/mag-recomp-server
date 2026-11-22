//! Query-only mesh over represented static reference geometry.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceManifest {
    pub format: String,
    pub map_name: String,
    pub world_sha256: String,
    pub geometry_sha256: String,
    pub profile: Profile,
    pub bounds: Bounds,
    pub upstream_commit: String,
    pub bake_config: String,
    pub detour_sha256: String,
    pub detour_bytes: u64,
}

pub fn bake_reference(input: BakeInput) -> Result<(ReferenceManifest, Vec<u8>), Error> {
    let artifact = bake_artifact(input, true)?;
    let manifest = ReferenceManifest {
        format: artifact.format,
        map_name: artifact.map_name,
        world_sha256: artifact.world_sha256,
        geometry_sha256: artifact.geometry_sha256,
        profile: artifact.profile,
        bounds: artifact.bounds,
        upstream_commit: artifact.upstream_commit,
        bake_config: artifact.bake_config,
        detour_sha256: artifact.detour_sha256,
        detour_bytes: artifact.detour_data.len() as u64,
    };
    Ok((manifest, artifact.detour_data))
}

/// A reference mesh offers offline queries. It cannot supply a runtime mesh.
///
/// ```compile_fail
/// use mag_gameplay::navmesh::{NavMesh, ReferenceNavMesh};
/// fn runtime_installation(mesh: ReferenceNavMesh) -> NavMesh { mesh }
/// ```
pub struct ReferenceNavMesh(NavMesh);

impl ReferenceNavMesh {
    pub fn load(
        manifest: &ReferenceManifest,
        data: Vec<u8>,
        world_sha256: &str,
        map_name: &str,
    ) -> Result<Self, Error> {
        if manifest.detour_bytes != data.len() as u64
            || manifest.bounds.max[1] - manifest.bounds.min[1] > 409.55
            || !valid_point(manifest.bounds.min)
            || !valid_point(manifest.bounds.max)
            || manifest.map_name.trim().is_empty()
            || manifest.map_name.len() > 128
        {
            return err("navmesh_reference_metadata_mismatch");
        }
        NavMesh::load_artifact(
            Artifact {
                format: manifest.format.clone(),
                map_name: manifest.map_name.clone(),
                world_sha256: manifest.world_sha256.clone(),
                geometry_sha256: manifest.geometry_sha256.clone(),
                profile: manifest.profile.clone(),
                bounds: manifest.bounds.clone(),
                upstream_commit: manifest.upstream_commit.clone(),
                bake_config: manifest.bake_config.clone(),
                detour_sha256: manifest.detour_sha256.clone(),
                detour_data: data,
                drop_links: Vec::new(),
            },
            world_sha256,
            map_name,
            true,
        )
        .map(|mut mesh| {
            // Detour already owns checked copies of the tiles.
            mesh.artifact.detour_data = Vec::new();
            Self(mesh)
        })
    }

    pub fn route(&self, start: Point, goal: Point, goal_radius_m: f64) -> Result<Route, Error> {
        let mut route = self.0.route(start, goal, goal_radius_m)?;
        route.format = "mag_navmesh_reference_route_v1".into();
        Ok(route)
    }

    pub fn tile_stats(&self) -> Result<(u32, u32), Error> {
        let native = self
            .0
            .native
            .lock()
            .map_err(|_| Error("navmesh_lock_poisoned"))?;
        let (mut tiles, mut max_polygons) = (0, 0);
        // The locked handle owns validated native tiles for the duration of the query.
        unsafe { mag_nav_tile_stats(native.0, &mut tiles, &mut max_polygons) };
        Ok((tiles as u32, max_polygons as u32))
    }
}
