//! World-owned firearm admission. AI policy is explicit; weapon values come
//! exclusively from the extracted retail combat catalog. No client hit spoofing.
use super::*;
#[path = "bot_recoil.rs"]
mod recoil;
use crate::actor_collision::Bounds;
use crate::bot_registry::WorldBot;
use crate::development_target::BotPhase;

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
#[derive(Clone, Default)]
pub(super) struct State {
    equipment: Option<(u32, u8, u8, u8, u16)>,
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
    fn bind(
        &mut self,
        key: (u32, u8, u8, u8, u16),
        p: &retail_combat::RetailBulletMetadata,
        recoil_profile: recoil::Profile,
        yaw: u16,
    ) {
        if self.equipment != Some(key) {
            *self = Self {
                equipment: Some(key),
                magazine: p.magazine_capacity,
                reserve: p.magazine_capacity * p.initial_magazine_count.saturating_sub(1),
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
struct Actor {
    life: Life,
    team: u8,
    feet: [f64; 3],
    health: u16,
    armor: Option<armor::ArmorProfile>,
}
pub(super) fn safe(bot: &WorldBot) -> bool {
    let r = bot.target.runtime.unwrap();
    bot.health > 0
        && !matches!(r.phase, BotPhase::Dead | BotPhase::Relocating)
        && r.physics_motion
            .is_some_and(|m| m.grounded && m.slide_velocity == [0.; 2])
        && !r.navmesh_state.movement.sprinting()
        && !r.navmesh_state.on_player
        && !bot.task.operator
        && !bot.task.recovering
        && !bot.drop_committed
        && bot.drop_waiting.is_none()
        && bot.drop_permit.is_none()
        && bot.drop_queue_passage.is_none()
        && bot.drop_holding_goal.is_none()
        && !bot.drop_resume
        && !bot.drop_egress
        && !bot.drop_retreat
        && !bot.drop_reroute
        && bot.traffic_yield.is_none()
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
#[allow(clippy::too_many_arguments)]
fn visible(
    world: Option<&collision::CollisionWorld>,
    map: &str,
    from: [f64; 3],
    to: [f64; 3],
    shooter: u16,
    target: u16,
    actors: &[Actor],
    rays: &mut usize,
    work: &mut usize,
) -> Result<(), &'static str> {
    let world = world.ok_or("geometry_unavailable")?;
    if world.metadata().map_name != map {
        return Err("geometry_map_mismatch");
    }
    if *rays == 0 {
        return Err("perception_budget");
    }
    *rays -= 1;
    if !world.metadata().physics_regions.iter().any(|r| {
        r.obstacles_verified
            && (0..3).all(|i| from[i].min(to[i]) >= r.min[i] && from[i].max(to[i]) <= r.max[i])
    }) {
        return Err("geometry_coverage_unknown");
    }
    if world
        .raycast_segment_bounded(from, to, work)
        .map_err(|_| "geometry_query_budget")?
        .nearest
        .is_some()
    {
        return Err("world_blocked");
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
    let catalog = items.retail_combat()?;
    let equipped = kit.primary;
    let profile = retail_combat::RetailBulletProfile::from_equipped(
        catalog,
        kit,
        equipped,
        equipped.gear_slot,
        equipped.ammo_pool,
    )?;
    recoil::Profile::from_weapon(items.weapon_parameters(profile.item_id())?).map(|_| ())
}
impl<W: Write> Engine<W> {
    pub(crate) fn tick_bot_firearms(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        // Invalidate even when unsafe/disabled: a new life or equipment version
        // must never publish the previous weapon's ammunition in a join snapshot.
        for bot in self.bot_registry.bots.values_mut() {
            let kit = &bot.target.loadout;
            let current = kit
                .inventories
                .get(usize::from(kit.handle.inventory_index))
                .and_then(|i| i.slots.get(usize::from(kit.primary.gear_slot)))
                .copied()
                .flatten()
                .map(|item| {
                    (
                        bot.target.runtime.unwrap().generation,
                        kit.handle.update_id,
                        kit.handle.inventory_index,
                        kit.primary.gear_slot,
                        item,
                    )
                });
            if bot.health == 0
                || bot
                    .firearm
                    .equipment
                    .is_some_and(|old| Some(old) != current)
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
            for bot in self.bot_registry.bots.values_mut() {
                bot.firearm.target = None;
                bot.firearm.last_tick = None;
            }
            return Ok(Vec::new());
        }
        let items = self.item_catalog.clone();
        let Ok(catalog) = items.retail_combat() else {
            return Ok(Vec::new());
        };
        let leadership = self.plan_leadership(now)?;
        let mut actors = Vec::new();
        for a in self
            .active
            .iter()
            .chain(self.peers.values())
            .filter(|a| a.entered_game && a.ready_for_updates && a.mounted_seat.is_none())
        {
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
            let kit = a.current_inventory_handle.and_then(|h| {
                a.reservation.character.replication_loadout(
                    &self.item_catalog,
                    h.inventory_index,
                    a.wire_profile,
                )
            });
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
        let mut rays = policy.max_rays_per_tick;
        let mut work = policy.geometry_work_per_tick;
        let map = self.map_name().to_owned();
        let mut ids: Vec<_> = self.bot_registry.bots.keys().copied().collect();
        if !ids.is_empty() {
            let offset =
                ids.partition_point(|&id| id < self.bot_registry.firearm_cursor) % ids.len();
            ids.rotate_left(offset);
        }
        for id in ids {
            if rays == 0 || work == 0 {
                self.bot_registry.firearm_cursor = id;
                break;
            }
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
            if !safe(bot) {
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
            let profile = retail_combat::RetailBulletProfile::from_equipped(
                catalog,
                kit,
                equipped,
                equipped.gear_slot,
                equipped.ammo_pool,
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
            state.bind(
                (
                    me.life.generation,
                    kit.handle.update_id,
                    kit.handle.inventory_index,
                    equipped.gear_slot,
                    profile.item_id(),
                ),
                metadata,
                recoil_profile,
                self.bot_registry.bots[&id].yaw,
            );
            let dt = state
                .last_tick
                .map_or(0., |last| now.saturating_sub(last).as_secs_f64().min(0.25));
            state.last_tick = Some(now);
            let reload_changed = state.reload(now, schedule, metadata.magazine_capacity);
            let reload_receipt=reload_changed.then(||json!({"op":"bot_firearm_reload","timestamp_ms":now.as_millis() as u64,"object_index":id,"generation":me.life.generation,"magazine":state.magazine,"reserve":state.reserve,"reloading":state.reload_until.is_some()}));
            let from = muzzle(me.feet);
            let mut blocked = None;
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
            if state.target.is_none() && now >= state.next_perception {
                state.next_perception = now + Duration::from_millis(policy.perception_interval_ms);
                let mut candidates: Vec<_> = actors
                    .iter()
                    .filter(|a| {
                        a.team != me.team
                            && a.health > 0
                            && a.armor.is_some()
                            && policy.can_acquire(
                                from,
                                torso(a.feet),
                                f64::from(self.bot_registry.bots[&id].yaw) * std::f64::consts::TAU
                                    / 1024.,
                                metadata.max_range,
                            )
                    })
                    .collect();
                candidates.sort_by(|a, b| {
                    length(from, torso(a.feet))
                        .total_cmp(&length(from, torso(b.feet)))
                        .then(a.life.id.cmp(&b.life.id))
                });
                let mut selected = None;
                for candidate in candidates.into_iter().take(policy.max_candidates_per_bot) {
                    match visible(
                        self.collision_world.as_ref(),
                        &map,
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
                            blocked = Some(reason);
                            if deferred(reason) {
                                state.next_perception = now;
                                break;
                            }
                        }
                    }
                }
                if selected != state.target {
                    if selected.is_some() {
                        // Route movement can publish another facing while no
                        // combat target is held. Turn from that actual pose.
                        state.yaw = f64::from(self.bot_registry.bots[&id].yaw)
                            * std::f64::consts::TAU
                            / 1024.;
                    }
                    state.target = selected;
                    state.acquired = now;
                }
            }
            let mut shot = None;
            if state.target.is_none() {
                state.recoil.tick(recoil_profile, dt, false, 0., 0.);
            }
            if let Some(target) = state
                .target
                .as_ref()
                .and_then(|life| actors.iter().find(|a| a.life == *life && a.health > 0))
                .cloned()
            {
                let to = torso(target.feet);
                let moving = matches!(
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
                if policy.within_range(length(from, to), metadata.max_range)
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
                        self.collision_world.as_ref(),
                        &map,
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
                                    target.armor.unwrap(),
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
                || (state.target.is_some() && yaw != self.bot_registry.bots[&id].yaw);
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
            if let Some((target, hit, (calculation, amount, resolved), aimed_end)) = shot {
                receipts.push(json!({"op":"bot_firearm_shot","timestamp_ms":now.as_millis() as u64,"object_index":id,"generation":me.life.generation,
                    "target":target.life.id,"target_generation":target.life.generation,"weapon_item":profile.item_id(),"magazine":state.magazine,"reserve":state.reserve,
                    "origin":from,"destination":torso(target.feet),"aim_endpoint":aimed_end,"yaw_radians":state.yaw,"pitch_radians":state.pitch,"hit":hit,"amount_hp":if hit {amount} else {0.},"damage_calculation":calculation,
                    "recoil_pitch_degrees":state.recoil.pitch_degrees,"recoil_yaw_degrees":state.recoil.yaw_degrees,"spread_degrees":state.recoil.width_degrees(recoil_profile),"recoil_compensation_per_mille":policy.recoil_compensation_per_mille,
                    "distance_m":length(from,torso(target.feet)),"accuracy_per_mille":policy.accuracy_at(length(from,torso(target.feet)),metadata.max_range),
                    "health_before":target.health,"health_after":if hit {resolved.map_or(target.health,|d|d.health_quarter_points)} else {target.health}}));
                if hit && let Some(mut damage) = resolved {
                    let mut human_life = None;
                    if let Some(reservation) = &target.life.reservation {
                        let victim = self
                            .active
                            .iter()
                            .chain(self.peers.values())
                            .find(|a| a.reservation.id == *reservation)
                            .ok_or("stale_bot_firearm_target")?;
                        let (model, life) = self.plan_human_damage(victim, amount, damage, now)?;
                        damage = model;
                        human_life = Some(life);
                        receipts.push(json!({"op":"bot_firearm_lifecycle_planned","reservation_id":reservation,"life":format!("{life:?}"),"conditions":damage.conditions}));
                    }
                    if let Some(world) = self.active.as_ref().map(|a| a.reservation.world) {
                        death_notification = self.plan_firearm_death(
                            world,
                            id,
                            target.life.id,
                            profile.item_id(),
                            0,
                            damage,
                        )?;
                        if let Some(plan) = &death_notification {
                            receipts.push(json!({"op":"bot_firearm_death_planned","timestamp_ms":now.as_millis() as u64,"notification":plan.audit()}));
                        }
                    }
                    pending_damage = Some((target, damage, human_life));
                }
            }
            for receipt in receipts {
                self.record(receipt, wall)?;
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
            let packet = publish
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
            let bot = self.bot_registry.bots.get_mut(&id).unwrap();
            bot.firearm = state;
            if publish {
                bot.yaw = yaw;
            }
            if let Some(packet) = packet {
                self.broadcast_bot_packet(packet);
            }
            if let Some((target, damage, human_life)) = pending_damage {
                let packet = replication::CharacterDamageUpdate {
                    sequence: 2,
                    timestamp_ms: now.as_millis() as u32,
                    object_index: target.life.id,
                    damage,
                    forced_state: (damage.health_quarter_points == 0).then_some(
                        replication::CharacterState {
                            state: 7,
                            stance: 0,
                            user_data: [0; 3],
                        },
                    ),
                }
                .encode_with_profile(self.wire_profile)?;
                if let Some(reservation) = &target.life.reservation {
                    if let Some(plan) = death_notification.take() {
                        self.commit_firearm_death(plan, damage, now)?;
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
                    if damage.health_quarter_points == 0 {
                        messages.extend(events::bot_killed_human(self, reservation)?);
                    }
                } else if let Some(plan) = death_notification.take() {
                    self.commit_firearm_death(plan, damage, now)?;
                } else {
                    self.commit_bot_damage(target.life.id, target.life.generation, damage, now)?;
                }
                self.broadcast_bot_packet(packet);
                if let Some(actor) = actors.iter_mut().find(|a| a.life == target.life) {
                    actor.health = damage.health_quarter_points;
                }
            }
        }
        Ok(messages)
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

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
