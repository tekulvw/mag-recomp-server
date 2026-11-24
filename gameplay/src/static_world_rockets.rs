//! Infantry launcher flight and blast against the loaded static map world.
//!
//! Sabotage has no represented projectile-contact scene: no dynamic body
//! catalog, collision filter rows or material bindings. Its launcher flights
//! use what it does load. Each flight step traces the rocket's centre line
//! against the static collision triangles and against one upright body per
//! living, on-foot Character. The terminal blast doses every Character whose
//! stance anchor lies inside the authored outer radius with the authored
//! falloff, unless static geometry stands between the blast and that anchor.
//! Friendly fire, armour, statistics and rewards follow the ordinary rocket
//! Character callback.
use super::*;
use crate::{
    acquisition_character_effects::{Request, Target},
    collision::CollisionWorld,
    domination_structural_world::CHARACTER_BLAST_ANCHOR_HEIGHT_M,
    projectile_scene::OwnedObstacleId,
};

/// Work allowed for one flight step or one blast's visibility rays.
const QUERY_BUDGET: usize = 1_000_000;
/// The obstacle root reported for a Character body contact.
pub(super) const CHARACTER_ROOT: &str = "static_world_character";

/// One living on-foot Character as a vertical capsule from its feet.
#[derive(Clone, Copy, Debug)]
pub(super) struct Body {
    pub(super) target: Target,
    pub(super) feet: [f64; 3],
    /// Stance-dependent blast anchor height above the feet.
    pub(super) anchor_height: f64,
}

impl Body {
    fn key(self) -> TargetKey {
        TargetKey {
            id: u64::from(self.target.character_id),
            generation: self.target.generation,
        }
    }
    fn anchor(self) -> [f64; 3] {
        [
            self.feet[0],
            self.feet[1] + self.anchor_height,
            self.feet[2],
        ]
    }
    /// The capsule's core segment; the body radius surrounds it.
    fn core(self) -> [[f64; 3]; 2] {
        let radius = body_radius();
        let low = [self.feet[0], self.feet[1] + radius, self.feet[2]];
        let high = [
            self.feet[0],
            self.feet[1] + (self.anchor_height - radius).max(radius),
            self.feet[2],
        ];
        [low, high]
    }
}

/// The replacement's existing upright Character body radius.
fn body_radius() -> f64 {
    crate::bot_physics::Body::default().radius
}

impl<W: Write> Engine<W> {
    /// The static collision world when this match has no represented
    /// projectile scene: retail Sabotage with a loaded world for its map.
    pub(crate) fn static_rocket_world(&self) -> Option<&CollisionWorld> {
        if self.acquisition_gameplay.is_some()
            || self.domination_gameplay.is_some()
            || self.wire_profile.application_id() != 20231
            || self.match_profile().game_type != 0
        {
            return None;
        }
        self.collision_world.as_ref().filter(|world| {
            world.stats().triangles != 0 && world.metadata().map_name == self.map_name()
        })
    }

