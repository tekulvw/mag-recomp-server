//! Disk-loaded neutral world geometry and an independent triangle BVH.
//!
//! Only `mag_collision_world` v1 with baked world-space vertices is accepted.
//! Resource-local catalogs, transforms and opaque physics data are not inputs.
//! Coverage is a producer claim, not certification: even `complete` does not
//! establish material/filter behavior or agreement with the running client.

use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt, fs::File, io::Read, path::Path};

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
#[derive(Debug, Serialize)]
pub struct SegmentQuery<'a> {
    pub nearest: Option<RayHit>,
    pub metadata: &'a WorldMetadata,
}
#[derive(Debug)]
struct Triangle {
    vertices: [Vec3; 3],
    bounds: Bounds,
    instance: usize,
    triangle_index: usize,
}
#[derive(Debug)]
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
#[derive(Debug)]
pub struct CollisionWorld {
    source_sha256: String,
    metadata: WorldMetadata,
    instance_ids: Vec<String>,
    triangles: Vec<Triangle>,
    nodes: Vec<Node>,
    root: Option<usize>,
    stats: CollisionStats,
    max_abs_coordinate: f64,
}
impl CollisionWorld {
    /// Read a regular JSON file once, validate all geometry, and build a BVH.
    /// Any read/schema/geometry failure is an error, never an empty world.
    pub fn load(path: impl AsRef<Path>, limits: CollisionLimits) -> Result<Self, CollisionError> {
        if !limits.max_abs_coordinate.is_finite()
            || limits.max_abs_coordinate <= 0.0
            || limits.max_abs_coordinate > 1e50
            || limits.max_file_bytes == 0
            || limits.max_file_bytes.checked_add(1).is_none()
        {
            return Err(invalid("invalid collision limits"));
        }
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
        let document: WorldDocument = serde_json::from_slice(&bytes)
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
        let mut seen = HashSet::new();
        let mut triangles = Vec::with_capacity(triangle_count);
        for instance in document.instances {
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
        use sha2::Digest;
        let source_sha256 = format!("{:x}", sha2::Sha256::digest(&bytes));
        Ok(Self {
            source_sha256,
            metadata,
            instance_ids,
            triangles,
            nodes,
            root,
            stats,
            max_abs_coordinate: limits.max_abs_coordinate,
        })
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
        if !valid_point(start, self.max_abs_coordinate)
            || !valid_point(end, self.max_abs_coordinate)
            || half
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v > 100.0)
        {
            return Err(invalid("invalid body sweep"));
        }
        let delta = sub(end, start);
        let mut nearest: Option<(f64, usize, Vec3)> = None;
        let mut stack = Vec::new();
        if let Some(root) = self.root {
            stack.push(root);
        }
        while let Some(index) = stack.pop() {
            consume_budget(budget)?;
            let node = &self.nodes[index];
            let b = node.bounds();
            let expanded = Bounds {
                min: std::array::from_fn(|i| b.min[i] - half[i]),
                max: std::array::from_fn(|i| b.max[i] + half[i]),
            };
            if !expanded.intersects(start, delta, nearest.map_or(1.0, |h| h.0)) {
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
                        if let Some((fraction, normal)) =
                            sweep_triangle(start, delta, half, t.vertices)
                            && nearest.is_none_or(|(f, i, _)| {
                                fraction < f
                                    || (fraction == f
                                        && (t.instance, t.triangle_index)
                                            < (
                                                self.triangles[i].instance,
                                                self.triangles[i].triangle_index,
                                            ))
                            })
                        {
                            nearest = Some((fraction, index, normal));
                        }
                    }
                }
            }
        }
        Ok(nearest.map(|(fraction, index, normal)| {
            let t = &self.triangles[index];
            RayHit {
                instance_id: self.instance_ids[t.instance].clone(),
                triangle_index: t.triangle_index,
                fraction,
                position: std::array::from_fn(|i| start[i] + fraction * delta[i]),
                normal,
            }
        }))
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
