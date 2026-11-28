//! Offline Recast bake and shared Detour navigation for verified static worlds.
//! Route queries never certify raw collision edges; physical movement remains authoritative.
pub use crate::bot_physics::BodyShape;
use mag_navmesh_sys::{
    mag_nav_add_gate, mag_nav_clear_gates, mag_nav_poly_heights, mag_nav_poly_info,
    mag_nav_polys_in_box,
};
use mag_navmesh_sys::{
    mag_nav_bake, mag_nav_column_goals, mag_nav_component, mag_nav_component_id, mag_nav_contains,
    mag_nav_contains_settlement_column, mag_nav_destroy, mag_nav_free_bytes, mag_nav_goals,
    mag_nav_load, mag_nav_route, mag_nav_tile_stats,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    ffi::c_int,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
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
#[path = "navmesh_artifact_read_tests.rs"]
mod artifact_read_tests;
#[cfg(test)]
#[path = "navmesh_foreground_tests.rs"]
mod foreground_tests;
#[cfg(test)]
#[path = "navmesh_goal_tests.rs"]
mod goal_tests;
#[cfg(test)]
#[path = "navmesh_movement_profile_tests.rs"]
mod movement_profile_tests;
#[cfg(test)]
#[path = "navmesh_staged_tests.rs"]
mod staged_tests;
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
/// Polygons, and straight-path points, one route may carry. A direct search
/// still stops at 1,024; staged routes over long distances use the rest.
pub const MAX_ROUTE_CORRIDOR: usize = 4096;
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
    /// Static collision layers character movement collides with, when the
    /// bake input left out the others (`prepare.py --filter-rows`). Bots
    /// moving on this mesh use the same layers (`NavMesh::load_into`), so the
    /// mesh and bot physics agree on which triangles are solid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_movement_layers: Option<u32>,
    /// The bot body shape this mesh and its drop certificates are for, when
    /// it is not the default box (`BodyShape::stamp`). A mesh loads only for
    /// that body (`NavMesh::load_with_body`); absent means the box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_shape: Option<BodyShape>,
}
/// An artifact read from a file whose Detour digest matched the bytes that
/// one read produced (`NavMesh::load_verified` takes it).
pub struct VerifiedArtifact(Artifact);
impl VerifiedArtifact {
    /// Read the artifact at `path` into memory once, parse it and hash its
    /// Detour data from those bytes. A read that does not verify is logged
    /// and read again (`verified_read`); when none does, the error names
    /// the file and both digests (`navmesh_binary_hash_mismatch: ...`).
    pub fn read(path: &std::path::Path) -> std::io::Result<Self> {
        Ok(Self(crate::verified_read::read(
            path,
            MAX_ARTIFACT_JSON_BYTES,
            "navmesh_artifact",
            decode_artifact::<Artifact>,
        )?))
    }
    #[cfg(test)]
    pub(crate) fn read_with(
        path: &std::path::Path,
        load: impl FnMut() -> std::io::Result<Vec<u8>>,
    ) -> std::io::Result<Self> {
        Ok(Self(crate::verified_read::read_with(
            path,
            "navmesh_artifact",
            load,
            std::time::Duration::ZERO,
            decode_artifact::<Artifact>,
        )?))
    }
    pub fn artifact(&self) -> &Artifact {
        &self.0
    }
}
/// A document carrying one Detour binary and the digest recorded for it.
pub(crate) trait DetourPayload: serde::de::DeserializeOwned {
    fn detour(&self) -> &Artifact;
}
impl DetourPayload for Artifact {
    fn detour(&self) -> &Artifact {
        self
    }
}
/// Parse `bytes` and compare the recorded Detour digest with the Detour
/// data parsed from those same bytes.
pub(crate) fn decode_artifact<T: DetourPayload>(
    bytes: &[u8],
) -> Result<T, crate::verified_read::Rejection<std::convert::Infallible>> {
    use crate::verified_read::Rejection;
    let value: T = serde_json::from_slice(bytes).map_err(|e| Rejection::Decode(e.to_string()))?;
    let artifact = value.detour();
    let actual = sha256(&artifact.detour_data);
    if actual != artifact.detour_sha256 {
        return Err(Rejection::Digest {
            code: "navmesh_binary_hash_mismatch",
            expected: artifact.detour_sha256.clone(),
            actual,
            bytes: artifact.detour_data.len(),
        });
    }
    Ok(value)
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
    /// Detour search nodes the query used (in-process diagnostic only).
    #[serde(skip)]
    pub search_nodes: u32,
    /// Corridor pieces of a staged route; zero for a single direct search.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub stages: u32,
}
fn is_zero(value: &u32) -> bool {
    *value == 0
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
        character_movement_layers: None,
        body_shape: None,
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
    /// The runtime body shape the mesh was loaded for.
    body_shape: BodyShape,
    native: Mutex<Native>,
    /// Key of the structure snapshot the installed gates came from.
    structure_gates: Mutex<Option<(u64, u32)>>,
    /// That snapshot, for steering routes around structures in a polygon.
    structure_snapshot:
        Mutex<Option<std::sync::Arc<crate::bot_structure_collision::StructureObstacles>>>,
    /// The installed gates by polygon, with the structures each came from.
    structure_gate_cache: Mutex<std::sync::Arc<GateCache>>,
    /// World ticks using the mesh now (see [`NavMesh::foreground`]).
    foreground: AtomicUsize,
}