    /// Install launcher rows for the static-world scene, normally derived
    /// with [`infantry_rockets::Catalog::from_item_catalog`].
    pub fn with_static_world_infantry_rockets(
        mut self,
        catalog: Catalog,
    ) -> Result<Self, &'static str> {
        if self.static_rocket_world().is_none() {
            return Err("static_world_infantry_rockets_require_sabotage_collision");
        }
        // Character rocket doses resolve friendly fire through these rules.
        if self.character_damage_rules.is_none() {
            return Err("static_world_infantry_rockets_require_character_damage_rules");
        }
        self.infantry_rockets = Some(Arc::new(catalog));
        Ok(self)
    }

    /// The detailed refusal for a gear-3 item without a launcher row.
    pub(super) fn unqualified_launcher(&self, actor: &Active) -> &'static str {
        let item = actor.equipped_loadout().and_then(|kit| {
            kit.inventories
                .get(usize::from(kit.handle.inventory_index))
                .and_then(|inventory| inventory.slots[usize::from(infantry_rockets::GEAR_SLOT)])
        });
        match item {
            None => "infantry_rocket_gear_empty",
            Some(id) if infantry_rockets::is_guided_launcher(&self.item_catalog, id) => {
                "infantry_rocket_guided_launcher_unqualified"
            }
            Some(_) => "infantry_rocket_weapon_unqualified",
        }
    }

    /// Living on-foot Characters in `world` that a static-world rocket can hit.
    pub(super) fn static_rocket_bodies(&self, world: u32) -> Vec<Body> {
        let anchor = |stance: u8| {
            CHARACTER_BLAST_ANCHOR_HEIGHT_M
                .get(usize::from(stance))
                .copied()
                .map(f64::from)
        };
        let mut bodies = Vec::new();
        for actor in self.active.iter().chain(self.peers.values()) {
            if actor.reservation.world != world
                || actor.is_mounted()
                || !actor.entered_game
                || !actor.ready_for_updates
                || actor.initial_objects.is_none()
            {
                continue;
            }
            let (Some((generation, _)), Some(sample)) =
                (actor.life.damage_health(), actor.tracking.sample)
            else {
                continue;
            };
            let stance = actor.owner_fields.state.map_or(0, |state| state.stance);
            let Some(anchor_height) = anchor(stance) else {
                continue;
            };
            let p = sample.position;
            bodies.push(Body {
                target: Target {
                    character_id: 0x0380_0000 | u32::from(actor.reservation.player),
                    generation,
                    world,
                },
                feet: [p.x, p.y, p.z].map(f64::from),
                anchor_height,
            });
        }
        if self.accepts_subscription_world(world) {
            for (&id, bot) in &self.bot_registry.bots {
                let Some(runtime) = bot.target.runtime.filter(|runtime| {
                    bot.health > 0 && !matches!(runtime.phase, development_target::BotPhase::Dead)
                }) else {
                    continue;
                };
                bodies.push(Body {
                    target: Target {
                        character_id: 0x0380_0000 | u32::from(id),
                        generation: runtime.generation,
                        world,
                    },
                    feet: runtime.physical_position(),
                    anchor_height: f64::from(CHARACTER_BLAST_ANCHOR_HEIGHT_M[0]),
                });
            }
        }
        bodies
    }

    pub(super) fn plan_static_rocket_steps(
        &self,
        world: &CollisionWorld,
        flights: &[Flight],
        now_ms: u64,
    ) -> Vec<Result<Step, &'static str>> {
        flights
            .iter()
            .map(|flight| {
                let attacker = self.infantry_attacker(flight)?;
                let bodies: Vec<_> = self
                    .static_rocket_bodies(flight.source.world)
                    .into_iter()
                    .filter(|body| body.target.character_id != attacker)
                    .collect();
                let mut budget = QUERY_BUDGET;
                flight.step(now_ms, None, None, |sweep| {
                    static_contact(world, &bodies, sweep, &mut budget)
                })
            })
            .collect()
    }

    pub(super) fn prepare_static_rocket_effects(
        &mut self,
        flight: &Flight,
        position: [f64; 3],
        now: Duration,
    ) -> Result<Effects, &'static str> {
        let world = self
            .static_rocket_world()
            .ok_or("infantry_rocket_static_world_required")?;
        let source = &flight.source;
        let attacker = self.hospital_rocket_blast_attacker(source)?;
        let blast = flight.variant.blast();
        let effects = static_blast(
            world,
            &self.static_rocket_bodies(source.world),
            position,
            blast,
            attacker,
            source.projectile_id,
        )?;
        let native = self.owned_acquisition_characters();
        let character = self.prepare_rocket_character_effects(source, &effects, &native, now)?;
        Ok(Effects {
            character,
            native,
            ground: None,
            helo: None,
            hospital: None,
        })
    }
}

/// First contact of a rocket's centre line, widened by its radius against
/// Character bodies, with the static triangles or a Character.
pub(super) fn static_contact(
    world: &CollisionWorld,
    bodies: &[Body],
    sweep: CapsuleSweep,
    budget: &mut usize,
) -> Result<Option<OwnedContact>, &'static str> {
    let delta = sub(sweep.end, sweep.start);
    let length = norm(delta);
    if !length.is_finite() || length == 0. {
        return Ok(None);
    }
    let query = world
        .raycast_segment_bounded(sweep.start, sweep.end, budget)
        .map_err(|_| {
            if *budget == 0 {
                "infantry_rocket_static_work_limit"
            } else {
                "infantry_rocket_static_query_invalid"
            }
        })?;
    // Stop the rocket's centre one radius short of the surface it struck.
    let mut best = query.nearest.map(|hit| {
        (
            (hit.fraction - sweep.radius / length).max(0.),
            OwnedObstacleId::Static {
                instance: hit.instance_id,
                triangle: hit.triangle_index,
            },
            Some(hit.normal),
        )
    });
    let reach = sweep.radius + body_radius();
    for body in bodies {
        let Some(fraction) = first_contact(sweep.start, delta, body.core(), reach) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|(nearest, _, _)| fraction < *nearest)
        {
            best = Some((
                fraction,
                OwnedObstacleId::Dynamic {
                    key: body.key(),
                    root: CHARACTER_ROOT.into(),
                },
                None,
            ));
        }
    }
    Ok(best.map(|(fraction, obstacle, normal)| OwnedContact {
        obstacle,
        fraction,
        center: std::array::from_fn(|axis| delta[axis].mul_add(fraction, sweep.start[axis])),
        normal,
    }))
}

