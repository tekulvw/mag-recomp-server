//! Round-owned leadership commands over the normal Engine transaction.
use crate::{
    active_abilities::{Ability, CommandRequest, CommandRole, Rules, Side},
    active_ability_effects::{Effect, EffectRules, Work},
    active_ability_sensor::{ObjectiveCursor, Surveillance},
    active_ability_state::{
        NativeAbilityData, NativeAbilityScope, NativeAbilityUpdate, NativeActivationOutcome,
        NativeCooldowns, NativeDetectionUpdate, native_ability_mask, native_command_availability,
    },
    domination_structural_effects::InstantaneousBlast,
    domination_structural_world::{AreaDamageState, BlastCollision},
    *,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Boundary {
    pub min_x: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_z: f32,
}
impl Boundary {
    fn contains(&self, p: [f32; 3]) -> bool {
        p.into_iter().all(f32::is_finite)
            && p[0] >= self.min_x
            && p[0] <= self.max_x
            && p[2] >= self.min_z
            && p[2] <= self.max_z
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AntiAir {
    pub object_index: u16,
    pub position: [f32; 3],
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorObjective {
    pub object_index: u16,
    pub position: [f32; 3],
    pub category: u32,
    pub subtype: u32,
    pub team: u8,
    pub platoons: Vec<u16>,
}
#[derive(Clone, Debug)]
pub struct Setup {
    pub rules: Rules,
    pub map_name: String,
    pub boundary: Boundary,
    pub vertical_bounds: [f32; 2],
    pub aaa: Vec<AntiAir>,
    pub objectives: Vec<SensorObjective>,
    pub effects: Vec<(Side, u8, Ability, EffectRules)>,
}
impl Setup {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.rules.validate()?;
        NativeCooldowns::new(&self.rules)?;
        if self.map_name.is_empty()
            || ![
                self.boundary.min_x,
                self.boundary.min_z,
                self.boundary.max_x,
                self.boundary.max_z,
            ]
            .into_iter()
            .all(f32::is_finite)
            || self.boundary.min_x >= self.boundary.max_x
            || self.boundary.min_z >= self.boundary.max_z
            || !self.vertical_bounds.into_iter().all(f32::is_finite)
            || self.vertical_bounds[0] >= self.vertical_bounds[1]
            || self
                .aaa
                .windows(2)
                .any(|r| r[0].object_index >= r[1].object_index)
            || self
                .aaa
                .iter()
                .any(|r| !r.position.into_iter().all(f32::is_finite))
            || self.objectives.iter().any(|r| {
                !r.position.into_iter().all(f32::is_finite)
                    || r.team > 1
                    || r.platoons.iter().any(|p| *p >= 8)
            })
        {
            return Err("invalid_active_ability_map_binding");
        }
        let mut seen = BTreeSet::new();
        for &(side, faction, ability, rules) in &self.effects {
            if faction > 2 || !seen.insert((side == Side::Defender, faction, ability.code())) {
                return Err("invalid_active_ability_effect_binding");
            }
            Effect::new(ability, side, 0, true, [0.; 3], rules)?;
        }
        if seen.len() != 12 {
            return Err("incomplete_active_ability_effects");
        }
        Ok(())
    }
    pub(crate) fn runtime(self: Arc<Self>) -> Result<Runtime, &'static str> {
        self.validate()?;
        Ok(Runtime {
            cooldowns: NativeCooldowns::new(&self.rules)?,
            setup: self,
            units: BTreeMap::new(),
            effects: BTreeMap::new(),
            aaa_intercepts: BTreeMap::new(),
            surveillance: BTreeMap::new(),
            histories: BTreeMap::new(),
            detected: BTreeMap::new(),
            last_tick: None,
            version: 1,
            publications: BTreeMap::new(),
            area: AreaDamageState::default(),
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
struct Unit {
    data: [Option<NativeAbilityData>; 6],
    active: u16,
}
#[derive(Clone, Debug)]
struct Running {
    effect: Effect,
    invoker: u16,
    world: u32,
    reservation: String,
    platoon: u16,
}
#[derive(Clone, Debug, Default)]
struct Published {
    platoons: BTreeMap<u16, u16>,
    squads: BTreeMap<u16, Unit>,
    detections: BTreeMap<u16, bool>,
}
#[derive(Clone, Debug)]
struct Motion {
    identity: String,
    position: [f32; 3],
    at: Duration,
    sequence: Option<u32>,
    speed: f32,
}
#[derive(Clone, Debug)]
struct Detection {
    identity: String,
    remaining: f32,
    detected: bool,
}
#[derive(Clone, Debug)]
struct ObjectiveSurveillance {
    sensor: Option<Surveillance>,
    cursor: ObjectiveCursor,
}
#[derive(Clone)]
pub(crate) struct Runtime {
    pub(crate) setup: Arc<Setup>,
    cooldowns: NativeCooldowns,
    units: BTreeMap<u16, Unit>,
    effects: BTreeMap<(u16, u16), Running>,
    aaa_intercepts: BTreeMap<u16, u32>,
    surveillance: BTreeMap<u16, ObjectiveSurveillance>,
    histories: BTreeMap<u16, Motion>,
    detected: BTreeMap<u16, Detection>,
    last_tick: Option<Duration>,
    version: u64,
    publications: BTreeMap<String, Published>,
    area: AreaDamageState,
}
impl Runtime {
    pub(crate) fn reset(&mut self) {
        *self = Arc::clone(&self.setup)
            .runtime()
            .expect("validated ability setup");
    }
    fn mark(&mut self, target: &Target) {
        let entry = self.detected.entry(target.index).or_insert(Detection {
            identity: target.identity.clone(),
            remaining: 0.,
            detected: false,
        });
        if entry.identity != target.identity || !entry.detected {
            self.version += 1;
        }
        *entry = Detection {
            identity: target.identity.clone(),
            remaining: 5.,
            detected: true,
        };
    }
    pub(crate) fn available(&self, mode: u16, side: Side, role: CommandRole) -> u16 {
        native_command_availability(&self.setup.rules, mode, side, role)
            .unwrap_or(&[])
            .iter()
            .filter_map(|a| native_ability_mask(*a).ok())
            .fold(0, |mask, bit| mask | bit)
    }
}
#[derive(Clone)]
struct Target {
    index: u16,
    squad: u16,
    team: u8,
    identity: String,
    position: [f32; 3],
    sample_at: Duration,
    sample_sequence: Option<u32>,
    speed: f32,
}

impl<W: Write> Engine<W> {
    pub fn with_active_abilities(mut self, setup: Setup) -> Result<Self, &'static str> {
        if self.active.is_some() || !self.peers.is_empty() || self.match_profiles.is_some() {
            return Err("active_abilities_require_idle_fixed_world");
        }
        if self.map_name() != setup.map_name || !matches!(self.match_profile().game_type, 1 | 2) {
            return Err("active_ability_match_mismatch");
        }
        if !self
            .leadership
            .as_ref()
            .is_some_and(|l| l.supports_match_profiles())
        {
            return Err("active_abilities_require_native_leadership");
        }
        self.active_abilities = Some(Arc::new(setup).runtime()?);
        Ok(self)
    }
    pub(crate) fn plan_command_ability(
        &self,
        actor: &Active,
        request: CommandRequest,
        now: Duration,
    ) -> Result<(Option<Runtime>, Value, Option<&'static str>), &'static str> {
        let denied = |reason| {
            Ok((
                None,
                json!({"type":"command_ability","accepted":false,"reason":reason,"ability":request.ability,"squad_index":request.squad_index}),
                Some(reason),
            ))
        };
        let Some(mut next) = self.active_abilities.clone() else {
            return denied("command_ability_execution_pending");
        };
        if !self.match_started || self.match_terminal.is_some() {
            return denied("match_not_playing");
        }
        if !actor.entered_game || !actor.ready_for_updates || actor.initial_objects.is_none() {
            return denied("command_actor_not_created");
        }
        if self.command_leadership_role(actor) != Some(CommandRole::SquadLeader) {
            return denied("command_requires_native_squad_leader");
        }
        if matches!(actor.life, CharacterLife::AwaitingRespawn { .. }) {
            return denied("command_character_dead");
        }
        let squad = actor.squad_catalog.squad_index();
        let platoon = squad / 4;
        if request.squad_index != squad {
            return denied("command_wrong_squad");
        }
        let side = if actor.team() == 0 {
            Side::Attacker
        } else {
            Side::Defender
        };
        let mask = native_ability_mask(request.ability).unwrap_or(0);
        if mask == 0
            || next.available(
                self.match_profile().game_type as u16,
                side,
                CommandRole::SquadLeader,
            ) & mask
                == 0
        {
            return denied("command_unavailable_in_mode");
        }
        if !matches!(request.ability, Ability::ClusterBomb | Ability::Sensor) {
            return denied("command_effect_not_implemented");
        }
        if (request.ability == Ability::ClusterBomb || side == Side::Attacker)
            && !next.setup.boundary.contains(request.position)
        {
            return denied("command_target_outside_level");
        }
        let now_ms = now.as_millis() as u32;
        if next.cooldowns.remaining_ms(
            NativeAbilityScope::Squad(squad),
            request.ability,
            now_ms,
            0.,
        )? != 0
        {
            return denied("command_cooldown");
        }
        let enabled = self
            .domination_gameplay
            .as_ref()
            .map_or(u16::MAX, |w| w.state.ability_enabled[usize::from(platoon)])
            & mask
            != 0;
        let mut aaa = None;
        let mut success = enabled;
        if side == Side::Attacker {
            let mut best = f32::MAX;
            let mut nearest = None;
            for candidate in &next.setup.aaa {
                let distance = distance_squared(candidate.position, request.position);
                if distance < best {
                    best = distance;
                    nearest = Some(candidate);
                }
            }
            let Some(nearest) = nearest else {
                return denied("command_anti_air_unavailable");
            };
            let last = *next.aaa_intercepts.get(&nearest.object_index).unwrap_or(&0);
            if now_ms.wrapping_sub(last) < next.setup.rules.command_global_cooldown_ms as u32 {
                return denied("command_anti_air_cooldown");
            }
            let state = self
                .objective_world
                .as_ref()
                .and_then(|w| w.state(nearest.object_index))
                .ok_or("active_ability_anti_air_state_missing")?;
            let active = !matches!(state as u8, 5 | 7 | 9);
            success &= !active;
            aaa = Some(nearest.object_index);
        } else if !enabled {
            return denied("command_disabled_by_platoon");
        }
        let rules = next
            .setup
            .effects
            .iter()
            .find(|(s, f, a, _)| {
                *s == side && *f == actor.reservation.character.faction && *a == request.ability
            })
            .map(|r| r.3)
            .ok_or("active_ability_effect_rules_missing")?;
        let key = (squad, request.ability.code());
        let effect = match next.effects.get(&key) {
            Some(old) => old.effect.rearm(
                request.ability,
                side,
                now_ms,
                success,
                request.position,
                rules,
            )?,
            None => Effect::new(
                request.ability,
                side,
                now_ms,
                success,
                request.position,
                rules,
            )?,
        };
        if let Some(sensor) = effect.start_work() {
            for objective in &next.setup.objectives {
                if objective.platoons.contains(&platoon) {
                    next.surveillance
                        .entry(objective.object_index)
                        .or_insert(ObjectiveSurveillance {
                            sensor: None,
                            cursor: ObjectiveCursor::default(),
                        })
                        .sensor = Some(sensor);
                }
            }
        }
        next.effects.insert(
            key,
            Running {
                effect,
                invoker: actor.reservation.player,
                world: actor.reservation.world,
                reservation: actor.reservation.id.clone(),
                platoon,
            },
        );
        let unit = next.units.entry(squad).or_default();
        unit.data[usize::from(request.ability.code() - 1)] = Some(NativeAbilityData {
            position: request.position,
            squad_index: squad,
            activate_time_ms: now_ms,
            enabled_at_start: success,
            target_id: request.target_id,
        });
        unit.active |= mask;
        next.cooldowns.record(
            NativeAbilityScope::Squad(squad),
            request.ability,
            now_ms,
            if success {
                NativeActivationOutcome::Activated
            } else {
                NativeActivationOutcome::AcceptedFailure
            },
        )?;
        if !success && let Some(index) = aaa {
            next.aaa_intercepts.insert(index, now_ms);
        }
        next.version += 1;
        Ok((
            Some(next),
            json!({"type":"command_ability","accepted":true,"enabled_at_start":success,"ability":request.ability,"squad_index":squad,"activate_time_ms":now_ms,"anti_air":aaa}),
            None,
        ))
    }
    fn ability_targets(&self, runtime: &mut Runtime, now: Duration) -> Vec<Target> {
        let mut targets = Vec::new();
        for actor in self.active.iter().chain(self.peers.values()) {
            let Some(sample) = actor.tracking.sample.as_ref() else {
                continue;
            };
            if !actor.entered_game {
                continue;
            }
            let p = sample.position;
            let position = [p.x, p.y, p.z];
            targets.push(Target {
                index: actor.reservation.player,
                squad: actor.squad_catalog.squad_index(),
                team: actor.team(),
                identity: detection_identity(actor),
                position,
                sample_at: Duration::from_millis(sample.received_at_ms),
                sample_sequence: sample.client_sequence,
                speed: -1.,
            });
        }
        for bot in self.bot_combat_snapshots() {
            targets.push(Target {
                index: bot.object_index,
                squad: (0..32)
                    .find(|s| {
                        self.bot_registry
                            .members(*s)
                            .any(|id| id == bot.object_index)
                    })
                    .unwrap_or(u16::MAX),
                team: bot.team,
                identity: format!("bot:{}:{}", bot.object_index, bot.generation),
                position: [bot.position.x, bot.position.y, bot.position.z],
                sample_at: now,
                sample_sequence: None,
                speed: -1.,
            });
        }
        for target in &mut targets {
            if let Some(previous) = runtime.histories.get(&target.index)
                && previous.identity == target.identity
            {
                target.speed = previous.speed;
                if target.sample_at == previous.at && target.sample_sequence == previous.sequence {
                    continue;
                }
                if target.sample_at > previous.at {
                    target.speed = distance_squared(target.position, previous.position).sqrt()
                        / target.sample_at.saturating_sub(previous.at).as_secs_f32();
                }
            }
            runtime.histories.insert(
                target.index,
                Motion {
                    identity: target.identity.clone(),
                    position: target.position,
                    at: target.sample_at,
                    sequence: target.sample_sequence,
                    speed: target.speed,
                },
            );
        }
        targets
    }
    /// Invoked after native ground/character/wave work and before objective updates.
    /// The surrounding world callback records or restores this whole candidate.
    pub(crate) fn stage_active_ability_update(
        &mut self,
        now: Duration,
        native: &mut BTreeMap<u32, acquisition_character_effects::NativeState>,
        collision: BlastCollision<'_>,
        ground_policy: Option<&acquisition_ground_damage::Policy>,
    ) -> Result<domination_structural_world::DamageCommit, &'static str> {
        let mut result = domination_structural_world::DamageCommit {
            audit: Vec::new(),
            messages: Vec::new(),
            changed_ground: BTreeSet::new(),
            helo_doses: Vec::new(),
        };
        let Some(mut runtime) = self.active_abilities.take() else {
            return Ok(result);
        };
        let outcome = (|| {
            let dt = runtime
                .last_tick
                .map_or(0., |t| now.saturating_sub(t).as_secs_f32());
            runtime.last_tick = Some(now);
            let targets = self.ability_targets(&mut runtime, now);
            for (index, mark) in &mut runtime.detected {
                if mark.detected {
                    mark.remaining -= dt;
                    if mark.remaining <= 0.
                        || !targets
                            .iter()
                            .any(|t| t.index == *index && t.identity == mark.identity)
                    {
                        mark.detected = false;
                        mark.remaining = 0.;
                        runtime.version += 1;
                    }
                }
            }
            if !self.match_started || self.match_terminal.is_some() {
                return Ok(());
            }
            let keys: Vec<_> = runtime.effects.keys().copied().collect();
            for key in keys {
                let mut running = runtime.effects[&key].clone();
                if running.effect.is_complete() {
                    continue;
                }
                let (next, work) = running.effect.prepare_tick(now.as_millis() as u32)?;
                running.effect = next;
                match work {
                    Work::ClusterSample(sample) => {
                        // A cached native leader ID must never credit a replacement occupant.
                        let owned = self.active.iter().chain(self.peers.values()).any(|a| {
                            a.reservation.player == running.invoker
                                && a.reservation.id == running.reservation
                        });
                        let attacker = if owned {
                            0x0380_0000 | u32::from(running.invoker)
                        } else {
                            u32::MAX
                        };
                        let b = sample.blast;
                        let team = u8::from(running.platoon >= 4);
                        let world = running.world;
                        let previous_team = self
                            .active_ability_damage_team
                            .replace((attacker, world, team));
                        let strike = (|| {
                            if let Some(work) = self.collect_air_strike_blast(
                                attacker,
                                sample.xz,
                                runtime.setup.vertical_bounds,
                                InstantaneousBlast {
                                    origins: Vec::new(),
                                    damage: b.damage,
                                    inner_radius: b.inner_radius,
                                    outer_radius: b.outer_radius,
                                    material_penetration: 0.,
                                },
                                collision,
                            )? {
                                let mut commit = self.stage_domination_damage_with_ground(
                                    &mut runtime.area,
                                    native,
                                    now,
                                    &[work],
                                    ground_policy,
                                )?;
                                for dose in &mut commit.helo_doses {
                                    dose.source_team = Some(team);
                                }
                                result.audit.extend(commit.audit);
                                result.messages.extend(commit.messages);
                                result.changed_ground.extend(commit.changed_ground);
                                result.helo_doses.extend(commit.helo_doses);
                            }
                            Ok(())
                        })();
                        self.active_ability_damage_team = previous_team;
                        strike?;
                    }
                    Work::ScanSquad(scan) => {
                        for target in targets.iter().filter(|t| t.squad == scan.slot) {
                            if scan
                                .detector
                                .detects(scan.origin, target.position, target.speed)
                            {
                                runtime.mark(target);
                            }
                        }
                    }
                    Work::ClearSurveillance => {
                        for objective in &runtime.setup.objectives {
                            if objective.platoons.contains(&running.platoon)
                                && let Some(s) =
                                    runtime.surveillance.get_mut(&objective.object_index)
                            {
                                s.sensor = None;
                            }
                        }
                    }
                    Work::Idle | Work::Complete => {}
                }
                if running.effect.is_complete() {
                    runtime.units.entry(key.0).or_default().active &=
                        !native_ability_mask(Ability::from_code(key.1)?)?;
                    runtime.version += 1;
                }
                runtime.effects.insert(key, running);
            }
            // Objective type12 runs after squad/platoon managers. Preserve each cursor.
            let objectives = runtime.setup.objectives.clone();
            for objective in objectives {
                let Some(state) = runtime.surveillance.get_mut(&objective.object_index) else {
                    continue;
                };
                let Some(Surveillance { detector }) = state.sensor else {
                    continue;
                };
                if !active_ability_sensor::objective_supports_surveillance(
                    objective.category,
                    objective.subtype,
                ) {
                    continue;
                }
                let allocated = self
                    .active
                    .iter()
                    .chain(self.peers.values())
                    .filter(|a| a.initial_objects.is_some())
                    .count()
                    + self.bot_combat_snapshots().len();
                let slots = state.cursor.take(
                    u16::try_from(allocated)
                        .map_err(|_| "active_ability_character_pool_overflow")?,
                );
                for target in targets
                    .iter()
                    .filter(|t| slots.contains(&t.index) && t.team != objective.team)
                {
                    if detector.detects(objective.position, target.position, target.speed) {
                        runtime.mark(target);
                    }
                }
            }
            result.audit.push(json!({"op":"active_ability_update","timestamp_ms":now.as_millis() as u32,"active_effects":runtime.effects.values().filter(|e|!e.effect.is_complete()).count(),"version":runtime.version}));
            Ok(())
        })();
        self.active_abilities = Some(runtime);
        outcome?;
        Ok(result)
    }
}
fn detection_identity(actor: &Active) -> String {
    let generation = match actor.life {
        CharacterLife::AwaitingFirstSpawn => 0,
        CharacterLife::Alive { generation, .. }
        | CharacterLife::Incapacitated { generation, .. }
        | CharacterLife::AwaitingRespawn { generation } => generation,
    };
    format!("human:{}:{}", actor.reservation.id, generation)
}
fn distance_squared(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = std::array::from_fn::<_, 3, _>(|i| a[i] - b[i]);
    (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]
}

impl<W: Write> Engine<W> {
    /// Publish changed objects after the owning transaction. Each recipient's
    /// retained state advances only when all of its bounded packets are queued.
    pub(crate) fn queue_active_ability_state(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let Some(runtime) = self.active_abilities.as_ref() else {
            return Ok(());
        };
        let mut next = runtime.clone();
        let actors: Vec<_> = self.active.iter().chain(self.peers.values()).collect();
        next.publications
            .retain(|id, _| actors.iter().any(|a| a.reservation.id == *id));
        let squads: BTreeSet<_> = actors
            .iter()
            .filter(|a| a.initial_objects.is_some())
            .map(|a| a.squad_catalog.squad_index())
            .collect();
        let enabled = self
            .domination_gameplay
            .as_ref()
            .map_or([u16::MAX; 8], |w| w.state.ability_enabled);
        let mut publications = Vec::new();
        for actor in actors
            .iter()
            .filter(|a| a.entered_game && a.ready_for_updates && a.initial_objects.is_some())
        {
            if actor.replication_backlogged {
                continue;
            }
            let mut published = next
                .publications
                .get(&actor.reservation.id)
                .cloned()
                .unwrap_or_default();
            let mut platoon_updates = Vec::new();
            let mut squad_updates = Vec::new();
            let mut detections = Vec::new();
            for platoon in 0..8 {
                let value = enabled[usize::from(platoon)];
                if published.platoons.get(&platoon) == Some(&value) {
                    continue;
                }
                let side = if platoon < 4 {
                    Side::Attacker
                } else {
                    Side::Defender
                };
                let mut update = NativeAbilityUpdate::new(NativeAbilityScope::Platoon(platoon));
                if !published.platoons.contains_key(&platoon) {
                    update.active = Some(true);
                    update.owner_index = Some(0x105);
                    update.leader_index = Some(u16::MAX);
                    update.active_mask = Some(0);
                    update.available_mask = Some(next.available(
                        self.match_profile().game_type as u16,
                        side,
                        CommandRole::PlatoonLeader,
                    ));
                    update.shadow_modifier_percent = Some(0);
                }
                update.enabled_mask = Some(value);
                platoon_updates.push(update);
                published.platoons.insert(platoon, value);
            }
            for &squad in &squads {
                let unit = next.units.get(&squad).cloned().unwrap_or_default();
                if published.squads.get(&squad) == Some(&unit) {
                    continue;
                }
                let side = if squad < 16 {
                    Side::Attacker
                } else {
                    Side::Defender
                };
                let mut update = NativeAbilityUpdate::new(NativeAbilityScope::Squad(squad));
                update.data = unit.data;
                update.active_mask = Some(unit.active);
                if !published.squads.contains_key(&squad) {
                    update.available_mask = Some(next.available(
                        self.match_profile().game_type as u16,
                        side,
                        CommandRole::SquadLeader,
                    ));
                    update.shadow_modifier_percent = Some(0);
                }
                squad_updates.push(update);
                published.squads.insert(squad, unit);
            }
            for (&index, detection) in &next.detected {
                let current = actors.iter().any(|a| {
                    a.reservation.player == index && detection_identity(a) == detection.identity
                }) || self.bot_combat_snapshots().iter().any(|b| {
                    b.object_index == index
                        && format!("bot:{}:{}", b.object_index, b.generation) == detection.identity
                });
                let value = detection.detected && current;
                if published.detections.get(&index) == Some(&value) {
                    continue;
                }
                detections.push(NativeDetectionUpdate {
                    character_index: index,
                    globally_detected: Some(value),
                    undetected_via_satellite_jammer: Some(false),
                });
                published.detections.insert(index, value);
            }
            let mut packets = Vec::new();
            for chunk in platoon_updates.chunks(8) {
                packets.push(NativeAbilityUpdate::encode_batch(
                    chunk,
                    2,
                    now.as_millis() as u32,
                    self.wire_profile,
                )?);
            }
            // Six full177-bit data fields per object still fit eight objects
            // under the1397-byte bridge budget.
            for chunk in squad_updates.chunks(8) {
                packets.push(NativeAbilityUpdate::encode_batch(
                    chunk,
                    2,
                    now.as_millis() as u32,
                    self.wire_profile,
                )?);
            }
            for chunk in detections.chunks(48) {
                packets.push(NativeDetectionUpdate::encode_batch(
                    chunk,
                    2,
                    now.as_millis() as u32,
                    self.wire_profile,
                )?);
            }
            if packets.is_empty()
                || actor.pending_shared.len() + packets.len() > multiplayer::MAX_SHARED_QUEUE
            {
                continue;
            }
            next.publications
                .insert(actor.reservation.id.clone(), published);
            publications.push((actor.reservation.id.clone(), packets));
        }
        if publications.is_empty() {
            return Ok(());
        }
        self.record(json!({"op":"active_ability_state_published","timestamp_ms":now.as_millis() as u32,"version":next.version,
            "recipients":publications.iter().map(|(id,p)|json!({"reservation_id":id,"packets":p.iter().map(|b|protocol::encode_hex(b)).collect::<Vec<_>>()})).collect::<Vec<_>>()}),wall)?;
        self.active_abilities = Some(next);
        for (id, packets) in publications {
            if let Some(actor) = self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .find(|a| a.reservation.id == id)
            {
                for packet in packets {
                    actor.queue_shared(packet);
                }
            }
        }
        Ok(())
    }
}