thread_local! {
    /// Whether this thread runs a world tick that holds a [`Foreground`].
    static FOREGROUND: Cell<bool> = const { Cell::new(false) };
}

/// Longest a background query waits for running world ticks before it takes
/// its turn anyway, so a world that is always ticking cannot starve it.
const BACKGROUND_WAIT_LIMIT: Duration = Duration::from_millis(200);

/// Marks a world tick's use of a [`NavMesh`]; see [`NavMesh::foreground`].
pub(crate) struct Foreground<'a> {
    mesh: &'a NavMesh,
    outer: bool,
}

impl Drop for Foreground<'_> {
    fn drop(&mut self) {
        self.mesh.foreground.fetch_sub(1, Ordering::Release);
        FOREGROUND.with(|f| f.set(self.outer));
    }
}
pub(crate) type GateCache =
    std::collections::HashMap<u64, (crate::navmesh_structure_gates::GateSources, PolyGate)>;
/// One navmesh polygon's outline and links (neighbour, shared edge).
pub(crate) struct PolyInfo {
    pub(crate) vertices: Vec<Point>,
    pub(crate) links: Vec<(u64, Point, Point)>,
}
/// Placed-structure gating of one polygon; see `navmesh_structure_gates`.
#[derive(Clone)]
pub(crate) struct PolyGate {
    pub(crate) poly: u64,
    pub(crate) origin_xz: [f64; 2],
    pub(crate) cell: f64,
    pub(crate) size: [usize; 2],
    pub(crate) labels: Vec<u8>,
    /// (neighbour, regions at the shared edge, those also reaching another link).
    pub(crate) links: Vec<(u64, u64, u64)>,
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
    /// Load a mesh for the runtime `body` shape. The mesh and every drop
    /// certificate in it must record that same shape (absent: the box), so
    /// the body that follows a certified drop is the one that traced it.
    pub fn load_with_body(
        artifact: Artifact,
        world_sha256: &str,
        map_name: &str,
        rules: Option<&MovementProfile>,
        body: BodyShape,
    ) -> Result<Self, Error> {
        Self::load_artifact_with_body(
            artifact,
            world_sha256,
            map_name,
            false,
            QUERY_NODE_LIMIT,
            rules,
            false,
            body,
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
        Self::load_artifact_with_body(
            artifact,
            world_sha256,
            map_name,
            reference,
            node_limit,
            rules,
            regional,
            BodyShape::Box,
        )
    }
    /// Load an artifact read by `VerifiedArtifact::read`, for the runtime
    /// `body` shape as `load_with_body` does. Its Detour digest was checked
    /// over the bytes that were read, so it is not hashed a second time.
    pub fn load_verified(
        artifact: VerifiedArtifact,
        world_sha256: &str,
        map_name: &str,
        rules: Option<&MovementProfile>,
        body: BodyShape,
    ) -> Result<Self, Error> {
        Self::load_artifact_checked(
            artifact.0,
            world_sha256,
            map_name,
            false,
            QUERY_NODE_LIMIT,
            rules,
            false,
            body,
            true,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn load_artifact_with_body(
        artifact: Artifact,
        world_sha256: &str,
        map_name: &str,
        reference: bool,
        node_limit: c_int,
        rules: Option<&MovementProfile>,
        regional: bool,
        body: BodyShape,
    ) -> Result<Self, Error> {
        Self::load_artifact_checked(
            artifact,
            world_sha256,
            map_name,
            reference,
            node_limit,
            rules,
            regional,
            body,
            false,
        )
    }
    /// `digest_checked`: the caller already compared the Detour digest with
    /// these exact bytes (`VerifiedArtifact`); the size checks still apply.
    #[allow(clippy::too_many_arguments)]
    fn load_artifact_checked(
        artifact: Artifact,
        world_sha256: &str,
        map_name: &str,
        reference: bool,
        node_limit: c_int,
        rules: Option<&MovementProfile>,
        regional: bool,
        body: BodyShape,
        digest_checked: bool,
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
        // A drop certificate binds the body it was traced with: the default
        // 45 degree body for a legacy mesh, the movement profile's for a
        // profiled one.
        for link in &artifact.drop_links {
            match (
                rules.map(MovementProfile::slope_degrees),
                link.body_slope_degrees,
            ) {
                (None, None) => {}
                (Some(profile), Some(certified)) if profile == certified => {}
                (Some(_), None) => return err("navmesh_profiled_drop_links_unsupported"),
                _ => return err("navmesh_drop_body_mismatch"),
            }
            // And the shape: a box certificate does not hold for a capsule,
            // nor the reverse.
            if BodyShape::from_stamp(link.body_shape) != body {
                return err("navmesh_drop_body_shape_mismatch");
            }
        }
        if BodyShape::from_stamp(artifact.body_shape) != body {
            return err("navmesh_body_shape_mismatch");
        }
        if artifact.detour_data.len() < 100
            || artifact.detour_data.len() as u64 > byte_limit(reference, wide)
            || (!digest_checked && artifact.detour_sha256 != sha256(&artifact.detour_data))
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
            body_shape: body,
            native: Mutex::new(Native(handle)),
            structure_gates: Mutex::new(None),
            structure_snapshot: Mutex::new(None),
            structure_gate_cache: Mutex::new(Default::default()),
            foreground: AtomicUsize::new(0),
        };
        mesh.validate_drop_links()?;
        // A wide mesh can hold up to 128 MiB of tiles; keep one copy, not two.
        mesh.artifact.detour_data = Vec::new();
        Ok(mesh)
    }
    /// Give this thread's queries priority over background queries until the
    /// guard drops. The world tick holds it for the whole tick.
    ///
    /// Every query locks the one native query object. Off-tick workers (spawn
    /// route checks, squad routes, structure gates) run whole-mesh searches
    /// that can hold it for tens of milliseconds each, back to back, while a
    /// tick locks it once or more per moving bot. Without a turn order every
    /// such lock could wait for a full background search, so one tick could
    /// last a second. With the guard held, a background query waits for the
    /// tick to finish (at most [`BACKGROUND_WAIT_LIMIT`]) before it starts, so
    /// a tick waits for at most the one background search already running.
    pub(crate) fn foreground(&self) -> Foreground<'_> {
        self.foreground.fetch_add(1, Ordering::Acquire);
        let outer = FOREGROUND.with(|f| f.replace(true));
        Foreground { mesh: self, outer }
    }
    /// Lock the native query object; a background thread first lets running
    /// world ticks finish.
    fn lock_native(&self) -> Result<MutexGuard<'_, Native>, Error> {
        if !FOREGROUND.with(Cell::get) {
            let began = Instant::now();
            while self.foreground.load(Ordering::Acquire) > 0
                && began.elapsed() < BACKGROUND_WAIT_LIMIT
            {
                std::thread::sleep(Duration::from_micros(500));
            }
        }
        self.native
            .lock()
            .map_err(|_| Error("navmesh_query_poisoned"))
    }
    /// Polygons whose bounds overlap the box `min`..`max`.
    pub(crate) fn polys_in_box(&self, min: Point, max: Point) -> Result<Vec<u64>, Error> {
        let center = std::array::from_fn::<f32, 3, _>(|i| ((min[i] + max[i]) * 0.5) as f32);
        let half = std::array::from_fn::<f32, 3, _>(|i| ((max[i] - min[i]) * 0.5) as f32);
        let h = self.lock_native()?;
        let mut out = vec![0u64; 4096];
        let n = unsafe {
            mag_nav_polys_in_box(h.0, center.as_ptr(), half.as_ptr(), out.as_mut_ptr(), 4096)
        };
        if n < 0 {
            return err("navmesh_structure_polygons_overflow");
        }
        out.truncate(n as usize);
        Ok(out)
    }
    pub(crate) fn poly_info(&self, poly: u64) -> Result<PolyInfo, Error> {
        let h = self.lock_native()?;
        let mut verts = [0f32; 18];
        let mut links = [0u64; 64];
        let mut portals = [0f32; 6 * 64];
        let (mut nv, mut nl) = (0, 0);
        let ok = unsafe {
            mag_nav_poly_info(
                h.0,
                poly,
                verts.as_mut_ptr(),
                &mut nv,
                links.as_mut_ptr(),
                portals.as_mut_ptr(),
                64,
                &mut nl,
            )
        };
        if ok == 0 {
            return err("navmesh_structure_polygon_invalid");
        }
        let at = |v: &[f32], i: usize| [v[i] as f64, v[i + 1] as f64, v[i + 2] as f64];
        Ok(PolyInfo {
            vertices: (0..nv as usize).map(|i| at(&verts, 3 * i)).collect(),
            links: (0..nl as usize)
                .map(|i| (links[i], at(&portals, 6 * i), at(&portals, 6 * i + 3)))
                .collect(),
        })
    }
    /// Polygon surface height at each XZ point; None outside the polygon.
    pub(crate) fn poly_heights(
        &self,
        poly: u64,
        xz: &[[f64; 2]],
    ) -> Result<Vec<Option<f64>>, Error> {
        let flat: Vec<f32> = xz.iter().flat_map(|p| [p[0] as f32, p[1] as f32]).collect();
        let mut y = vec![0f32; xz.len()];
        let h = self.lock_native()?;
        unsafe {
            mag_nav_poly_heights(h.0, poly, flat.as_ptr(), xz.len() as c_int, y.as_mut_ptr())
        };
        Ok(y.into_iter()
            .map(|v| v.is_finite().then_some(v as f64))
            .collect())
    }
    /// Key of the installed structure gates (None: none installed).
    pub(crate) fn structure_gate_key(&self) -> Option<(u64, u32)> {
        self.structure_gates.lock().ok().and_then(|key| *key)
    }
    /// Replace every structure gate. Routes from then on avoid sealed
    /// crossings and end before one when nothing else reaches the goal.
    /// The installed gates by polygon (shared; cheap to take).
    pub(crate) fn structure_gate_cache(&self) -> std::sync::Arc<GateCache> {
        self.structure_gate_cache
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default()
    }
    pub(crate) fn set_structure_gates(
        &self,
        snapshot: Option<&std::sync::Arc<crate::bot_structure_collision::StructureObstacles>>,
        gates: &[PolyGate],
        cache: GateCache,
    ) -> Result<(), Error> {
        *self
            .structure_gate_cache
            .lock()
            .map_err(|_| Error("navmesh_query_poisoned"))? = std::sync::Arc::new(cache);
        let key = snapshot.map(|s| s.key());
        *self
            .structure_snapshot
            .lock()
            .map_err(|_| Error("navmesh_query_poisoned"))? = None;
        let h = self.lock_native()?;
        let mut installed = self
            .structure_gates
            .lock()
            .map_err(|_| Error("navmesh_query_poisoned"))?;
        unsafe { mag_nav_clear_gates(h.0) };
        *installed = None;
        for gate in gates {
            let refs: Vec<u64> = gate.links.iter().map(|l| l.0).collect();
            let full: Vec<u64> = gate.links.iter().map(|l| l.1).collect();
            let through: Vec<u64> = gate.links.iter().map(|l| l.2).collect();
            if gate.labels.len() != gate.size[0] * gate.size[1] {
                unsafe { mag_nav_clear_gates(h.0) };
                return err("navmesh_structure_gate_invalid");
            }
            let ok = unsafe {
                mag_nav_add_gate(
                    h.0,
                    gate.poly,
                    gate.origin_xz[0] as f32,
                    gate.origin_xz[1] as f32,
                    gate.cell as f32,
                    gate.size[0] as c_int,
                    gate.size[1] as c_int,
                    gate.labels.as_ptr(),
                    refs.as_ptr(),
                    full.as_ptr(),
                    through.as_ptr(),
                    refs.len() as c_int,
                )
            };
            if ok == 0 {
                unsafe { mag_nav_clear_gates(h.0) };
                return err("navmesh_structure_gate_invalid");
            }
        }
        *installed = key;
        drop(installed);
        drop(h);
        *self
            .structure_snapshot
            .lock()
            .map_err(|_| Error("navmesh_query_poisoned"))? = snapshot.cloned();
        Ok(())
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
    /// The bot body for this mesh: its slope policy and the body shape it
    /// was loaded for.
    pub fn body(&self) -> crate::bot_physics::Body {
        crate::bot_physics::Body {
            max_slope_degrees: self.artifact.profile.max_slope_degrees,
            ..Default::default()
        }
        .with_shape(self.body_shape)
    }
    pub fn route(&self, start: Point, goal: Point, goal_radius: f64) -> Result<Route, Error> {
        self.route_to(start, goal, goal_radius, false)
    }
    /// As `route`, but a goal a standing structure covers or walls off is
    /// reached beside it: the route ends at the nearest free spot it reaches
    /// within `goal_radius` and counts as complete. For checks that only need
    /// to get near the goal (a spawn's route to its lane objective), never
    /// for an approach that must stand in an interaction range.
    pub fn route_beside(
        &self,
        start: Point,
        goal: Point,
        goal_radius: f64,
    ) -> Result<Route, Error> {
        self.route_to(start, goal, goal_radius, true)
    }
    pub(crate) fn route_to(
        &self,
        start: Point,
        goal: Point,
        goal_radius: f64,
        beside: bool,
    ) -> Result<Route, Error> {
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
        // A staged route's corridor and straight path hold up to this many.
        let mut points = vec![0f32; 3 * MAX_ROUTE_CORRIDOR];
        let mut refs = vec![0u64; MAX_ROUTE_CORRIDOR];
        let mut np = 0;
        let mut nr = 0;
        let mut nodes = 0;
        let mut stages = 0;
        // Placed structures gate the route, unless it starts in a structure's
        // enclosed room: a life started there walks out through the walls.
        let snapshot = self
            .structure_snapshot
            .lock()
            .map_err(|_| Error("navmesh_query_poisoned"))?
            .clone()
            .filter(|s| !s.enclosed(start));
        let h = self.lock_native()?;
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
                &mut nodes,
                &mut stages,
                c_int::from(snapshot.is_some()) | (c_int::from(beside) << 1),
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
        let limit = MAX_ROUTE_CORRIDOR as c_int;
        if !(0..=limit).contains(&np) || !(0..=limit).contains(&nr) {
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
        drop(h);
        refs.truncate(nr as usize);
        // Placed structures inside a corridor polygon: steer around them.
        let waypoints = match snapshot {
            Some(snapshot) => {
                crate::navmesh_structure_gates::refine(self, &snapshot, waypoints, &refs)?
            }
            None => waypoints,
        };
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
            search_nodes: nodes.max(0) as u32,
            stages: stages.max(0) as u32,
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
        let h = self.lock_native()?;
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
        let h = self.lock_native()?;
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

    /// The nearest point of each walking component on every floor within
    /// `radius` horizontally and `height` vertically of `goal`: the floors a
    /// raw point above or below layered geometry could stand on.
    pub fn column_candidates(
        &self,
        goal: Point,
        radius: f64,
        height: f64,
    ) -> Result<Vec<(i32, Point)>, Error> {
        if !valid_point(goal)
            || !radius.is_finite()
            || !(0.05..=5.).contains(&radius)
            || !height.is_finite()
            || !(0.05..=32.).contains(&height)
        {
            return err("navmesh_invalid_request");
        }
        self.goal_query(goal, |h, g, ids, points, count| unsafe {
            mag_nav_column_goals(h, g, radius as f32, height as f32, ids, points, count)
        })
    }

    fn goals(&self, goal: Point, radius: f64) -> Result<Vec<(i32, Point)>, Error> {
        if !valid_point(goal) || !radius.is_finite() || !(0.05..=5.).contains(&radius) {
            return err("navmesh_invalid_request");
        }
        self.goal_query(goal, |h, g, ids, points, count| unsafe {
            mag_nav_goals(h, g, radius as f32, ids, points, count)
        })
    }

    fn goal_query(
        &self,
        goal: Point,
        query: impl FnOnce(*mut std::ffi::c_void, *const f32, *mut c_int, *mut f32, *mut c_int) -> c_int,
    ) -> Result<Vec<(i32, Point)>, Error> {
        let h = self.lock_native()?;
        let g = goal.map(|v| v as f32);
        let mut ids = [0; 128];
        let mut points = [0f32; 384];
        let mut count = 0;
        let ok = query(
            h.0,
            g.as_ptr(),
            ids.as_mut_ptr(),
            points.as_mut_ptr(),
            &mut count,
        );
        if ok == 0 || !(0..=128).contains(&count) {
            return err("navmesh_goal_query_failed");
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
            || route.corridor_refs.len() > MAX_ROUTE_CORRIDOR
        {
            return false;
        }
        let Ok(h) = self.lock_native() else {
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
