//! Disk-loaded neutral world geometry and an independent triangle BVH.
//!
//! Only `mag_collision_world` v1 with baked world-space vertices is accepted.
//! Resource-local catalogs, transforms and opaque physics data are not inputs.
//! Coverage is a producer claim, not certification: even `complete` does not
//! establish material/filter behavior or agreement with the running client.

use crate::capsule_sweep::sweep_capsule_triangle;
pub use crate::capsule_sweep::{TriangleFeature, VerticalCapsule};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashSet},
    fmt,
    fs::File,
    io::Read,
    path::Path,
};

type Vec3 = [f64; 3];

#[derive(Clone, Copy, Debug)]
pub struct CollisionLimits {
    pub max_file_bytes: u64,
    pub max_instances: usize,
    pub max_vertices: usize,
    pub max_triangles: usize,
    pub max_abs_coordinate: f64,
}
impl Default for CollisionLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 64 * 1024 * 1024,
            max_instances: 100_000,
            max_vertices: 2_000_000,
            max_triangles: 2_000_000,
            max_abs_coordinate: 10_000_000.0,
        }
    }
}

#[derive(Debug)]
pub struct CollisionError(String);
impl fmt::Display for CollisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CollisionError {}
fn invalid(message: impl Into<String>) -> CollisionError {
    CollisionError(message.into())
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    Provisional,
    Complete,
}

/// A source-verified world-space domain, not inferred from triangle bounds.
/// Both capabilities are required to authorize horizontal body motion.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicsRegion {
    pub min: Vec3,
    pub max: Vec3,
    pub ground_verified: bool,
    pub obstacles_verified: bool,
    pub provenance: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldMetadata {
    pub map_name: String,
    pub build: String,
    pub coverage: Coverage,
    pub provenance: Vec<String>,
    pub unknowns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub physics_regions: Vec<PhysicsRegion>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldDocument {
    format: String,
    schema_version: u32,
    coordinate_space: String,
    metadata: WorldMetadata,
    instances: Vec<InstanceDocument>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceDocument {
    id: String,
    vertices_xyz: Vec<Vec3>,
    triangles: Vec<[usize; 3]>,
    #[serde(default)]
    collision: Option<StaticProperties>,
    #[serde(default)]
    material_index: Option<u8>,
    #[serde(default)]
    terrain_material: bool,
}

#[derive(Clone, Copy)]
enum RayFilter {
    All,
    Movement,
    Row(u32),
}

/// Retail query layer of bullets and other projectile flights.
pub const PROJECTILE_QUERY_LAYER: usize = 23;
/// Retail query layer of blast exposure against the static world.
pub const BLAST_QUERY_LAYER: usize = 24;

/// Explicit physics inputs. Missing metadata remains unavailable to body casts;
/// legacy ray queries do not infer layer0 or zero radius for an absent value.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticProperties {
    pub filter_info: u32,
    /// Explicit uniform child filter for ray-enabled native shape containers.
    /// When absent, this projected instance's child filter matches the root.
    #[serde(default)]
    pub ray_child_filter_info: Option<u32>,
    pub convex_radius: f64,
    pub provenance: String,
    pub child_filter_policy: ChildFilterPolicy,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildFilterPolicy {
    Uniform,
}
pub(crate) struct StaticTriangle<'a> {
    pub vertices: [Vec3; 3],
    pub properties: &'a StaticProperties,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}
impl Bounds {
    fn triangle(vertices: &[Vec3; 3]) -> Self {
        let mut result = Self {
            min: vertices[0],
            max: vertices[0],
        };
        for vertex in &vertices[1..] {
            for (axis, &coordinate) in vertex.iter().enumerate() {
                result.min[axis] = result.min[axis].min(coordinate);
                result.max[axis] = result.max[axis].max(coordinate);
            }
        }
        result
    }
    fn union(self, other: Self) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i].min(other.min[i])),
            max: std::array::from_fn(|i| self.max[i].max(other.max[i])),
        }
    }
    // Finite-segment slab test, inclusive at faces. Parallel axes avoid 0/0.
    fn intersects(self, start: Vec3, delta: Vec3, nearest: f64) -> bool {
        let mut lo: f64 = 0.0;
        let mut hi = nearest;
        for axis in 0..3 {
            if delta[axis] == 0.0 {
                if start[axis] < self.min[axis] || start[axis] > self.max[axis] {
                    return false;
                }
            } else {
                let a = (self.min[axis] - start[axis]) / delta[axis];
                let b = (self.max[axis] - start[axis]) / delta[axis];
                lo = lo.max(a.min(b));
                hi = hi.min(a.max(b));
                if lo > hi {
                    return false;
                }
            }
        }
        true
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct CollisionStats {
    pub instances: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub bvh_nodes: usize,
    pub bounds: Option<Bounds>,
}
#[derive(Clone, Debug, Serialize)]
pub struct RayHit {
    pub instance_id: String,
    pub triangle_index: usize,
    /// Fraction of the requested segment, inclusive of both endpoints.
    pub fraction: f64,
    pub position: Vec3,
    /// Unit face normal following stored winding; queries are double-sided.
    pub normal: Vec3,
}
/// A capsule sweep hit: `hit.position` is the capsule centre at impact and
/// `hit.normal` the contact normal; `contact` is the touched triangle point.
#[derive(Clone, Debug, Serialize)]
pub struct CapsuleHit {
    pub hit: RayHit,
    pub contact: Vec3,
    pub feature: TriangleFeature,
}
#[derive(Debug, Serialize)]
pub struct SegmentQuery<'a> {
    pub nearest: Option<RayHit>,
    pub metadata: &'a WorldMetadata,
}
#[derive(Clone, Debug)]
struct Triangle {
    vertices: [Vec3; 3],
    bounds: Bounds,
    instance: usize,
    triangle_index: usize,
}
#[derive(Clone, Debug)]
enum Node {
    Leaf {
        bounds: Bounds,
        start: usize,
        end: usize,
    },
    Branch {
        bounds: Bounds,
        left: usize,
        right: usize,
    },
}
impl Node {
    fn bounds(&self) -> Bounds {
        match self {
            Self::Leaf { bounds, .. } | Self::Branch { bounds, .. } => *bounds,
        }
    }
}
pub(crate) struct MaterialRayHit {
    pub(crate) fraction: f64,
    pub(crate) material_index: Option<u8>,
    pub(crate) terrain: bool,
    pub(crate) front: bool,
}

