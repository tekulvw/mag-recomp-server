//! Offline Recast bake and shared Detour navigation for verified static worlds.
//! Route queries never certify raw collision edges; physical movement remains authoritative.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{ffi::c_int, sync::Mutex, time::Instant};
#[path = "navmesh_drops.rs"]
mod drops;
pub use drops::{DropLink, WaypointKind};
#[path = "navmesh_movement_profile.rs"]
mod movement_profile;
#[path = "navmesh_reference.rs"]
mod reference;
#[path = "navmesh_regional.rs"]
pub mod regional;
pub use movement_profile::MovementProfile;
#[cfg(test)]
#[path = "navmesh_movement_profile_tests.rs"]
mod movement_profile_tests;
pub use reference::{
    ReferenceManifest, ReferenceNavMesh, bake_reference, bake_reference_with_movement_profile,
};
pub type Point = [f64; 3];
pub const UPSTREAM_COMMIT: &str = "6dc1667f580357e8a2154c28b7867bea7e8ad3a7";
pub const LEGACY_BAKE_CONFIG: &str = "single-v1-cs0.1-ch0.05-height37-climb7-radius6-slope45-contour0.5-edge120-region64-merge400-detail0.6-error0.05-nvp6-nodes8192";
pub const BAKE_CONFIG: &str = "tiled-v2-tile512-border9-cs0.1-ch0.05-height37-climb7-radius6-slope45-contour0.5-edge120-region64-merge400-detail0.6-error0.05-nvp6-nodes16384";
/// Whole-map runtime layout: 512 tiles of 8,192 polygons each. With 32-bit
/// polygon references, 9 tile bits and 13 polygon bits leave Detour 10 salt bits.
pub const WIDE_BAKE_CONFIG: &str = "tiled-v4-tile512-border9-tiles512-polys8192-cs0.1-ch0.05-height37-climb7-radius6-slope45-contour0.5-edge120-region64-merge400-detail0.6-error0.05-nvp6-nodes16384";
/// Binary cap for the 256-tile runtime formats (v1 to v3).
pub const MAX_ARTIFACT_BYTES: u64 = 32 * 1024 * 1024;
/// Binary cap for the 512-tile wide runtime formats (v4 and v5).
pub const MAX_WIDE_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_RUNTIME_TILES: u32 = 256;
pub const MAX_WIDE_RUNTIME_TILES: u32 = 512;
// JSON Vec<u8> uses up to four characters per byte, plus bounded metadata.
pub const MAX_ARTIFACT_JSON_BYTES: u64 = MAX_WIDE_ARTIFACT_BYTES * 4 + 1024 * 1024;
// Recast span heights use 13 bits of 0.05 m cells.
const MAX_TILED_HEIGHT_M: f64 = 409.55;
const TILE_WIDTH_M: f32 = 51.2;
pub const MAX_REFERENCE_BYTES: u64 = 192 * 1024 * 1024;
const REFERENCE_INPUT: &str = "mag_navmesh_reference_bake_input_v1";
const REFERENCE_FORMAT: &str = "mag_navmesh_reference_v1";
const REFERENCE_CONFIG: &str = "reference-v1-tile512-border9-tiles512-polys8192-cs0.1-ch0.05-height37-climb7-radius6-slope45-contour0.5-edge120-region64-merge400-detail0.6-error0.05-nvp6-nodes16384";
const PROFILED_RUNTIME_FORMAT: &str = "mag_navmesh_world_v3";
const PROFILED_REFERENCE_FORMAT: &str = "mag_navmesh_reference_v2";
const PROFILED_RUNTIME_CONFIG: &str = "tiled-v3-tile512-border9-cs0.1-ch0.05-height37-climb7-radius6-slope-source-v1-contour0.5-edge120-region64-merge400-detail0.6-error0.05-nvp6-nodes16384";
const WIDE_RUNTIME_FORMAT: &str = "mag_navmesh_world_v4";
const WIDE_PROFILED_RUNTIME_FORMAT: &str = "mag_navmesh_world_v5";
const WIDE_PROFILED_RUNTIME_CONFIG: &str = "tiled-v5-tile512-border9-tiles512-polys8192-cs0.1-ch0.05-height37-climb7-radius6-slope-source-v1-contour0.5-edge120-region64-merge400-detail0.6-error0.05-nvp6-nodes16384";
const PROFILED_REFERENCE_CONFIG: &str = "reference-v2-tile512-border9-tiles512-polys8192-cs0.1-ch0.05-height37-climb7-radius6-slope-source-v1-contour0.5-edge120-region64-merge400-detail0.6-error0.05-nvp6-nodes16384";
const QUERY_NODE_LIMIT: c_int = 16_384;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub radius: f64,
    pub height: f64,
    pub max_step: f64,
    pub max_slope_degrees: f64,
    pub skin: f64,
    pub bake_radius: f64,
    pub cell_size: f64,
    pub cell_height: f64,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            radius: 0.35,
            height: 1.8,
            max_step: 0.4,
            max_slope_degrees: 45.,
            skin: 0.005,
            bake_radius: 0.6,
            cell_size: 0.1,
            cell_height: 0.05,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub min: Point,
    pub max: Point,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BakeInput {
    pub format: String,
    pub map_name: String,
    pub world_sha256: String,
    pub geometry_sha256: String,
    pub bounds: Bounds,
    pub physics_regions: Vec<crate::collision::PhysicsRegion>,
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
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
    pub detour_data: Vec<u8>,
    #[serde(default)]
    pub drop_links: Vec<DropLink>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Complete,
    Partial,
    Disconnected,
    OutOfNodes,
    InvalidStart,
    InvalidGoal,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Route {
    pub format: String,
    pub status: Status,
    pub world_sha256: String,
    pub profile: Profile,
    pub start: Point,
    pub requested_goal: Point,
    pub projected_start: Point,
    pub projected_goal: Point,
    pub waypoints: Vec<Point>,
    pub corridor_refs: Vec<u64>,
    pub length_m: f64,
    pub query_us: u64,
    #[serde(default)]
    pub waypoint_kinds: Vec<WaypointKind>,
    #[serde(default)]
    pub waypoint_drop_ids: Vec<Option<u32>>,
    #[serde(default)]
    pub predicted_remaining_health: Option<u16>,
}
impl Route {
    /// Follow only a bounded prefix of a complete, validated corridor. Never
    /// replace a failed query with a straight-line guess or cut a certified drop.
    pub fn bounded_prefix(&self, maximum_m: f64) -> Result<Self, Error> {
        if self.status != Status::Complete
            || !maximum_m.is_finite()
            || !(1.0..=256.0).contains(&maximum_m)
            || self.waypoints.len() < 2
            || self.waypoint_kinds.len() != self.waypoints.len()
            || self.waypoint_drop_ids.len() != self.waypoints.len()
            || self.waypoints.iter().any(|p| !valid_point(*p))
        {
            return err("navmesh_invalid_segment_request");
        }
        if self.length_m <= maximum_m {
            return Ok(self.clone());
        }
        let mut result = self.clone();
        let mut length = 0.;
        let mut safe_end = 0;
        let mut safe_length = 0.;
        let mut interpolated = None;
        for i in 1..self.waypoints.len() {
            let a = self.waypoints[i - 1];
            let b = self.waypoints[i];
            let edge = (b[0] - a[0]).hypot(b[2] - a[2]);
            if length + edge > maximum_m {
                if self.waypoint_kinds[i - 1] == WaypointKind::Walk
                    && self.waypoint_kinds[i] == WaypointKind::Walk
                    && edge > 0.
                {
                    let fraction = (maximum_m - length) / edge;
                    interpolated = Some((
                        i,
                        std::array::from_fn(|axis| a[axis] + fraction * (b[axis] - a[axis])),
                    ));
                    safe_length = maximum_m;
                }
                break;
            }
            length += edge;
            if self.waypoint_kinds[i] == WaypointKind::Walk {
                safe_end = i;
                safe_length = length;
            }
        }
        let count = interpolated.map_or(safe_end + 1, |(i, _)| i);
        result.waypoints.truncate(count);
        result.waypoint_kinds.truncate(count);
        result.waypoint_drop_ids.truncate(count);
        if let Some((_, point)) = interpolated {
            result.waypoints.push(point);
            result.waypoint_kinds.push(WaypointKind::Walk);
            result.waypoint_drop_ids.push(None);
        }
        if result.waypoints.len() < 2 || safe_length < 0.03125 {
            return err("navmesh_no_supported_segment");
        }
        result.projected_goal = *result.waypoints.last().unwrap();
        result.requested_goal = result.projected_goal;
        result.length_m = safe_length;
        // A full-route prediction includes drops beyond this prefix.
        result.predicted_remaining_health = None;
        Ok(result)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}
fn err<T>(reason: &'static str) -> Result<T, Error> {
    Err(Error(reason))
}
pub fn sha256(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn valid_sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn valid_point(p: Point) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= 100000.)
}
fn valid_bounds(b: &Bounds) -> bool {
    (0..3).all(|i| {
        b.min[i].is_finite()
            && b.max[i].is_finite()
            && b.max[i] > b.min[i]
            && b.max[i] - b.min[i] <= 4096.
    })
}
unsafe extern "C" {
    fn mag_nav_bake(
        verts: *const f32,
        nv: c_int,
        tris: *const c_int,
        nt: c_int,
        min: *const f32,
        max: *const f32,
        out: *mut *mut u8,
        size: *mut c_int,
        mode: c_int,
        slope_degrees: f32,
    ) -> c_int;
    fn mag_nav_free_bytes(data: *mut u8);
    fn mag_nav_load(
        data: *const u8,
        size: c_int,
        min: *const f32,
        max: *const f32,
        mode: c_int,
        node_limit: c_int,
    ) -> *mut std::ffi::c_void;
    fn mag_nav_destroy(handle: *mut std::ffi::c_void);
    fn mag_nav_tile_stats(
        handle: *const std::ffi::c_void,
        occupied: *mut c_int,
        max_polygons: *mut c_int,
    );
    fn mag_nav_route(
        handle: *mut std::ffi::c_void,
        start: *const f32,
        goal: *const f32,
        radius: f32,
        ps: *mut f32,
        pg: *mut f32,
        points: *mut f32,
        npoints: *mut c_int,
        refs: *mut u64,
        nrefs: *mut c_int,
    ) -> c_int;
    fn mag_nav_component(
        handle: *mut std::ffi::c_void,
        start: *const f32,
        goal: *const f32,
        closest: *mut f32,
        min: *mut f32,
        max: *mut f32,
        count: *mut c_int,
        total: *mut c_int,
    ) -> c_int;
    fn mag_nav_component_id(handle: *mut std::ffi::c_void, position: *const f32) -> c_int;
    fn mag_nav_goals(
        handle: *mut std::ffi::c_void,
        goal: *const f32,
        radius: f32,
        components: *mut c_int,
        points: *mut f32,
        count: *mut c_int,
    ) -> c_int;
    fn mag_nav_contains(
        handle: *mut std::ffi::c_void,
        p: *const f32,
        refs: *const u64,
        nrefs: c_int,
    ) -> c_int;
    fn mag_nav_contains_settlement_column(
        handle: *mut std::ffi::c_void,
        p: *const f32,
        refs: *const u64,
        nrefs: c_int,
    ) -> c_int;
}
/// Runtime tile layout. `Auto` keeps the 256-tile formats (v2/v3, byte for
/// byte unchanged) whenever the grid fits and uses the wide layout otherwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Auto,
    Wide,
}
pub fn bake(input: BakeInput) -> Result<Artifact, Error> {
    bake_artifact(input, false, None, None)
}
pub fn bake_with_movement_profile(
    input: BakeInput,
    rules: &MovementProfile,
) -> Result<Artifact, Error> {
    bake_artifact(input, false, Some(rules), None)
}
pub fn bake_with_layout(
    input: BakeInput,
    rules: Option<&MovementProfile>,
    layout: Layout,
) -> Result<Artifact, Error> {
    bake_layout(input, false, rules, None, layout)
}

