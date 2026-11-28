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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movement_profile_sha256: Option<String>,
    pub bounds: Bounds,
    pub upstream_commit: String,
    pub bake_config: String,
    pub detour_sha256: String,
    pub detour_bytes: u64,
}

pub fn bake_reference(input: BakeInput) -> Result<(ReferenceManifest, Vec<u8>), Error> {
    bake_scoped(input, None)
}

pub fn bake_reference_with_movement_profile(
    input: BakeInput,
    rules: &MovementProfile,
) -> Result<(ReferenceManifest, Vec<u8>), Error> {
    bake_scoped(input, Some(rules))
}

fn bake_scoped(
    input: BakeInput,
    rules: Option<&MovementProfile>,
) -> Result<(ReferenceManifest, Vec<u8>), Error> {
    let artifact = bake_artifact(input, true, rules, None)?;
    let manifest = ReferenceManifest {
        format: artifact.format,
        map_name: artifact.map_name,
        world_sha256: artifact.world_sha256,
        geometry_sha256: artifact.geometry_sha256,
        profile: artifact.profile,
        movement_profile_sha256: artifact.movement_profile_sha256,
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
        Self::load_with_node_limit(
            manifest,
            data,
            world_sha256,
            map_name,
            QUERY_NODE_LIMIT,
            None,
        )
    }

    pub fn load_with_movement_profile(
        manifest: &ReferenceManifest,
        data: Vec<u8>,
        world_sha256: &str,
        map_name: &str,
        rules: &MovementProfile,
    ) -> Result<Self, Error> {
        Self::load_with_node_limit(
            manifest,
            data,
            world_sha256,
            map_name,
            QUERY_NODE_LIMIT,
            Some(rules),
        )
    }

    fn load_with_node_limit(
        manifest: &ReferenceManifest,
        data: Vec<u8>,
        world_sha256: &str,
        map_name: &str,
        node_limit: c_int,
        rules: Option<&MovementProfile>,
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
                movement_profile_sha256: manifest.movement_profile_sha256.clone(),
                bounds: manifest.bounds.clone(),
                upstream_commit: manifest.upstream_commit.clone(),
                bake_config: manifest.bake_config.clone(),
                detour_sha256: manifest.detour_sha256.clone(),
                detour_data: data,
                drop_links: Vec::new(),
                character_movement_layers: None,
                body_shape: None,
            },
            world_sha256,
            map_name,
            true,
            node_limit,
            rules,
            false,
        )
        .map(Self)
    }

    pub fn route(&self, start: Point, goal: Point, goal_radius_m: f64) -> Result<Route, Error> {
        let mut route = self.0.route(start, goal, goal_radius_m)?;
        route.format = if self.0.artifact.movement_profile_sha256.is_some() {
            "mag_navmesh_reference_route_v2"
        } else {
            "mag_navmesh_reference_route_v1"
        }
        .into();
        Ok(route)
    }

    pub fn tile_stats(&self) -> Result<(u32, u32), Error> {
        let (mut tiles, mut max_polygons) = (0, 0);
        // The loaded mesh owns validated native tiles and is only read now.
        unsafe { mag_nav_tile_stats(self.0.native.0, &mut tiles, &mut max_polygons) };
        Ok((tiles as u32, max_polygons as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: Point = [11., 0., 31.];
    const GOAL: Point = [17., 0., 31.];

    fn floor_bundle(split: bool) -> (ReferenceManifest, Vec<u8>) {
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        for x in 0..20 {
            for z in 0..20 {
                if (split && x == 10) || (matches!(x % 5, 2 | 3) && z % 4 == 2) {
                    continue;
                }
                let (x, z) = ((2 * x) as f32, (2 * z) as f32);
                let offset = vertices.len() as u32;
                vertices.extend([
                    [x, 0., z],
                    [x + 2., 0., z],
                    [x + 2., 0., z + 2.],
                    [x, 0., z + 2.],
                ]);
                triangles.extend([
                    [offset, offset + 2, offset + 1],
                    [offset, offset + 3, offset + 2],
                ]);
            }
        }
        let mut hash = Sha256::new();
        hash.update(b"MAG-NAV-GEOMETRY-v1\0");
        hash.update((vertices.len() as u32).to_le_bytes());
        hash.update((triangles.len() as u32).to_le_bytes());
        for value in vertices.iter().flatten() {
            hash.update(value.to_le_bytes());
        }
        for value in triangles.iter().flatten() {
            hash.update(value.to_le_bytes());
        }
        bake_reference(BakeInput {
            format: REFERENCE_INPUT.into(),
            map_name: "synthetic".into(),
            world_sha256: "0".repeat(64),
            geometry_sha256: format!("{:x}", hash.finalize()),
            bounds: Bounds {
                min: [0., -3., 0.],
                max: [40., 3., 40.],
            },
            physics_regions: Vec::new(),
            vertices,
            triangles,
        })
        .unwrap()
    }

    fn floor_with_holes(split: bool, node_limit: c_int) -> ReferenceNavMesh {
        let (manifest, data) = floor_bundle(split);
        open_floor(&manifest, data, node_limit)
    }

    fn open_floor(
        manifest: &ReferenceManifest,
        data: Vec<u8>,
        node_limit: c_int,
    ) -> ReferenceNavMesh {
        ReferenceNavMesh::load_with_node_limit(
            manifest,
            data,
            &manifest.world_sha256,
            &manifest.map_name,
            node_limit,
            None,
        )
        .unwrap()
    }

    fn polygon_neighbors(data: &mut [u8], reference: u64) -> &mut [[u8; 2]] {
        // Pinned Detour layout for these single-tile floor fixtures.
        const TILE: usize = 44;
        const HEADER: usize = 100;
        const POLYGON_BYTES: usize = 32;
        const NEIGHBOR_OFFSET: usize = 16;
        assert_eq!(u32::from_le_bytes(data[8..12].try_into().unwrap()), 1);
        let vertices = u32::from_le_bytes(data[TILE + 28..TILE + 32].try_into().unwrap()) as usize;
        let polygon = (reference & 8191) as usize;
        let offset = TILE + HEADER + 12 * vertices + POLYGON_BYTES * polygon + NEIGHBOR_OFFSET;
        data[offset..offset + 12].as_chunks_mut::<2>().0
    }

    #[test]
    fn reference_query_reports_proven_disconnection_before_node_exhaustion() {
        let mesh = floor_with_holes(true, 15);
        let route = mesh.route(START, [31., 0., 31.], 0.5).unwrap();
        assert_eq!(route.status, Status::Disconnected);
        assert!(route.corridor_refs.is_empty());
        assert!(route.waypoints.is_empty());
        assert_eq!(route.projected_start[0], START[0]);
        assert_eq!(route.projected_goal[0], 31.);
    }

    #[test]
    fn reference_query_recovers_exact_goal_with_a_bounded_reverse_corridor() {
        let mesh = floor_with_holes(false, 15);
        let reverse = mesh.route(GOAL, START, 0.5).unwrap();
        assert_eq!(reverse.status, Status::Complete);

        let forward = mesh.route(START, GOAL, 0.5).unwrap();
        assert_eq!(forward.status, Status::Complete);
        assert_eq!(forward.projected_start, reverse.projected_goal);
        assert_eq!(forward.projected_goal, reverse.projected_start);
        assert!(
            forward
                .corridor_refs
                .iter()
                .eq(reverse.corridor_refs.iter().rev())
        );
        assert_eq!(forward.waypoints.first(), Some(&forward.projected_start));
        assert_eq!(forward.waypoints.last(), Some(&forward.projected_goal));
    }

    #[test]
    fn reference_query_keeps_out_of_nodes_when_neither_direction_completes() {
        let mesh = floor_with_holes(false, 4);
        assert_eq!(
            mesh.route(START, GOAL, 0.5).unwrap().status,
            Status::OutOfNodes
        );
        assert_eq!(
            mesh.route(GOAL, START, 0.5).unwrap().status,
            Status::OutOfNodes
        );
    }

    #[test]
    fn reference_query_rejects_reversed_corridor_with_a_missing_forward_edge() {
        let (mut manifest, mut data) = floor_bundle(false);
        let baseline = open_floor(&manifest, data.clone(), 15)
            .route(GOAL, START, 0.5)
            .unwrap();
        assert_eq!(baseline.status, Status::Complete);
        // One synthetic tile. Remove the forward neighbor into the goal while
        // leaving the opposite directed link and the tile layout intact.
        let to = (baseline.corridor_refs[0] & 8191) as u16;
        let neighbor = polygon_neighbors(&mut data, baseline.corridor_refs[1])
            .iter_mut()
            .find(|entry| u16::from_le_bytes(**entry) == to + 1)
            .unwrap();
        *neighbor = [0; 2];
        manifest.detour_sha256 = sha256(&data);
        let mesh = open_floor(&manifest, data, 15);
        let reverse = mesh.route(GOAL, START, 0.5).unwrap();
        assert_eq!(reverse.status, Status::Complete);
        assert_eq!(reverse.corridor_refs, baseline.corridor_refs);
        assert_eq!(
            mesh.route(START, GOAL, 0.5).unwrap().status,
            Status::OutOfNodes
        );
    }

    #[test]
    fn reference_query_keeps_unproved_single_polygon_partial() {
        let (mut manifest, mut data) = floor_bundle(false);
        let baseline = open_floor(&manifest, data.clone(), QUERY_NODE_LIMIT)
            .route(START, GOAL, 0.5)
            .unwrap();
        assert_eq!(baseline.status, Status::Complete);
        // Leave all inbound links intact while removing the source's exits.
        let source = baseline.corridor_refs[0];
        polygon_neighbors(&mut data, source).fill([0; 2]);
        manifest.detour_sha256 = sha256(&data);
        let route = open_floor(&manifest, data, QUERY_NODE_LIMIT)
            .route(START, GOAL, 0.5)
            .unwrap();
        assert_eq!(route.status, Status::Partial);
        assert_eq!(route.corridor_refs, [source]);
    }
}