#[derive(Clone, Debug)]
pub struct CollisionWorld {
    source_sha256: String,
    bot_regional_guard: Option<std::sync::Arc<crate::bot_physics::RegionalGuard>>,
    /// The mode's placed structures in their current state, as bot movement
    /// sees them; None outside a structural mode.
    bot_structures: Option<std::sync::Arc<crate::bot_structure_collision::StructureObstacles>>,
    /// Static collision layers a character's movement collides with, as a
    /// retail filter-row bitmask; None keeps every triangle solid.
    movement_layers: Option<u32>,
    /// Retail collision filter rows (bit n of row m: layers m and n collide),
    /// when loaded; rays issued for a query layer then skip static triangles
    /// on layers that row passes through.
    filter_rows: Option<std::sync::Arc<[u32; 32]>>,
    coverage_sidecar: Option<std::sync::Arc<crate::collision_coverage::CoverageSidecar>>,
    metadata: WorldMetadata,
    // Geometry is immutable after BVH construction. Preloaded profiles share
    // these buffers instead of duplicating map geometry on each selection.
    instance_ids: std::sync::Arc<[String]>,
    static_properties: std::sync::Arc<[Option<StaticProperties>]>,
    material_indices: std::sync::Arc<[(Option<u8>, bool)]>,
    triangles: std::sync::Arc<[Triangle]>,
    nodes: std::sync::Arc<[Node]>,
    root: Option<usize>,
    stats: CollisionStats,
    max_abs_coordinate: f64,
}
impl CollisionWorld {
    /// Check the same root/child eligibility used by filtered material rays.
    /// Null is the native collision-null slot; its ray semantics remain in the
    /// penetration consumer, independently of the ordinary cost table.
    pub(crate) fn validate_material_ray_bindings(
        &self,
        filter_row: u32,
        known_slots: &BTreeSet<u8>,
        terrain_slots: Option<&BTreeSet<u8>>,
    ) -> Result<(), &'static str> {
        let known = |slot: u8| slot == u8::MAX || known_slots.contains(&slot);
        for (properties, &(material, terrain)) in self
            .static_properties
            .iter()
            .zip(self.material_indices.iter())
        {
            let properties = properties
                .as_ref()
                .ok_or("static_material_filter_binding_required")?;
            let root = properties.filter_info & 31;
            let child = properties
                .ray_child_filter_info
                .unwrap_or(properties.filter_info)
                & 31;
            if filter_row & (1 << root) == 0 || filter_row & (1 << child) == 0 {
                continue;
            }
            if terrain {
                let slots = terrain_slots.ok_or("static_terrain_material_grid_required")?;
                if slots.is_empty() || !slots.iter().copied().all(known) {
                    return Err("static_terrain_material_cost_required");
                }
            } else {
                let slot = material.ok_or("static_material_binding_required")?;
                if !known(slot) {
                    return Err("static_material_cost_required");
                }
            }
        }
        Ok(())
    }

    /// Enumerate immutable represented-world leaves for the separate combined
    /// projectile collector. Callers must also supply every eligible live body
    /// and overlay before using a clear result as a world-flight decision.
    ///
    /// `ReferencePose` deliberately accepts this file's represented poses and
    /// preserves its provisional metadata. It does not establish current parent
    /// transforms. Root and explicit uniform child layer masks are intersected
    /// as a replacement query policy, not Havok collision-agent certification.
    pub fn projectile_obstacles(
        &self,
        start: Vec3,
        end: Vec3,
        radius: f64,
        scope: crate::projectile_contact::Coverage,
        layer_mask: u32,
        budget: &mut usize,
    ) -> Result<Vec<crate::projectile_contact::TriangleObstacle<'_>>, &'static str> {
        use crate::projectile_contact::{Coverage as Scope, TriangleId, TriangleObstacle};
        if scope == Scope::Unavailable {
            return Err("projectile_static_coverage_unavailable");
        }
        if !valid_point(start, self.max_abs_coordinate)
            || !valid_point(end, self.max_abs_coordinate)
            || start == end
            || !radius.is_finite()
            || !(0.0..=100.).contains(&radius)
            || radius == 0.
        {
            return Err("projectile_static_invalid_segment");
        }
        let mut margin: f64 = 0.;
        for row in self.static_properties.iter() {
            *budget = budget
                .checked_sub(1)
                .ok_or("projectile_contact_work_limit")?;
            let properties = row
                .as_ref()
                .ok_or("projectile_static_properties_unavailable")?;
            if layer_mask & (1 << (properties.filter_info & 31)) != 0 {
                margin = margin.max(properties.convex_radius);
            }
        }
        let min = std::array::from_fn(|i| start[i].min(end[i]) - radius - margin);
        let max = std::array::from_fn(|i| start[i].max(end[i]) + radius + margin);
        if scope == Scope::CompleteCurrent
            && self.metadata.coverage != Coverage::Complete
            && !self.metadata.physics_regions.iter().any(|r| {
                r.ground_verified
                    && r.obstacles_verified
                    && (0..3).all(|i| r.min[i] <= min[i] && r.max[i] >= max[i])
            })
        {
            return Err("projectile_static_coverage_unavailable");
        }
        let candidates = self.triangle_indices_in_box(min, max, budget)?;
        let mut result = Vec::with_capacity(candidates.len());
        for index in candidates {
            let triangle = &self.triangles[index];
            let properties = self.static_properties[triangle.instance]
                .as_ref()
                .ok_or("projectile_static_properties_unavailable")?;
            let child = properties
                .ray_child_filter_info
                .unwrap_or(properties.filter_info);
            if layer_mask & (1 << (properties.filter_info & 31)) == 0
                || layer_mask & (1 << (child & 31)) == 0
            {
                continue;
            }
            result.push(TriangleObstacle {
                id: TriangleId::Static {
                    instance: &self.instance_ids[triangle.instance],
                    triangle: triangle.triangle_index,
                },
                vertices: triangle.vertices,
                pose: crate::dynamic_collision::Pose::at([0.; 3]),
                margin: properties.convex_radius,
            });
        }
        Ok(result)
    }
    pub(crate) fn static_query_radius(&self) -> Result<f64, &'static str> {
        if self.metadata.coverage != Coverage::Complete
            && !self
                .metadata
                .physics_regions
                .iter()
                .any(|r| r.ground_verified && r.obstacles_verified)
        {
            return Err("world_cast_incomplete_static_coverage");
        }
        let mut radius: f64 = 0.;
        for row in self.static_properties.iter() {
            let properties = row
                .as_ref()
                .ok_or("world_cast_static_properties_unavailable")?;
            if properties
                .ray_child_filter_info
                .is_some_and(|child| child != properties.filter_info)
            {
                return Err("world_cast_static_child_filter_unqualified");
            }
            radius = radius.max(properties.convex_radius);
        }
        Ok(radius)
    }
    pub(crate) fn static_groups(&self) -> std::collections::BTreeSet<u32> {
        self.static_properties
            .iter()
            .flatten()
            .map(|p| p.filter_info >> 16)
            .collect()
    }
    /// Bounded BVH traversal. The caller expands bounds for convex margins.
    pub(crate) fn static_triangles(
        &self,
        min: Vec3,
        max: Vec3,
        budget: &mut usize,
    ) -> Result<Vec<StaticTriangle<'_>>, &'static str> {
        if self.metadata.coverage != Coverage::Complete
            && !self.metadata.physics_regions.iter().any(|r| {
                r.ground_verified
                    && r.obstacles_verified
                    && (0..3).all(|i| r.min[i] <= min[i] && r.max[i] >= max[i])
            })
        {
            return Err("world_cast_outside_verified_region");
        }
        self.triangle_indices_in_box(min, max, budget)?
            .into_iter()
            .map(|index| {
                let triangle = &self.triangles[index];
                Ok(StaticTriangle {
                    vertices: triangle.vertices,
                    properties: self.static_properties[triangle.instance]
                        .as_ref()
                        .ok_or("world_cast_static_properties_unavailable")?,
                })
            })
            .collect()
    }

    /// Source-bound moving meshes carry geometry directly. They have no
    /// static-world filter record or global partition coverage of their own.
    pub(crate) fn local_triangles_in_box(
        &self,
        min: Vec3,
        max: Vec3,
        budget: &mut usize,
    ) -> Result<Vec<[Vec3; 3]>, &'static str> {
        Ok(self
            .triangle_indices_in_box(min, max, budget)?
            .into_iter()
            .map(|index| self.triangles[index].vertices)
            .collect())
    }

    pub(crate) fn instance_ids(&self) -> &[String] {
        &self.instance_ids
    }

    pub(crate) fn local_projectile_triangles_in_box(
        &self,
        min: Vec3,
        max: Vec3,
        budget: &mut usize,
    ) -> Result<impl Iterator<Item = (&str, usize, [Vec3; 3])>, &'static str> {
        Ok(self
            .triangle_indices_in_box(min, max, budget)?
            .into_iter()
            .map(|index| {
                let triangle = &self.triangles[index];
                (
                    self.instance_ids[triangle.instance].as_str(),
                    triangle.triangle_index,
                    triangle.vertices,
                )
            }))
    }

    fn triangle_indices_in_box(
        &self,
        min: Vec3,
        max: Vec3,
        budget: &mut usize,
    ) -> Result<Vec<usize>, &'static str> {
        if !(0..3).all(|i| min[i].is_finite() && max[i].is_finite() && min[i] <= max[i]) {
            return Err("world_cast_invalid_bounds");
        }
        let overlaps = |b: Bounds| (0..3).all(|i| b.min[i] <= max[i] && b.max[i] >= min[i]);
        let mut stack = Vec::new();
        if let Some(root) = self.root {
            stack.push(root);
        }
        let mut rows = Vec::new();
        while let Some(index) = stack.pop() {
            if *budget == 0 {
                return Err("world_cast_work_limit");
            }
            *budget -= 1;
            let node = &self.nodes[index];
            if !overlaps(node.bounds()) {
                continue;
            }
            match node {
                Node::Branch { left, right, .. } => {
                    stack.push(*right);
                    stack.push(*left);
                }
                Node::Leaf { start, end, .. } => {
                    for (triangle, t) in self.triangles[*start..*end].iter().enumerate() {
                        if *budget == 0 {
                            return Err("world_cast_work_limit");
                        }
                        *budget -= 1;
                        if overlaps(t.bounds) {
                            rows.push(*start + triangle);
                        }
                    }
                }
            }
        }
        Ok(rows)
    }
    /// Read a regular JSON file once, validate all geometry, and build a BVH.
    /// Any read/schema/geometry failure is an error, never an empty world.
    pub fn load(path: impl AsRef<Path>, limits: CollisionLimits) -> Result<Self, CollisionError> {
        Self::validate_limits(limits)?;
        let path = path.as_ref();
        let info =
            std::fs::metadata(path).map_err(|e| invalid(format!("collision metadata: {e}")))?;
        if !info.is_file() {
            return Err(invalid("collision input must be a regular file"));
        }
        if info.len() > limits.max_file_bytes {
            return Err(invalid("collision file byte limit exceeded"));
        }
        let file = File::open(path).map_err(|e| invalid(format!("collision open: {e}")))?;
        let mut bytes = Vec::new();
        file.take(limits.max_file_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| invalid(format!("collision read: {e}")))?;
        if bytes.len() as u64 > limits.max_file_bytes {
            return Err(invalid("collision file byte limit exceeded"));
        }
        Self::from_bytes(&bytes, limits)
    }

    /// `load` for a world whose file SHA-256 another artifact recorded (a
    /// navmesh's `world_sha256`). The digest is taken over the bytes read,
    /// before parsing; a read that does not match is logged and read again
    /// (`verified_read`), and when none matches the error names the file and
    /// both digests (`collision_world_hash_mismatch: ...`).
    pub fn load_expecting(
        path: impl AsRef<Path>,
        limits: CollisionLimits,
        expected_sha256: &str,
    ) -> Result<Self, CollisionError> {
        Self::validate_limits(limits)?;
        let path = path.as_ref();
        let info =
            std::fs::metadata(path).map_err(|e| invalid(format!("collision metadata: {e}")))?;
        if !info.is_file() {
            return Err(invalid("collision input must be a regular file"));
        }
        crate::verified_read::read(path, limits.max_file_bytes, "collision_world", |bytes| {
            let actual = crate::verified_read::sha256(bytes);
            if actual != expected_sha256 {
                return Err(crate::verified_read::Rejection::Digest {
                    code: "collision_world_hash_mismatch",
                    expected: expected_sha256.to_string(),
                    actual,
                    bytes: bytes.len(),
                });
            }
            Self::from_digested_bytes(bytes, actual, limits)
                .map_err(crate::verified_read::Rejection::Final)
        })
        .map_err(|failure| match failure {
            crate::verified_read::Failure::Final(error) => error,
            other => invalid(other.to_string()),
        })
    }

    pub(crate) fn from_bytes(
        bytes: &[u8],
        limits: CollisionLimits,
    ) -> Result<Self, CollisionError> {
        Self::from_digested_bytes(bytes, crate::verified_read::sha256(bytes), limits)
    }

    /// `from_bytes` with the SHA-256 of `bytes` already taken.
    fn from_digested_bytes(
        bytes: &[u8],
        source_sha256: String,
        limits: CollisionLimits,
    ) -> Result<Self, CollisionError> {
        Self::validate_limits(limits)?;
        if bytes.len() as u64 > limits.max_file_bytes {
            return Err(invalid("collision file byte limit exceeded"));
        }
        let document: WorldDocument = serde_json::from_slice(bytes)
            .map_err(|e| invalid(format!("collision JSON schema: {e}")))?;
        if document.format != "mag_collision_world"
            || document.schema_version != 1
            || document.coordinate_space != "world"
        {
            return Err(invalid(
                "expected mag_collision_world v1 in world coordinates",
            ));
        }
        let metadata = document.metadata;
        if metadata.map_name.trim().is_empty()
            || metadata.build.trim().is_empty()
            || metadata.provenance.is_empty()
            || metadata.provenance.iter().any(|v| v.trim().is_empty())
            || metadata.unknowns.iter().any(|v| v.trim().is_empty())
            || (metadata.coverage == Coverage::Complete && !metadata.unknowns.is_empty())
            || metadata.physics_regions.len() > 256
            || metadata.physics_regions.iter().any(|r| {
                !valid_point(r.min, limits.max_abs_coordinate)
                    || !valid_point(r.max, limits.max_abs_coordinate)
                    || (0..3).any(|i| r.min[i] >= r.max[i])
                    || r.provenance.is_empty()
                    || r.provenance.iter().any(|p| p.trim().is_empty())
            })
        {
            return Err(invalid("invalid collision provenance or coverage metadata"));
        }
        if document.instances.len() > limits.max_instances {
            return Err(invalid("collision instance limit exceeded"));
        }
        let mut vertex_count = 0usize;
        let mut triangle_count = 0usize;
        for instance in &document.instances {
            vertex_count = vertex_count
                .checked_add(instance.vertices_xyz.len())
                .ok_or_else(|| invalid("vertex count overflow"))?;
            triangle_count = triangle_count
                .checked_add(instance.triangles.len())
                .ok_or_else(|| invalid("triangle count overflow"))?;
        }
        if vertex_count > limits.max_vertices || triangle_count > limits.max_triangles {
            return Err(invalid("collision vertex/triangle limit exceeded"));
        }
        let mut instance_ids = Vec::with_capacity(document.instances.len());
        let mut static_properties = Vec::with_capacity(document.instances.len());
        let mut material_indices = Vec::with_capacity(document.instances.len());
        let mut seen = HashSet::new();
        let mut triangles = Vec::with_capacity(triangle_count);
        for instance in document.instances {
            if instance.collision.as_ref().is_some_and(|p| {
                !p.convex_radius.is_finite()
                    || !(0.0..=100.).contains(&p.convex_radius)
                    || p.provenance.trim().is_empty()
            }) {
                return Err(invalid("invalid static collision properties"));
            }
            if instance.id.trim().is_empty() || !seen.insert(instance.id.clone()) {
                return Err(invalid("empty or duplicate collision instance id"));
            }
            if instance
                .vertices_xyz
                .iter()
                .any(|&v| !valid_point(v, limits.max_abs_coordinate))
            {
                return Err(invalid(format!(
                    "nonfinite or out-of-bounds vertex in {}",
                    instance.id
                )));
            }
            let instance_index = instance_ids.len();
            for (triangle_index, indices) in instance.triangles.into_iter().enumerate() {
                if indices.iter().any(|&i| i >= instance.vertices_xyz.len()) {
                    return Err(invalid(format!(
                        "triangle index outside vertex array in {}",
                        instance.id
                    )));
                }
                let vertices = indices.map(|i| instance.vertices_xyz[i]);
                let normal = cross(sub(vertices[1], vertices[0]), sub(vertices[2], vertices[0]));
                let area_squared = dot(normal, normal);
                if area_squared == 0.0 || !area_squared.is_finite() {
                    return Err(invalid(format!("degenerate triangle in {}", instance.id)));
                }
                triangles.push(Triangle {
                    bounds: Bounds::triangle(&vertices),
                    vertices,
                    instance: instance_index,
                    triangle_index,
                });
            }
            instance_ids.push(instance.id);
            static_properties.push(instance.collision);
            if instance.material_index.is_some() && instance.terrain_material {
                return Err(invalid("ambiguous terrain material binding"));
            }
            material_indices.push((instance.material_index, instance.terrain_material));
        }
        let mut nodes = Vec::new();
        let root = if triangles.is_empty() {
            None
        } else {
            Some(build_bvh(&mut triangles, 0, &mut nodes))
        };
        let stats = CollisionStats {
            instances: instance_ids.len(),
            vertices: vertex_count,
            triangles: triangles.len(),
            bvh_nodes: nodes.len(),
            bounds: root.map(|i| nodes[i].bounds()),
        };
        Ok(Self {
            source_sha256,
            bot_regional_guard: None,
            bot_structures: None,
            movement_layers: None,
            filter_rows: None,
            coverage_sidecar: None,
            metadata,
            instance_ids: instance_ids.into(),
            static_properties: static_properties.into(),
            material_indices: material_indices.into(),
            triangles: triangles.into(),
            nodes: nodes.into(),
            root,
            stats,
            max_abs_coordinate: limits.max_abs_coordinate,
        })
    }
    fn validate_limits(limits: CollisionLimits) -> Result<(), CollisionError> {
        if !limits.max_abs_coordinate.is_finite()
            || limits.max_abs_coordinate <= 0.0
            || limits.max_abs_coordinate > 1e50
            || limits.max_file_bytes == 0
            || limits.max_file_bytes.checked_add(1).is_none()
        {
            return Err(invalid("invalid collision limits"));
        }
        Ok(())
    }

    /// Share immutable geometry with a bot-only guard; disk metadata stays unchanged.
    pub(crate) fn with_bot_regional_guard(&self, guard: crate::bot_physics::RegionalGuard) -> Self {
        let mut world = self.clone();
        world.bot_regional_guard = Some(std::sync::Arc::new(guard));
        world
    }
    /// Restrict character movement queries to static collision layers in
    /// `layers` (bit n set: layer n collides). Triangles on other layers, such
    /// as retail vehicle-only blockers, stop vehicles but not characters.
    /// Instances without explicit filter metadata stay solid. Other queries
    /// (rays, projectiles, seats) are unaffected.
    pub fn with_character_movement_layers(mut self, layers: Option<u32>) -> Self {
        self.movement_layers = layers;
        self
    }
    pub fn character_movement_layers(&self) -> Option<u32> {
        self.movement_layers
    }
    /// Read the 32 retail collision filter rows from a JSON file: a list, or
    /// the first `filter_rows` list found in an object (an Acquisition
    /// gameplay configuration carries one).
    pub fn load_filter_rows(path: impl AsRef<std::path::Path>) -> std::io::Result<[u32; 32]> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(std::io::Error::other("collision_filter_rows_too_large"));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        fn rows(value: &serde_json::Value) -> Option<[u32; 32]> {
            match value {
                serde_json::Value::Array(items) if items.len() == 32 => {
                    let parsed: Option<Vec<u32>> = items
                        .iter()
                        .map(|v| v.as_u64().and_then(|v| u32::try_from(v).ok()))
                        .collect();
                    parsed.and_then(|v| v.try_into().ok())
                }
                serde_json::Value::Object(map) => map
                    .get("filter_rows")
                    .and_then(rows)
                    .or_else(|| map.values().find_map(rows)),
                serde_json::Value::Array(items) => items.iter().find_map(rows),
                _ => None,
            }
        }
        rows(&value).ok_or_else(|| std::io::Error::other("collision_filter_rows_not_found"))
    }
    /// Install the retail collision filter rows used by `query_row`.
    pub fn with_filter_rows(mut self, rows: Option<[u32; 32]>) -> Self {
        self.filter_rows = rows.map(std::sync::Arc::new);
        self
    }
    /// The filter row of query `layer` when rows are loaded; None (every
    /// static triangle collides) otherwise.
    pub fn query_row(&self, layer: usize) -> Option<u32> {
        self.filter_rows
            .as_ref()
            .and_then(|rows| rows.get(layer).copied())
    }
    fn layer_solid(&self, instance: usize, row: u32) -> bool {
        self.static_properties[instance]
            .as_ref()
            .is_none_or(|p| row & (1 << (p.filter_info & 31)) != 0)
    }
    fn movement_solid(&self, instance: usize) -> bool {
        self.movement_layers.is_none_or(|layers| {
            self.static_properties[instance]
                .as_ref()
                .is_none_or(|p| layers & (1 << (p.filter_info & 31)) != 0)
        })
    }
    pub(crate) fn bot_regional_guard(&self) -> Option<&crate::bot_physics::RegionalGuard> {
        self.bot_regional_guard.as_deref()
    }
    /// Placed structures bot movement collides with, in their current state.
    pub(crate) fn bot_structures(
        &self,
    ) -> Option<&crate::bot_structure_collision::StructureObstacles> {
        self.bot_structures.as_deref()
    }
    pub(crate) fn bot_structures_arc(
        &self,
    ) -> Option<&std::sync::Arc<crate::bot_structure_collision::StructureObstacles>> {
        self.bot_structures.as_ref()
    }
    /// Replace the structure snapshot bot movement sees; geometry is shared.
    pub(crate) fn set_bot_structures(
        &mut self,
        structures: Option<std::sync::Arc<crate::bot_structure_collision::StructureObstacles>>,
    ) {
        self.bot_structures = structures;
    }
    /// Sweep a world-axis-aligned box (centre `start` to `end`, half extents
    /// `half`) against this mesh placed at `pose` (world = rotation * local +
    /// translation). Triangles are moved into world space and tested exactly
    /// as `sweep_aabb` tests static ones. With `escape`, a moving sweep skips
    /// triangles the box already penetrates at `start`, so a body a structure
    /// appeared on can leave it; it still stops at every other triangle.
    pub(crate) fn posed_sweep_aabb(
        &self,
        pose: crate::dynamic_collision::Pose,
        start: Vec3,
        end: Vec3,
        half: Vec3,
        escape: bool,
        budget: &mut usize,
    ) -> Result<Option<(f64, Vec3)>, &'static str> {
        if !start.iter().chain(end.iter()).all(|v| v.is_finite())
            || half
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v > 100.0)
        {
            return Err("posed_sweep_invalid");
        }
        let to_local = |p: Vec3| -> Vec3 {
            let d = sub(p, pose.translation);
            std::array::from_fn(|axis| (0..3).map(|row| d[row] * pose.rotation[row][axis]).sum())
        };
        let to_world = |p: Vec3| -> Vec3 {
            std::array::from_fn(|row| {
                (0..3)
                    .map(|axis| pose.rotation[row][axis] * p[axis])
                    .sum::<f64>()
                    + pose.translation[row]
            })
        };
        // The swept box's world bounds, as a local box enclosing its corners.
        let lo: Vec3 = std::array::from_fn(|i| start[i].min(end[i]) - half[i]);
        let hi: Vec3 = std::array::from_fn(|i| start[i].max(end[i]) + half[i]);
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for corner in 0..8 {
            let world = [
                if corner & 1 == 0 { lo[0] } else { hi[0] },
                if corner & 2 == 0 { lo[1] } else { hi[1] },
                if corner & 4 == 0 { lo[2] } else { hi[2] },
            ];
            let local = to_local(world);
            for i in 0..3 {
                min[i] = min[i].min(local[i]);
                max[i] = max[i].max(local[i]);
            }
        }
        let delta = sub(end, start);
        let moving = delta != [0.0; 3];
        let mut nearest: Option<(f64, Vec3)> = None;
        for index in self.triangle_indices_in_box(min, max, budget)? {
            let vertices = self.triangles[index].vertices.map(to_world);
            let Some((fraction, normal)) = sweep_triangle(start, delta, half, vertices) else {
                continue;
            };
            if escape
                && moving
                && fraction <= 0.0
                && sweep_triangle(start, [0.0; 3], half, vertices).is_some()
            {
                continue;
            }
            if nearest.is_none_or(|(f, _)| fraction < f) {
                nearest = Some((fraction, normal));
            }
        }
        Ok(nearest)
    }
    /// Nearest crossing of the world segment `start`..`end` with this mesh
    /// placed at `pose`: (fraction, world position, world unit normal).
    pub(crate) fn posed_raycast(
        &self,
        pose: crate::dynamic_collision::Pose,
        start: Vec3,
        end: Vec3,
        budget: &mut usize,
    ) -> Result<Option<(f64, Vec3, Vec3)>, &'static str> {
        let to_local = |p: Vec3| -> Vec3 {
            let d = sub(p, pose.translation);
            std::array::from_fn(|axis| (0..3).map(|row| d[row] * pose.rotation[row][axis]).sum())
        };
        let hit = self
            .raycast_segment_bounded(to_local(start), to_local(end), budget)
            .map_err(|_| "posed_ray_failed")?
            .nearest;
        Ok(hit.map(|h| {
            let normal: Vec3 = std::array::from_fn(|row| {
                (0..3)
                    .map(|axis| pose.rotation[row][axis] * h.normal[axis])
                    .sum()
            });
            let position = std::array::from_fn(|i| start[i] + (end[i] - start[i]) * h.fraction);
            (h.fraction, position, normal)
        }))
    }
    /// Share immutable geometry with a sidecar bound to this world's exact
    /// bytes and map. The disk metadata and source hash stay unchanged.
    pub fn with_coverage_sidecar(
        &self,
        sidecar: crate::collision_coverage::CoverageSidecar,
    ) -> Result<Self, &'static str> {
        if self.coverage_sidecar.is_some() {
            return Err("collision_coverage_already_attached");
        }
        if sidecar.world_sha256() != self.source_sha256
            || sidecar.map_name() != self.metadata.map_name
        {
            return Err("collision_coverage_world_mismatch");
        }
        let mut world = self.clone();
        world.coverage_sidecar = Some(std::sync::Arc::new(sidecar));
        Ok(world)
    }
    pub fn coverage_sidecar(&self) -> Option<&crate::collision_coverage::CoverageSidecar> {
        self.coverage_sidecar.as_deref()
    }
    /// Regions that authorize bot navigation: the world's own regions, then any
    /// attached sidecar's. Static world casts use only `metadata().physics_regions`.
    pub fn verified_physics_regions(&self) -> impl Iterator<Item = &PhysicsRegion> {
        self.metadata.physics_regions.iter().chain(
            self.coverage_sidecar
                .iter()
                .flat_map(|sidecar| sidecar.regions()),
        )
    }
    /// One ground- and obstacle-verified navigation region contains the box.
    pub fn covers_box(&self, min: Vec3, max: Vec3) -> bool {
        crate::collision_coverage::regions_cover(self.verified_physics_regions(), min, max)
    }
    /// Fingerprint of the validated disk input, cached once at load.
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
    pub fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }
    pub fn stats(&self) -> CollisionStats {
        self.stats
    }
    /// Return all interior crossings with source subpart material and winding.
    pub(crate) fn material_ray_hits(
        &self,
        start: Vec3,
        end: Vec3,
        budget: &mut usize,
    ) -> Result<Vec<MaterialRayHit>, CollisionError> {
        self.collect_material_ray_hits(start, end, None, budget)
    }

    /// Traverse eligible interior ray hits using projected root/child filters.
    /// Missing collision properties on an intersected triangle are an error.
    pub(crate) fn filtered_material_ray_hits(
        &self,
        start: Vec3,
        end: Vec3,
        filter_row: u32,
        budget: &mut usize,
    ) -> Result<Vec<MaterialRayHit>, CollisionError> {
        self.collect_material_ray_hits(start, end, Some(filter_row), budget)
    }

    fn collect_material_ray_hits(
        &self,
        start: Vec3,
        end: Vec3,
        filter_row: Option<u32>,
        budget: &mut usize,
    ) -> Result<Vec<MaterialRayHit>, CollisionError> {
        if !valid_point(start, self.max_abs_coordinate)
            || !valid_point(end, self.max_abs_coordinate)
            || start == end
        {
            return Err(invalid("invalid material ray"));
        }
        let delta = sub(end, start);
        let mut hits = Vec::new();
        let mut stack: Vec<_> = self.root.into_iter().collect();
        while let Some(index) = stack.pop() {
            consume_budget(budget)?;
            let node = &self.nodes[index];
            if !node.bounds().intersects(start, delta, 1.0) {
                continue;
            }
            match node {
                Node::Branch { left, right, .. } => stack.extend([*right, *left]),
                Node::Leaf {
                    start: first,
                    end: last,
                    ..
                } => {
                    for triangle in &self.triangles[*first..*last] {
                        consume_budget(budget)?;
                        if let Some(fraction) = intersect_triangle(start, delta, triangle.vertices)
                            && fraction > 0.0
                            && fraction < 1.0
                        {
                            if let Some(row) = filter_row {
                                let properties = self.static_properties[triangle.instance]
                                    .as_ref()
                                    .ok_or_else(|| {
                                        invalid("filtered ray missing collision properties")
                                    })?;
                                let root = properties.filter_info & 31;
                                let child = properties
                                    .ray_child_filter_info
                                    .unwrap_or(properties.filter_info)
                                    & 31;
                                if row & (1 << root) == 0 || row & (1 << child) == 0 {
                                    continue;
                                }
                            }
                            let (material_index, terrain) =
                                self.material_indices[triangle.instance];
                            hits.push(MaterialRayHit {
                                fraction,
                                material_index,
                                terrain,
                                // The native triangle caster orients its returned
                                // normal toward the ray's starting half-space.
                                front: true,
                            });
                        }
                    }
                }
            }
        }
        Ok(hits)
    }

    /// Test a world-space axis aligned box against this mesh in the
    /// supplied rigid placement. BVH pruning is in the mesh coordinate space.
    pub(crate) fn overlaps_placed_aabb(
        &self,
        pose: crate::dynamic_collision::Pose,
        centre: Vec3,
        half: Vec3,
        budget: &mut usize,
    ) -> Result<bool, CollisionError> {
        if !valid_point(centre, self.max_abs_coordinate)
            || half
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v > 100.0)
        {
            return Err(invalid("invalid placed body box"));
        }
        let offset = sub(centre, pose.translation);
        let local: Vec3 = std::array::from_fn(|axis| {
            (0..3)
                .map(|row| offset[row] * pose.rotation[row][axis])
                .sum()
        });
        let extent: Vec3 = std::array::from_fn(|axis| {
            (0..3)
                .map(|row| half[row] * pose.rotation[row][axis].abs())
                .sum()
        });
        let mut stack: Vec<_> = self.root.into_iter().collect();
        while let Some(index) = stack.pop() {
            consume_budget(budget)?;
            let node = &self.nodes[index];
            let bounds = node.bounds();
            if (0..3).any(|axis| {
                bounds.max[axis] < local[axis] - extent[axis]
                    || bounds.min[axis] > local[axis] + extent[axis]
            }) {
                continue;
            }
            match node {
                Node::Branch { left, right, .. } => stack.extend([*right, *left]),
                Node::Leaf { start, end, .. } => {
                    for triangle in &self.triangles[*start..*end] {
                        consume_budget(budget)?;
                        let vertices = triangle.vertices.map(|vertex| {
                            std::array::from_fn(|axis| {
                                dot(pose.rotation[axis], vertex) + pose.translation[axis]
                            })
                        });
                        if sweep_triangle(centre, [0.; 3], half, vertices).is_some() {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    /// Nearest double-sided triangle crossing on [start,end]. A zero-length or
    /// invalid segment is rejected. Coplanar travel has no isolated crossing.
    /// A miss says only that supplied triangles did not intersect the segment.
    pub fn raycast_segment(
        &self,
        start: Vec3,
        end: Vec3,
    ) -> Result<SegmentQuery<'_>, CollisionError> {
        let mut budget = usize::MAX;
        self.raycast_segment_bounded(start, end, &mut budget)
    }
    /// As above, with a shared node/triangle work budget. Exhaustion is an error,
    /// never a clear segment; callers can bound all queries in one bot tick.
    pub fn raycast_segment_bounded(
        &self,
        start: Vec3,
        end: Vec3,
        budget: &mut usize,
    ) -> Result<SegmentQuery<'_>, CollisionError> {
        self.raycast_filtered(start, end, budget, RayFilter::All)
    }
    /// As `raycast_segment_bounded`, seeing only triangles that block
    /// character movement (`with_character_movement_layers`).
    pub fn movement_raycast_bounded(
        &self,
        start: Vec3,
        end: Vec3,
        budget: &mut usize,
    ) -> Result<SegmentQuery<'_>, CollisionError> {
        self.raycast_filtered(start, end, budget, RayFilter::Movement)
    }
    /// As `raycast_segment_bounded`, for a query whose filter row is `row`:
    /// static triangles on layers the row passes through are skipped, and
    /// instances without filter metadata stay solid. None sees every triangle.
    pub fn raycast_row_bounded(
        &self,
        start: Vec3,
        end: Vec3,
        row: Option<u32>,
        budget: &mut usize,
    ) -> Result<SegmentQuery<'_>, CollisionError> {
        let filter = row.map_or(RayFilter::All, RayFilter::Row);
        self.raycast_filtered(start, end, budget, filter)
    }
    fn raycast_filtered(
        &self,
        start: Vec3,
        end: Vec3,
        budget: &mut usize,
        filter: RayFilter,
    ) -> Result<SegmentQuery<'_>, CollisionError> {
        if !valid_point(start, self.max_abs_coordinate)
            || !valid_point(end, self.max_abs_coordinate)
            || start == end
        {
            return Err(invalid(
                "collision segment must be finite, bounded and nonzero",
            ));
        }
        let delta = sub(end, start);
        let mut nearest: Option<(f64, usize)> = None;
        let mut stack = Vec::new();
        if let Some(root) = self.root {
            stack.push(root);
        }
        while let Some(index) = stack.pop() {
            consume_budget(budget)?;
            let node = &self.nodes[index];
            if !node
                .bounds()
                .intersects(start, delta, nearest.map_or(1.0, |h| h.0))
            {
                continue;
            }
            match node {
                Node::Branch { left, right, .. } => {
                    stack.push(*right);
                    stack.push(*left);
                }
                Node::Leaf {
                    start: first,
                    end: last,
                    ..
                } => {
                    for index in *first..*last {
                        consume_budget(budget)?;
                        let triangle = &self.triangles[index];
                        let solid = match filter {
                            RayFilter::All => true,
                            RayFilter::Movement => self.movement_solid(triangle.instance),
                            RayFilter::Row(row) => self.layer_solid(triangle.instance, row),
                        };
                        if !solid {
                            continue;
                        }
                        if let Some(fraction) = intersect_triangle(start, delta, triangle.vertices)
                        {
                            let replace = nearest.is_none_or(|(previous, previous_index)| {
                                let previous_triangle = &self.triangles[previous_index];
                                fraction < previous
                                    || (fraction == previous
                                        && (triangle.instance, triangle.triangle_index)
                                            < (
                                                previous_triangle.instance,
                                                previous_triangle.triangle_index,
                                            ))
                            });
                            if replace {
                                nearest = Some((fraction, index));
                            }
                        }
                    }
                }
            }
        }
        let nearest = nearest.map(|(fraction, index)| {
            let triangle = &self.triangles[index];
            let normal = cross(
                sub(triangle.vertices[1], triangle.vertices[0]),
                sub(triangle.vertices[2], triangle.vertices[0]),
            );
            let length = dot(normal, normal).sqrt();
            RayHit {
                instance_id: self.instance_ids[triangle.instance].clone(),
                triangle_index: triangle.triangle_index,
                fraction,
                position: std::array::from_fn(|i| start[i] + fraction * delta[i]),
                normal: normal.map(|n| n / length),
            }
        });
        Ok(SegmentQuery {
            nearest,
            metadata: &self.metadata,
        })
    }

    /// Continuous collision of a translated axis-aligned body against supplied
    /// double-sided triangles. Half extents describe a conservative upright body,
    /// not a capsule. Includes initial overlap (fraction zero), does not depenetrate.
    pub fn sweep_aabb(
        &self,
        start: Vec3,
        end: Vec3,
        half: Vec3,
        budget: &mut usize,
    ) -> Result<Option<RayHit>, CollisionError> {
        self.sweep_filtered(start, end, half, budget, false)
    }
    /// As `sweep_aabb`, against only triangles that block character movement
    /// (`with_character_movement_layers`).
    pub fn movement_sweep_aabb(
        &self,
        start: Vec3,
        end: Vec3,
        half: Vec3,
        budget: &mut usize,
    ) -> Result<Option<RayHit>, CollisionError> {
        self.sweep_filtered(start, end, half, budget, true)
    }
    fn sweep_filtered(
        &self,
        start: Vec3,
        end: Vec3,
        half: Vec3,
        budget: &mut usize,
        movement: bool,
    ) -> Result<Option<RayHit>, CollisionError> {
        if !valid_point(start, self.max_abs_coordinate)
            || !valid_point(end, self.max_abs_coordinate)
            || half
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v > 100.0)
        {
            return Err(invalid("invalid body sweep"));
        }
        let delta = sub(end, start);
        let nearest = self.sweep_traverse(start, delta, half, budget, movement, |t, _| {
            sweep_triangle(start, delta, half, t.vertices)
        })?;
        Ok(nearest
            .map(|(fraction, index, normal)| self.body_hit(start, delta, fraction, index, normal)))
    }

    /// Continuous collision of a translated upright capsule (see
    /// [`crate::capsule_sweep`]) against double-sided triangles. `start` and
    /// `end` are the capsule centre, midway along its axis. Exact time of
    /// impact; includes initial overlap (fraction zero), does not depenetrate;
    /// tangent motion along a touched surface is not a hit. The hit's
    /// `position` is the centre at impact, as for `sweep_aabb`, and its normal
    /// is the contact normal: the face normal for a face contact, otherwise
    /// from the touched edge or corner toward the axis.
    pub fn sweep_capsule(
        &self,
        start: Vec3,
        end: Vec3,
        capsule: VerticalCapsule,
        budget: &mut usize,
    ) -> Result<Option<CapsuleHit>, CollisionError> {
        self.capsule_filtered(start, end, capsule, budget, false)
    }
    /// As `sweep_capsule`, against only triangles that block character
    /// movement (`with_character_movement_layers`). Same broad phase and
    /// budget accounting as `movement_sweep_aabb`.
    pub fn movement_sweep_capsule(
        &self,
        start: Vec3,
        end: Vec3,
        capsule: VerticalCapsule,
        budget: &mut usize,
    ) -> Result<Option<CapsuleHit>, CollisionError> {
        self.capsule_filtered(start, end, capsule, budget, true)
    }
    fn capsule_filtered(
        &self,
        start: Vec3,
        end: Vec3,
        capsule: VerticalCapsule,
        budget: &mut usize,
        movement: bool,
    ) -> Result<Option<CapsuleHit>, CollisionError> {
        if !valid_point(start, self.max_abs_coordinate)
            || !valid_point(end, self.max_abs_coordinate)
            || !capsule.valid()
        {
            return Err(invalid("invalid body sweep"));
        }
        let delta = sub(end, start);
        let half = capsule.half_extents();
        let nearest = self.sweep_traverse(start, delta, half, budget, movement, |t, nearest| {
            // Exact per-triangle cull: the swept capsule box against the
            // triangle's box, before the narrow phase.
            let expanded = Bounds {
                min: std::array::from_fn(|i| t.bounds.min[i] - half[i]),
                max: std::array::from_fn(|i| t.bounds.max[i] + half[i]),
            };
            if !expanded.intersects(start, delta, nearest) {
                return None;
            }
            sweep_capsule_triangle(start, delta, capsule, t.vertices).map(|c| (c.fraction, c))
        })?;
        Ok(nearest.map(|(fraction, index, contact)| CapsuleHit {
            hit: self.body_hit(start, delta, fraction, index, contact.normal),
            contact: contact.contact,
            feature: contact.feature,
        }))
    }
    fn body_hit(
        &self,
        start: Vec3,
        delta: Vec3,
        fraction: f64,
        index: usize,
        normal: Vec3,
    ) -> RayHit {
        let t = &self.triangles[index];
        RayHit {
            instance_id: self.instance_ids[t.instance].clone(),
            triangle_index: t.triangle_index,
            fraction,
            position: std::array::from_fn(|i| start[i] + fraction * delta[i]),
            normal,
        }
    }
    /// Shared body-sweep broad phase: node bounds grown by `expand` against the
    /// centre ray, pruned by the nearest hit so far, one budget unit per node
    /// and per leaf triangle, ties broken by (instance, triangle index).
    /// `narrow` receives each solid candidate and the current nearest fraction.
    fn sweep_traverse<H>(
        &self,
        start: Vec3,
        delta: Vec3,
        expand: Vec3,
        budget: &mut usize,
        movement: bool,
        mut narrow: impl FnMut(&Triangle, f64) -> Option<(f64, H)>,
    ) -> Result<Option<(f64, usize, H)>, CollisionError> {
        let mut nearest: Option<(f64, usize, H)> = None;
        let mut stack = Vec::new();
        if let Some(root) = self.root {
            stack.push(root);
        }
        while let Some(index) = stack.pop() {
            consume_budget(budget)?;
            let node = &self.nodes[index];
            let b = node.bounds();
            let expanded = Bounds {
                min: std::array::from_fn(|i| b.min[i] - expand[i]),
                max: std::array::from_fn(|i| b.max[i] + expand[i]),
            };
            if !expanded.intersects(start, delta, nearest.as_ref().map_or(1.0, |h| h.0)) {
                continue;
            }
            match node {
                Node::Branch { left, right, .. } => {
                    stack.push(*right);
                    stack.push(*left);
                }
                Node::Leaf {
                    start: first,
                    end: last,
                    ..
                } => {
                    for index in *first..*last {
                        consume_budget(budget)?;
                        let t = &self.triangles[index];
                        if movement && !self.movement_solid(t.instance) {
                            continue;
                        }
                        if let Some((fraction, value)) =
                            narrow(t, nearest.as_ref().map_or(1.0, |h| h.0))
                            && nearest.as_ref().is_none_or(|(f, i, _)| {
                                fraction < *f
                                    || (fraction == *f
                                        && (t.instance, t.triangle_index)
                                            < (
                                                self.triangles[*i].instance,
                                                self.triangles[*i].triangle_index,
                                            ))
                            })
                        {
                            nearest = Some((fraction, index, value));
                        }
                    }
                }
            }
        }
        Ok(nearest)
    }
}
fn consume_budget(budget: &mut usize) -> Result<(), CollisionError> {
    *budget = budget
        .checked_sub(1)
        .ok_or_else(|| invalid("collision query work limit"))?;
    Ok(())
}
// Continuous separating-axis test. Triangle/box separation axes are box axes,
// triangle normal and edge×box axes. Intersect each moving projection interval.
fn sweep_triangle(
    start: Vec3,
    delta: Vec3,
    half: Vec3,
    vertices: [Vec3; 3],
) -> Option<(f64, Vec3)> {
    let edges = [
        sub(vertices[1], vertices[0]),
        sub(vertices[2], vertices[1]),
        sub(vertices[0], vertices[2]),
    ];
    let bases = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let normal = cross(edges[0], edges[1]);
    let mut axes = [[0.0; 3]; 13];
    axes[..3].copy_from_slice(&bases);
    axes[3] = normal;
    for e in 0..3 {
        for b in 0..3 {
            axes[4 + e * 3 + b] = cross(edges[e], bases[b]);
        }
    }
    let mut enter: f64 = 0.0;
    let mut leave: f64 = 1.0;
    let mut contact = [0.0; 3];
    let mut starts_overlapping = true;
    for axis in axes {
        let length = dot(axis, axis).sqrt();
        if length < 1e-12 {
            continue;
        }
        let axis = axis.map(|v| v / length);
        let p = vertices.map(|v| dot(sub(v, start), axis));
        let radius = (0..3).map(|i| half[i] * axis[i].abs()).sum::<f64>();
        let lo = p.into_iter().fold(f64::INFINITY, f64::min) - radius;
        let hi = p.into_iter().fold(f64::NEG_INFINITY, f64::max) + radius;
        let speed = dot(delta, axis);
        // A stationary body query asks for penetration, not tangency. Dynamic
        // sweeps still retain endpoint/touch hits to stop motion into a plane.
        if delta == [0.0; 3] && (lo >= -1e-9 || hi <= 1e-9) {
            return None;
        }
        if lo > 0.0 || hi < 0.0 {
            starts_overlapping = false;
        }
        if speed.abs() < 1e-12 {
            if lo > 0.0 || hi < 0.0 {
                return None;
            }
            continue;
        }
        let a = lo / speed;
        let b = hi / speed;
        let first = a.min(b);
        let last = a.max(b);
        if first > enter {
            enter = first;
            contact = axis.map(|v| if speed > 0.0 { -v } else { v });
        }
        leave = leave.min(last);
        if enter > leave {
            return None;
        }
    }
    if leave < 0.0 || enter > 1.0 {
        return None;
    }
    if starts_overlapping {
        let length = dot(normal, normal).sqrt();
        contact = normal.map(|v| v / length * if dot(normal, delta) > 0.0 { -1.0 } else { 1.0 });
    }
    Some((enter.max(0.0), contact))
}