/// Tiles in the regular 51.2 m grid over the bounds, computed in the same f32
/// arithmetic as the native baker.
pub fn grid_tiles(bounds: &Bounds) -> u64 {
    let axis = |i: usize| {
        ((bounds.max[i] as f32 - bounds.min[i] as f32) / TILE_WIDTH_M)
            .ceil()
            .max(0.) as u64
    };
    axis(0) * axis(2)
}

fn bake_mode(reference: bool, profiled: bool, wide: bool) -> c_int {
    c_int::from(reference) + 2 * c_int::from(profiled) + 4 * c_int::from(wide)
}

fn format_and_config(reference: bool, profiled: bool, wide: bool) -> (&'static str, &'static str) {
    match (reference, profiled, wide) {
        (false, false, false) => ("mag_navmesh_world_v2", BAKE_CONFIG),
        (true, false, _) => (REFERENCE_FORMAT, REFERENCE_CONFIG),
        (false, true, false) => (PROFILED_RUNTIME_FORMAT, PROFILED_RUNTIME_CONFIG),
        (true, true, _) => (PROFILED_REFERENCE_FORMAT, PROFILED_REFERENCE_CONFIG),
        (false, false, true) => (WIDE_RUNTIME_FORMAT, WIDE_BAKE_CONFIG),
        (false, true, true) => (WIDE_PROFILED_RUNTIME_FORMAT, WIDE_PROFILED_RUNTIME_CONFIG),
    }
}

