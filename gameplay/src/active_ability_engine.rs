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
    pub damage_triggers: Vec<crate::structure_damage_world::Rule>,
    pub precision_targets: Vec<crate::active_ability_precision::PrecisionTarget>,
}
impl Setup {
    pub fn has_strategic_effects(&self) -> bool {
        self.rules
            .strategic
            .iter()
            .any(|rule| rule.spawn_interval_modifier.is_some())
    }
    pub fn has_platoon_effects(&self) -> bool {
        self.effects
            .iter()
            .any(|(_, _, ability, _)| !matches!(ability, Ability::ClusterBomb | Ability::Sensor))
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.rules.validate()?;
        crate::structure_damage_world::World::new(&self.damage_triggers)?;
        NativeCooldowns::new(&self.rules)?;
        if self.has_strategic_effects() {
            crate::strategic_abilities::StrategicState::new(&self.rules)?;
        }

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
        let mut targets = BTreeSet::new();
        if self
            .precision_targets
            .iter()
            .any(|target| target.team > 1 || !targets.insert(target.object_index))
        {
            return Err("invalid_precision_target_binding");
        }
        let mut seen = BTreeSet::new();
        for &(side, faction, ability, rules) in &self.effects {
            if faction > 2 || !seen.insert((side == Side::Defender, faction, ability.code())) {
                return Err("invalid_active_ability_effect_binding");
            }
            Effect::new(ability, side, 0, true, [0.; 3], rules)?;
        }
        let full = self
            .effects
            .iter()
            .any(|(_, _, ability, _)| !matches!(ability, Ability::ClusterBomb | Ability::Sensor));
        for side in [false, true] {
            for faction in 0..3 {
                for ability in [
                    Ability::ClusterBomb,
                    Ability::Sensor,
                    Ability::PrecisionStrike,
                    Ability::GuidedArtillery,
                    Ability::GasBombardment,
                    Ability::StrafingRun,
                ] {
                    if !full && !matches!(ability, Ability::ClusterBomb | Ability::Sensor) {
                        continue;
                    }
                    if side && ability == Ability::PrecisionStrike {
                        continue;
                    }
                    if !seen.contains(&(side, faction, ability.code())) {
                        return Err("incomplete_active_ability_effects");
                    }
                }
            }
        }
        if seen.len() != if full { 33 } else { 12 } {
            return Err("incomplete_active_ability_effects");
        }
        Ok(())
    }
    pub(crate) fn runtime(self: Arc<Self>) -> Result<Runtime, &'static str> {
        self.validate()?;
        let structures = crate::structure_damage_world::World::new(&self.damage_triggers)?;
        Ok(Runtime {
            structures,
            cooldowns: NativeCooldowns::new(&self.rules)?,
            strategic: if self.has_strategic_effects() {
                Some(crate::strategic_abilities::StrategicState::new(
                    &self.rules,
                )?)
            } else {
                None
            },
            setup: self,
            units: BTreeMap::new(),
            effects: BTreeMap::new(),
            aaa_intercepts: BTreeMap::new(),
            command_enabled: None,
            surveillance: BTreeMap::new(),
            histories: BTreeMap::new(),
            detected: BTreeMap::new(),
            last_tick: None,
            version: 1,
            publications: BTreeMap::new(),
            area: AreaDamageState::default(),
            gas: crate::active_ability_gas_world::GasWorld::default(),
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
    companies: BTreeMap<u8, PublishedCompany>,
    platoons: BTreeMap<u16, PublishedPlatoon>,
    squads: BTreeMap<u16, Unit>,
    detections: BTreeMap<u16, (String, bool)>,
}
#[derive(Clone, Debug, PartialEq)]
struct PublishedCompany {
    leader: u16,
    snapshot: crate::strategic_abilities::CompanySnapshot,
}
#[derive(Clone, Debug, PartialEq)]
struct PublishedPlatoon {
    enabled: u16,
    leader: u16,
    unit: Unit,
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
    structures: crate::structure_damage_world::World,
    cooldowns: NativeCooldowns,
    pub(crate) strategic: Option<crate::strategic_abilities::StrategicState>,
    units: BTreeMap<NativeAbilityScope, Unit>,
    effects: BTreeMap<(NativeAbilityScope, u16), Running>,
    aaa_intercepts: BTreeMap<u16, u32>,
    command_enabled: Option<[u16; 8]>,
    surveillance: BTreeMap<u16, ObjectiveSurveillance>,
    histories: BTreeMap<u16, Motion>,
    detected: BTreeMap<u16, Detection>,
    last_tick: Option<Duration>,
    pub(crate) version: u64,
    publications: BTreeMap<String, Published>,
    area: AreaDamageState,
    gas: crate::active_ability_gas_world::GasWorld,
}
impl Runtime {
    pub(crate) fn reset(&mut self) {
        *self = Arc::clone(&self.setup)
            .runtime()
            .expect("validated ability setup");
    }
    fn mark(&mut self, target: &Target) {
        self.mark_character(target.index, target.identity.clone());
    }
    pub(crate) fn mark_character(&mut self, index: u16, identity: String) {
        let entry = self.detected.entry(index).or_insert(Detection {
            identity: identity.clone(),
            remaining: 0.,
            detected: false,
        });
        if entry.identity != identity || !entry.detected {
            self.version += 1;
        }
        *entry = Detection {
            identity,
            remaining: 5.,
            detected: true,
        };
    }
    pub(crate) fn refresh_command_team(&mut self, team: u8) -> Result<(), &'static str> {
        self.cooldowns.refresh_team(team)?;
        for (scope, unit) in &mut self.units {
            let owner = match scope {
                NativeAbilityScope::Squad(index) => index / 16,
                NativeAbilityScope::Platoon(index) => index / 4,
            };
            if owner == u16::from(team) {
                for data in unit.data.iter_mut().flatten() {
                    data.activate_time_ms = 0;
                }
            }
        }
        self.version += 1;
        Ok(())
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
        let supports_platoon = self
            .leadership
            .as_ref()
            .is_some_and(|leadership| leadership.supports_platoon_election());
        let has_platoon_effects = setup.has_platoon_effects();
        if has_platoon_effects && !supports_platoon {
            return Err("active_abilities_require_native_platoon_leadership");
        }
        if !has_platoon_effects && supports_platoon {
            return Err("platoon_leadership_requires_complete_active_abilities");
        }
        if self.match_profile().game_type == 2
            && self
                .leadership
                .as_ref()
                .is_some_and(|l| l.supports_oic_election())
            && !setup.has_strategic_effects()
        {
            return Err("oic_leadership_requires_strategic_effects");
        }
        if !setup.damage_triggers.is_empty() && self.match_profile().game_type != 2 {
            return Err("structure_damage_requires_native_domination_objective_lifecycle");
        }
        self.active_abilities = Some(Arc::new(setup).runtime()?);
        Ok(self)
    }
    /// Called inside the Acquisition world candidate by qualified tactical-asset hooks.
    pub(crate) fn set_acquisition_command_enabled(
        &mut self,
        platoon: u16,
        mask: u16,
        enabled: bool,
    ) -> Result<(), &'static str> {
        if platoon >= 8 || mask == 0 || mask & !0x3f != 0 {
            return Err("invalid_acquisition_command_mask");
        }
        let Some(runtime) = self.active_abilities.as_mut() else {
            return Ok(());
        };
        let row = &mut runtime.command_enabled.get_or_insert([u16::MAX; 8])[usize::from(platoon)];
        let next = if enabled { *row | mask } else { *row & !mask };
        if *row != next {
            *row = next;
            runtime.version += 1;
        }
        Ok(())
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
        let Some(role) = self.command_leadership_role(actor) else {
            return denied("command_requires_native_squad_leader");
        };
        if matches!(actor.life, CharacterLife::AwaitingRespawn { .. }) {
            return denied("command_character_dead");
        }
        if next
            .strategic
            .as_ref()
            .is_some_and(|state| state.commands_locked(actor.team()))
        {
            return denied("command_company_jammed");
        }
        let squad = actor.squad_catalog.squad_index();
        let platoon = squad / 4;
        let scope = match role {
            CommandRole::SquadLeader => {
                if request.squad_index != squad {
                    return denied("command_wrong_squad");
                }
                NativeAbilityScope::Squad(squad)
            }
            CommandRole::PlatoonLeader => {
                // The retail client addresses a platoon via its canonical first squad.
                if request.squad_index != platoon * 4 {
                    return denied("command_wrong_platoon");
                }
                NativeAbilityScope::Platoon(platoon)
            }
        };
        let side = if actor.team() == 0 {
            Side::Attacker
        } else {
            Side::Defender
        };
        let mask = native_ability_mask(request.ability).unwrap_or(0);
        if mask == 0
            || next.available(self.match_profile().game_type as u16, side, role) & mask == 0
        {
            return denied("command_unavailable_in_mode");
        }
        // Every admitted command must have verified authored effect rules.
        let rules = next
            .setup
            .effects
            .iter()
            .find(|(s, f, a, _)| {
                (*s == side || request.ability == Ability::PrecisionStrike)
                    && *f == actor.reservation.character.faction
                    && *a == request.ability
            })
            .map(|r| r.3);
        let Some(rules) = rules else {
            return denied("command_effect_not_implemented");
        };
        if (request.ability != Ability::Sensor || side == Side::Attacker)
            && !next.setup.boundary.contains(request.position)
        {
            return denied("command_target_outside_level");
        }
        if request.ability == Ability::PrecisionStrike
            && !self.precision_target_allowed(request.target_id, actor.team())?
        {
            return denied("command_invalid_precision_target");
        }
        let now_ms = now.as_millis() as u32;
        if next
            .cooldowns
            .remaining_ms(scope, request.ability, now_ms, 0.)?
            != 0
        {
            return denied("command_cooldown");
        }
        let enabled = next.command_enabled.unwrap_or_else(|| {
            self.domination_gameplay
                .as_ref()
                .map_or([u16::MAX; 8], |w| w.state.ability_enabled)
        })[usize::from(platoon)]
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
        let key = (scope, request.ability.code());
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
        let unit = next.units.entry(scope).or_default();
        unit.data[usize::from(request.ability.code() - 1)] = Some(NativeAbilityData {
            position: request.position,
            squad_index: request.squad_index,
            activate_time_ms: now_ms,
            enabled_at_start: success,
            target_id: request.target_id,
        });
        unit.active |= mask;
        next.cooldowns.record(
            scope,
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
    fn command_source(&self, running: &Running) -> (u32, Option<crate::structure_damage::Source>) {
        let actor = self.active.iter().chain(self.peers.values()).find(|a| {
            a.reservation.player == running.invoker
                && a.reservation.id == running.reservation
                && a.reservation.world == running.world
        });
        let Some(actor) = actor else {
            return (u32::MAX, None);
        };
        let generation = match actor.life {
            CharacterLife::AwaitingFirstSpawn => 0,
            CharacterLife::Alive { generation, .. }
            | CharacterLife::Incapacitated { generation, .. }
            | CharacterLife::AwaitingRespawn { generation } => generation,
        };
        (
            0x0380_0000 | u32::from(running.invoker),
            Some(crate::structure_damage::Source {
                player: running.invoker,
                generation,
            }),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn stage_command_blast(
        &mut self,
        runtime: &mut Runtime,
        running: &Running,
        native: &mut BTreeMap<u32, acquisition_character_effects::NativeState>,
        now: Duration,
        work: domination_structural_world::DamageWork,
        collision: BlastCollision<'_>,
        ground_policy: Option<&acquisition_ground_damage::Policy>,
        objectives: Option<&mut objective_control::ObjectiveWorld>,
    ) -> Result<domination_structural_world::DamageCommit, &'static str> {
        use domination_structural_world::{BlastSource, DamageWork};
        let DamageWork::Blast { source, blast, .. } = &work else {
            return Err("command_requires_blast_work");
        };
        let raw_damage = blast.damage;
        let structure_hits = if runtime.structures.has_targets()
            && matches!(
                source,
                BlastSource::AirStrike | BlastSource::CommandBomblet(_)
            ) {
            if (collision.filter_rows[19] | collision.filter_rows[24]) & (1 << 6) != 0 {
                return Err("structure_phantom_raycast_filter_unqualified");
            }
            runtime.structures.collect_with_scale(
                blast,
                collision.filter_rows[6],
                collision.limits,
                |distance| source.scale(blast, distance),
            )?
        } else {
            Vec::new()
        };
        let attacker = self.command_source(running).0;
        let team = u8::from(running.platoon >= 4);
        let previous = self
            .active_ability_damage_team
            .replace((attacker, running.world, team));
        let result = (|| {
            let mut commit = self.stage_domination_damage_with_ground(
                &mut runtime.area,
                native,
                now,
                &[work],
                ground_policy,
            )?;
            if !structure_hits.is_empty() {
                commit.audit.extend(self.stage_structure_strike(
                    &mut runtime.structures,
                    objectives,
                    &structure_hits,
                    raw_damage,
                    attacker,
                    now,
                )?);
            }
            for dose in &mut commit.helo_doses {
                dose.source_team = Some(team);
            }
            Ok(commit)
        })();
        self.active_ability_damage_team = previous;
        result
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
    #[cfg(test)]
    pub(crate) fn stage_active_ability_update(
        &mut self,
        now: Duration,
        native: &mut BTreeMap<u32, acquisition_character_effects::NativeState>,
        collision: BlastCollision<'_>,
        ground_policy: Option<&acquisition_ground_damage::Policy>,
    ) -> Result<domination_structural_world::DamageCommit, &'static str> {
        let mut objectives = self
            .active_abilities
            .as_ref()
            .filter(|r| r.structures.has_targets() || !r.setup.precision_targets.is_empty())
            .and_then(|_| self.objective_world.clone());
        let has_world = objectives.is_some();
        let result = self.stage_active_ability_update_in_world(
            now,
            native,
            collision,
            ground_policy,
            objectives.as_mut(),
        );
        if result.is_ok() && has_world {
            self.objective_world = objectives;
        }
        result
    }

    pub(crate) fn stage_active_ability_update_in_world(
        &mut self,
        now: Duration,
        native: &mut BTreeMap<u32, acquisition_character_effects::NativeState>,
        collision: BlastCollision<'_>,
        ground_policy: Option<&acquisition_ground_damage::Policy>,
        mut objectives: Option<&mut objective_control::ObjectiveWorld>,
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
        let active_effects = |runtime: &Runtime| {
            runtime
                .effects
                .values()
                .filter(|e| !e.effect.is_complete())
                .count()
        };
        let before = (runtime.version, active_effects(&runtime));
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
                        || self.current_detection_identity(*index).as_deref()
                            != Some(mark.identity.as_str())
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
            let (gas_audit, gas_messages) = runtime.gas.stage(self, native, now, collision)?;
            result.audit.extend(gas_audit);
            result.messages.extend(gas_messages);
            let keys: Vec<_> = runtime.effects.keys().copied().collect();
            for key in keys {
                let mut running = runtime.effects[&key].clone();
                if running.effect.is_complete() {
                    continue;
                }
                let unit = runtime
                    .units
                    .get(&key.0)
                    .ok_or("active_ability_unit_missing")?;
                let data =
                    unit.data[usize::from(key.1 - 1)].ok_or("active_ability_data_missing")?;
                let origin = self
                    .spawn_catalog
                    .as_ref()
                    .and_then(|catalog| {
                        catalog.selected_squad(data.squad_index).and_then(|squad| {
                            squad
                                .available
                                .iter()
                                .filter_map(|index| catalog.point(*index))
                                .find(|point| {
                                    matches!(point.kind, spawn_catalog::SpawnKind::Alternate)
                                })
                        })
                    })
                    .map(|point| [point.position.x, point.position.y, point.position.z]);
                let (next, work) = running
                    .effect
                    .prepare_tick_with_platoon(now.as_millis() as u32, origin)?;
                running.effect = next;
                match work {
                    Work::ClusterSample(sample) => {
                        let attacker = self.command_source(&running).0;
                        if let Some(work) = self.collect_air_strike_blast(
                            attacker,
                            sample.xz,
                            runtime.setup.vertical_bounds,
                            InstantaneousBlast {
                                origins: Vec::new(),
                                damage: sample.blast.damage,
                                inner_radius: sample.blast.inner_radius,
                                outer_radius: sample.blast.outer_radius,
                                material_penetration: 0.,
                            },
                            collision,
                        )? {
                            let mut commit = self.stage_command_blast(
                                &mut runtime,
                                &running,
                                native,
                                now,
                                work,
                                collision,
                                ground_policy,
                                objectives.as_deref_mut(),
                            )?;
                            result.audit.append(&mut commit.audit);
                            result.messages.append(&mut commit.messages);
                            result.changed_ground.append(&mut commit.changed_ground);
                            result.helo_doses.append(&mut commit.helo_doses);
                        }
                    }
                    Work::PrecisionStrike => {
                        let world = objectives
                            .as_deref_mut()
                            .ok_or("precision_objective_world_required")?;
                        let source = self.command_source(&running).1;
                        let changed = self.stage_precision_strike(
                            world,
                            &runtime.setup.precision_targets,
                            data.target_id,
                            source,
                            running.invoker,
                            running.world,
                        )?;
                        result.audit.push(
                            json!({"op":"precision_strike_target","target_id":data.target_id,
                            "source":source.map(|s|s.player),"staged":changed}),
                        );
                    }
                    Work::GuidedArtillery { origin, radius } => {
                        let attacker = self.command_source(&running).0;
                        let work = self
                            .collect_guided_artillery_blast(attacker, origin, radius, collision)?;
                        let mut commit = self.stage_command_blast(
                            &mut runtime,
                            &running,
                            native,
                            now,
                            work,
                            collision,
                            ground_policy,
                            objectives.as_deref_mut(),
                        )?;
                        result.audit.append(&mut commit.audit);
                        result.messages.append(&mut commit.messages);
                        result.changed_ground.append(&mut commit.changed_ground);
                        result.helo_doses.append(&mut commit.helo_doses);
                    }
                    Work::GasBombardment { volume, bomblet } => {
                        let attacker = self.command_source(&running).0;
                        if let Some(volume) = volume {
                            runtime.gas.register(
                                volume,
                                crate::active_ability_gas_world::GasSource {
                                    allocation: None,
                                    attacker,
                                    world: running.world,
                                    team: u8::from(running.platoon >= 4),
                                    reservation: running.reservation.clone(),
                                },
                                now,
                            )?;
                        }
                        if let Some(sample) = bomblet
                            && let Some(work) = self.collect_command_bomblet(
                                attacker,
                                sample,
                                runtime.setup.vertical_bounds,
                                true,
                                collision,
                            )?
                        {
                            let mut commit = self.stage_command_blast(
                                &mut runtime,
                                &running,
                                native,
                                now,
                                work,
                                collision,
                                ground_policy,
                                objectives.as_deref_mut(),
                            )?;
                            result.audit.append(&mut commit.audit);
                            result.messages.append(&mut commit.messages);
                            result.changed_ground.append(&mut commit.changed_ground);
                            result.helo_doses.append(&mut commit.helo_doses);
                        }
                    }
                    Work::StrafingRun(sample) => {
                        let attacker = self.command_source(&running).0;
                        if let Some(work) = self.collect_command_bomblet(
                            attacker,
                            sample,
                            [-1000., 1000.],
                            false,
                            collision,
                        )? {
                            let mut commit = self.stage_command_blast(
                                &mut runtime,
                                &running,
                                native,
                                now,
                                work,
                                collision,
                                ground_policy,
                                objectives.as_deref_mut(),
                            )?;
                            result.audit.append(&mut commit.audit);
                            result.messages.append(&mut commit.messages);
                            result.changed_ground.append(&mut commit.changed_ground);
                            result.helo_doses.append(&mut commit.helo_doses);
                        }
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
            self.stage_company_strategics(&mut runtime, now)?;
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
            // An unchanged ability state is not a gameplay result. Callers
            // treat any audit row as a change, so report only a transition.
            let after = (runtime.version, active_effects(&runtime));
            if after != before {
                result.audit.push(json!({"op":"active_ability_update","timestamp_ms":now.as_millis() as u32,"active_effects":after.1,"version":after.0}));
            }
            Ok(())
        })();
        self.active_abilities = Some(runtime);
        outcome?;
        Ok(result)
    }
}
pub(crate) fn detection_identity(actor: &Active) -> String {
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
            .chain(next.units.keys().filter_map(|scope| match scope {
                NativeAbilityScope::Squad(index) => Some(*index),
                NativeAbilityScope::Platoon(_) => None,
            }))
            .collect();
        let enabled = next.command_enabled.unwrap_or_else(|| {
            self.domination_gameplay
                .as_ref()
                .map_or([u16::MAX; 8], |w| w.state.ability_enabled)
        });
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
            let mut company_updates = Vec::new();
            if self.match_profile().game_type == 2
                && let Some(strategic) = &next.strategic
            {
                for team in 0..2 {
                    let value = PublishedCompany {
                        leader: self.company_leader_index(team),
                        snapshot: strategic.company(team).ok_or("invalid_strategic_team")?,
                    };
                    if published.companies.get(&team) == Some(&value) {
                        continue;
                    }
                    let mut update = crate::strategic_replication::CompanyUpdate::new(team);
                    if !published.companies.contains_key(&team) {
                        update.active = Some(true);
                        update.owner_index = Some(0x105);
                        update.available_mask = Some(value.snapshot.available_mask);
                        update.shadow_modifier_percent = Some(0);
                    }
                    update.data = value.snapshot.data;
                    update.leader_index = Some(value.leader);
                    update.active_mask = Some(value.snapshot.active_mask);
                    company_updates.push(update);
                    published.companies.insert(team, value);
                }
            }
            let mut platoon_updates = Vec::new();
            let mut squad_updates = Vec::new();
            let mut detections = Vec::new();
            for platoon in 0..8 {
                let value = PublishedPlatoon {
                    enabled: enabled[usize::from(platoon)],
                    leader: self.platoon_leader_index(platoon),
                    unit: next
                        .units
                        .get(&NativeAbilityScope::Platoon(platoon))
                        .cloned()
                        .unwrap_or_default(),
                };
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

                    update.available_mask = Some(next.available(
                        self.match_profile().game_type as u16,
                        side,
                        CommandRole::PlatoonLeader,
                    ));
                    update.shadow_modifier_percent = Some(0);
                }
                update.leader_index = Some(value.leader);
                update.data = value.unit.data;
                update.active_mask = Some(value.unit.active);
                update.enabled_mask = Some(value.enabled);
                platoon_updates.push(update);
                published.platoons.insert(platoon, value);
            }
            // A vacated squad can still have an active Unit or a previously
            // published mask. Publish only to recipients with that native object.
            let candidates: BTreeSet<_> = squads
                .iter()
                .chain(published.squads.keys())
                .copied()
                .collect();
            let catalog = &actor.squad_catalog;
            let selected_squad = catalog.squad_index();
            let auto_assign = catalog.match_profile().auto_assign_teams;
            for squad in candidates {
                let registered = if catalog.map_pool().squads.is_empty() {
                    squad == selected_squad
                } else {
                    catalog.selected_squad(squad).is_some_and(|binding| {
                        auto_assign
                            || (u16::from(binding.team) == selected_squad / 16
                                && u16::from(binding.platoon) == (selected_squad % 16) / 4)
                    })
                };
                if !registered {
                    continue;
                }
                let unit = next
                    .units
                    .get(&NativeAbilityScope::Squad(squad))
                    .cloned()
                    .unwrap_or_default();
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
                if published
                    .detections
                    .get(&index)
                    .is_some_and(|(identity, detected)| {
                        *identity == detection.identity && *detected == value
                    })
                {
                    continue;
                }
                detections.push(NativeDetectionUpdate {
                    character_index: index,
                    globally_detected: Some(value),
                    undetected_via_satellite_jammer: Some(false),
                });
                published
                    .detections
                    .insert(index, (detection.identity.clone(), value));
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
            if !company_updates.is_empty() {
                packets.push(crate::strategic_replication::CompanyUpdate::encode_batch(
                    &company_updates,
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
                || actor.ordinary_shared_len() + packets.len() > multiplayer::MAX_SHARED_QUEUE
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
