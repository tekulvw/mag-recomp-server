//! Owned complete-roster dynamic convex queries for explicit source resources.
//! A snapshot preserves supplied traversal order and every registered root.
use crate::{
    acquisition_atomic::WorldKey,
    acquisition_volume_damage::{Target, TargetKind},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use std::{io::Read, path::Path};
#[path = "dynamic_collision_geometry.rs"]
pub(crate) mod geometry;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pose {
    pub translation: [f64; 3],
    /// Row-major rigid basis; supplied by the qualified source/current authority.
    pub rotation: [[f64; 3]; 3],
}
impl Pose {
    pub fn at(translation: [f64; 3]) -> Self {
        Self {
            translation,
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        }
    }
    pub(crate) fn validate(self) -> Result<(), &'static str> {
        if !self
            .translation
            .into_iter()
            .chain(self.rotation.into_iter().flatten())
            .all(|v| v.is_finite() && v.abs() <= 100_000.)
        {
            return Err("invalid_dynamic_body_pose");
        }
        for i in 0..3 {
            for j in 0..3 {
                if (geometry::dot(self.rotation[i], self.rotation[j])
                    - if i == j { 1. } else { 0. })
                .abs()
                    > 1e-5
                {
                    return Err("invalid_dynamic_body_rotation");
                }
            }
        }
        let r = self.rotation;
        let determinant = r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
            - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
            + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0]);
        if (determinant - 1.).abs() > 1e-5 {
            return Err("invalid_dynamic_body_rotation");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    Sphere {
        radius: f64,
    },
    Capsule {
        a: [f64; 3],
        b: [f64; 3],
        radius: f64,
    },
    Box {
        half_extents: [f64; 3],
        radius: f64,
    },
    Hull {
        vertices: Vec<[f64; 3]>,
        radius: f64,
    },
    /// Internal static-world leaf; not accepted as an external body template.
    #[serde(skip)]
    Triangle {
        vertices: [[f64; 3]; 3],
        radius: f64,
    },
}
impl Geometry {
    pub fn radius(&self) -> f64 {
        match self {
            Self::Sphere { radius }
            | Self::Capsule { radius, .. }
            | Self::Box { radius, .. }
            | Self::Hull { radius, .. }
            | Self::Triangle { radius, .. } => *radius,
        }
    }
    fn validate(&self) -> Result<(), &'static str> {
        let valid = |p: [f64; 3]| p.into_iter().all(|v| v.is_finite() && v.abs() <= 10_000.);
        let r = self.radius();
        if !r.is_finite() || !(0.0..=100.).contains(&r) {
            return Err("invalid_dynamic_body_radius");
        }
        let ok = match self {
            Self::Sphere { .. } => r > 0.,
            Self::Capsule { a, b, .. } => valid(*a) && valid(*b) && r > 0.,
            Self::Box { half_extents, .. } => {
                valid(*half_extents) && half_extents.iter().all(|v| *v >= 0.)
            }
            Self::Hull { vertices, .. } => {
                (4..=16384).contains(&vertices.len()) && vertices.iter().all(|v| valid(*v))
            }
            Self::Triangle { vertices, .. } => vertices.iter().all(|v| valid(*v)),
        };
        if !ok {
            return Err("invalid_dynamic_body_geometry");
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Character,
    GroundVehicle,
    Mine,
}
impl Kind {
    fn target(self) -> TargetKind {
        match self {
            Self::Character => TargetKind::Character,
            Self::GroundVehicle => TargetKind::GroundVehicle,
            Self::Mine => TargetKind::Mine,
        }
    }
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Rotation {
    Entity,
    Identity,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RootDefinition {
    id: String,
    layer: u8,
    dead_layer: Option<u8>,
    translation: [f64; 3],
    rotation: Rotation,
    shape: Geometry,
}
struct Root {
    id: String,
    layer: u8,
    dead_layer: Option<u8>,
    translation: [f64; 3],
    rotation: Rotation,
    shape: Arc<Geometry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateDefinition {
    id: String,
    kind: Kind,
    roots: Vec<RootDefinition>,
}
struct Template {
    kind: Kind,
    roots: Vec<Root>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    native_binding_sha256: String,
    shape_data_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    schema_version: u32,
    source: Source,
    templates: Vec<TemplateDefinition>,
    runtime_bindings: Option<crate::dynamic_collision_runtime::RuntimeBindings>,
}
pub struct Catalog {
    fingerprint: String,
    runtime_bindings: Option<crate::dynamic_collision_runtime::RuntimeBindings>,
    templates: BTreeMap<String, Template>,
}
impl Catalog {
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let info = std::fs::metadata(path)?;
        if !info.is_file() || info.len() > 16_777_216 {
            return Err(std::io::Error::other("invalid_dynamic_body_file"));
        }
        let mut text = String::new();
        std::fs::File::open(path)?
            .take(16_777_217)
            .read_to_string(&mut text)?;
        Self::from_json(&text).map_err(std::io::Error::other)
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn runtime_bindings(&self) -> Option<&crate::dynamic_collision_runtime::RuntimeBindings> {
        self.runtime_bindings.as_ref()
    }
    /// Hashes bind external evidence, not independent certification of its claims.
    pub fn from_json(text: &str) -> Result<Self, &'static str> {
        if text.len() > 16_777_216 {
            return Err("dynamic_body_catalog_too_large");
        }
        let d: Document = serde_json::from_str(text).map_err(|_| "invalid_dynamic_body_catalog")?;
        if d.format != "mag_dynamic_bodies"
            || d.schema_version != 1
            || d.templates.is_empty()
            || d.templates.len() > 1024
        {
            return Err("invalid_dynamic_body_catalog");
        }
        for hash in [&d.source.native_binding_sha256, &d.source.shape_data_sha256] {
            if hash.len() != 64 || !hash.bytes().all(|v| v.is_ascii_hexdigit()) {
                return Err("invalid_dynamic_body_source");
            }
        }
        let mut templates = BTreeMap::new();
        let mut total = 0;
        for t in d.templates {
            if !identifier(&t.id) || t.roots.is_empty() || t.roots.len() > 64 {
                return Err("invalid_dynamic_body_template");
            }
            let mut names = BTreeSet::new();
            let mut roots = Vec::new();
            for r in t.roots {
                if !identifier(&r.id)
                    || !names.insert(r.id.clone())
                    || r.layer > 31
                    || r.dead_layer.is_some_and(|v| v > 31)
                    || !r
                        .translation
                        .iter()
                        .all(|v| v.is_finite() && v.abs() <= 10000.)
                {
                    return Err("invalid_dynamic_body_root");
                }
                r.shape.validate()?;
                total += if let Geometry::Hull { vertices, .. } = &r.shape {
                    vertices.len()
                } else {
                    1
                };
                if total > 262144 {
                    return Err("dynamic_body_catalog_geometry_limit");
                }
                roots.push(Root {
                    id: r.id,
                    layer: r.layer,
                    dead_layer: r.dead_layer,
                    translation: r.translation,
                    rotation: r.rotation,
                    shape: Arc::new(r.shape),
                });
            }
            if templates
                .insert(
                    t.id,
                    Template {
                        kind: t.kind,
                        roots,
                    },
                )
                .is_some()
            {
                return Err("duplicate_dynamic_body_template");
            }
        }
        Ok(Self {
            fingerprint: format!("{:x}", Sha256::digest(text.as_bytes())),
            runtime_bindings: d.runtime_bindings,
            templates,
        })
    }
    pub(crate) fn template_kind(&self, id: &str) -> Option<TargetKind> {
        self.templates.get(id).map(|v| v.kind.target())
    }
    /// Qualified allocated state0 ground objects have geometry but no world roots.
    /// Keep their target in the independent roster; do not confuse this with a
    /// missing template or omit allocated entities from completeness validation.
    pub fn instantiate_inactive_ground(
        &self,
        template: &str,
        target: Target,
        pose: Pose,
    ) -> Result<BodySet, &'static str> {
        if target.kind != TargetKind::GroundVehicle {
            return Err("invalid_unregistered_dynamic_body_kind");
        }
        let mut row = self.instantiate(template, target, pose, false)?;
        row.bodies.clear();
        Ok(row)
    }
    pub fn instantiate(
        &self,
        template: &str,
        target: Target,
        pose: Pose,
        dead: bool,
    ) -> Result<BodySet, &'static str> {
        pose.validate()?;
        validate_target(target)?;
        let t = self
            .templates
            .get(template)
            .ok_or("missing_dynamic_body_template")?;
        if t.kind.target() != target.kind {
            return Err("dynamic_body_template_kind_mismatch");
        }
        let mut bodies = Vec::new();
        for root in &t.roots {
            let center = geometry::add(pose.translation, geometry::rotate(pose, root.translation));
            let mut p = match root.rotation {
                Rotation::Entity => pose,
                Rotation::Identity => Pose::at(center),
            };
            p.translation = center;
            p.validate()?;
            bodies.push(Body {
                root: root.id.clone(),
                target,
                pose: p,
                shape: Arc::clone(&root.shape),
                layer: if dead {
                    root.dead_layer.unwrap_or(root.layer)
                } else {
                    root.layer
                },
                live_layer: root.layer,
                dead_layer: root.dead_layer,
            });
        }
        Ok(BodySet { target, bodies })
    }
}
fn identifier(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 128
        && v.bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"-_.".contains(&v))
}
fn validate_target(t: Target) -> Result<(), &'static str> {
    if t.key.id == 0 || t.key.generation == 0 || t.kind == TargetKind::Other {
        Err("invalid_dynamic_body_target")
    } else {
        Ok(())
    }
}
pub(crate) struct Body {
    pub(crate) root: String,
    pub(crate) target: Target,
    pub(crate) pose: Pose,
    pub(crate) shape: Arc<Geometry>,
    pub(crate) layer: u8,
    pub(crate) live_layer: u8,
    pub(crate) dead_layer: Option<u8>,
}
pub struct BodySet {
    target: Target,
    pub(crate) bodies: Vec<Body>,
}
pub struct Snapshot {
    origin: Option<Arc<()>>,
    world: WorldKey,
    revision: u64,
    entities: usize,
    bodies: Vec<Body>,
    row6: u32,
}
#[derive(Clone, Debug)]
pub struct Penetration {
    pub root: String,
    pub target: Target,
}
#[derive(Clone, Copy, Debug)]
pub struct QueryBox {
    pub pose: Pose,
    pub half_extents: [f64; 3],
    pub radius: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct QueryLimits {
    pub max_support_points: usize,
    pub max_iterations: usize,
    pub boundary_epsilon: f64,
}
impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_support_points: 1_000_000,
            max_iterations: 64,
            boundary_epsilon: 1e-8,
        }
    }
}
impl Snapshot {
    pub(crate) fn bodies(&self) -> &[Body] {
        &self.bodies
    }
    /// `roster` is the actual authority's complete set, separately enumerated
    /// from shape rows. Neither source catalogs nor caller-set coverage booleans
    /// establish that this roster represents all native game objects.
    pub fn capture(
        world: WorldKey,
        revision: u64,
        roster: &[Target],
        rows: Vec<BodySet>,
        row6: u32,
    ) -> Result<Self, &'static str> {
        if world.world_id == 0 || world.incarnation == 0 || roster.len() > 1024 {
            return Err("invalid_dynamic_body_context");
        }
        let mut expected = BTreeMap::new();
        for &t in roster {
            validate_target(t)?;
            if expected.insert(t.key.id, t).is_some() {
                return Err("duplicate_dynamic_body_roster");
            }
        }
        let mut bodies = Vec::new();
        for row in rows {
            let t = row.target;
            if expected.remove(&t.key.id) != Some(t) {
                return Err("dynamic_body_coverage_mismatch");
            }
            bodies.extend(row.bodies);
            if bodies.len() > 4096 {
                return Err("dynamic_body_root_limit");
            }
        }
        if !expected.is_empty() {
            return Err("dynamic_body_coverage_incomplete");
        }
        Ok(Self {
            origin: None,
            world,
            revision,
            entities: roster.len(),
            bodies,
            row6,
        })
    }
    pub(crate) fn bind_engine(mut self, origin: &Arc<()>) -> Self {
        self.origin = Some(Arc::clone(origin));
        self
    }
    pub(crate) fn is_from_engine(&self, origin: &Arc<()>) -> bool {
        self.origin.as_ref().is_some_and(|v| Arc::ptr_eq(v, origin))
    }
    pub fn context(&self) -> (WorldKey, u64) {
        (self.world, self.revision)
    }
    pub fn entity_count(&self) -> usize {
        self.entities
    }
    pub fn root_count(&self) -> usize {
        self.bodies.len()
    }
    /// Returns all supported registered roots in supplied snapshot traversal
    /// order. No partial result is released on geometry/budget/boundary errors.
    /// This order is explicit replacement authority, not native broadphase parity.
    pub fn penetrations(
        &self,
        query: QueryBox,
        limits: QueryLimits,
    ) -> Result<Vec<Penetration>, &'static str> {
        query.pose.validate()?;
        if limits.max_support_points == 0
            || limits.max_support_points > 100_000_000
            || !(1..=256).contains(&limits.max_iterations)
            || !limits.boundary_epsilon.is_finite()
            || limits.boundary_epsilon <= 0.
            || limits.boundary_epsilon > 0.01
        {
            return Err("invalid_dynamic_collision_limits");
        }
        let shape = Geometry::Box {
            half_extents: query.half_extents,
            radius: query.radius,
        };
        shape.validate()?;
        let mut budget = limits.max_support_points;
        let mut result = Vec::new();
        for body in &self.bodies {
            if !crate::acquisition_volume_geometry::filter6_allows(self.row6, u32::from(body.layer))
            {
                continue;
            }
            if geometry::penetrates(
                &shape,
                query.pose,
                &body.shape,
                body.pose,
                limits,
                &mut budget,
            )? {
                result.push(Penetration {
                    root: body.root.clone(),
                    target: body.target,
                });
            }
        }
        Ok(result)
    }
}