fn byte_limit(reference: bool, wide: bool) -> u64 {
    if reference {
        MAX_REFERENCE_BYTES
    } else if wide {
        MAX_WIDE_ARTIFACT_BYTES
    } else {
        MAX_ARTIFACT_BYTES
    }
}

fn selected_profile(rules: Option<&MovementProfile>) -> Profile {
    let mut profile = Profile::default();
    if let Some(rules) = rules {
        profile.max_slope_degrees = rules.slope_degrees();
    }
    profile
}

fn bake_artifact(
    input: BakeInput,
    reference: bool,
    rules: Option<&MovementProfile>,
    coverage: Option<&regional::Coverage>,
) -> Result<Artifact, Error> {
    bake_layout(input, reference, rules, coverage, Layout::Auto)
}

fn bake_layout(
    input: BakeInput,
    reference: bool,
    rules: Option<&MovementProfile>,
    coverage: Option<&regional::Coverage>,
    layout: Layout,
) -> Result<Artifact, Error> {
    let expected_format = if coverage.is_some() {
        regional::INPUT_SCHEMA
    } else if reference {
        REFERENCE_INPUT
    } else {
        "mag_navmesh_bake_input_v1"
    };
    if input.format != expected_format
        || input.map_name.is_empty()
        || input.map_name.len() > 128
        || !valid_sha(&input.world_sha256)
        || !valid_sha(&input.geometry_sha256)
        || !valid_bounds(&input.bounds)
        || !(3..=2000000).contains(&input.vertices.len())
        || !(1..=2000000).contains(&input.triangles.len())
    {
        return err("navmesh_invalid_input");
    }
    if let Some(coverage) = coverage {
        coverage.validate_mesh(
            &input.map_name,
            &input.world_sha256,
            &input.geometry_sha256,
            &input.bounds,
        )?;
        if !input.physics_regions.is_empty() {
            return err("navmesh_regional_physics_regions_unsupported");
        }
    } else if !reference
        && !crate::collision_coverage::regions_cover(
            &input.physics_regions,
            input.bounds.min,
            input.bounds.max,
        )
    {
        return err("navmesh_geometry_coverage_unknown");
    }
    let runtime = !reference && coverage.is_none();
    let wide = runtime
        && (layout == Layout::Wide || grid_tiles(&input.bounds) > u64::from(MAX_RUNTIME_TILES));
    if wide && grid_tiles(&input.bounds) > u64::from(MAX_WIDE_RUNTIME_TILES) {
        return err("navmesh_grid_limit");
    }
    if wide && input.bounds.max[1] - input.bounds.min[1] > MAX_TILED_HEIGHT_M {
        return err("navmesh_envelope_invalid");
    }
    if reference
        && (input.bounds.max[1] - input.bounds.min[1] > MAX_TILED_HEIGHT_M
            || input.vertices.iter().any(|point| {
                (0..3).any(|axis| {
                    let value = f64::from(point[axis]);
                    value < input.bounds.min[axis] || value > input.bounds.max[axis]
                })
            }))
    {
        return err("navmesh_reference_envelope_invalid");
    }
    if input
        .vertices
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || v.abs() > 100000.)
        || input
            .triangles
            .iter()
            .flatten()
            .any(|&i| i as usize >= input.vertices.len())
    {
        return err("navmesh_invalid_geometry");
    }
    let mut hash = Sha256::new();
    hash.update(b"MAG-NAV-GEOMETRY-v1\0");
    hash.update((input.vertices.len() as u32).to_le_bytes());
    hash.update((input.triangles.len() as u32).to_le_bytes());
    for v in input.vertices.iter().flatten() {
        hash.update(v.to_le_bytes());
    }
    for i in input.triangles.iter().flatten() {
        hash.update(i.to_le_bytes());
    }
    if format!("{:x}", hash.finalize()) != input.geometry_sha256 {
        return err("navmesh_geometry_hash_mismatch");
    }
    let vertices: Vec<f32> = input.vertices.into_iter().flatten().collect();
    let triangles: Vec<c_int> = input
        .triangles
        .into_iter()
        .flatten()
        .map(|i| i as c_int)
        .collect();
    let min = input.bounds.min.map(|v| v as f32);
    let max = input.bounds.max.map(|v| v as f32);
    let mut data = std::ptr::null_mut();
    let mut size = 0;
    let profile = selected_profile(rules);
    // The bridge borrows validated contiguous arrays and returns dtAlloc-owned bytes.
    let result = unsafe {
        mag_nav_bake(
            vertices.as_ptr(),
            (vertices.len() / 3) as c_int,
            triangles.as_ptr(),
            (triangles.len() / 3) as c_int,
            min.as_ptr(),
            max.as_ptr(),
            &mut data,
            &mut size,
            bake_mode(reference, rules.is_some(), wide),
            profile.max_slope_degrees as f32,
        )
    };
    if result != 0 {
        if reference {
            return err(match result {
                1 => "navmesh_reference_grid_limit",
                5 => "navmesh_reference_tile_polygon_limit",
                6 => "navmesh_reference_size_limit",
                _ => "navmesh_reference_bake_failed",
            });
        }
        return err(match result {
            1 if wide => "navmesh_grid_limit",
            5 => "navmesh_tile_polygon_limit",
            6 => "navmesh_bake_size_limit",
            _ => "navmesh_bake_failed",
        });
    }
    if data.is_null() || size <= 0 || size as u64 > byte_limit(reference, wide) {
        if !data.is_null() {
            unsafe { mag_nav_free_bytes(data) }
        }
        return err("navmesh_bake_size_limit");
    }
    let bytes = unsafe { std::slice::from_raw_parts(data, size as usize).to_vec() };
    unsafe { mag_nav_free_bytes(data) };
    let (format, config) = format_and_config(reference, rules.is_some(), wide);
    Ok(Artifact {
        format: if coverage.is_some() {
            regional::MESH_FORMAT
        } else {
            format
        }
        .into(),
        map_name: input.map_name,
        world_sha256: input.world_sha256,
        geometry_sha256: input.geometry_sha256,
        profile,
        movement_profile_sha256: rules.map(|rules| rules.source_sha256().to_owned()),
        bounds: input.bounds,
        upstream_commit: UPSTREAM_COMMIT.into(),
        bake_config: config.into(),
        detour_sha256: sha256(&bytes),
        detour_data: bytes,
        drop_links: Vec::new(),
    })
}
struct Native(*mut std::ffi::c_void);
// Native queries have mutable scratch state and are accessed only through the mutex.
unsafe impl Send for Native {}
impl Drop for Native {
    fn drop(&mut self) {
        unsafe { mag_nav_destroy(self.0) }
    }
}
pub struct NavMesh {
    /// Loaded metadata. Detour owns checked copies of the tiles, so the
    /// artifact's `detour_data` is released after a successful load.
    artifact: Artifact,
    detour_bytes: usize,
    regional_coverage: Option<regional::Coverage>,
    native: Mutex<Native>,
}
impl NavMesh {
    pub fn load(artifact: Artifact, world_sha256: &str, map_name: &str) -> Result<Self, Error> {
        Self::load_artifact(
            artifact,
            world_sha256,
            map_name,
            false,
            QUERY_NODE_LIMIT,
            None,
            false,
        )
    }
    pub fn load_with_movement_profile(
        artifact: Artifact,
        world_sha256: &str,
        map_name: &str,
        rules: &MovementProfile,
    ) -> Result<Self, Error> {
        Self::load_artifact(
            artifact,
            world_sha256,
            map_name,
            false,
            QUERY_NODE_LIMIT,
            Some(rules),
            false,
        )
    }
    fn load_artifact(
        artifact: Artifact,
        world_sha256: &str,
        map_name: &str,
        reference: bool,
        node_limit: c_int,
        rules: Option<&MovementProfile>,
        regional: bool,
    ) -> Result<Self, Error> {
        let wide = !reference
            && !regional
            && artifact.format == format_and_config(false, rules.is_some(), true).0;
        let (format, config) = format_and_config(reference, rules.is_some(), wide);
        let format = if regional {
            regional::MESH_FORMAT
        } else {
            format
        };
        let legacy_single = !regional
            && !reference
            && rules.is_none()
            && artifact.format == "mag_navmesh_world_v1"
            && artifact.bake_config == LEGACY_BAKE_CONFIG;
        let format_matches =
            legacy_single || (artifact.format == format && artifact.bake_config == config);
        if !format_matches
            || artifact.upstream_commit != UPSTREAM_COMMIT
            || artifact.profile != selected_profile(rules)
            || artifact.movement_profile_sha256.as_deref()
                != rules.map(MovementProfile::source_sha256)
            || !valid_bounds(&artifact.bounds)
            || !valid_sha(&artifact.world_sha256)
            || !valid_sha(&artifact.geometry_sha256)
            || artifact.world_sha256 != world_sha256
            || artifact.map_name != map_name
        {
            return err("navmesh_identity_mismatch");
        }
        if rules.is_some() && !artifact.drop_links.is_empty() {
            return err("navmesh_profiled_drop_links_unsupported");
        }
        if artifact.detour_data.len() < 100
            || artifact.detour_data.len() as u64 > byte_limit(reference, wide)
            || artifact.detour_sha256 != sha256(&artifact.detour_data)
        {
            return err("navmesh_binary_hash_mismatch");
        }
        let handle = unsafe {
            mag_nav_load(
                artifact.detour_data.as_ptr(),
                artifact.detour_data.len() as c_int,
                artifact.bounds.min.map(|v| v as f32).as_ptr(),
                artifact.bounds.max.map(|v| v as f32).as_ptr(),
                bake_mode(reference, rules.is_some(), wide),
                node_limit,
            )
        };
        if handle.is_null() {
            return err("navmesh_invalid_binary");
        }
        let mut mesh = Self {
            detour_bytes: artifact.detour_data.len(),
            artifact,
            regional_coverage: None,
            native: Mutex::new(Native(handle)),
        };
        mesh.validate_drop_links()?;
        // A wide mesh can hold up to 128 MiB of tiles; keep one copy, not two.
        mesh.artifact.detour_data = Vec::new();
        Ok(mesh)
    }
    /// Runtime coverage comes from the loaded collision world, not artifact assertions.
    pub fn validate_coverage(&self, world: &crate::collision::CollisionWorld) -> Result<(), Error> {
        if let Some(scope) = &self.regional_coverage {
            return scope.validate_world(world);
        }
        if self.artifact.movement_profile_sha256.is_some()
            && world.source_sha256() != self.artifact.world_sha256
        {
            return err("navmesh_source_mismatch");
        }
        if world.metadata().map_name != self.artifact.map_name
            || !world.covers_box(self.artifact.bounds.min, self.artifact.bounds.max)
        {
            return err("navmesh_geometry_coverage_unknown");
        }
        Ok(())
    }
    pub(crate) fn regional_coverage(&self) -> Option<&regional::Coverage> {
        self.regional_coverage.as_ref()
    }
    /// Artifact metadata; `detour_data` is empty once loaded (see `detour_bytes`).
    pub fn artifact(&self) -> &Artifact {
        &self.artifact
    }
    /// Size of the verified Detour data that was loaded.
    pub fn detour_bytes(&self) -> usize {
        self.detour_bytes
    }
    pub fn body(&self) -> crate::bot_physics::Body {
        crate::bot_physics::Body {
            max_slope_degrees: self.artifact.profile.max_slope_degrees,
            ..Default::default()
        }
    }
    pub fn route(&self, start: Point, goal: Point, goal_radius: f64) -> Result<Route, Error> {
        if !valid_point(start)
            || !valid_point(goal)
            || !goal_radius.is_finite()
            || !(0.05..=5.).contains(&goal_radius)
        {
            return err("navmesh_invalid_request");
        }
        let in_region = |point: Point| {
            self.regional_coverage.as_ref().is_none_or(|coverage| {
                (0..3).all(|axis| {
                    point[axis] >= coverage.bounds().min[axis]
                        && point[axis] <= coverage.bounds().max[axis]
                })
            })
        };
        if !in_region(start) || !in_region(goal) {
            return err("navmesh_route_outside_walking_coverage");
        }
        let began = Instant::now();
        let s = start.map(|v| v as f32);
        let g = goal.map(|v| v as f32);
        let mut ps = [0f32; 3];
        let mut pg = [0f32; 3];
        let mut points = vec![0f32; 3072];
        let mut refs = vec![0u64; 1024];
        let mut np = 0;
        let mut nr = 0;
        let h = self
            .native
            .lock()
            .map_err(|_| Error("navmesh_query_poisoned"))?;
        let status = unsafe {
            mag_nav_route(
                h.0,
                s.as_ptr(),
                g.as_ptr(),
                goal_radius as f32,
                ps.as_mut_ptr(),
                pg.as_mut_ptr(),
                points.as_mut_ptr(),
                &mut np,
                refs.as_mut_ptr(),
                &mut nr,
            )
        };
        let status = match status {
            0 => Status::Complete,
            1 => Status::Partial,
            2 => Status::Disconnected,
            3 => Status::OutOfNodes,
            4 => Status::InvalidStart,
            5 => Status::InvalidGoal,
            _ => return err("navmesh_query_failed"),
        };
        if !(0..=1024).contains(&np) || !(0..=1024).contains(&nr) {
            return err("navmesh_query_failed");
        }
        let waypoints: Vec<Point> = points[..np as usize * 3]
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64])
            .collect();
        if waypoints.iter().any(|&point| !in_region(point)) {
            return err("navmesh_route_outside_walking_coverage");
        }
        refs.truncate(nr as usize);
        let length_m = waypoints
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][2] - w[0][2]))
            .sum();
        let waypoint_count = waypoints.len();
        Ok(Route {
            format: "mag_navmesh_route_v1".into(),
            status,
            world_sha256: self.artifact.world_sha256.clone(),
            profile: self.artifact.profile.clone(),
            start,
            requested_goal: goal,
            projected_start: ps.map(f64::from),
            projected_goal: pg.map(f64::from),
            waypoints,
            corridor_refs: refs,
            length_m,
            waypoint_kinds: vec![WaypointKind::Walk; waypoint_count],
            waypoint_drop_ids: vec![None; waypoint_count],
            predicted_remaining_health: None,
            query_us: began.elapsed().as_micros().min(u64::MAX as u128) as u64,
        })
    }
    pub fn component_diagnostic(
        &self,
        start: Point,
        goal: Point,
    ) -> Result<serde_json::Value, Error> {
        if !valid_point(start) || !valid_point(goal) {
            return err("navmesh_invalid_request");
        }
        let h = self
            .native
            .lock()
            .map_err(|_| Error("navmesh_query_poisoned"))?;
        let s = start.map(|v| v as f32);
        let g = goal.map(|v| v as f32);
        let mut closest = [0f32; 3];
        let mut min = [0f32; 3];
        let mut max = [0f32; 3];
        let mut count = 0;
        let mut total = 0;
        let ok = unsafe {
            mag_nav_component(
                h.0,
                s.as_ptr(),
                g.as_ptr(),
                closest.as_mut_ptr(),
                min.as_mut_ptr(),
                max.as_mut_ptr(),
                &mut count,
                &mut total,
            )
        };
        if ok == 0 {
            return err("navmesh_invalid_start");
        }
        Ok(
            serde_json::json!({"component_polygons":count,"total_polygons":total,"bounds":{"min":min,"max":max},"closest_to_goal":closest}),
        )
    }
    pub fn component_id(&self, position: Point) -> Result<i32, Error> {
        if !valid_point(position) {
            return err("navmesh_invalid_request");
        }
        let h = self
            .native
            .lock()
            .map_err(|_| Error("navmesh_query_poisoned"))?;
        let p = position.map(|v| v as f32);
        let id = unsafe { mag_nav_component_id(h.0, p.as_ptr()) };
        if id < 0 {
            return err("navmesh_invalid_start");
        }
        Ok(id)
    }
    /// Project a bounded sample onto at most one polygon per walking component.
    pub fn goal_candidates(&self, goal: Point, radius: f64) -> Result<Vec<(i32, Point)>, Error> {
        self.goals(goal, radius)
    }

    fn goals(&self, goal: Point, radius: f64) -> Result<Vec<(i32, Point)>, Error> {
        if !valid_point(goal) || !radius.is_finite() || !(0.05..=5.).contains(&radius) {
            return err("navmesh_invalid_request");
        }
        let h = self
            .native
            .lock()
            .map_err(|_| Error("navmesh_query_poisoned"))?;
        let g = goal.map(|v| v as f32);
        let mut ids = [0; 128];
        let mut points = [0f32; 384];
        let mut count = 0;
        let ok = unsafe {
            mag_nav_goals(
                h.0,
                g.as_ptr(),
                radius as f32,
                ids.as_mut_ptr(),
                points.as_mut_ptr(),
                &mut count,
            )
        };
        if ok == 0 || !(0..=128).contains(&count) {
            return err("navmesh_goal_candidate_limit");
        }
        let results: Vec<_> = (0..count as usize)
            .map(|i| {
                (
                    ids[i],
                    [
                        points[i * 3] as f64,
                        points[i * 3 + 1] as f64,
                        points[i * 3 + 2] as f64,
                    ],
                )
            })
            .collect();
        if results
            .iter()
            .any(|(id, point)| *id < 0 || !valid_point(*point))
        {
            return err("navmesh_invalid_goal_candidate");
        }
        Ok(results)
    }
    pub fn contains(&self, position: Point, route: &Route) -> bool {
        self.contains_route_position(position, route, mag_nav_contains)
    }

    /// Bounded horizontal corridor evidence for vertical settlement only.
    pub(super) fn contains_settlement_column(&self, position: Point, route: &Route) -> bool {
        self.contains_route_position(position, route, mag_nav_contains_settlement_column)
    }

    fn contains_route_position(
        &self,
        position: Point,
        route: &Route,
        query: unsafe extern "C" fn(*mut std::ffi::c_void, *const f32, *const u64, c_int) -> c_int,
    ) -> bool {
        if !valid_point(position)
            || route.status != Status::Complete
            || route.world_sha256 != self.artifact.world_sha256
            || route.corridor_refs.is_empty()
            || route.corridor_refs.len() > 1024
        {
            return false;
        }
        let Ok(h) = self.native.lock() else {
            return false;
        };
        let p = position.map(|v| v as f32);
        unsafe {
            query(
                h.0,
                p.as_ptr(),
                route.corridor_refs.as_ptr(),
                route.corridor_refs.len() as c_int,
            ) != 0
        }
    }
}
