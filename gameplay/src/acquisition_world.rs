//! Concrete capsule contact/cast provider over owned dynamic roots and a static
//! triangle BVH. All physics inputs are explicit; no missing-data no-hit path.
use crate::{
    acquisition_spawn_blockers::{BlockerQueries, CastHit, Contact},
    acquisition_volume_damage::{Target, TargetKind},
    collision::CollisionWorld,
    dynamic_collision::{self, Body, Catalog, Geometry, Pose, QueryLimits, Snapshot},
};
use dynamic_collision::geometry::{self, Point};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RootKey {
    pub entity: u64,
    pub generation: u32,
    pub root: String,
}
impl RootKey {
    fn of(body: &Body) -> Self {
        Self {
            entity: body.target.key.id,
            generation: body.target.key.generation,
            root: body.root.clone(),
        }
    }
}
#[derive(Clone)]
pub struct Filters {
    pub rows: [u32; 32],
    pub roots: BTreeMap<RootKey, u32>,
}
impl Filters {
    /// Owned replacement grouping: one distinct system group per currently
    /// owned character/vehicle, shared by its roots. Group numbers are internal
    /// identity tokens, not native allocation-number parity. Static groups are
    /// reserved. Mine ownership needs a separate qualified owner-group mapping.
    pub fn owned_entities(
        snapshot: &Snapshot,
        statics: &CollisionWorld,
        rows: [u32; 32],
    ) -> Result<Self, &'static str> {
        statics.static_query_radius()?;
        let reserved = statics.static_groups();
        let mut groups = BTreeMap::new();
        let mut roots = BTreeMap::new();
        let mut next = 1u32;
        for body in snapshot.bodies() {
            if !matches!(
                body.target.kind,
                TargetKind::Character | TargetKind::GroundVehicle
            ) {
                return Err("world_query_owner_group_unavailable");
            }
            let entity = (body.target.key.id, body.target.key.generation);
            let group = if let Some(group) = groups.get(&entity) {
                *group
            } else {
                while reserved.contains(&next) {
                    next += 1;
                }
                if next > u32::from(u16::MAX) {
                    return Err("world_query_system_groups_exhausted");
                }
                let group = next;
                next += 1;
                groups.insert(entity, group);
                group
            };
            roots.insert(RootKey::of(body), u32::from(body.layer) | (group << 16));
        }
        Ok(Self { rows, roots })
    }
    pub fn allows(rows: &[u32; 32], a: u32, b: u32) -> bool {
        let group = a >> 16;
        if group != 0 && group == b >> 16 {
            ((a >> 5) & 31) != ((b >> 10) & 31) && ((b >> 5) & 31) != ((a >> 10) & 31)
        } else {
            rows[(a & 31) as usize] & (1 << (b & 31)) != 0
        }
    }
}
#[derive(Clone, Copy)]
pub struct Settings {
    pub contact_tolerance: f64,
    pub max_extra_penetration: f64,
    pub query_limits: QueryLimits,
}
impl Default for Settings {
    /// Explicit replacement tuning; not an inferred native runtime default.
    fn default() -> Self {
        Self {
            contact_tolerance: 0.0001,
            max_extra_penetration: 0.0001,
            query_limits: QueryLimits::default(),
        }
    }
}
pub struct World<'a> {
    snapshot: &'a Snapshot,
    statics: &'a CollisionWorld,
    filters: Filters,
    settings: Settings,
    static_radius: f64,
    budget: usize,
    moved: BTreeMap<(u64, u32), Point>,
    allocated: BTreeMap<RootKey, Body>,
}
#[derive(Clone, Debug)]
pub struct SpawnResolution {
    pub moved: Vec<(u64, u32, Point)>,
    pub lethal_characters: Vec<Target>,
    pub lethal_ground: Vec<Target>,
}
impl<'a> World<'a> {
    pub fn new(
        snapshot: &'a Snapshot,
        statics: &'a CollisionWorld,
        filters: Filters,
        settings: Settings,
    ) -> Result<Self, &'static str> {
        let limits = settings.query_limits;
        if !settings.contact_tolerance.is_finite()
            || !(0.000001..=0.01).contains(&settings.contact_tolerance)
            || !settings.max_extra_penetration.is_finite()
            || !(0.0..=0.01).contains(&settings.max_extra_penetration)
            || !(1..=256).contains(&limits.max_iterations)
            || limits.max_support_points == 0
            || limits.max_support_points > 100_000_000
            || !limits.boundary_epsilon.is_finite()
            || limits.boundary_epsilon <= 0.
            || limits.boundary_epsilon > settings.contact_tolerance / 10.
        {
            return Err("invalid_world_query_settings");
        }
        if filters.roots.len() != snapshot.root_count()
            || snapshot.bodies().iter().any(|b| {
                filters
                    .roots
                    .get(&RootKey::of(b))
                    .is_none_or(|f| f & 31 != u32::from(b.layer))
            })
        {
            return Err("world_query_dynamic_filter_coverage_mismatch");
        }
        let static_radius = statics.static_query_radius()?;
        Ok(Self {
            snapshot,
            statics,
            filters,
            settings,
            static_radius,
            budget: limits.max_support_points,
            moved: BTreeMap::new(),
            allocated: BTreeMap::new(),
        })
    }
    /// Allocated shape used by respawn before ordinary Update registers its
    /// roots. It participates in direct pair contact, never in world casts.
    pub fn add_allocated_spawn(
        &mut self,
        catalog: &Catalog,
        template: &str,
        target: Target,
        pose: Pose,
        root: &str,
    ) -> Result<RootKey, &'static str> {
        if target.kind != TargetKind::GroundVehicle {
            return Err("invalid_allocated_spawn_kind");
        }
        let rows = catalog.instantiate(template, target, pose, false)?;
        let body = rows
            .bodies
            .into_iter()
            .find(|b| b.root == root)
            .ok_or("allocated_spawn_root_missing")?;
        let key = RootKey::of(&body);
        if self.snapshot.bodies().iter().any(|b| RootKey::of(b) == key)
            || self.allocated.contains_key(&key)
        {
            return Err("duplicate_allocated_spawn_root");
        }
        self.allocated.insert(key.clone(), body);
        Ok(key)
    }
    fn body(&self, key: &RootKey) -> Result<&Body, &'static str> {
        self.snapshot
            .bodies()
            .iter()
            .find(|b| RootKey::of(b) == *key)
            .or_else(|| self.allocated.get(key))
            .ok_or("world_query_root_unavailable")
    }
    pub fn has_root(&self, key: &RootKey) -> bool {
        self.body(key).is_ok()
    }
    /// Actual pair query and bounded character push control, in retained root
    /// order. Effects remain owned preparation data until an Engine transaction
    /// applies movement/damage and records all consequences.
    pub fn resolve_spawn(
        &mut self,
        spawn: &RootKey,
        operational_ground: &std::collections::BTreeSet<(u64, u32)>,
    ) -> Result<SpawnResolution, &'static str> {
        let source_id = self.body(spawn)?.target.key.id;
        let pairs = self.spawn_pairs(spawn)?;
        let mut result = SpawnResolution {
            moved: vec![],
            lethal_characters: vec![],
            lethal_ground: vec![],
        };
        for (key, target) in pairs {
            if target.key.id == source_id {
                continue;
            }
            match target.kind {
                TargetKind::Character => {
                    if !crate::acquisition_spawn_blockers::resolve_character(
                        &mut self.character(key, spawn.clone())?,
                    )? {
                        result.lethal_characters.push(target);
                    }
                }
                TargetKind::GroundVehicle => {
                    if operational_ground.contains(&(target.key.id, target.key.generation))
                        && self.contact(&key, spawn)?.signed_distance < -0.1_f32
                    {
                        result.lethal_ground.push(target);
                    }
                }
                _ => return Err("world_query_spawn_target_unsupported"),
            }
        }
        result.moved = self.moved_positions().collect();
        Ok(result)
    }
    /// Collect once, then consume each pair against staged current authority.
    /// Damage can change filters before a later character's world cast.
    pub(crate) fn spawn_pairs(
        &mut self,
        spawn: &RootKey,
    ) -> Result<Vec<(RootKey, Target)>, &'static str> {
        let source = self.body(spawn)?;
        let shape = source.shape.clone();
        let pose = self.pose(source);
        let mut pairs = Vec::new();
        for body in self.snapshot.bodies() {
            if self.budget == 0 {
                return Err("world_cast_work_limit");
            }
            self.budget -= 1;
            if !crate::acquisition_volume_geometry::filter6_allows(
                self.filters.rows[6],
                u32::from(body.layer),
            ) {
                continue;
            }
            if geometry::penetrates(
                &shape,
                pose,
                &body.shape,
                self.pose(body),
                self.settings.query_limits,
                &mut self.budget,
            )? {
                pairs.push((RootKey::of(body), body.target));
            }
        }
        Ok(pairs)
    }
    pub(crate) fn contact_between(
        &mut self,
        a: &RootKey,
        b: &RootKey,
    ) -> Result<Contact, &'static str> {
        self.contact(a, b)
    }
    pub(crate) fn character_dead(
        &mut self,
        target: Target,
        dead: bool,
    ) -> Result<(), &'static str> {
        let mut found = false;
        for body in self.snapshot.bodies().iter().filter(|b| b.target == target) {
            if body.target.kind != TargetKind::Character {
                return Err("world_query_character_required");
            }
            let f = self
                .filters
                .roots
                .get_mut(&RootKey::of(body))
                .ok_or("world_query_character_filter_unavailable")?;
            *f = (*f & !31)
                | u32::from(if dead {
                    body.dead_layer.unwrap_or(body.live_layer)
                } else {
                    body.live_layer
                });
            found = true;
        }
        if !found {
            return Err("world_query_root_unavailable");
        }
        Ok(())
    }
    fn pose(&self, body: &Body) -> Pose {
        let mut p = body.pose;
        if let Some(delta) = self
            .moved
            .get(&(body.target.key.id, body.target.key.generation))
        {
            p.translation = geometry::add(p.translation, *delta);
        }
        p
    }
    pub fn character<'w>(
        &'w mut self,
        character: RootKey,
        spawn: RootKey,
    ) -> Result<Character<'w, 'a>, &'static str> {
        let body = self.body(&character)?;
        if body.target.kind != TargetKind::Character
            || !matches!(*body.shape, Geometry::Capsule { .. })
            || character == spawn
        {
            return Err("world_query_character_capsule_required");
        }
        self.body(&spawn)?;
        Ok(Character {
            world: self,
            character,
            spawn,
        })
    }
    pub fn moved_positions(&self) -> impl Iterator<Item = (u64, u32, Point)> + '_ {
        self.moved.iter().map(|(&(id, generation), &delta)| {
            let body = self
                .snapshot
                .bodies()
                .iter()
                .find(|b| b.target.key.id == id && b.target.key.generation == generation)
                .expect("moved owned character root");
            (id, generation, geometry::add(body.pose.translation, delta))
        })
    }
    fn contact(&mut self, a: &RootKey, b: &RootKey) -> Result<Contact, &'static str> {
        let first = self.body(a)?;
        let second = self.body(b)?;
        let ap = self.pose(first);
        let bp = self.pose(second);
        let sa = first.shape.clone();
        let sb = second.shape.clone();
        let d = geometry::distance(
            &sa,
            ap,
            &sb,
            bp,
            self.settings.query_limits,
            &mut self.budget,
        )?;
        let radius = sa.radius() + sb.radius();
        let (signed, normal) = if let Some(normal) = d.normal {
            (d.upper - radius, normal)
        } else {
            let (depth, normal) = crate::convex_penetration::penetration(
                &sa,
                ap,
                &sb,
                bp,
                self.settings.query_limits,
                &mut self.budget,
            )?;
            (-depth - radius, normal)
        };
        Ok(Contact {
            signed_distance: signed as f32,
            normal: normal.map(|v| v as f32),
        })
    }
    fn cast(
        &mut self,
        character: &RootKey,
        spawn: &RootKey,
        target: Point,
    ) -> Result<Option<CastHit>, &'static str> {
        let first = self.body(character)?;
        let pose = self.pose(first);
        let shape = first.shape.clone();
        let own_filter = *self
            .filters
            .roots
            .get(character)
            .ok_or("world_query_character_filter_unavailable")?;
        let Geometry::Capsule { a, b, radius } = *shape else {
            return Err("world_query_character_capsule_required");
        };
        Pose::at(target).validate()?;
        let delta = geometry::sub(target, pose.translation);
        let endpoints = [a, b].map(|p| geometry::add(pose.translation, geometry::rotate(pose, p)));
        let margin = radius + self.static_radius + self.settings.contact_tolerance;
        let min = std::array::from_fn(|i| {
            endpoints[0][i].min(endpoints[1][i]) + delta[i].min(0.) - margin
        });
        let max = std::array::from_fn(|i| {
            endpoints[0][i].max(endpoints[1][i]) + delta[i].max(0.) + margin
        });
        let mut nearest: Option<(f64, Point)> = None;
        let settings = self.settings;
        for row in self.statics.static_triangles(min, max, &mut self.budget)? {
            if !Filters::allows(&self.filters.rows, own_filter, row.properties.filter_info) {
                continue;
            }
            let obstacle = Geometry::Triangle {
                vertices: row.vertices,
                radius: row.properties.convex_radius,
            };
            if let Some(hit) = geometry::sweep(
                &shape,
                pose,
                target,
                &obstacle,
                Pose::at([0.; 3]),
                geometry::SweepLimits {
                    tolerance: settings.contact_tolerance,
                    extra: settings.max_extra_penetration,
                    query: settings.query_limits,
                },
                &mut self.budget,
            )? && nearest.is_none_or(|v| hit.0 < v.0)
            {
                nearest = Some(hit);
            }
        }
        for body in self.snapshot.bodies() {
            if self.budget == 0 {
                return Err("world_cast_work_limit");
            }
            self.budget -= 1;
            let key = RootKey::of(body);
            if key == *character || key == *spawn {
                continue;
            }
            if !Filters::allows(&self.filters.rows, own_filter, self.filters.roots[&key]) {
                continue;
            }
            if let Some(hit) = geometry::sweep(
                &shape,
                pose,
                target,
                &body.shape,
                self.pose(body),
                geometry::SweepLimits {
                    tolerance: settings.contact_tolerance,
                    extra: settings.max_extra_penetration,
                    query: settings.query_limits,
                },
                &mut self.budget,
            )? && nearest.is_none_or(|v| hit.0 < v.0)
            {
                nearest = Some(hit);
            }
        }
        Ok(nearest.map(|(fraction, normal)| CastHit {
            fraction: fraction as f32,
            normal: normal.map(|v| v as f32),
        }))
    }
}
pub struct Character<'w, 'a> {
    world: &'w mut World<'a>,
    character: RootKey,
    spawn: RootKey,
}
impl BlockerQueries for Character<'_, '_> {
    fn position(&self) -> [f32; 3] {
        self.world
            .pose(
                self.world
                    .body(&self.character)
                    .expect("owned character root"),
            )
            .translation
            .map(|v| v as f32)
    }
    fn closest_spawn_contact(&mut self) -> Result<Option<Contact>, &'static str> {
        self.world.contact(&self.character, &self.spawn).map(Some)
    }
    fn cast_world(&mut self, target: [f32; 3]) -> Result<Option<CastHit>, &'static str> {
        self.world
            .cast(&self.character, &self.spawn, target.map(f64::from))
    }
    fn move_character(&mut self, target: [f32; 3]) -> Result<(), &'static str> {
        let p = target.map(f64::from);
        Pose::at(p).validate()?;
        let initial = self.world.body(&self.character)?.pose.translation;
        self.world.moved.insert(
            (self.character.entity, self.character.generation),
            geometry::sub(p, initial),
        );
        Ok(())
    }
}