/// Character doses from one blast at `origin`: authored falloff from the
/// inner to the outer radius, refused behind static geometry.
pub(super) fn static_blast(
    world: &CollisionWorld,
    bodies: &[Body],
    origin: [f64; 3],
    blast: hospital_rockets::Blast,
    attacker: u32,
    projectile_id: u32,
) -> Result<Vec<(Target, Request)>, &'static str> {
    if !blast.character_hp.is_finite()
        || blast.character_hp <= 0.
        || !(blast.half_radius_m > 0. && blast.outer_radius_m > blast.half_radius_m)
    {
        return Err("invalid_infantry_rocket_character_blast");
    }
    let mut budget = QUERY_BUDGET;
    let mut effects = Vec::new();
    for body in bodies {
        let anchor = body.anchor();
        let displacement: [f32; 3] =
            std::array::from_fn(|axis| (anchor[axis] - origin[axis]) as f32);
        let distance = displacement
            .iter()
            .map(|axis| axis * axis)
            .sum::<f32>()
            .sqrt();
        let scale =
            armor::native_radial_scale(distance, blast.half_radius_m, blast.outer_radius_m)?;
        if scale <= 0. {
            continue;
        }
        if distance > 0.
            && world
                .raycast_segment_bounded(origin, anchor, &mut budget)
                .map_err(|_| "infantry_rocket_static_blast_query_invalid")?
                .nearest
                .is_some()
        {
            continue;
        }
        let from_direction = if distance == 0. {
            [0.; 3]
        } else {
            displacement.map(|axis| axis / distance)
        };
        effects.push((
            body.target,
            Request::HospitalRocketExplosion {
                raw_hp: blast.character_hp * scale,
                from_direction,
                attacker,
                projectile_id,
            },
        ));
    }
    if effects.len() > crate::blast_visibility::MAX_BLAST_TARGETS {
        return Err("infantry_rocket_blast_target_limit");
    }
    Ok(effects)
}

/// Smallest `t` in [0,1] at which `start + t * delta` comes within `reach` of
/// the segment `core`. The distance is convex in `t`, so the minimum is found
/// first and the earliest crossing is then bisected before it.
fn first_contact(start: [f64; 3], delta: [f64; 3], core: [[f64; 3]; 2], reach: f64) -> Option<f64> {
    let distance = |t: f64| {
        let point: [f64; 3] = std::array::from_fn(|axis| delta[axis].mul_add(t, start[axis]));
        point_segment_distance(point, core)
    };
    if distance(0.) <= reach {
        return Some(0.);
    }
    let (mut low, mut high) = (0., 1.);
    for _ in 0..80 {
        let left = low + (high - low) / 3.;
        let right = high - (high - low) / 3.;
        if distance(left) <= distance(right) {
            high = right;
        } else {
            low = left;
        }
    }
    let nearest = (low + high) / 2.;
    if distance(nearest) > reach {
        return None;
    }
    let (mut outside, mut inside) = (0., nearest);
    for _ in 0..60 {
        let middle = (outside + inside) / 2.;
        if distance(middle) <= reach {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    Some(inside)
}

fn point_segment_distance(point: [f64; 3], [a, b]: [[f64; 3]; 2]) -> f64 {
    let axis = sub(b, a);
    let length_squared = dot(axis, axis);
    let t = if length_squared == 0. {
        0.
    } else {
        (dot(sub(point, a), axis) / length_squared).clamp(0., 1.)
    };
    let closest: [f64; 3] = std::array::from_fn(|i| axis[i].mul_add(t, a[i]));
    norm(sub(point, closest))
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

#[cfg(test)]
#[path = "static_world_rockets_tests.rs"]
mod tests;