fn build_bvh(triangles: &mut [Triangle], offset: usize, nodes: &mut Vec<Node>) -> usize {
    let bounds = triangles
        .iter()
        .skip(1)
        .fold(triangles[0].bounds, |b, t| b.union(t.bounds));
    if triangles.len() <= 8 {
        let index = nodes.len();
        nodes.push(Node::Leaf {
            bounds,
            start: offset,
            end: offset + triangles.len(),
        });
        return index;
    }
    let axis = (0..3)
        .max_by(|&a, &b| {
            (bounds.max[a] - bounds.min[a]).total_cmp(&(bounds.max[b] - bounds.min[b]))
        })
        .unwrap();
    let middle = triangles.len() / 2;
    triangles.select_nth_unstable_by(middle, |a, b| {
        let center = |t: &Triangle| (t.bounds.min[axis] + t.bounds.max[axis]) * 0.5;
        center(a)
            .total_cmp(&center(b))
            .then_with(|| (a.instance, a.triangle_index).cmp(&(b.instance, b.triangle_index)))
    });
    let (left, right) = triangles.split_at_mut(middle);
    let left = build_bvh(left, offset, nodes);
    let right = build_bvh(right, offset + middle, nodes);
    let index = nodes.len();
    nodes.push(Node::Branch {
        bounds,
        left,
        right,
    });
    index
}
fn valid_point(point: Vec3, bound: f64) -> bool {
    point.into_iter().all(|v| v.is_finite() && v.abs() <= bound)
}
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn intersect_triangle(start: Vec3, delta: Vec3, vertices: [Vec3; 3]) -> Option<f64> {
    let edge1 = sub(vertices[1], vertices[0]);
    let edge2 = sub(vertices[2], vertices[0]);
    let p = cross(delta, edge2);
    let determinant = dot(edge1, p);
    if determinant == 0.0 {
        return None;
    }
    let t = sub(start, vertices[0]);
    let u = dot(t, p) / determinant;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(t, edge1);
    let v = dot(delta, q) / determinant;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let fraction = dot(edge2, q) / determinant;
    (0.0..=1.0).contains(&fraction).then_some(fraction)
}

