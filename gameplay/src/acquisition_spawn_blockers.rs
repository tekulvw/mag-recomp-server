//! Native-bound character spawn-blocker movement control.
//! World contact/cast semantics remain an explicit required provider.
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub signed_distance: f32,
    pub normal: [f32; 3],
}
#[derive(Clone, Copy, Debug)]
pub struct CastHit {
    pub fraction: f32,
    pub normal: [f32; 3],
}
/// Implement on an ISOLATED owned callback candidate. Moves must not publish or
/// mutate live Engine authority. The caller must retain intermediate moves on an
/// unresolved result before applying ordinary lethal damage; preparation errors
/// discard the entire candidate, not merely this routine's last movement.
pub trait BlockerQueries {
    fn position(&self) -> [f32; 3];
    /// Character-first closest contact with the spawning collidable; retain the
    /// smallest signed distance and earlier contact on ties. Normal is xyz.
    fn closest_spawn_contact(&mut self) -> Result<Option<Contact>, &'static str>;
    /// Cast the current character against the QUALIFIED WORLD, including static
    /// geometry/current filters/tolerances. Missing/incomplete coverage must return
    /// Err, never a fabricated no-hit. Exclude the exact spawning collidable
    /// as bodyB; an entity-only overlap query cannot implement this contract.
    fn cast_world(&mut self, target: [f32; 3]) -> Result<Option<CastHit>, &'static str>;
    fn move_character(&mut self, target: [f32; 3]) -> Result<(), &'static str>;
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] * b[0] + a[1] * b[1]) + a[2] * b[2]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn target(a: [f32; 3], n: [f32; 3], distance: f32) -> Result<[f32; 3], &'static str> {
    let p = std::array::from_fn(|i| a[i] + n[i] * distance);
    valid_vector(p)?;
    Ok(p)
}
fn valid_vector(p: [f32; 3]) -> Result<(), &'static str> {
    if p.iter().all(|v| v.is_finite() && v.abs() <= 100_000.) {
        Ok(())
    } else {
        Err("unsupported_spawn_blocker_vector")
    }
}
fn tangent(n: [f32; 3], hit_normal: [f32; 3]) -> Result<[f32; 3], &'static str> {
    valid_vector(hit_normal)?;
    let projection = dot(n, hit_normal);
    let mut t = std::array::from_fn(|i| n[i] - hit_normal[i] * projection);
    let squared = dot(t, t);
    if squared > 0.00001 {
        let length = squared.sqrt();
        for v in &mut t {
            *v /= length;
        }
    }
    valid_vector(t)?;
    Ok(t)
}
fn valid_hit(hit: CastHit) -> Result<CastHit, &'static str> {
    valid_vector(hit.normal)?;
    if !hit.fraction.is_finite() || !(0.0..=1.0).contains(&hit.fraction) {
        return Err("unsupported_spawn_blocker_cast_fraction");
    }
    Ok(hit)
}
fn contact<W: BlockerQueries>(world: &mut W) -> Result<Option<Contact>, &'static str> {
    let c = world.closest_spawn_contact()?;
    if let Some(c) = c {
        if !c.signed_distance.is_finite() {
            return Err("unsupported_spawn_blocker_contact");
        }
        valid_vector(c.normal)?;
    }
    Ok(c)
}
/// At most two outer pushes and two world casts per push. A false result is an
/// unresolved blocker, not rollback: the native caller then damages the actor.
/// Arithmetic is the finite admitted replacement path; native VMX/dispatcher and
/// cast tolerance equivalence must be separately qualified by the world provider.
pub fn resolve_character<W: BlockerQueries>(world: &mut W) -> Result<bool, &'static str> {
    for _ in 0..2 {
        let Some(c) = contact(world)? else {
            return Ok(true);
        };
        if c.signed_distance >= 0. {
            return Ok(true);
        }
        let depth = -c.signed_distance;
        if depth >= 0.125 {
            return Ok(false);
        }
        let origin = world.position();
        valid_vector(origin)?;
        let normal = c.normal;
        let full = target(origin, normal, depth + 0.001)?;
        let Some(hit) = world.cast_world(full)? else {
            world.move_character(full)?;
            continue;
        };
        let hit = valid_hit(hit)?;
        // Native omits the epsilon when multiplying by the cast fraction.
        let partial = target(origin, normal, depth * hit.fraction)?;
        if dot(sub(partial, origin), sub(partial, origin)) > 0.25 {
            return Ok(false);
        }
        let slide = tangent(normal, hit.normal)?;
        let alignment = dot(slide, normal);
        if alignment < 0.0001 {
            return Ok(false);
        }
        world.move_character(partial)?;
        let remaining = (depth - dot(sub(partial, origin), normal)) / alignment;
        if !remaining.is_finite() {
            return Err("unsupported_spawn_blocker_arithmetic");
        }
        let second_target = target(partial, slide, remaining + 0.001)?;
        let Some(second) = world.cast_world(second_target)? else {
            world.move_character(second_target)?;
            continue;
        };
        let second = valid_hit(second)?;
        let last = target(partial, slide, remaining * second.fraction)?;
        if dot(sub(last, origin), sub(last, origin)) > 0.25 {
            return Ok(false);
        }
        if dot(tangent(slide, second.normal)?, normal) < 0.0001 {
            return Ok(false);
        }
        world.move_character(last)?;
        return Ok(false); // No third world cast.
    }
    Ok(contact(world)?.is_none_or(|v| v.signed_distance >= 0.))
}
