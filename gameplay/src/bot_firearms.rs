//! World-owned firearm admission. AI policy is explicit; weapon values come
//! exclusively from the extracted retail combat catalog. No client hit spoofing.
use super::*;
#[path = "bot_recoil.rs"]
mod recoil;
use crate::actor_collision::Bounds;
use crate::bot_registry::WorldBot;
use crate::development_target::BotPhase;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
    pub reaction_ms: u64,
    pub turn_degrees_per_second: f64,
    pub aim_tolerance_degrees: f64,
    pub accuracy_per_mille: u16,
    pub far_accuracy_per_mille: u16,
    pub recoil_compensation_per_mille: u16,
    pub max_engagement_distance_m: f64,
    pub acquisition_fov_degrees: f64,
    pub perception_interval_ms: u64,
    pub max_candidates_per_bot: usize,
    pub max_rays_per_tick: usize,
    pub geometry_work_per_tick: usize,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            reaction_ms: 800,
            turn_degrees_per_second: 90.,
            aim_tolerance_degrees: 3.,
            accuracy_per_mille: 300,
            far_accuracy_per_mille: 100,
            recoil_compensation_per_mille: 250,
            max_engagement_distance_m: 60.,
            acquisition_fov_degrees: 120.,
            perception_interval_ms: 200,
            max_candidates_per_bot: 8,
            max_rays_per_tick: 64,
            geometry_work_per_tick: 65_536,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if !(1..=10_000).contains(&self.reaction_ms)
            || !self.turn_degrees_per_second.is_finite()
            || !(1.0..=720.).contains(&self.turn_degrees_per_second)
            || !self.aim_tolerance_degrees.is_finite()
            || !(0.1..=15.).contains(&self.aim_tolerance_degrees)
            || self.accuracy_per_mille > 1000
            || self.far_accuracy_per_mille > 1000
            || self.recoil_compensation_per_mille > 1000
            || !self.max_engagement_distance_m.is_finite()
            || !(1.0..=2048.).contains(&self.max_engagement_distance_m)
            || !self.acquisition_fov_degrees.is_finite()
            || !(1.0..=360.).contains(&self.acquisition_fov_degrees)
            || !(50..=5000).contains(&self.perception_interval_ms)
            || !(1..=32).contains(&self.max_candidates_per_bot)
            || !(1..=256).contains(&self.max_rays_per_tick)
            || !(1024..=1_000_000).contains(&self.geometry_work_per_tick)
        {
            return Err("invalid_bot_firearm_policy");
        }
        Ok(())
    }
    fn within_range(&self, distance: f64, weapon_range: f64) -> bool {
        distance.is_finite()
            && weapon_range.is_finite()
            && weapon_range > 0.
            && distance >= 0.
            && distance < self.max_engagement_distance_m.min(weapon_range)
    }
    fn can_acquire(&self, from: [f64; 3], to: [f64; 3], yaw: f64, weapon_range: f64) -> bool {
        if !self.within_range(length(from, to), weapon_range) {
            return false;
        }
        let wanted = (to[0] - from[0]).atan2(to[2] - from[2]);
        let (_, angle) = turn(yaw, wanted, 0.);
        angle <= self.acquisition_fov_degrees.to_radians() * 0.5
    }
    fn accuracy_at(&self, distance: f64, weapon_range: f64) -> u16 {
        if !self.within_range(distance, weapon_range) {
            return 0;
        }
        let fraction = distance / self.max_engagement_distance_m.min(weapon_range);
        let near = f64::from(self.accuracy_per_mille);
        let far = f64::from(self.far_accuracy_per_mille.min(self.accuracy_per_mille));
        (near + (far - near) * fraction).round() as u16
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Life {
    id: u16,
    generation: u32,
    reservation: Option<String>,
}
pub(super) type EquipmentKey = (u32, u8, u8, u8, u16);

pub(super) fn current_equipment_key(bot: &WorldBot) -> Option<EquipmentKey> {
    let kit = &bot.target.loadout;
    let item = kit
        .inventories
        .get(usize::from(kit.handle.inventory_index))?
        .slots
        .get(usize::from(kit.primary.gear_slot))
        .copied()
        .flatten()?;
    Some((
        bot.target.runtime?.generation,
        kit.handle.update_id,
        kit.handle.inventory_index,
        kit.primary.gear_slot,
        item,
    ))
}

pub(super) fn resupply_planning_ammo(
    items: &crate::item_catalog::ItemCatalog,
    bot: &WorldBot,
) -> Option<bot_resupply::AmmoBinding> {
    let catalog = items.retail_combat().ok()?;
    let kit = &bot.target.loadout;
    let item = kit.primary;
    let profile = retail_combat::RetailBulletProfile::from_equipped_with_items(
        items,
        kit,
        item,
        item.gear_slot,
        item.ammo_pool,
        true,
    )
    .ok()?;
    let metadata = catalog.bullet_metadata(profile.item_id())?;
    if metadata.firearm?.infinite_ammo {
        return None;
    }
    let equipment = current_equipment_key(bot)?;
    let reserve_cap = State::reserve_capacity(metadata);
    (bot.firearm.is_resupply_dry() && bot.firearm.equipment_matches(equipment) && reserve_cap > 0)
        .then_some(bot_resupply::AmmoBinding {
            equipment,
            ammo_pool: item.ammo_pool,
            reserve_cap,
        })
}

#[derive(Clone, Default)]
pub(super) struct State {
    equipment: Option<EquipmentKey>,
    pub(super) magazine: u32,
    pub(super) reserve: u32,
    reload_until: Option<Duration>,
    next_shot: Duration,
    target: Option<Life>,
    acquired: Duration,
    next_perception: Duration,
    last_tick: Option<Duration>,
    yaw: f64,
    pitch: f64,
    shot_number: u64,
    last_diagnostic: Option<Duration>,
    recoil: recoil::State,
    recoil_profile: Option<recoil::Profile>,
    last_fired: Option<Duration>,
}
impl State {
    pub(super) fn item(&self, equipped: replication::InventoryItem) -> replication::InventoryItem {
        if self.equipment.is_some() {
            replication::InventoryItem {
                ammo_count: self.magazine as u8,
                reloading: self.reload_until.is_some(),
                ..equipped
            }
        } else {
            equipped
        }
    }
    pub(super) fn packed_pitch(&self) -> u8 {
        pack_pitch(self.pitch).unwrap_or(127)
    }
    fn reserve_capacity(metadata: &retail_combat::RetailBulletMetadata) -> u32 {
        metadata.magazine_capacity * metadata.initial_magazine_count.saturating_sub(1)
    }
    pub(super) fn is_resupply_dry(&self) -> bool {
        self.equipment.is_some()
            && self.magazine == 0
            && self.reserve == 0
            && self.reload_until.is_none()
    }
    pub(super) fn equipment_matches(&self, key: EquipmentKey) -> bool {
        self.equipment == Some(key)
    }
    #[cfg(test)]
    pub(crate) fn set_target_for_test(&mut self, id: u16) {
        self.target = Some(Life {
            id,
            generation: 1,
            reservation: None,
        });
    }
    /// Object index of the enemy this weapon currently holds, if any.
    pub(super) fn target_index(&self) -> Option<u16> {
        self.target.as_ref().map(|life| life.id)
    }
    fn bind(
        &mut self,
        key: EquipmentKey,
        p: &retail_combat::RetailBulletMetadata,
        recoil_profile: recoil::Profile,
        yaw: u16,
    ) {
        if self.equipment != Some(key) {
            *self = Self {
                equipment: Some(key),
                magazine: p.magazine_capacity,
                reserve: Self::reserve_capacity(p),
                yaw: f64::from(yaw) * std::f64::consts::TAU / 1024.,
                recoil: recoil::State::new(recoil_profile),
                recoil_profile: Some(recoil_profile),
                ..Default::default()
            };
        }
    }
    fn reload(
        &mut self,
        now: Duration,
        schedule: retail_combat::RetailFirearmSchedule,
        capacity: u32,
    ) -> bool {
        if let Some(until) = self.reload_until {
            if now < until {
                return false;
            }
            let count = capacity.min(self.reserve);
            self.magazine = count;
            self.reserve -= count;
            self.reload_until = None;
            return true;
        }
        if self.magazine == 0 && self.reserve > 0 {
            self.reload_until = Some(now + Duration::from_secs_f64(schedule.reload_seconds));
            return true;
        }
        false
    }
    fn consume(&mut self, now: Duration, schedule: retail_combat::RetailFirearmSchedule) -> bool {
        if self.magazine == 0 || self.reload_until.is_some() || now < self.next_shot {
            return false;
        }
        self.magazine -= 1;
        self.shot_number = self.shot_number.wrapping_add(1);
        self.next_shot = now + Duration::from_secs_f64(schedule.refire_seconds);
        true
    }
}
#[derive(Clone)]
pub(crate) struct Actor {
    life: Life,
    team: u8,
    feet: [f64; 3],
    health: u16,
    armor: Option<armor::ArmorProfile>,
}

impl Actor {
    fn bot_stat_life(&self) -> bot_firearm_kd::Life {
        bot_firearm_kd::Life {
            index: self.life.id,
            generation: self.life.generation,
            team: self.team,
            health: self.health,
        }
    }
}

pub(super) fn safe(bot: &WorldBot) -> bool {
    safe_for_phase(bot, false)
}

pub(super) fn safe_for_supply_observation(bot: &WorldBot) -> bool {
    safe_for_phase(
        bot,
        bot.resupply_seek
            .as_ref()
            .is_some_and(|seek| seek.owns(bot)),
    )
}

fn safe_for_phase(bot: &WorldBot, allow_owned_walk: bool) -> bool {
    safe_for(bot, allow_owned_walk, false)
}

/// A route paused by a perception reaction is still relocating (and may be
/// an operator's approach), but its body stands still and may fire.
pub(super) fn safe_while_held(bot: &WorldBot) -> bool {
    safe_for(bot, false, true)
}

fn safe_for(bot: &WorldBot, allow_owned_walk: bool, held: bool) -> bool {
    let r = bot.target.runtime.unwrap();
    // A body standing still while it waits its turn (a drop queue, a holding
    // spot or a traffic yield) may fight like any standing bot; walking to
    // those spots or taking a passage may not.
    let standing = !matches!(r.phase, BotPhase::Relocating);
    bot.health > 0
        && !matches!(r.phase, BotPhase::Dead)
        && (allow_owned_walk || held || !matches!(r.phase, BotPhase::Relocating))
        && r.physics_motion
            .is_some_and(|m| m.grounded && m.slide_velocity == [0.; 2])
        && (held || !r.navmesh_state.movement.sprinting())
        && !r.navmesh_state.on_player
        && (held || !bot.task.operator)
        && !bot.task.recovering
        && !bot.drop_committed
        && (standing || bot.drop_waiting.is_none())
        && bot.drop_permit.is_none()
        && (standing || bot.drop_queue_passage.is_none())
        && (standing || bot.drop_holding_goal.is_none())
        && !bot.drop_resume
        && !bot.drop_egress
        && !bot.drop_retreat
        && !bot.drop_reroute
        && (standing || bot.traffic_yield.is_none())
}
fn torso(feet: [f64; 3]) -> [f64; 3] {
    [
        feet[0],
        feet[1] + bot_physics::Body::default().height * 0.6,
        feet[2],
    ]
}
fn muzzle(feet: [f64; 3]) -> [f64; 3] {
    [
        feet[0],
        feet[1] + bot_physics::Body::default().height * 0.6,
        feet[2],
    ]
}
fn length(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}
fn intersects(from: [f64; 3], to: [f64; 3], bounds: Bounds) -> bool {
    let mut lo: f64 = 0.;
    let mut hi: f64 = 1.;
    for i in 0..3 {
        let d = to[i] - from[i];
        if d.abs() <= f64::EPSILON {
            if from[i] < bounds.min[i] || from[i] > bounds.max[i] {
                return false;
            }
        } else {
            let a = (bounds.min[i] - from[i]) / d;
            let b = (bounds.max[i] - from[i]) / d;
            lo = lo.max(a.min(b));
            hi = hi.min(a.max(b));
            if lo > hi {
                return false;
            }
        }
    }
    true
}
/// Placed objective geometry a sight line or bullet must not cross: the mode's
/// structures, hospitals and helicopters, with the mode's projectile row.
#[derive(Clone, Copy)]
pub(crate) struct Structures<'a> {
    pub(crate) sequences: &'a crate::domination_structural_world::SequenceCollisionState,
    pub(crate) row: u32,
}
impl Structures<'_> {
    /// `Err` is a failed or over-budget query, never a clear line. A
    /// prefetched answer is taken when the remaining work covers it.
    pub(crate) fn blocks_with(
        self,
        prefetch: Option<&crate::bot_sight_prefetch::SightPrefetch>,
        from: [f64; 3],
        to: [f64; 3],
        work: &mut usize,
    ) -> Result<bool, &'static str> {
        crate::bot_sight_prefetch::structures_blocked(prefetch, self, from, to, work).map_err(
            |_| {
                if *work == 0 {
                    "geometry_query_budget"
                } else {
                    "structure_query_failed"
                }
            },
        )
    }
}
impl Structures<'_> {
    /// The geometry's own answer, before `blocks` names a failure.
    pub(crate) fn raw_blocks(
        self,
        from: [f64; 3],
        to: [f64; 3],
        work: &mut usize,
    ) -> Result<bool, &'static str> {
        self.sequences.blocks_ray(from, to, self.row, work)
    }
}
impl<W: Write> Engine<W> {
    /// The current mode's placed structures, if it has any. Sabotage and
    /// development worlds have only the static map.
    pub(crate) fn bot_sight_structures(&self) -> Option<Structures<'_>> {
        let collision = match (&self.acquisition_gameplay, &self.domination_gameplay) {
            (Some(runtime), _) => runtime.blast_collision()?,
            (None, Some(world)) => world.frag_blast_collision(),
            (None, None) => return None,
        };
        Some(Structures {
            sequences: collision.sequences,
            row: collision.filter_rows[crate::collision::PROJECTILE_QUERY_LAYER],
        })
    }
}
/// What a sight line is checked against: the static world and the map it
/// must belong to, the mode's placed structures, and any answers prefetched
/// for this phase.
#[derive(Clone, Copy)]
pub(crate) struct SightGeometry<'a> {
    pub(crate) world: Option<&'a collision::CollisionWorld>,
    pub(crate) structures: Option<Structures<'a>>,
    pub(crate) map: &'a str,
    pub(crate) prefetch: Option<&'a crate::bot_sight_prefetch::SightPrefetch>,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn visible(
    geometry: SightGeometry<'_>,
    from: [f64; 3],
    to: [f64; 3],
    shooter: u16,
    target: u16,
    actors: &[Actor],
    rays: &mut usize,
    work: &mut usize,
) -> Result<(), &'static str> {
    let SightGeometry {
        world,
        structures,
        map,
        prefetch,
    } = geometry;
    let world = world.ok_or("geometry_unavailable")?;
    if world.metadata().map_name != map {
        return Err("geometry_map_mismatch");
    }
    if *rays == 0 {
        return Err("perception_budget");
    }
    *rays -= 1;
    if !world.verified_physics_regions().any(|r| {
        r.obstacles_verified
            && (0..3).all(|i| from[i].min(to[i]) >= r.min[i] && from[i].max(to[i]) <= r.max[i])
    }) {
        return Err("geometry_coverage_unknown");
    }
    // A shot crosses what the retail projectile row collides with.
    if crate::bot_sight_prefetch::world_blocked(prefetch, world, from, to, work)
        .map_err(|_| "geometry_query_budget")?
    {
        return Err("world_blocked");
    }
    if let Some(structures) = structures
        && structures.blocks_with(prefetch, from, to, work)?
    {
        return Err("structure_blocked");
    }
    for actor in actors
        .iter()
        .filter(|a| a.health > 0 && a.life.id != shooter && a.life.id != target)
    {
        if *work == 0 {
            return Err("body_query_budget");
        }
        *work -= 1;
        if intersects(from, to, Bounds::body(actor.feet, Default::default())) {
            return Err("body_blocked");
        }
    }
    Ok(())
}
/// Enemies a searching bot checks this tick, in order: `preferred` first,
/// then nearest, at most `max_candidates_per_bot`.
fn acquisition_candidates<'a>(
    actors: &'a [Actor],
    team: u8,
    from: [f64; 3],
    yaw: u16,
    policy: &Policy,
    weapon_range: f64,
    preferred: Option<u16>,
) -> Vec<&'a Actor> {
    let yaw = f64::from(yaw) * std::f64::consts::TAU / 1024.;
    let mut candidates: Vec<_> = actors
        .iter()
        .filter(|a| {
            a.team != team
                && a.health > 0
                && a.armor.is_some()
                && policy.can_acquire(from, torso(a.feet), yaw, weapon_range)
        })
        .collect();
    candidates.sort_by(|a, b| {
        (Some(b.life.id) == preferred)
            .cmp(&(Some(a.life.id) == preferred))
            .then(length(from, torso(a.feet)).total_cmp(&length(from, torso(b.feet))))
            .then(a.life.id.cmp(&b.life.id))
    });
    candidates.truncate(policy.max_candidates_per_bot);
    candidates
}
/// The order bots spend this tick's shared ray budget in: round robin from
/// `cursor`, with every bot already holding a target ahead of every bot still
/// looking for one. A held target needs at most one ray per shot, while a
/// search can spend `max_candidates_per_bot`; served in plain id order, a
/// large lobby's searches used the whole budget and an engaged bot was only
/// reached every few ticks, firing single shots instead of at its refire rate.
fn visit_order(bots: &BTreeMap<u16, WorldBot>, cursor: u16) -> Vec<u16> {
    order_engaged_first(bots.keys().copied(), cursor, |id| {
        bots[&id].firearm.target.is_some()
    })
}
fn order_engaged_first(
    ids: impl Iterator<Item = u16>,
    cursor: u16,
    engaged: impl Fn(u16) -> bool,
) -> Vec<u16> {
    let ids: Vec<_> = ids.collect();
    let start = ids.partition_point(|&id| id < cursor);
    let (mut order, searching): (Vec<_>, Vec<_>) = ids[start..]
        .iter()
        .chain(&ids[..start])
        .copied()
        .partition(|&id| engaged(id));
    order.extend(searching);
    order
}
// Character pitch pack operation is statically crosschecked against original
// retail0x61fde0..0x61fe60; see docs/BOT-FIREARMS.md for provenance and limits.
fn deferred(reason: &str) -> bool {
    matches!(
        reason,
        "perception_budget" | "geometry_query_budget" | "body_query_budget"
    )
}
fn pack_pitch(pitch: f64) -> Result<u8, &'static str> {
    if !pitch.is_finite() {
        return Err("invalid_bot_aim_pitch");
    }
    let shifted = (pitch as f32) + f32::from_bits(0x3fc90fdb);
    let scaled = shifted * f32::from_bits(0x42a2568a);
    Ok(scaled.clamp(0., 255.) as u8)
}
fn aim_endpoint(from: [f64; 3], yaw: f64, pitch: f64, distance: f64) -> [f64; 3] {
    [
        from[0] + yaw.sin() * pitch.cos() * distance,
        from[1] + pitch.sin() * distance,
        from[2] + yaw.cos() * pitch.cos() * distance,
    ]
}
fn turn(yaw: f64, wanted: f64, maximum: f64) -> (f64, f64) {
    let delta = (wanted - yaw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
        - std::f64::consts::PI;
    let moved = delta.clamp(-maximum, maximum);
    (
        (yaw + moved).rem_euclid(std::f64::consts::TAU),
        (delta - moved).abs(),
    )
}
fn accurate(id: u16, generation: u32, shot: u64, threshold: u16) -> bool {
    let mut x =
        (u64::from(id) << 32) ^ u64::from(generation) ^ shot.wrapping_mul(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    (x ^ (x >> 31)) % 1000 < u64::from(threshold)
}
pub(crate) fn validate_weapon_recoil(
    items: &item_catalog::ItemCatalog,
    kit: &replication::CharacterLoadout,
) -> Result<(), &'static str> {
    let equipped = kit.primary;
    let profile = retail_combat::RetailBulletProfile::from_equipped_with_items(
        items,
        kit,
        equipped,
        equipped.gear_slot,
        equipped.ammo_pool,
        true,
    )?;
    recoil::Profile::from_weapon(items.weapon_parameters(profile.item_id())?).map(|_| ())
}
impl<W: Write> Engine<W> {
    /// With bot workers, answer ahead the sight lines this tick's searching
    /// bots will check, in the order they will spend the ray budget, until
    /// the budget would be spent. Engaged bots' shot lines depend on aim this
    /// tick and stay on the tick thread. A prediction that differs from what
    /// the loop then asks only costs a direct query.
    #[allow(clippy::too_many_arguments)]
    fn prefetch_firearm_sight(
        &self,
        ids: &[u16],
        actors: &[Actor],
        now: Duration,
        policy: &Policy,
        perception: Option<&crate::bot_perception::Policy>,
        holds: &std::collections::BTreeSet<u16>,
        items: &item_catalog::ItemCatalog,
        catalog: &retail_combat::RetailCombatCatalog,
    ) -> Option<crate::bot_sight_prefetch::SightPrefetch> {
        let world = self
            .collision_world
            .as_ref()
            .filter(|_| self.bot_threads > 1)?;
        let mut lines = Vec::new();
        for &id in ids {
            if lines.len() >= policy.max_rays_per_tick {
                break;
            }
            let bot = &self.bot_registry.bots[&id];
            if bot.firearm.target.is_some()
                || now < bot.firearm.next_perception
                || !(safe_for_supply_observation(bot)
                    || holds.contains(&id) && safe_while_held(bot))
            {
                continue;
            }
            let Some(me) = actors.iter().find(|a| a.life.id == id && a.health > 0) else {
                continue;
            };
            let kit = &bot.target.loadout;
            let Some(range) = retail_combat::RetailBulletProfile::from_equipped_with_items(
                items,
                kit,
                kit.primary,
                kit.primary.gear_slot,
                kit.primary.ammo_pool,
                true,
            )
            .ok()
            .and_then(|p| catalog.bullet_metadata(p.item_id()))
            .map(|m| m.max_range) else {
                continue;
            };
            let policy = match perception {
                Some(p) => &Policy {
                    max_engagement_distance_m: crate::bot_perception::bot_sight_range(
                        p, items, bot,
                    ),
                    ..policy.clone()
                },
                None => policy,
            };
            let preferred = self
                .armed_priority_target(bot)
                .or_else(|| self.bot_perception.threat_enemy(id, now));
            let from = muzzle(me.feet);
            lines.extend(
                acquisition_candidates(actors, me.team, from, bot.yaw, policy, range, preferred)
                    .into_iter()
                    .map(|a| (from, torso(a.feet))),
            );
        }
        Some(crate::bot_sight_prefetch::SightPrefetch::compute(
            world,
            self.bot_sight_structures(),
            policy.geometry_work_per_tick,
            &lines,
            self.bot_threads,
        ))
    }
    pub(crate) fn tick_bot_firearms(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let mut resupply_observations = BTreeMap::new();
        let resupply_enabled = self.bot_resupply_enabled();
        // Invalidate even when unsafe/disabled: a new life or equipment version
        // must never publish the previous weapon's ammunition in a join snapshot.
        for bot in self.bot_registry.bots.values_mut() {
            // Service cancellation records before a same-life equipment rebind.
            if !bot.has_supply_action()
                && (bot.health == 0
                    || bot
                        .firearm
                        .equipment
                        .is_some_and(|old| Some(old) != current_equipment_key(bot)))
            {
                bot.firearm = Default::default();
            }
        }
        let Some(policy) = self
            .bot_registry
            .config
            .as_ref()
            .map(|c| c.firearms.clone())
        else {
            self.tick_bot_resupply(now, wall, &resupply_observations)?;
            return Ok(Vec::new());
        };
        if !policy.enabled
            || !self.match_started
            || self.match_terminal.is_some()
            || self.wire_profile.application_id() != 20231
            || !self
                .active
                .iter()
                .chain(self.peers.values())
                .any(|a| a.ready_for_updates && a.initial_objects.is_some())
        {
            self.tick_bot_resupply(now, wall, &resupply_observations)?;
            for bot in self.bot_registry.bots.values_mut() {
                bot.firearm.target = None;
                bot.firearm.last_tick = None;
            }
            return Ok(Vec::new());
        }
        let items = self.item_catalog.clone();
        let Ok(catalog) = items.retail_combat() else {
            self.tick_bot_resupply(now, wall, &resupply_observations)?;
            return Ok(Vec::new());
        };
        let world = self
            .active
            .as_ref()
            .or_else(|| self.peers.values().next())
            .ok_or("bot_firearm_world_unavailable")?
            .reservation
            .world;
        let leadership = self.plan_leadership(now)?;
        let mut actors = Vec::new();
        for a in self.active.iter().chain(self.peers.values()).filter(|a| {
            a.reservation.world == world && a.entered_game && a.ready_for_updates && !a.is_mounted()
        }) {
            let CharacterLife::Alive {
                generation,
                health_quarter_points,
            } = a.life
            else {
                continue;
            };
            let Some(sample) = a.tracking.sample else {
                continue;
            };
            let kit = a.equipped_loadout();
            let armor = kit
                .and_then(|k| armor::ArmorProfile::from_retail_loadout(&k, catalog).ok())
                .map(|p| p.with_modifiers(&leadership.modifiers(a)));
            actors.push(Actor {
                life: Life {
                    id: a.reservation.player,
                    generation,
                    reservation: Some(a.reservation.id.clone()),
                },
                team: a.team(),
                feet: [
                    sample.position.x.into(),
                    sample.position.y.into(),
                    sample.position.z.into(),
                ],
                health: health_quarter_points,
                armor,
            });
        }
        for (&id, bot) in &self.bot_registry.bots {
            let r = bot.target.runtime.unwrap();
            if bot.health == 0 {
                continue;
            }
            actors.push(Actor {
                life: Life {
                    id,
                    generation: r.generation,
                    reservation: None,
                },
                team: bot.target.squad.team(),
                feet: r.physical_position(),
                health: bot.health,
                armor: armor::ArmorProfile::from_retail_loadout(&bot.target.loadout, catalog).ok(),
            });
        }
        let mut messages = Vec::new();
        let perception_holds = self.bot_perception.held_ids();
        let perception = self.bot_perception_policy();
        let mut rays = policy.max_rays_per_tick;
        let mut work = policy.geometry_work_per_tick;
        let map = self.map_name().to_owned();
        let ids = visit_order(&self.bot_registry.bots, self.bot_registry.firearm_cursor);
        let base_policy = policy;
        let prefetch = self.prefetch_firearm_sight(
            &ids,
            &actors,
            now,
            &base_policy,
            perception.as_ref(),
            &perception_holds,
            &items,
            catalog,
        );
        for id in ids {
            if rays == 0 || work == 0 {
                self.bot_registry.firearm_cursor = id;
                break;
            }
            // With perception on, the weapon's retail AI usage band sets how
            // far this bot engages (and its accuracy falloff), so a sniper
            // reaches farther than a shotgun.
            let policy = &match &perception {
                Some(p) => Policy {
                    max_engagement_distance_m: crate::bot_perception::bot_sight_range(
                        p,
                        &items,
                        &self.bot_registry.bots[&id],
                    ),
                    ..base_policy.clone()
                },
                None => base_policy.clone(),
            };
            self.bot_registry.firearm_cursor = id.wrapping_add(1);
            // Each accepted shot updates current health before the next shooter.
            let Some(me) = actors
                .iter()
                .find(|a| a.life.id == id && a.health > 0)
                .cloned()
            else {
                continue;
            };
            let bot = &self.bot_registry.bots[&id];
            let paused = perception_holds.contains(&id) && safe_while_held(bot);
            if !safe_for_supply_observation(bot) && !paused {
                let state = &mut self.bot_registry.bots.get_mut(&id).unwrap().firearm;
                if let Some(profile) = state.recoil_profile {
                    let dt = state
                        .last_tick
                        .map_or(0., |last| now.saturating_sub(last).as_secs_f64().min(0.25));
                    state.recoil.tick(profile, dt, false, 0., 0.);
                }
                state.target = None;
                state.last_tick = Some(now);
                continue;
            }
            let kit = &bot.target.loadout;
            let equipped = kit.primary;
            let servicing = bot.has_supply_action();
            let service_due = bot.resupply.is_some_and(|session| session.due(now))
                || bot.resupply_planning.is_some()
                || (bot.resupply_seek.is_some()
                    && matches!(bot.target.runtime.unwrap().phase, BotPhase::Complete));
            let damage_revision = bot.damage_revision;
            if servicing && current_equipment_key(bot) != bot.firearm.equipment {
                continue;
            }
            let profile = retail_combat::RetailBulletProfile::from_equipped_with_items(
                &items,
                kit,
                equipped,
                equipped.gear_slot,
                equipped.ammo_pool,
                true,
            );
            let supported = profile.and_then(|p| {
                let metadata = catalog
                    .bullet_metadata(p.item_id())
                    .ok_or("unsupported_weapon")?;
                let schedule = metadata.firearm.ok_or("unsupported_firearm_schedule")?;
                if schedule.infinite_ammo {
                    return Err("unsupported_infinite_ammunition");
                }
                let key = (
                    me.life.generation,
                    kit.handle.update_id,
                    kit.handle.inventory_index,
                    equipped.gear_slot,
                    p.item_id(),
                );
                let existing = &self.bot_registry.bots[&id].firearm;
                let recoil_profile = if existing.equipment == Some(key) {
                    existing
                        .recoil_profile
                        .ok_or("missing_bound_weapon_recoil")?
                } else {
                    recoil::Profile::from_weapon(items.weapon_parameters(p.item_id())?)?
                };
                Ok((p, metadata, schedule, recoil_profile))
            });
            let (profile, metadata, schedule, recoil_profile) = match supported {
                Ok(v) => v,
                Err(reason) => {
                    if servicing {
                        resupply_observations.insert(
                            id,
                            bot_resupply::Observation {
                                ammo: None,
                                threat: bot_resupply::Threat::Deferred,
                                damage_revision,
                            },
                        );
                    }
                    let s = &mut self.bot_registry.bots.get_mut(&id).unwrap().firearm;
                    if s.last_diagnostic
                        .is_none_or(|last| now.saturating_sub(last) >= Duration::from_secs(5))
                    {
                        s.last_diagnostic = Some(now);
                        self.record(json!({"op":"bot_firearm_unavailable","object_index":id,"reason":reason}),wall)?;
                    }
                    continue;
                }
            };
            let mut state = self.bot_registry.bots[&id].firearm.clone();
            let equipment = (
                me.life.generation,
                kit.handle.update_id,
                kit.handle.inventory_index,
                equipped.gear_slot,
                profile.item_id(),
            );
            state.bind(
                equipment,
                metadata,
                recoil_profile,
                self.bot_registry.bots[&id].yaw,
            );
            let dt = state
                .last_tick
                .map_or(0., |last| now.saturating_sub(last).as_secs_f64().min(0.25));
            state.last_tick = Some(now);
            let reload_changed =
                !servicing && state.reload(now, schedule, metadata.magazine_capacity);
            let reload_receipt=reload_changed.then(||json!({"op":"bot_firearm_reload","timestamp_ms":now.as_millis() as u64,"object_index":id,"generation":me.life.generation,"magazine":state.magazine,"reserve":state.reserve,"reloading":state.reload_until.is_some()}));
            let from = muzzle(me.feet);
            // An attacker hunting a disarmer looks for that enemy first.
            let priority = self.armed_priority_target(&self.bot_registry.bots[&id]);
            // Otherwise a new acquisition prefers the enemy perception last
            // reacted to (for example its attacker); a held target is kept.
            let preferred = priority.or_else(|| self.bot_perception.threat_enemy(id, now));
            if priority.is_some_and(|p| state.target.as_ref().is_some_and(|t| t.id != p))
                && now >= state.next_perception
            {
                state.target = None;
            }
            let mut blocked = None;
            let mut threat = bot_resupply::Threat::Deferred;
            if state.target.as_ref().is_some_and(|life| {
                !actors.iter().any(|a| {
                    a.life == *life
                        && a.health > 0
                        && policy.within_range(length(from, torso(a.feet)), metadata.max_range)
                })
            }) {
                state.target = None;
                state.next_perception = now;
            }
            // Retain a current target instead of spending the last ray on
            // periodic reacquisition; every actual shot still gets fresh LOS.
            if state.target.is_none() && (now >= state.next_perception || service_due) {
                state.next_perception = now + Duration::from_millis(policy.perception_interval_ms);
                let candidates = acquisition_candidates(
                    &actors,
                    me.team,
                    from,
                    self.bot_registry.bots[&id].yaw,
                    policy,
                    metadata.max_range,
                    preferred,
                );
                let mut selected = None;
                let mut confirmed_quiet = true;
                for candidate in candidates {
                    match visible(
                        SightGeometry {
                            world: self.collision_world.as_ref(),
                            structures: self.bot_sight_structures(),
                            map: &map,
                            prefetch: prefetch.as_ref(),
                        },
                        from,
                        torso(candidate.feet),
                        id,
                        candidate.life.id,
                        &actors,
                        &mut rays,
                        &mut work,
                    ) {
                        Ok(()) => {
                            selected = Some(candidate.life.clone());
                            break;
                        }
                        Err(reason) => {
                            confirmed_quiet &= matches!(
                                reason,
                                "world_blocked" | "structure_blocked" | "body_blocked"
                            );
                            blocked = Some(reason);
                            if deferred(reason) {
                                state.next_perception = now;
                                break;
                            }
                        }
                    }
                }
                if selected.is_none() && confirmed_quiet {
                    threat = bot_resupply::Threat::Quiet;
                }
                if selected != state.target {
                    if selected.is_some() {
                        // Route movement can publish another facing while no
                        // combat target is held. Turn from that actual pose.
                        state.yaw = f64::from(self.bot_registry.bots[&id].yaw)
                            * std::f64::consts::TAU
                            / 1024.;
                    }
                    // A bot that has already reacted to a threat (turned toward
                    // gunfire, or was hit) needs only the shorter primed delay.
                    let primed = perception
                        .as_ref()
                        .filter(|_| {
                            selected.is_some() && self.bot_perception.attention(id, now).is_some()
                        })
                        .map(|p| p.primed_reaction_ms.min(policy.reaction_ms));
                    state.target = selected;
                    state.acquired = now.saturating_sub(Duration::from_millis(
                        primed.map_or(0, |primed| policy.reaction_ms - primed),
                    ));
                }
            }
            let mut shot = None;
            if state.target.is_none() {
                state.recoil.tick(recoil_profile, dt, false, 0., 0.);
            }
            // Without a target, face whatever perception last noticed.
            let attending = state
                .target
                .is_none()
                .then(|| self.bot_perception.attention(id, now))
                .flatten();
            if let Some(point) = attending {
                let facing =
                    f64::from(self.bot_registry.bots[&id].yaw) * std::f64::consts::TAU / 1024.;
                let wanted = (point[0] - from[0]).atan2(point[2] - from[2]);
                let rate = perception
                    .as_ref()
                    .map_or(policy.turn_degrees_per_second, |p| {
                        p.attention_turn_degrees_per_second
                            .max(policy.turn_degrees_per_second)
                    });
                state.yaw = turn(facing, wanted, rate.to_radians() * dt).0;
            }
            if let Some(target) = state
                .target
                .as_ref()
                .and_then(|life| actors.iter().find(|a| a.life == *life && a.health > 0))
                .cloned()
            {
                threat = bot_resupply::Threat::Enemy;
                let to = torso(target.feet);
                let moving = !paused
                    && matches!(
                        self.bot_registry.bots[&id].target.runtime.unwrap().phase,
                        BotPhase::Relocating
                    );
                let held = matches!(
                    schedule.initial_fire_mode,
                    retail_combat::RetailFireMode::Auto
                ) && state.last_fired.is_some_and(|last| {
                    now.saturating_sub(last).as_secs_f64() <= schedule.refire_seconds + 0.05
                });
                state
                    .recoil
                    .tick(recoil_profile, dt, held, if moving { 1. } else { 0. }, 0.);
                let wanted = (to[0] - from[0]).atan2(to[2] - from[2]);
                let (yaw, error) = turn(
                    state.yaw,
                    wanted,
                    policy.turn_degrees_per_second.to_radians() * dt,
                );
                state.yaw = yaw;
                let wanted_pitch =
                    (to[1] - from[1]).atan2((to[0] - from[0]).hypot(to[2] - from[2]));
                let pitch_delta = wanted_pitch - state.pitch;
                let pitch_move = pitch_delta.clamp(
                    -policy.turn_degrees_per_second.to_radians() * dt,
                    policy.turn_degrees_per_second.to_radians() * dt,
                );
                state.pitch += pitch_move;
                let pitch_error = (pitch_delta - pitch_move).abs();
                let aimed_end = aim_endpoint(from, state.yaw, state.pitch, length(from, to));
                if !servicing
                    && policy.within_range(length(from, to), metadata.max_range)
                    && error <= policy.aim_tolerance_degrees.to_radians()
                    && pitch_error <= policy.aim_tolerance_degrees.to_radians()
                    && intersects(
                        from,
                        aimed_end,
                        Bounds::body(target.feet, Default::default()),
                    )
                    && now.saturating_sub(state.acquired)
                        >= Duration::from_millis(policy.reaction_ms)
                    && state.magazine > 0
                    && state.reload_until.is_none()
                    && now >= state.next_shot
                {
                    let seed = (u64::from(id) << 32)
                        ^ u64::from(me.life.generation)
                        ^ state
                            .shot_number
                            .wrapping_add(1)
                            .wrapping_mul(0xd6e8feb86659fd93);
                    let mut next_recoil = state.recoil;
                    next_recoil.fire(recoil_profile, seed);
                    let shot_end = next_recoil.endpoint(
                        recoil_profile,
                        from,
                        state.yaw,
                        state.pitch,
                        length(from, to),
                        policy.recoil_compensation_per_mille,
                        seed,
                    );
                    match visible(
                        SightGeometry {
                            world: self.collision_world.as_ref(),
                            structures: self.bot_sight_structures(),
                            map: &map,
                            prefetch: prefetch.as_ref(),
                        },
                        from,
                        shot_end,
                        id,
                        target.life.id,
                        &actors,
                        &mut rays,
                        &mut work,
                    ) {
                        Ok(()) => {
                            if state.consume(now, schedule) {
                                state.recoil = next_recoil;
                                state.last_fired = Some(now);
                                let hit = intersects(
                                    from,
                                    shot_end,
                                    Bounds::body(target.feet, Default::default()),
                                ) && accurate(
                                    id,
                                    me.life.generation,
                                    state.shot_number,
                                    policy.accuracy_at(length(from, to), metadata.max_range),
                                );
                                let resolved = damage::resolve_character_hit(
                                    profile.raw_character_damage(length(from, to) as f32, 1., 0)?,
                                    target.armor.unwrap().with_incoming_base(
                                        self.character_incoming_base(
                                            0x0380_0000 | u32::from(id),
                                            0x0380_0000 | u32::from(target.life.id),
                                            world,
                                        ),
                                    )?,
                                    damage::DamageKind::Bullet,
                                    0x03800000 | u32::from(id),
                                    0,
                                    target.health,
                                )?;
                                shot = Some((target, hit, resolved, shot_end));
                            }
                        }
                        Err(reason) => {
                            blocked = Some(reason);
                            if !deferred(reason) {
                                state.target = None;
                            }
                        }
                    }
                }
            }
            let yaw = ((state.yaw / std::f64::consts::TAU * 1024.) as u16) & 1023;
            let pitch = state.packed_pitch();
            let pitch_changed = pitch != self.bot_registry.bots[&id].firearm.packed_pitch();
            let publish = pitch_changed
                || reload_changed
                || shot.is_some()
                || ((state.target.is_some() || attending.is_some())
                    && yaw != self.bot_registry.bots[&id].yaw);
            let fired_path = shot.as_ref().map(|(_, _, _, end)| *end);
            let mut receipts = Vec::new();
            if let Some(reason) = blocked
                && state
                    .last_diagnostic
                    .is_none_or(|last| now.saturating_sub(last) >= Duration::from_secs(1))
            {
                state.last_diagnostic = Some(now);
                receipts.push(json!({"op":"bot_firearm_blocked","object_index":id,"generation":me.life.generation,"reason":reason}));
            }
            if let Some(receipt) = reload_receipt {
                receipts.push(receipt);
            }

            // Record before committing ammunition, health, or outgoing packets.
            let mut pending_damage = None;
            let mut death_notification = None;
            let mut bot_kd = None;
            let mut human_death = None;
            let mut firearm_assists = None;
            let mut assist_statistics = None;
            if let Some((target, hit, (calculation, amount, resolved), aimed_end)) = shot {
                let shot_receipt = receipts.len();
                receipts.push(json!({"op":"bot_firearm_shot","timestamp_ms":now.as_millis() as u64,"object_index":id,"generation":me.life.generation,
                    "target":target.life.id,"target_generation":target.life.generation,"weapon_item":profile.item_id(),"magazine":state.magazine,"reserve":state.reserve,
                    "origin":from,"destination":torso(target.feet),"aim_endpoint":aimed_end,"yaw_radians":state.yaw,"pitch_radians":state.pitch,"hit":hit,"amount_hp":if hit {amount} else {0.},"damage_calculation":calculation,
                    "recoil_pitch_degrees":state.recoil.pitch_degrees,"recoil_yaw_degrees":state.recoil.yaw_degrees,"spread_degrees":state.recoil.width_degrees(recoil_profile),"recoil_compensation_per_mille":policy.recoil_compensation_per_mille,
                    "distance_m":length(from,torso(target.feet)),"accuracy_per_mille":policy.accuracy_at(length(from,torso(target.feet)),metadata.max_range),
                    "health_before":target.health,"health_after":if hit {resolved.map_or(target.health,|d|d.health_quarter_points)} else {target.health}}));
                if hit
                    && let Some(assists) = firearm_assists::Plan::prepare(
                        self,
                        world,
                        now,
                        firearm_assists::Fighter {
                            index: id,
                            reservation: None,
                            generation: me.life.generation,
                        },
                        firearm_assists::Fighter {
                            index: target.life.id,
                            reservation: target.life.reservation.as_deref(),
                            generation: target.life.generation,
                        },
                        resolved.is_some_and(|damage| damage.health_quarter_points == 0),
                    )
                {
                    receipts[shot_receipt]["firearm_assists"] = json!(assists.audit());
                    if assists.humans().next().is_some() {
                        let staged = events::plan_bot_firearm_assists(
                            self,
                            &assists,
                            Some(target.feet.map(|value| value as f32)),
                        )?;
                        staged.validate(self)?;
                        receipts[shot_receipt]["assist_settlement"] = staged.audit();
                        assist_statistics = Some(staged);
                    }
                    firearm_assists = Some(assists);
                }
                if hit && let Some(mut damage) = resolved {
                    let mut human_life = None;
                    if let Some(reservation) = &target.life.reservation {
                        let victim = self
                            .active
                            .iter()
                            .chain(self.peers.values())
                            .find(|a| {
                                a.reservation.id == *reservation && a.reservation.world == world
                                    && a.reservation.player == target.life.id && a.team() == target.team
                                    && matches!(a.life, CharacterLife::Alive {generation, health_quarter_points}
                                        if generation == target.life.generation && health_quarter_points == target.health && health_quarter_points > 0)
                            })
                            .ok_or("stale_bot_firearm_target")?;
                        let (model, life) = self.plan_human_damage(victim, amount, damage, now)?;
                        damage = model;
                        human_life = Some(life);
                        receipts.push(json!({"op":"bot_firearm_lifecycle_planned","reservation_id":reservation,"life":format!("{life:?}"),"conditions":damage.conditions}));
                        if damage.health_quarter_points == 0 {
                            for removal in victim.projectile_replicas.clone().take_all_removals() {
                                removal.encode_with_profile(
                                    1,
                                    now.as_millis() as u32,
                                    victim.wire_profile,
                                )?;
                            }
                            human_death = Some(events::plan_bot_killed_human(
                                self,
                                victim,
                                target.life.generation,
                            )?);
                        }
                    }
                    death_notification = self.plan_weapon_death(
                        world,
                        id,
                        target.life.id,
                        death_notifications::Weapon::Firearm(profile.item_id()),
                        0,
                        damage,
                    )?;
                    if let Some(plan) = &death_notification {
                        receipts.push(json!({"op":"bot_firearm_death_planned","timestamp_ms":now.as_millis() as u64,"notification":plan.audit()}));
                    }
                    if damage.health_quarter_points == 0 {
                        let plan = bot_firearm_kd::Plan::prepare(
                            self,
                            world,
                            (me.team != target.team).then(|| me.bot_stat_life()),
                            target
                                .life
                                .reservation
                                .is_none()
                                .then(|| target.bot_stat_life()),
                        )?;
                        receipts[shot_receipt]["bot_firearm_kd"] = json!(plan.audit());
                        bot_kd = Some(plan);
                    }
                    let packet = self
                        .character_damage_update(
                            2,
                            now.as_millis() as u32,
                            target.life.id,
                            damage,
                            damage.health_quarter_points == 0,
                        )
                        .encode_with_profile(self.wire_profile)?;
                    pending_damage = Some((target, damage, human_life, packet));
                }
            }
            let fields = player_tracking::ClientCharacterFields {
                yaw: Some(yaw),
                pitch: Some(pitch),
                primary: Some(replication::InventoryItem {
                    ammo_count: state.magazine as u8,
                    reloading: state.reload_until.is_some(),
                    ..equipped
                }),
                ..Default::default()
            };
            let owner_packet = publish
                .then(|| {
                    replication::multiplayer::CharacterOwnerDelta {
                        sequence: 2,
                        timestamp_ms: now.as_millis() as u32,
                        player: id,
                        fields: &fields,
                    }
                    .encode_with_profile(self.wire_profile)
                })
                .transpose()?;
            if let Some(assists) = &firearm_assists {
                assists.check_before_record(self)?;
            }
            for receipt in receipts {
                self.record(receipt, wall)?;
            }
            let damage_packet = if let Some((target, damage, human_life, packet)) = pending_damage {
                if let Some(reservation) = &target.life.reservation {
                    if let Some(plan) = death_notification.take() {
                        self.commit_weapon_death(plan, damage, now)?;
                    } else {
                        let victim = self
                            .active
                            .as_mut()
                            .filter(|a| a.reservation.id == *reservation)
                            .or_else(|| self.peers.get_mut(reservation))
                            .ok_or("stale_bot_firearm_target")?;
                        victim.apply_peer_damage(target.life.generation, damage, now)?;
                    }
                    if let Some(life) = human_life {
                        let victim = self
                            .active
                            .as_mut()
                            .filter(|a| a.reservation.id == *reservation)
                            .or_else(|| self.peers.get_mut(reservation))
                            .ok_or("stale_bot_firearm_target")?;
                        victim.life = life;
                    }
                } else if let Some(plan) = death_notification.take() {
                    self.commit_weapon_death(plan, damage, now)?;
                } else {
                    self.commit_bot_damage(target.life.id, target.life.generation, damage, now)?;
                }
                if let Some(kd) = bot_kd {
                    kd.install(self)?;
                }
                if let Some(death) = human_death {
                    messages.extend(death.install(self)?);
                }
                if let Some(actor) = actors.iter_mut().find(|a| a.life == target.life) {
                    actor.health = damage.health_quarter_points;
                }
                Some(packet)
            } else {
                None
            };
            if let Some(assists) = firearm_assists {
                assists.install(self)?;
            }
            if let Some(statistics) = assist_statistics {
                messages.extend(statistics.commit(self));
            }
            if resupply_enabled && (servicing || state.is_resupply_dry()) {
                resupply_observations.insert(
                    id,
                    bot_resupply::Observation {
                        ammo: Some(bot_resupply::AmmoBinding {
                            equipment,
                            ammo_pool: equipped.ammo_pool,
                            reserve_cap: State::reserve_capacity(metadata),
                        }),
                        threat,
                        damage_revision,
                    },
                );
            }
            if let Some(end) = fired_path {
                // The other team hears the shot; its path past them is a near miss.
                let kind =
                    crate::bot_perception::bot_firearm_kind(&items, &self.bot_registry.bots[&id]);
                self.bot_perception.push(crate::bot_perception::Sound {
                    kind,
                    team: me.team,
                    source: id,
                    origin: from,
                    path_end: Some([0, 1, 2].map(|i| from[i] + (end[i] - from[i]) * 1.25)),
                });
            }
            let bot = self.bot_registry.bots.get_mut(&id).unwrap();
            bot.firearm = state;
            if publish {
                bot.yaw = yaw;
            }
            // The shooter's own pose follows its snapshot; damage can be to a human.
            if let Some(packet) = owner_packet {
                self.broadcast_bot_state_packet(packet);
            }
            if let Some(packet) = damage_packet {
                self.broadcast_bot_packet(packet);
            }
        }
        self.record_sight_usage(prefetch);
        self.tick_bot_resupply(now, wall, &resupply_observations)?;
        Ok(messages)
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn engaged_bots_spend_the_ray_budget_before_searching_bots() {
        let ids = [2u16, 5, 7, 9, 12, 20];
        let engaged = |id: u16| matches!(id, 5 | 12 | 20);
        // Round robin from the cursor inside each group, engaged group first.
        assert_eq!(
            order_engaged_first(ids.into_iter(), 9, engaged),
            [12, 20, 5, 9, 2, 7]
        );
        // A cursor past every id wraps to the start, as before.
        assert_eq!(
            order_engaged_first(ids.into_iter(), 300, engaged),
            [5, 12, 20, 2, 7, 9]
        );
        // With nobody engaged it is the previous plain rotation.
        assert_eq!(
            order_engaged_first(ids.into_iter(), 7, |_| false),
            [7, 9, 12, 20, 2, 5]
        );
    }

    #[test]
    fn explicit_engagement_policy_loads_and_rejects_invalid_limits() {
        let policy = serde_json::from_value::<Policy>(json!({
            "max_engagement_distance_m": 60.0,
            "acquisition_fov_degrees": 120.0,
            "far_accuracy_per_mille": 100,
            "recoil_compensation_per_mille": 250
        }));
        assert!(
            policy.is_ok(),
            "external engagement limits must be supported"
        );
        assert!(policy.unwrap().validate().is_ok());
    }

    #[test]
    fn default_policy_gives_a_human_time_to_react() {
        assert_eq!(Policy::default().reaction_ms, 800);
        assert_eq!(Policy::default().accuracy_per_mille, 300);
    }
    #[test]
    fn range_and_field_of_view_bound_both_acquisition_and_retention() {
        let p = Policy::default();
        assert!(p.can_acquire([0.; 3], [0., 0., 59.], 0., 300.));
        assert!(!p.can_acquire([0.; 3], [0., 0., 60.], 0., 300.));
        assert!(!p.within_range(60., 300.));
        assert!(!p.within_range(30., 30.));
        assert!(!p.within_range(f64::NAN, 300.));
        assert!(!p.can_acquire([0.; 3], [0., 0., -10.], 0., 300.));
        assert!(!p.can_acquire([0.; 3], [10., 0., 0.], 0., 300.));
        assert!(p.can_acquire([0.; 3], [10., 0., 0.], std::f64::consts::FRAC_PI_2, 300.));
        // Angular wraparound must not hide a target just across zero yaw.
        assert!(p.can_acquire([0.; 3], [0., 0., 10.], std::f64::consts::TAU - 0.01, 300.));
    }

    #[test]
    fn distance_accuracy_decreases_and_never_exceeds_explicit_near_accuracy() {
        let p = Policy::default();
        assert_eq!(p.accuracy_at(0., 300.), 300);
        assert_eq!(p.accuracy_at(30., 300.), 200);
        assert_eq!(p.accuracy_at(60., 300.), 0);
        let mut zero = p.clone();
        zero.accuracy_per_mille = 0;
        assert_eq!(zero.accuracy_at(1., 300.), 0);
        for i in 0..600 {
            let a = p.accuracy_at(i as f64 / 10., 300.);
            let b = p.accuracy_at((i + 1) as f64 / 10., 300.);
            assert!(a >= b);
        }
        assert!(accurate(3, 1, 1, 1000));
        assert!(!accurate(3, 1, 1, 0));
    }

    #[test]
    fn invalid_engagement_limits_fail_validation() {
        for range in [0., -1., f64::NAN, f64::INFINITY, 2049.] {
            let p = Policy {
                max_engagement_distance_m: range,
                ..Policy::default()
            };
            assert!(p.validate().is_err());
        }
        for fov in [0., 361., f64::NAN] {
            let p = Policy {
                acquisition_fov_degrees: fov,
                ..Policy::default()
            };
            assert!(p.validate().is_err());
        }
        let p = Policy {
            far_accuracy_per_mille: 1001,
            ..Policy::default()
        };
        assert!(p.validate().is_err());
    }
    #[test]
    fn recoil_changes_hit_geometry_instead_of_only_an_accuracy_coin_flip() {
        let p=recoil::Profile::from_weapon(&json!({"reticule_params_stand":{
            "KnockAdd":0,"KnockFire":3.,"KnockReturn":1.,"SideKnockFire":0.,"SideKnockReturn":1.,
            "SpreadFire":0.,"SpreadReturn":1.,"SpreadMove":0.,"SpreadRotate":0.,"MoveSpreadReturn":1.,
            "cone_width_range":[0.,0.],"pitch_offset_range":[0.,9.],"yaw_offset_range":[-1.,1.],
            "SpreadNormalization":0,"WidthCombineOp":"kWCO_Add"}})).unwrap();
        let from = [0.; 3];
        let body = Bounds::body([0., -1., 30.], Default::default());
        let mut state = recoil::State::new(p);
        assert!(intersects(
            from,
            state.endpoint(p, from, 0., 0., 30., 0, 1),
            body
        ));
        state.fire(p, 1);
        assert!(!intersects(
            from,
            state.endpoint(p, from, 0., 0., 30., 0, 1),
            body
        ));
        state.tick(p, 5., false, 0., 0.);
        assert!(intersects(
            from,
            state.endpoint(p, from, 0., 0., 30., 0, 1),
            body
        ));
    }
}