#[cfg(test)]
#[path = "collision_filter_rows_tests.rs"]
mod filter_rows_tests;
#[cfg(test)]
#[path = "collision_movement_layers_tests.rs"]
mod movement_layers_tests;

#[cfg(test)]
mod material_ray_binding_tests {
    use super::*;
    use serde_json::{Value, json};

    fn world(material: Option<u8>, terrain: bool, root: u32, child: u32) -> CollisionWorld {
        let mut instance = json!({
            "id":"synthetic-face", "terrain_material":terrain,
            "collision":{"filter_info":root,"ray_child_filter_info":child,
                "convex_radius":0.,"child_filter_policy":"uniform","provenance":"synthetic"},
            "vertices_xyz":[[0.,0.,0.],[1.,0.,0.],[0.,0.,1.]],"triangles":[[0,1,2]]
        });
        if let Some(material) = material {
            instance["material_index"] = Value::from(material);
        }
        CollisionWorld::from_bytes(
            &serde_json::to_vec(&json!({
                "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
                "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                    "provenance":["synthetic"],"unknowns":[]},
                "instances":[instance]
            }))
            .unwrap(),
            Default::default(),
        )
        .unwrap()
    }

    #[test]
    fn eligible_material_rays_reject_unannotated_or_unknown_source_slots() {
        let known = BTreeSet::from([2]);
        assert_eq!(
            world(None, false, 9, 9).validate_material_ray_bindings(1 << 9, &known, None),
            Err("static_material_binding_required")
        );
        assert_eq!(
            world(Some(3), false, 9, 9).validate_material_ray_bindings(1 << 9, &known, None),
            Err("static_material_cost_required")
        );
        assert!(
            world(Some(2), false, 9, 9)
                .validate_material_ray_bindings(1 << 9, &known, None)
                .is_ok()
        );
    }

