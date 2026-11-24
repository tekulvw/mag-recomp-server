//! Perception layer above bot goals: bots notice enemies that cross their view
//! while busy, hear gunfire, explosions, footsteps and near misses, and may
//! interrupt an ordinary route to react before resuming it. The behaviour is
//! specified in docs/bot-perception.md; this is an original design.
use super::*;
use crate::bot_registry::WorldBot;
use crate::development_target::BotPhase;
use std::collections::{BTreeMap, BTreeSet};

/// Population-file policy (`perception`). Omitted fields use these defaults.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
    /// Forward view cone while moving, centred on the published facing.
    pub fov_degrees: f64,
    pub view_distance_m: f64,
    /// Enemies this close are noticed in any direction.
    pub proximity_m: f64,
    /// Delay between first noticing a threat and reacting to it; each bot
    /// life draws a fixed value in this range from its identity.
    pub reaction_min_ms: u64,
    pub reaction_max_ms: u64,
    pub sight_interval_ms: u64,
    /// How long a lost enemy's last known position keeps the bot reacting.
    pub sight_memory_ms: u64,
    /// How long a heard sound keeps the bot alert and facing its source.
    pub sound_alert_ms: u64,
    /// Upper bound on one continuous interruption of a route.
    pub max_hold_ms: u64,
    /// After the threat clears, sounds alone cannot interrupt again for this
    /// long; after a forced release nothing can.
    pub resume_cooldown_ms: u64,
    /// After the maximum hold, the bot walks on for this long before any
    /// further interruption.
    pub max_hold_cooldown_ms: u64,
    /// Sounds closer than this fraction of their hearing radius interrupt a
    /// moving bot; farther audible sounds only turn a stationary bot.
    pub interrupt_fraction: f64,
    pub hearing: Hearing,
    pub max_sight_rays_per_tick: usize,
    pub geometry_work_per_tick: usize,
    pub max_sounds_per_tick: usize,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            fov_degrees: 110.,
            view_distance_m: 25.,
            proximity_m: 4.,
            reaction_min_ms: 200,
            reaction_max_ms: 600,
            sight_interval_ms: 250,
            sight_memory_ms: 4000,
            sound_alert_ms: 2500,
            max_hold_ms: 4000,
            resume_cooldown_ms: 3000,
            max_hold_cooldown_ms: 12_000,
            interrupt_fraction: 0.35,
            hearing: Hearing::default(),
            max_sight_rays_per_tick: 32,
            geometry_work_per_tick: 32_768,
            max_sounds_per_tick: 64,
        }
    }
}
/// Hearing radius per sound class, in metres.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hearing {
    pub firearm_m: f64,
    pub sniper_m: f64,
    pub suppressed_m: f64,
    pub explosion_m: f64,
    pub sprint_m: f64,
    pub walk_m: f64,
    /// A bullet passing this close to a bot is a near miss.
    pub near_miss_m: f64,
}
impl Default for Hearing {
    fn default() -> Self {
        Self {
            firearm_m: 70.,
            sniper_m: 110.,
            suppressed_m: 18.,
            explosion_m: 90.,
            sprint_m: 15.,
            walk_m: 6.,
            near_miss_m: 3.,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let metres = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        let h = &self.hearing;
        if !metres(self.fov_degrees, 1., 360.)
            || !metres(self.view_distance_m, 1., 2048.)
            || !metres(self.proximity_m, 0., 50.)
            || self.reaction_min_ms > self.reaction_max_ms
            || self.reaction_max_ms > 10_000
            || !(50..=5000).contains(&self.sight_interval_ms)
            || !(100..=60_000).contains(&self.sight_memory_ms)
            || !(100..=60_000).contains(&self.sound_alert_ms)
            || !(500..=120_000).contains(&self.max_hold_ms)
            || self.resume_cooldown_ms > 60_000
            || self.max_hold_cooldown_ms > 120_000
            || !metres(self.interrupt_fraction, 0., 1.)
            || ![
                h.firearm_m,
                h.sniper_m,
                h.suppressed_m,
                h.explosion_m,
                h.sprint_m,
                h.walk_m,
            ]
            .into_iter()
            .all(|r| metres(r, 0., 1000.))
            || !metres(h.near_miss_m, 0., 20.)
            || !(1..=256).contains(&self.max_sight_rays_per_tick)
            || !(1024..=1_000_000).contains(&self.geometry_work_per_tick)
            || !(1..=1024).contains(&self.max_sounds_per_tick)
        {
            return Err("invalid_bot_perception_policy");
        }
        Ok(())
    }
    fn radius(&self, kind: SoundKind) -> f64 {
        let h = &self.hearing;
        match kind {
            SoundKind::Firearm => h.firearm_m,
            SoundKind::Sniper => h.sniper_m,
            SoundKind::Suppressed => h.suppressed_m,
            SoundKind::Explosion => h.explosion_m,
            SoundKind::Sprint => h.sprint_m,
            SoundKind::Walk => h.walk_m,
        }
    }
    /// Fixed per-life reaction delay inside the configured range.
    pub(crate) fn reaction(&self, id: u16, generation: u32) -> Duration {
        let span = self.reaction_max_ms - self.reaction_min_ms;
        let mut x =
            (u64::from(id) << 32 | u64::from(generation)).wrapping_mul(0x2545_f491_4f6c_dd1d);
        x ^= x >> 29;
        Duration::from_millis(self.reaction_min_ms + x % (span + 1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SoundKind {
    Walk,
    Sprint,
    Suppressed,
    Firearm,
    Sniper,
    Explosion,
}
/// One audible event. A shot also carries its bullet path for near misses.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sound {
    pub kind: SoundKind,
    pub team: u8,
    pub source: u16,
    pub origin: [f64; 3],
    pub path_end: Option<[f64; 3]>,
}
/// Why a bot is reacting; later variants outrank earlier ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Cause {
    Heard(SoundKind),
    NearMiss,
    Sight,
}
/// What one bot perceived during one tick.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Observation {
    /// Ordinary route following that may be paused.
    pub interruptible: bool,
    /// Position of the enemy the bot's weapon currently holds.
    pub engaged: Option<(u16, [f64; 3])>,
    pub seen: Option<(u16, [f64; 3])>,
    pub heard: Option<Heard>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Heard {
    pub cause: Cause,
    pub point: [f64; 3],
    /// Close or dangerous enough to interrupt a route.
    pub urgent: bool,
}
#[derive(Clone, Copy, Debug)]
struct Threat {
    point: [f64; 3],
    enemy: Option<u16>,
    cause: Cause,
    react_at: Duration,
    last: Duration,
    linger: Duration,
    urgent: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transition {
    Interrupt(Cause),
    Resume(&'static str),
}
/// Per-life perception and arbitration state.
#[derive(Clone, Debug, Default)]
pub(crate) struct Mind {
    life: (u32, u32),
    threat: Option<Threat>,
    hold_since: Option<Duration>,
    cooldown_until: Duration,
    hard_cooldown: bool,
    next_sight: Duration,
    next_footstep: Duration,
}
impl Mind {
    pub(crate) fn held(&self) -> bool {
        self.hold_since.is_some()
    }
    /// Where the bot should face, once its reaction delay has passed.
    pub(crate) fn attention(&self, now: Duration) -> Option<[f64; 3]> {
        self.threat.filter(|t| now >= t.react_at).map(|t| t.point)
    }
    fn notice(
        &mut self,
        policy: &Policy,
        reaction: Duration,
        now: Duration,
        (cause, point, enemy, urgent): (Cause, [f64; 3], Option<u16>, bool),
    ) {
        let linger = Duration::from_millis(if cause == Cause::Sight {
            policy.sight_memory_ms
        } else {
            policy.sound_alert_ms
        });
        let quick = Duration::from_millis(policy.reaction_min_ms);
        match &mut self.threat {
            Some(t) => {
                // A fresher or stronger stimulus moves attention; a sound never
                // replaces an enemy that is still being tracked by sight.
                if cause >= t.cause || t.cause != Cause::Sight {
                    t.point = point;
                    t.enemy = enemy;
                    t.cause = t.cause.max(cause);
                    t.linger = t.linger.max(linger);
                }
                t.last = now;
                t.urgent |= urgent;
                if cause == Cause::NearMiss {
                    t.react_at = t.react_at.min(now + quick);
                }
            }
            None => {
                self.threat = Some(Threat {
                    point,
                    enemy,
                    cause,
                    react_at: now
                        + if cause == Cause::NearMiss {
                            quick
                        } else {
                            reaction
                        },
                    last: now,
                    linger,
                    urgent,
                })
            }
        }
    }
    /// Advance one bot by one tick. Pure apart from `self`, for tests.
    pub(crate) fn step(
        &mut self,
        policy: &Policy,
        reaction: Duration,
        now: Duration,
        obs: Observation,
    ) -> Option<Transition> {
        if let Some((enemy, point)) = obs.engaged {
            self.notice(
                policy,
                reaction,
                now,
                (Cause::Sight, point, Some(enemy), true),
            );
        }
        if let Some((enemy, point)) = obs.seen {
            self.notice(
                policy,
                reaction,
                now,
                (Cause::Sight, point, Some(enemy), true),
            );
        }
        if let Some(h) = obs.heard {
            self.notice(policy, reaction, now, (h.cause, h.point, None, h.urgent));
        }
        if self
            .threat
            .is_some_and(|t| now.saturating_sub(t.last) > t.linger)
        {
            self.threat = None;
        }
        match self.hold_since {
            None => {
                let wants = self
                    .threat
                    .is_some_and(|t| t.urgent && now >= t.react_at && obs.interruptible);
                let sight = self.threat.is_some_and(|t| t.cause == Cause::Sight);
                let allowed = now >= self.cooldown_until || (!self.hard_cooldown && sight);
                if wants && allowed {
                    self.hold_since = Some(now);
                    return Some(Transition::Interrupt(self.threat.unwrap().cause));
                }
                None
            }
            Some(since) => {
                // A forced release (the route needs the bot elsewhere, or the
                // hold ran out) must not be undone by the next stimulus.
                let (reason, hard, cooldown) = if !obs.interruptible {
                    ("route_unavailable", true, policy.resume_cooldown_ms)
                } else if self.threat.is_none() {
                    ("threat_cleared", false, policy.resume_cooldown_ms)
                } else if now.saturating_sub(since) >= Duration::from_millis(policy.max_hold_ms) {
                    ("max_hold", true, policy.max_hold_cooldown_ms)
                } else {
                    return None;
                };
                self.hold_since = None;
                self.hard_cooldown = hard;
                self.cooldown_until = now + Duration::from_millis(cooldown);
                if reason == "max_hold" {
                    self.threat = None;
                }
                Some(Transition::Resume(reason))
            }
        }
    }
}

#[derive(Clone, Default)]
struct Ear {
    life: u32,
    weapon: Option<(u8, u8, bool)>,
    ammo: u8,
    stride: Option<([f64; 3], Duration)>,
    next_footstep: Duration,
    detonations: [Option<u32>; 4],
}
#[derive(Default, Clone, Copy, Serialize)]
struct Stats {
    interrupts: u64,
    resumes: u64,
    sounds: u64,
    near_misses: u64,
    sight_rays: u64,
    deferred_checks: u64,
    busy_us: u64,
    max_tick_us: u64,
    /// Slowest tick since the previous summary.
    window_max_us: u64,
    slow_ticks_over_2ms: u64,
}
/// Engine-owned perception world. Entries are keyed by bot index and reset
/// whenever that index starts another allocation or life.
#[derive(Default)]
pub(crate) struct World {
    override_enabled: Option<bool>,
    minds: BTreeMap<u16, Mind>,
    pending: Vec<Sound>,
    ears: BTreeMap<String, Ear>,
    cursor: u16,
    stats: Stats,
    last_summary: Option<Duration>,
}
impl World {
    pub(crate) fn held_ids(&self) -> BTreeSet<u16> {
        self.minds
            .iter()
            .filter(|(_, m)| m.held())
            .map(|(&id, _)| id)
            .collect()
    }
    pub(crate) fn attention(&self, id: u16, now: Duration) -> Option<[f64; 3]> {
        self.minds.get(&id).and_then(|m| m.attention(now))
    }
    #[cfg(test)]
    pub(crate) fn stats_rays(&self) -> u64 {
        self.stats.sight_rays
    }
    #[cfg(test)]
    pub(crate) fn noticed_ids(&self) -> Vec<u16> {
        self.minds
            .iter()
            .filter(|(_, m)| m.threat.is_some())
            .map(|(&id, _)| id)
            .collect()
    }
    /// A bot's accepted shot, heard by the other team on the next tick.
    pub(crate) fn push(&mut self, sound: Sound) {
        if self.minds.is_empty() && self.ears.is_empty() {
            return;
        }
        self.pending.push(sound);
    }
    fn clear(&mut self) {
        self.minds.clear();
        self.pending.clear();
        self.ears.clear();
    }
}

pub(crate) fn torso(feet: [f64; 3]) -> [f64; 3] {
    [
        feet[0],
        feet[1] + bot_physics::Body::default().height * 0.6,
        feet[2],
    ]
}
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}
fn yaw_radians(packed: u16) -> f64 {
    f64::from(packed & 1023) * std::f64::consts::TAU / 1024.
}
/// Horizontal angle between a facing and the direction to a point.
pub(crate) fn off_axis(from: [f64; 3], yaw: f64, to: [f64; 3]) -> f64 {
    let wanted = (to[0] - from[0]).atan2(to[2] - from[2]);
    ((wanted - yaw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI)
        .abs()
}
/// Could a bot facing `yaw` at `eye` notice `target` without hearing it?
pub(crate) fn in_view(policy: &Policy, eye: [f64; 3], yaw: f64, target: [f64; 3]) -> bool {
    let d = distance(eye, target);
    d <= policy.proximity_m
        || (d <= policy.view_distance_m
            && off_axis(eye, yaw, target) <= policy.fov_degrees.to_radians() * 0.5)
}
/// Shortest distance from `p` to the segment `a`-`b`.
pub(crate) fn segment_distance(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len2 = ab.iter().map(|v| v * v).sum::<f64>();
    let t = if len2 <= f64::EPSILON {
        0.
    } else {
        ((0..3).map(|i| (p[i] - a[i]) * ab[i]).sum::<f64>() / len2).clamp(0., 1.)
    };
    distance(p, [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t])
}
/// What a listener at `ear` makes of `sound`, if anything.
pub(crate) fn hear(policy: &Policy, ear: [f64; 3], sound: &Sound) -> Option<Heard> {
    if let Some(end) = sound.path_end
        && segment_distance(ear, sound.origin, end) <= policy.hearing.near_miss_m
        && distance(ear, sound.origin) > policy.hearing.near_miss_m
    {
        return Some(Heard {
            cause: Cause::NearMiss,
            point: sound.origin,
            urgent: true,
        });
    }
    let radius = policy.radius(sound.kind);
    let d = distance(ear, sound.origin);
    (d <= radius).then_some(Heard {
        cause: Cause::Heard(sound.kind),
        point: sound.origin,
        urgent: d <= radius * policy.interrupt_fraction,
    })
}
fn stronger(a: Option<Heard>, b: Heard, ear: [f64; 3]) -> Heard {
    match a {
        Some(a)
            if (a.cause, a.urgent) > (b.cause, b.urgent)
                || ((a.cause, a.urgent) == (b.cause, b.urgent)
                    && distance(ear, a.point) <= distance(ear, b.point)) =>
        {
            a
        }
        _ => b,
    }
}
/// Classify a firearm by its authored category and muzzle attachment slot.
fn firearm_kind(
    items: &item_catalog::ItemCatalog,
    kit: &replication::CharacterLoadout,
    gear_slot: u8,
) -> SoundKind {
    let Some(inventory) = kit.inventories.get(usize::from(kit.handle.inventory_index)) else {
        return SoundKind::Firearm;
    };
    // Muzzle attachments occupy a fixed slot per gear role (see retail_combat).
    let muzzle = match gear_slot {
        0 => inventory.slots[16],
        1 => inventory.slots[20],
        _ => None,
    };
    if muzzle.is_some() {
        return SoundKind::Suppressed;
    }
    let weapon = inventory
        .slots
        .get(usize::from(gear_slot))
        .copied()
        .flatten();
    if weapon.and_then(|id| items.retail_firearm_category(id)) == Some("kWepCategorySniperRifle") {
        SoundKind::Sniper
    } else {
        SoundKind::Firearm
    }
}
pub(crate) fn bot_firearm_kind(items: &item_catalog::ItemCatalog, bot: &WorldBot) -> SoundKind {
    let kit = &bot.target.loadout;
    firearm_kind(items, kit, kit.primary.gear_slot)
}
/// Drop passage geometry that keeps a bot near a passage out of reactions.
#[derive(Clone, Copy)]
pub(crate) struct DropGuard {
    pub entry_buffer_m: f64,
    pub queue_distance_m: f64,
}
/// Ordinary route following that a reaction may pause without disturbing
/// passages, queues, supply, recovery or objective holds. A route that is
/// about to reach a certified drop's admission queue is left alone too.
pub(crate) fn interruptible(bot: &WorldBot, guard: DropGuard) -> bool {
    let Some(r) = bot.target.runtime else {
        return false;
    };
    bot.health > 0
        && matches!(r.phase, BotPhase::Relocating)
        && bot.target.pending_move.is_some()
        && r.physics_motion
            .is_some_and(|m| m.grounded && m.slide_velocity == [0.; 2])
        && !r.navmesh_state.on_player
        && !r.navmesh_state.navigation_aborted
        && !bot.has_supply_action()
        && bot.resupply_seek.is_none()
        && bot.resupply_planning.is_none()
        && !bot.task.recovering
        && !bot.target.registry_ground_recovery()
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
        && bot.planned_drop(guard.entry_buffer_m).is_none_or(|p| {
            p.phase == crate::development_bot_movement::DropPhase::Approaching
                && p.route_distance_to_launch > guard.queue_distance_m + 2.
        })
}
/// Line of sight through static collision only; bodies do not hide a person.
fn clear_view(
    world: Option<&collision::CollisionWorld>,
    map: &str,
    from: [f64; 3],
    to: [f64; 3],
    rays: &mut usize,
    work: &mut usize,
) -> Result<bool, ()> {
    let Some(world) = world.filter(|w| w.metadata().map_name == map) else {
        return Ok(false);
    };
    if *rays == 0 || *work == 0 {
        return Err(());
    }
    *rays -= 1;
    if !world.verified_physics_regions().any(|r| {
        r.obstacles_verified
            && (0..3).all(|i| from[i].min(to[i]) >= r.min[i] && from[i].max(to[i]) <= r.max[i])
    }) {
        return Ok(false);
    }
    match world.raycast_segment_bounded(from, to, work) {
        Ok(query) => Ok(query.nearest.is_none()),
        Err(_) if *work == 0 => Err(()),
        Err(_) => Ok(false),
    }
}
#[derive(Clone, Copy)]
struct Body {
    id: u16,
    team: u8,
    feet: [f64; 3],
}

impl<W: Write> Engine<W> {
    /// Process-wide switch (`--bot-perception off|on`) over every population.
    pub fn set_bot_perception_override(&mut self, enabled: Option<bool>) {
        self.bot_perception.override_enabled = enabled;
    }
    fn bot_perception_policy(&self) -> Option<Policy> {
        let config = self.bot_registry.config.as_ref()?;
        let enabled = self
            .bot_perception
            .override_enabled
            .unwrap_or(config.perception.enabled);
        (enabled && config.firearms.enabled).then(|| config.perception.clone())
    }
    pub(crate) fn tick_bot_perception(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let started = Instant::now();
        let policy = self.bot_perception_policy().filter(|_| {
            self.match_started
                && self.match_terminal.is_none()
                && self.wire_profile.application_id() == 20231
                && self
                    .active
                    .iter()
                    .chain(self.peers.values())
                    .any(|a| a.ready_for_updates && a.initial_objects.is_some())
        });
        let Some(policy) = policy else {
            self.bot_perception.clear();
            return Ok(());
        };
        let Some(world_id) = self
            .active
            .as_ref()
            .or_else(|| self.peers.values().next())
            .map(|a| a.reservation.world)
        else {
            self.bot_perception.clear();
            return Ok(());
        };
        let guard = self
            .bot_registry
            .config
            .as_ref()
            .and_then(|c| c.objective_policy.as_ref())
            .map_or(
                DropGuard {
                    entry_buffer_m: 0.5,
                    queue_distance_m: 12.,
                },
                |p| DropGuard {
                    entry_buffer_m: p.drop_entry_buffer_m,
                    queue_distance_m: p.drop_queue_distance_m,
                },
            );
        let items = self.item_catalog.clone();
        let mut bodies = Vec::new();
        let mut sounds = std::mem::take(&mut self.bot_perception.pending);
        let mut present = BTreeSet::new();
        for a in self.active.iter().chain(self.peers.values()).filter(|a| {
            a.reservation.world == world_id
                && a.entered_game
                && a.ready_for_updates
                && !a.is_mounted()
        }) {
            let CharacterLife::Alive { generation, .. } = a.life else {
                continue;
            };
            let Some(sample) = a.tracking.sample else {
                continue;
            };
            let feet = [
                f64::from(sample.position.x),
                f64::from(sample.position.y),
                f64::from(sample.position.z),
            ];
            let team = a.team();
            let id = a.reservation.player;
            bodies.push(Body { id, team, feet });
            present.insert(a.reservation.id.clone());
            let ear = self
                .bot_perception
                .ears
                .entry(a.reservation.id.clone())
                .or_default();
            if ear.life != generation {
                *ear = Ear {
                    life: generation,
                    ..Default::default()
                };
            }
            // Gunfire: the owner's reported magazine count fell on one weapon.
            let equipped = if a.primary_equipped {
                a.observed_primary
            } else {
                a.observed_secondary
            };
            if let Some(item) = equipped {
                let key = (item.inventory_update_id, item.gear_slot, a.primary_equipped);
                if ear.weapon == Some(key) && item.ammo_count < ear.ammo && !item.reloading {
                    let kind = a.equipped_loadout().map_or(SoundKind::Firearm, |kit| {
                        firearm_kind(&items, &kit, item.gear_slot)
                    });
                    let origin = torso(feet);
                    let yaw = yaw_radians(a.owner_fields.yaw.unwrap_or(0));
                    let pitch = f64::from(a.owner_fields.pitch.unwrap_or(127))
                        * std::f64::consts::PI
                        / 255.
                        - std::f64::consts::FRAC_PI_2;
                    let reach = policy.view_distance_m.max(policy.hearing.firearm_m);
                    sounds.push(Sound {
                        kind,
                        team,
                        source: id,
                        origin,
                        path_end: a.owner_fields.yaw.map(|_| {
                            [
                                origin[0] + yaw.sin() * pitch.cos() * reach,
                                origin[1] + pitch.sin() * reach,
                                origin[2] + yaw.cos() * pitch.cos() * reach,
                            ]
                        }),
                    });
                }
                ear.weapon = Some(key);
                ear.ammo = item.ammo_count;
            }
            // Footsteps from the reported pace over at least a quarter second.
            match ear.stride {
                Some((from, at)) if now.saturating_sub(at) >= Duration::from_millis(250) => {
                    let speed = (feet[0] - from[0]).hypot(feet[2] - from[2])
                        / now.saturating_sub(at).as_secs_f64();
                    ear.stride = Some((feet, now));
                    let kind = if speed >= 5. {
                        Some(SoundKind::Sprint)
                    } else if speed >= 1. {
                        Some(SoundKind::Walk)
                    } else {
                        None
                    };
                    if let Some(kind) = kind
                        && now >= ear.next_footstep
                    {
                        ear.next_footstep = now + Duration::from_millis(500);
                        sounds.push(Sound {
                            kind,
                            team,
                            source: id,
                            origin: feet,
                            path_end: None,
                        });
                    }
                }
                Some(_) => {}
                None => ear.stride = Some((feet, now)),
            }
            // Thrown projectiles that reported their detonation point.
            for slot in 0..projectile::MAX_PROJECTILE_SLOTS as u8 {
                let Some(flight) = a.projectiles.get(slot) else {
                    continue;
                };
                let seen = &mut ear.detonations[usize::from(slot)];
                if flight.completed
                    && flight.terminal_reported
                    && *seen != Some(flight.generation)
                    && let Some(p) = flight.last_position
                {
                    *seen = Some(flight.generation);
                    sounds.push(Sound {
                        kind: SoundKind::Explosion,
                        team,
                        source: id,
                        origin: [p.x.into(), p.y.into(), p.z.into()],
                        path_end: None,
                    });
                }
            }
        }
        self.bot_perception
            .ears
            .retain(|id, _| present.contains(id));
        // Bot lives, stale state and footsteps of bots still walking.
        let held = self.bot_perception.held_ids();
        let mut live = BTreeSet::new();
        for (&id, bot) in &self.bot_registry.bots {
            let Some(r) = bot.target.runtime else {
                continue;
            };
            if bot.health == 0 || matches!(r.phase, BotPhase::Dead) {
                continue;
            }
            live.insert(id);
            let feet = r.physical_position();
            let team = bot.target.squad.team();
            bodies.push(Body { id, team, feet });
            let mind = self.bot_perception.minds.entry(id).or_default();
            if mind.life != (bot.allocation_generation, r.generation) {
                *mind = Mind {
                    life: (bot.allocation_generation, r.generation),
                    next_sight: now + Duration::from_millis(u64::from(id % 5) * 50),
                    ..Default::default()
                };
            }
            if matches!(r.phase, BotPhase::Relocating)
                && !held.contains(&id)
                && now >= mind.next_footstep
            {
                mind.next_footstep = now + Duration::from_millis(500);
                sounds.push(Sound {
                    kind: if r.navmesh_state.movement.sprinting() {
                        SoundKind::Sprint
                    } else {
                        SoundKind::Walk
                    },
                    team,
                    source: id,
                    origin: feet,
                    path_end: None,
                });
            }
        }
        self.bot_perception.minds.retain(|id, _| live.contains(id));
        // Hearing: the strongest audible enemy sound per bot this tick.
        sounds.truncate(policy.max_sounds_per_tick);
        let mut heard: BTreeMap<u16, Heard> = BTreeMap::new();
        for sound in &sounds {
            self.bot_perception.stats.sounds += 1;
            for body in bodies.iter().filter(|b| live.contains(&b.id)) {
                if body.team == sound.team || body.id == sound.source {
                    continue;
                }
                let ear = torso(body.feet);
                if let Some(h) = hear(&policy, ear, sound) {
                    if h.cause == Cause::NearMiss {
                        self.bot_perception.stats.near_misses += 1;
                    }
                    let best = stronger(heard.get(&body.id).copied(), h, ear);
                    heard.insert(body.id, best);
                }
            }
        }
        // Sight: moving bots scan their view cone; stationary bots rely on
        // their weapon's own acquisition and only add close all-round checks.
        let mut rays = policy.max_sight_rays_per_tick;
        let mut work = policy.geometry_work_per_tick;
        let map = self.map_name().to_owned();
        let mut seen = BTreeMap::new();
        let mut ids: Vec<_> = live.iter().copied().collect();
        if !ids.is_empty() {
            let offset = ids.partition_point(|&id| id < self.bot_perception.cursor) % ids.len();
            ids.rotate_left(offset);
        }
        for id in ids {
            let bot = &self.bot_registry.bots[&id];
            let mind = &self.bot_perception.minds[&id];
            if now < mind.next_sight || mind.held() {
                continue;
            }
            let moving = interruptible(bot, guard);
            if !moving && !crate::bot_firearms::safe(bot) {
                continue;
            }
            if rays == 0 || work == 0 {
                self.bot_perception.cursor = id;
                self.bot_perception.stats.deferred_checks += 1;
                break;
            }
            self.bot_perception.cursor = id.wrapping_add(1);
            let me = bot.target.runtime.unwrap().physical_position();
            let team = bot.target.squad.team();
            let eye = torso(me);
            let yaw = yaw_radians(bot.yaw);
            let mut candidates: Vec<_> = bodies
                .iter()
                .filter(|b| b.team != team)
                .map(|b| (distance(eye, torso(b.feet)), *b))
                .filter(|(d, b)| {
                    if moving {
                        in_view(&policy, eye, yaw, torso(b.feet))
                    } else {
                        *d <= policy.proximity_m
                    }
                })
                .collect();
            candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
            let mut deferred = false;
            for (_, candidate) in candidates.into_iter().take(2) {
                let before = rays;
                match clear_view(
                    self.collision_world.as_ref(),
                    &map,
                    eye,
                    torso(candidate.feet),
                    &mut rays,
                    &mut work,
                ) {
                    Ok(true) => {
                        seen.insert(id, (candidate.id, candidate.feet));
                        self.bot_perception.stats.sight_rays += (before - rays) as u64;
                        break;
                    }
                    Ok(false) => self.bot_perception.stats.sight_rays += (before - rays) as u64,
                    Err(()) => {
                        deferred = true;
                        break;
                    }
                }
            }
            if deferred {
                self.bot_perception.cursor = id;
                self.bot_perception.stats.deferred_checks += 1;
                break;
            }
            self.bot_perception.minds.get_mut(&id).unwrap().next_sight =
                now + Duration::from_millis(policy.sight_interval_ms);
        }
        // Arbitration and transitions.
        let mut receipts = Vec::new();
        for &id in &live {
            let bot = &self.bot_registry.bots[&id];
            let generation = bot.target.runtime.unwrap().generation;
            let engaged = bot.firearm.target_index().and_then(|enemy| {
                bodies
                    .iter()
                    .find(|b| b.id == enemy)
                    .map(|b| (enemy, torso(b.feet)))
            });
            let obs = Observation {
                interruptible: interruptible(bot, guard),
                engaged,
                seen: seen.get(&id).map(|&(enemy, feet)| (enemy, torso(feet))),
                heard: heard.get(&id).copied(),
            };
            let reaction = policy.reaction(id, generation);
            let mind = self.bot_perception.minds.get_mut(&id).unwrap();
            let since = mind.hold_since;
            let threat = mind.threat;
            match mind.step(&policy, reaction, now, obs) {
                Some(Transition::Interrupt(cause)) => {
                    self.bot_perception.stats.interrupts += 1;
                    let t = mind.threat.unwrap();
                    receipts.push(json!({"op":"bot_perception_interrupt","timestamp_ms":now.as_millis() as u64,
                        "object_index":id,"generation":generation,"cause":cause,"source":t.point,"enemy":t.enemy,
                        "objective":bot.task.objective,"operator":bot.task.operator}));
                }
                Some(Transition::Resume(reason)) => {
                    self.bot_perception.stats.resumes += 1;
                    receipts.push(
                        json!({"op":"bot_perception_resume","timestamp_ms":now.as_millis() as u64,
                        "object_index":id,"generation":generation,"reason":reason,
                        "held_ms":since.map_or(0, |s| now.saturating_sub(s).as_millis() as u64),
                        "last_cause":threat.map(|t| t.cause)}),
                    );
                }
                None => {}
            }
        }
        let busy = started.elapsed().as_micros() as u64;
        let stats = &mut self.bot_perception.stats;
        stats.busy_us += busy;
        stats.max_tick_us = stats.max_tick_us.max(busy);
        stats.window_max_us = stats.window_max_us.max(busy);
        stats.slow_ticks_over_2ms += u64::from(busy > 2000);
        if self
            .bot_perception
            .last_summary
            .is_none_or(|last| now.saturating_sub(last) >= Duration::from_secs(10))
        {
            self.bot_perception.last_summary = Some(now);
            let held = self.bot_perception.held_ids().len();
            receipts.push(
                json!({"op":"bot_perception_summary","timestamp_ms":now.as_millis() as u64,
                "held":held,"bots":live.len(),"totals":self.bot_perception.stats}),
            );
            self.bot_perception.stats.window_max_us = 0;
        }
        for receipt in receipts {
            self.record(receipt, wall)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    fn shot(kind: SoundKind, origin: [f64; 3], toward: Option<[f64; 3]>) -> Sound {
        Sound {
            kind,
            team: 1,
            source: 9,
            origin,
            path_end: toward,
        }
    }

    #[test]
    fn default_policy_validates_and_rejects_inverted_reaction() {
        Policy::default().validate().unwrap();
        let bad = Policy {
            reaction_min_ms: 700,
            reaction_max_ms: 600,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        let parsed: Policy = serde_json::from_str(r#"{"enabled":false}"#).unwrap();
        assert!(!parsed.enabled && parsed.fov_degrees == 110.);
        assert!(serde_json::from_str::<Policy>(r#"{"unknown":1}"#).is_err());
    }

    #[test]
    fn reaction_delay_is_deterministic_and_bounded() {
        let p = Policy::default();
        let delays: BTreeSet<_> = (0..64).map(|id| p.reaction(id, 1)).collect();
        assert!(delays.iter().all(|d| (ms(200)..=ms(600)).contains(d)));
        assert!(delays.len() > 16, "jitter spreads bots out");
        assert_eq!(p.reaction(7, 3), p.reaction(7, 3));
    }

    #[test]
    fn forward_cone_sees_ahead_and_proximity_covers_behind() {
        let p = Policy::default();
        let eye = [0., 1., 0.];
        // Facing +Z (yaw 0).
        assert!(in_view(&p, eye, 0., [10., 1., 20.]));
        assert!(!in_view(&p, eye, 0., [0., 1., -10.]), "behind");
        assert!(!in_view(&p, eye, 0., [20., 1., 0.]), "90 degrees off axis");
        assert!(in_view(&p, eye, 0., [0., 1., -3.]), "very close behind");
        assert!(!in_view(&p, eye, 0., [0., 1., 26.]), "beyond view distance");
    }

    #[test]
    fn suppressed_fire_is_heard_at_a_shorter_range() {
        let p = Policy::default();
        let ear = [0., 1., 0.];
        let at = [30., 1., 0.];
        assert!(hear(&p, ear, &shot(SoundKind::Firearm, at, None)).is_some());
        assert!(hear(&p, ear, &shot(SoundKind::Suppressed, at, None)).is_none());
        let near = hear(&p, ear, &shot(SoundKind::Suppressed, [5., 1., 0.], None)).unwrap();
        assert!(near.urgent);
        assert!(
            !hear(&p, ear, &shot(SoundKind::Suppressed, [12., 1., 0.], None))
                .unwrap()
                .urgent
        );
        let close = hear(&p, ear, &shot(SoundKind::Firearm, [20., 1., 0.], None)).unwrap();
        assert!(close.urgent, "20 m is inside the firearm interrupt range");
        let distant = hear(&p, ear, &shot(SoundKind::Firearm, at, None)).unwrap();
        assert!(
            !distant.urgent,
            "distant gunfire only turns a stationary bot"
        );
        assert!(hear(&p, ear, &shot(SoundKind::Sniper, [100., 1., 0.], None)).is_some());
        assert!(hear(&p, ear, &shot(SoundKind::Walk, [7., 1., 0.], None)).is_none());
        assert!(hear(&p, ear, &shot(SoundKind::Sprint, [7., 1., 0.], None)).is_some());
    }

    #[test]
    fn a_bullet_path_past_the_bot_is_a_near_miss_even_when_quiet() {
        let p = Policy::default();
        let ear = [0., 1., 0.];
        let suppressed = shot(SoundKind::Suppressed, [0., 1., 40.], Some([1., 1., -40.]));
        let h = hear(&p, ear, &suppressed).unwrap();
        assert_eq!(h.cause, Cause::NearMiss);
        assert!(h.urgent);
        let wide = shot(SoundKind::Suppressed, [0., 1., 40.], Some([20., 1., -40.]));
        assert!(hear(&p, ear, &wide).is_none());
    }

    fn sight(enemy: [f64; 3]) -> Observation {
        Observation {
            interruptible: true,
            seen: Some((5, enemy)),
            ..Default::default()
        }
    }

    #[test]
    fn enemy_in_view_interrupts_after_the_reaction_delay_and_resumes_after_memory() {
        let p = Policy {
            max_hold_ms: 20_000,
            ..Default::default()
        };
        let mut mind = Mind::default();
        let reaction = ms(300);
        let walking = Observation {
            interruptible: true,
            ..Default::default()
        };
        assert_eq!(mind.step(&p, reaction, ms(0), sight([0., 1., 10.])), None);
        assert_eq!(mind.step(&p, reaction, ms(250), walking), None);
        assert_eq!(
            mind.step(&p, reaction, ms(300), walking),
            Some(Transition::Interrupt(Cause::Sight))
        );
        assert_eq!(mind.attention(ms(300)), Some([0., 1., 10.]));
        // Engaged by the weapon: hold continues.
        let engaged = Observation {
            interruptible: true,
            engaged: Some((5, [0., 1., 12.])),
            ..Default::default()
        };
        for t in (350..3000).step_by(50) {
            assert_eq!(mind.step(&p, reaction, ms(t), engaged), None);
        }
        // Enemy gone: last known position is watched for the memory period.
        for t in (3000..7000).step_by(50) {
            assert_eq!(mind.step(&p, reaction, ms(t), walking), None, "{t}");
            assert!(mind.held());
            assert_eq!(mind.attention(ms(t)), Some([0., 1., 12.]));
        }
        assert_eq!(
            mind.step(&p, reaction, ms(7000), walking),
            Some(Transition::Resume("threat_cleared"))
        );
        assert!(!mind.held() && mind.attention(ms(7000)).is_none());
    }

    #[test]
    fn flickering_sight_does_not_oscillate_the_hold() {
        let p = Policy {
            max_hold_ms: 20_000,
            ..Default::default()
        };
        let mut mind = Mind::default();
        let mut transitions = Vec::new();
        for t in (0..6000).step_by(50) {
            let obs = if (t / 250) % 2 == 0 {
                sight([3., 1., 20.])
            } else {
                Observation {
                    interruptible: true,
                    ..Default::default()
                }
            };
            transitions.extend(mind.step(&p, ms(200), ms(t), obs));
        }
        assert_eq!(transitions, vec![Transition::Interrupt(Cause::Sight)]);
    }

    #[test]
    fn repeated_distant_gunfire_after_resuming_waits_for_the_cooldown() {
        let p = Policy::default();
        let mut mind = Mind::default();
        let gunfire = Observation {
            interruptible: true,
            heard: Some(Heard {
                cause: Cause::Heard(SoundKind::Firearm),
                point: [0., 1., -20.],
                urgent: true,
            }),
            ..Default::default()
        };
        let quiet = Observation {
            interruptible: true,
            ..Default::default()
        };
        let mut log = Vec::new();
        mind.step(&p, ms(200), ms(0), gunfire);
        for t in (50..=2700).step_by(50) {
            log.extend(mind.step(&p, ms(200), ms(t), quiet));
        }
        assert_eq!(
            log,
            vec![
                Transition::Interrupt(Cause::Heard(SoundKind::Firearm)),
                Transition::Resume("threat_cleared")
            ]
        );
        // Gunfire again inside the cooldown keeps the bot walking.
        let resumed = 2550;
        for t in (resumed + 50..resumed + 3000).step_by(50) {
            assert_eq!(mind.step(&p, ms(200), ms(t), gunfire), None, "{t}");
        }
        assert!(
            mind.attention(ms(resumed + 3000)).is_some(),
            "still faces it"
        );
        // A visible enemy is still allowed to interrupt during the cooldown.
        let mut sighted = Mind {
            cooldown_until: ms(10_000),
            ..Default::default()
        };
        sighted.step(&p, ms(200), ms(0), sight([0., 1., 5.]));
        assert_eq!(
            sighted.step(&p, ms(200), ms(200), sight([0., 1., 5.])),
            Some(Transition::Interrupt(Cause::Sight))
        );
    }

    #[test]
    fn non_urgent_sounds_turn_but_never_interrupt() {
        let p = Policy::default();
        let mut mind = Mind::default();
        let faint = Observation {
            interruptible: true,
            heard: Some(Heard {
                cause: Cause::Heard(SoundKind::Firearm),
                point: [0., 1., -60.],
                urgent: false,
            }),
            ..Default::default()
        };
        for t in (0..2000).step_by(50) {
            assert_eq!(mind.step(&p, ms(200), ms(t), faint), None);
        }
        assert_eq!(mind.attention(ms(2000)), Some([0., 1., -60.]));
    }

    #[test]
    fn hold_is_capped_and_then_cools_down_hard() {
        let p = Policy {
            max_hold_ms: 1000,
            max_hold_cooldown_ms: 3000,
            ..Default::default()
        };
        let mut mind = Mind::default();
        let mut log = Vec::new();
        for t in (0..4100).step_by(50) {
            log.extend(mind.step(&p, ms(0), ms(t), sight([0., 1., 5.])));
        }
        assert_eq!(
            log,
            vec![
                Transition::Interrupt(Cause::Sight),
                Transition::Resume("max_hold"),
                Transition::Interrupt(Cause::Sight),
            ]
        );
    }

    #[test]
    fn leaving_ordinary_route_following_releases_the_hold_at_once() {
        let p = Policy::default();
        let mut mind = Mind::default();
        mind.step(&p, ms(0), ms(0), sight([0., 1., 5.]));
        assert!(mind.held());
        let queued = Observation {
            interruptible: false,
            seen: Some((5, [0., 1., 5.])),
            ..Default::default()
        };
        assert_eq!(
            mind.step(&p, ms(0), ms(50), queued),
            Some(Transition::Resume("route_unavailable"))
        );
        assert_eq!(mind.step(&p, ms(0), ms(100), queued), None);
    }
}