    #[test]
    fn material_binding_admission_uses_root_and_child_and_preserves_native_null() {
        let known = BTreeSet::new();
        assert!(
            world(None, false, 9, 22)
                .validate_material_ray_bindings(1 << 9, &known, None)
                .is_ok()
        );
        assert!(
            world(None, false, 22, 9)
                .validate_material_ray_bindings(1 << 9, &known, None)
                .is_ok()
        );
        assert!(
            world(Some(u8::MAX), false, 9, 9)
                .validate_material_ray_bindings(1 << 9, &known, None)
                .is_ok()
        );
    }

    #[test]
    fn terrain_material_rays_require_the_bound_grid_and_each_used_cost() {
        let known = BTreeSet::from([2]);
        let terrain = world(None, true, 9, 9);
        assert_eq!(
            terrain.validate_material_ray_bindings(1 << 9, &known, None),
            Err("static_terrain_material_grid_required")
        );
        assert_eq!(
            terrain.validate_material_ray_bindings(1 << 9, &known, Some(&BTreeSet::from([2, 3]))),
            Err("static_terrain_material_cost_required")
        );
        assert!(
            terrain
                .validate_material_ray_bindings(1 << 9, &known, Some(&BTreeSet::from([2, u8::MAX])))
                .is_ok()
        );
    }
}
