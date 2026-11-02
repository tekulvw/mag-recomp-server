//! Immediate character effects for Acquisition world callbacks.
//!
//! Planning is read-only. The caller records the complete outer callback before
//! committing a plan on its owned candidate. No owner-update damage queue, audit
//! write, service, or network delivery is performed here. Publication queues are
//! the normal reliable Engine queues and must be installed with the candidate.
use super::*;
use acquisition_damage_metadata as metadata;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub character_id: u32,
    pub generation: u32,
    pub world: u32,
}
/// Values must come from the owned, current character state. Absence is an
/// explicit unsupported input, never an implicit unprotected/on-foot default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeState {
    pub invulnerability: Option<u8>,
    pub associated_vehicle: Option<protocol::VictimWasMounted>,
}
#[derive(Clone, Copy, Debug)]
pub enum Request {
    VolumeLethal {
        victim_vector: [f32; 3],
    },
    VehicleDeathOccupant {
        last_attacker: u32,
        vehicle_vector: [f32; 3],
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    DeadIgnored,
    Protected,
    NoDamage,
    Applied,
}

#[derive(PartialEq)]
enum LifeIdentity {
    Alive,
    Incapacitated(incapacitation::Downed),
    Dead,
    Bot(u8),
}
#[derive(PartialEq)]
struct Snapshot {
    reservation: Option<String>,
    health: u16,
    life: LifeIdentity,
    last_damage: (u8, u16, u8, u32, u32, u32),
    mounted: bool,
}
fn damage_identity(d: replication::CharacterDamage) -> (u8, u16, u8, u32, u32, u32) {
    (
        d.last_location_and_type,
        d.health_quarter_points,
        d.conditions,
        d.last_attacker,
        d.last_source_object,
        d.last_user_data,
    )
}
/// A single collector-pair effect. Consume in collector order, re-planning the
/// next pair against the already-mutated candidate, including duplicate pairs.
/// This is not an atomic clone of Engine or an account-storage transaction.
pub struct Plan {
    target: Target,
    expected: Snapshot,
    native_before: NativeState,
    native_after: NativeState,
    now: Duration,
    outcome: Outcome,
    damage: Option<replication::CharacterDamage>,
    life: Option<CharacterLife>,
    packet: Option<Vec<u8>>,
    consequences: Option<events::CharacterEffectConsequences>,
    notice: Option<death_notifications::CharacterEffectNotice>,
    audit: Value,
    clear_healing: bool,
}
impl Plan {
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub fn audit(&self) -> &Value {
        &self.audit
    }
    pub fn native_after(&self) -> NativeState {
        self.native_after
    }
}
impl<W: Write> Engine<W> {
    fn acquisition_character_snapshot(&self, target: Target) -> Result<Snapshot, &'static str> {
        if target.character_id & 0xffff_0000 != 0x0380_0000 || target.generation == 0 {
            return Err("invalid_acquisition_character_identity");
        }
        if let Some(actor) = self.active.iter().chain(self.peers.values()).find(|a| {
            a.reservation.world == target.world
                && (0x0380_0000 | u32::from(a.reservation.player)) == target.character_id
        }) {
            if !actor.entered_game || !actor.ready_for_updates || actor.initial_objects.is_none() {
                return Err("acquisition_character_not_in_world");
            }
            let generation = match actor.life {
                CharacterLife::Alive { generation, .. }
                | CharacterLife::Incapacitated { generation, .. }
                | CharacterLife::AwaitingRespawn { generation } => generation,
                _ => return Err("acquisition_character_has_no_life"),
            };
            if generation != target.generation {
                return Err("stale_acquisition_character_generation");
            }
            return Ok(Snapshot {
                reservation: Some(actor.reservation.id.clone()),
                health: actor.life.damage_health().map_or(0, |(_, h)| h),
                life: match actor.life {
                    CharacterLife::Alive { .. } => LifeIdentity::Alive,
                    CharacterLife::Incapacitated { downed, .. } => {
                        LifeIdentity::Incapacitated(downed)
                    }
                    _ => LifeIdentity::Dead,
                },
                last_damage: damage_identity(actor.last_damage),
                mounted: actor.is_mounted(),
            });
        }
        let bot = self
            .bot_registry
            .bots
            .get(&(target.character_id as u16))
            .ok_or("acquisition_character_missing")?;
        if !self.accepts_subscription_world(target.world) {
            return Err("acquisition_character_world_mismatch");
        }
        let runtime = bot.target.runtime.ok_or("bot_runtime_missing")?;
        if runtime.generation != target.generation {
            return Err("stale_acquisition_character_generation");
        }
        Ok(Snapshot {
            reservation: None,
            health: bot.health,
            life: LifeIdentity::Bot(match runtime.phase {
                development_target::BotPhase::Moving => 0,
                development_target::BotPhase::Dead => 1,
                development_target::BotPhase::Complete => 2,
                development_target::BotPhase::Relocating => 3,
            }),
            last_damage: damage_identity(bot.damage),
            mounted: false,
        })
    }

    pub(crate) fn plan_acquisition_character_effect(
        &self,
        target: Target,
        request: Request,
        native: NativeState,
        now: Duration,
    ) -> Result<Plan, &'static str> {
        if self.wire_profile.application_id() != 20231 {
            return Err("unsupported_acquisition_character_profile");
        }
        let expected = self.acquisition_character_snapshot(target)?;
        let initial_invulnerability = native
            .invulnerability
            .ok_or("acquisition_character_invulnerability_missing")?;
        let (raw, kind, attacker, source_type, source_index, from_direction, bypass) = match request
        {
            Request::VolumeLethal { victim_vector } => {
                let damage = metadata::Metadata::from_symbols(
                    "DamageType.ExternalKinetic",
                    "DamageSourceType.eDamageSourceTypeCrushed",
                )?
                .character_lethal(
                    target.character_id,
                    victim_vector,
                    initial_invulnerability,
                )?;
                (
                    armor::RawCharacterDamage {
                        amount_hp: damage.amount_hp,
                        armor_rating_redux: damage.armor_reduction[0],
                        armor_prevention_redux: damage.armor_reduction[1],
                        hit_location: damage.location,
                    },
                    damage::DamageKind::ExternalKinetic,
                    damage.attacker,
                    damage.source_type,
                    damage.source_index,
                    damage.from_direction,
                    damage.clear_invulnerability,
                )
            }
            Request::VehicleDeathOccupant {
                last_attacker,
                vehicle_vector,
            } => {
                let rules = self
                    .character_damage_rules
                    .as_ref()
                    .ok_or("acquisition_occupant_damage_rules_missing")?;
                let selected = if last_attacker >> 24 == 3 {
                    last_attacker
                } else {
                    target.character_id
                };
                let same_team = self
                    .character_team_in_world(selected, target.world)
                    .zip(self.character_team_in_world(target.character_id, target.world))
                    .map(|(a, v)| a == v);
                let damage = metadata::vehicle_death_occupant(
                    target.character_id,
                    last_attacker,
                    vehicle_vector,
                    same_team,
                    rules.friendly_fire_multiplier(),
                )?;
                (
                    armor::RawCharacterDamage {
                        amount_hp: damage.amount_hp,
                        armor_rating_redux: damage.armor_rating_reduction,
                        armor_prevention_redux: damage.armor_prevention_reduction,
                        hit_location: damage.location,
                    },
                    damage::DamageKind::Explosive,
                    damage.attacker,
                    damage.source_type,
                    damage.source_index,
                    damage.from_direction,
                    false,
                )
            }
        };
        // Mirror the native bool temporary scope on the owned state value. The
        // restored byte is canonical1 even when the original nonzero byte was2.
        let mut native_during = native;
        if bypass {
            native_during.invulnerability = Some(0);
        }
        let mut native_after = native_during;
        if bypass {
            native_after.invulnerability = Some(1);
        }
        let mut plan = Plan {
            target,
            expected,
            native_before: native,
            native_after,
            now,
            outcome: Outcome::NoDamage,
            damage: None,
            life: None,
            packet: None,
            consequences: None,
            notice: None,
            audit: Value::Null,
            clear_healing: false,
        };
        // OnCharacterHit precedes the native dead/protection guards. Preserve
        // every pair in the caller's outer audit, including ignored duplicates.
        let mut audit = json!({"on_character_hit":true,"character_id":target.character_id,
            "generation":target.generation,"world":target.world,"raw_amount_hp":raw.amount_hp,
            "damage_type":match kind { damage::DamageKind::ExternalKinetic => 5, _ => 2 },
            "damage_source_type":source_type,"damage_source_index":source_index,
            "attacker":attacker,"source_object":u32::MAX,"location":raw.hit_location,"user_data":0,
            "from_direction":from_direction,"invulnerability_before":initial_invulnerability,
            "invulnerability_during":native_during.invulnerability,"invulnerability_after":native_after.invulnerability});
        let human = plan.expected.reservation.as_ref().and_then(|id| {
            self.active
                .iter()
                .chain(self.peers.values())
                .find(|a| a.reservation.id == *id)
        });
        let dead = human.map_or(plan.expected.health == 0, |a| {
            a.life.damage_health().is_none()
        });
        if dead {
            plan.outcome = Outcome::DeadIgnored;
        } else if native_during.invulnerability != Some(0) {
            plan.outcome = Outcome::Protected;
        } else {
            let catalog = self.item_catalog.retail_combat()?;
            let mut armor = if let Some(victim) = human {
                let kit = victim
                    .equipped_loadout()
                    .ok_or("acquisition_character_live_loadout_missing")?;
                let modifiers = self.plan_leadership(now)?.modifiers(victim);
                let armor = armor::ArmorProfile::from_retail_loadout(&kit, catalog)?
                    .with_modifiers(&modifiers);
                if matches!(victim.life, CharacterLife::Incapacitated { .. }) {
                    armor.for_incapacitated(catalog)?
                } else {
                    armor
                }
            } else {
                let bot = self
                    .bot_registry
                    .bots
                    .get(&(target.character_id as u16))
                    .ok_or("acquisition_character_missing")?;
                armor::ArmorProfile::from_retail_loadout(&bot.target.loadout, catalog)?
            };
            // No default FF tuning for a live world effect. Self source is
            // base1, and distinct resolved same-team attackers use bound rules.
            if attacker != target.character_id {
                self.character_damage_rules
                    .as_ref()
                    .ok_or("acquisition_character_damage_rules_missing")?;
            }
            armor = armor.with_incoming_base(self.character_incoming_base(
                attacker,
                target.character_id,
                target.world,
            ))?;
            let amount = armor.adjust_for_kind(raw, kind)?;
            audit["calculation"] = armor.calculation(raw, kind)?;
            plan.clear_healing = raw.amount_hp * armor.incoming_scale(kind) > 0.0;
            if amount > 0.0 {
                // The native internal request admits positive compensated raw
                // amounts beyond the external command's1000 limit. Reuse the
                // shared remaining-HP quantizer; never force terminal health.
                let mut damage = replication::CharacterDamage {
                    last_location_and_type: (match kind {
                        damage::DamageKind::ExternalKinetic => 5,
                        _ => 2,
                    } << 4)
                        | raw.hit_location,
                    health_quarter_points: damage::remaining_health_quarter_points(
                        plan.expected.health,
                        amount,
                    )?,
                    conditions: 0,
                    last_attacker: attacker,
                    last_source_object: u32::MAX,
                    last_user_data: 0,
                };
                if let Some(victim) = human {
                    let (model, life) = self.plan_human_damage(victim, amount, damage, now)?;
                    damage = model;
                    plan.life = Some(life);
                    // Preflight the only fallible cleanup in apply_peer_damage.
                    if damage.health_quarter_points == 0 {
                        let removals = victim
                            .projectile_replicas
                            .clone()
                            .take_all_removals()
                            .into_iter()
                            .map(|removal| {
                                removal
                                    .encode_with_profile(
                                        1,
                                        now.as_millis() as u32,
                                        victim.wire_profile,
                                    )
                                    .map(|wire| protocol::encode_hex(&wire))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        audit["projectile_removal_publications_hex"] = json!(removals);
                    }
                }
                plan.packet = Some(
                    replication::CharacterDamageUpdate {
                        sequence: 2,
                        timestamp_ms: now.as_millis() as u32,
                        object_index: target.character_id as u16,
                        damage,
                        forced_state: (damage.health_quarter_points == 0).then_some(
                            replication::CharacterState {
                                state: 7,
                                stance: 0,
                                user_data: [0; 3],
                            },
                        ),
                    }
                    .encode_with_profile(self.wire_profile)?,
                );
                audit["damage_publication_hex"] =
                    json!(protocol::encode_hex(plan.packet.as_ref().unwrap()));
                if plan.expected.health > 0 && damage.health_quarter_points == 0 {
                    plan.notice = Some(
                        self.plan_character_effect_notice(
                            death_notifications::CharacterEffectNoticeRequest {
                                world: target.world,
                                victim: target.character_id as u16,
                                attacker,
                                source_type,
                                source_index,
                                location: raw.hit_location,
                                mounted: native
                                    .associated_vehicle
                                    .ok_or("acquisition_character_vehicle_association_missing")?,
                            },
                        )?,
                    );
                }
                plan.consequences = Some(events::plan_character_effect_consequences(
                    self,
                    events::CharacterEffectInput {
                        world: target.world,
                        victim_id: target.character_id as u16,
                        generation: target.generation,
                        attacker,
                        damage,
                        life: plan.life,
                        now,
                    },
                )?);
                audit["health_after_quarter_points"] = json!(damage.health_quarter_points);
                audit["life_after"] = json!(format!("{:?}", plan.life));
                audit["consequences"] = plan.consequences.as_ref().unwrap().audit();
                audit["death_notice"] = plan.notice.as_ref().map_or(Value::Null, |n| n.audit());
                plan.damage = Some(damage);
                plan.outcome = Outcome::Applied;
            }
        }
        audit["outcome"] = json!(format!("{:?}", plan.outcome));
        plan.audit = audit;
        Ok(plan)
    }

    /// No audit or direct delivery: call only on the isolated candidate after
    /// outer planning succeeds. Returned application messages join that outer
    /// transaction. Account persistence is the normal progression outbox.
    pub(crate) fn commit_acquisition_character_effect(
        &mut self,
        plan: Plan,
        native: &mut NativeState,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if *native != plan.native_before
            || self.acquisition_character_snapshot(plan.target)? != plan.expected
        {
            return Err("stale_acquisition_character_effect");
        }
        if let Some(consequences) = &plan.consequences {
            consequences.validate(self)?;
        }
        if let Some(damage) = plan.damage {
            if let Some(id) = &plan.expected.reservation {
                let victim = self
                    .active
                    .as_mut()
                    .filter(|a| a.reservation.id == *id)
                    .or_else(|| self.peers.get_mut(id))
                    .ok_or("acquisition_character_missing")?;
                victim.apply_peer_damage(plan.target.generation, damage, plan.now)?;
                if let Some(life) = plan.life {
                    victim.life = life;
                }
                if !victim.life.is_alive() {
                    victim
                        .grenade_damage
                        .retain(|hit| hit.target_character_id != plan.target.character_id);
                }
            } else {
                self.commit_bot_damage(
                    plan.target.character_id as u16,
                    plan.target.generation,
                    damage,
                    plan.now,
                )?;
            }
        }
        if plan.clear_healing {
            if let Some(id) = &plan.expected.reservation {
                if let Some(victim) = self
                    .active
                    .as_mut()
                    .filter(|a| a.reservation.id == *id)
                    .or_else(|| self.peers.get_mut(id))
                {
                    victim.healing = None;
                }
            } else if let Some(bot) = self
                .bot_registry
                .bots
                .get_mut(&(plan.target.character_id as u16))
            {
                bot.healing = Default::default();
            }
        }
        if let Some(packet) = plan.packet {
            self.broadcast_bot_packet(packet);
        }
        if let Some(notice) = plan.notice {
            notice.commit(self);
        }
        *native = plan.native_after;
        Ok(plan.consequences.map_or_else(Vec::new, |c| c.commit(self)))
    }
}

macro_rules! actor_effect_state {
    ($($field:ident : $ty:ty),* $(,)?) => {
        struct ActorEffects {
            $($field: $ty,)*
            notifications: std::collections::VecDeque<(bool, Vec<u8>)>,
        }
        impl ActorEffects {
            fn capture(actor: &Active) -> Self {
                Self { $($field: actor.$field.clone(),)* notifications: actor.pending_notifications.iter().map(|p| match p {
                    match_flow::Publication::Application(wire) => (false, wire.clone()),
                    match_flow::Publication::Replication(wire) => (true, wire.clone()),
                }).collect() }
            }
            fn install(self, actor: &mut Active) {
                $(actor.$field = self.$field;)*
                actor.pending_notifications = self.notifications.into_iter().map(|(replication, wire)|
                    if replication { match_flow::Publication::Replication(wire) } else { match_flow::Publication::Application(wire) }).collect();
            }
        }
    };
}
actor_effect_state! {
    life: CharacterLife,
    tracking: player_tracking::PlayerTracking,
    owner_fields: player_tracking::ClientCharacterFields,
    fall_tracker: fall_damage::Tracker,
    revive_state_pending: bool,
    pending_damage: Option<PendingDamage>,
    last_damage: replication::CharacterDamage,
    healing: Option<progression::HealingSession>,
    reward_statistics: std::collections::BTreeMap<u8, i32>,
    projectiles: projectile::ProjectileFlights,
    projectile_replicas: projectile_replication::ProjectileReplicas,
    projectile_publications: Vec<ProjectilePublication>,
    grenade_damage: std::collections::VecDeque<PendingDamage>,
    pending_shared: std::collections::VecDeque<Vec<u8>>,
    replication_backlogged: bool,
    mounted_seat: Option<u8>,
    ground_seat: Option<ground_seats::PassengerSeat>,
    last_exit: Option<protocol::DismountSeat>,
    pending_spawn_deadline: Option<Duration>,
    published_wave_deadline: Option<Duration>,
}
struct EffectState {
    actors: std::collections::BTreeMap<String, ActorEffects>,
    bots: bot_registry::CharacterDamageCandidate,
    rewards: std::collections::BTreeMap<String, progression::RewardEvent>,
    reward_order: std::collections::VecDeque<String>,
}
impl EffectState {
    fn capture<W: Write>(engine: &Engine<W>) -> Self {
        Self {
            actors: engine
                .active
                .iter()
                .chain(engine.peers.values())
                .map(|a| (a.reservation.id.clone(), ActorEffects::capture(a)))
                .collect(),
            bots: engine.bot_registry.character_damage_candidate(),
            rewards: engine.pending_rewards.clone(),
            reward_order: engine.reward_order.clone(),
        }
    }
    fn same_actors<W: Write>(&self, engine: &Engine<W>) -> bool {
        let ids: std::collections::BTreeSet<_> = engine
            .active
            .iter()
            .chain(engine.peers.values())
            .map(|a| a.reservation.id.as_str())
            .collect();
        ids == self.actors.keys().map(String::as_str).collect()
    }
    fn install<W: Write>(mut self, engine: &mut Engine<W>) {
        for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
            self.actors
                .remove(&actor.reservation.id)
                .expect("validated character-effect candidate topology")
                .install(actor);
        }
        engine
            .bot_registry
            .install_character_damage_candidate(self.bots);
        engine.pending_rewards = self.rewards;
        engine.reward_order = self.reward_order;
    }
}
/// Maximum number of collector effects/corrections staged by one callback.
/// Duplicates count separately because they retain native collector order.
pub const MAX_CHARACTER_EFFECTS: usize = 2048;
type NativeMap = std::collections::BTreeMap<u32, NativeState>;
#[derive(PartialEq, Eq)]
struct Topology {
    humans: std::collections::BTreeMap<String, (u32, u16, String)>,
    bots: std::collections::BTreeMap<u16, (u32, u16)>,
}
impl Topology {
    fn capture<W: Write>(engine: &Engine<W>) -> Self {
        Self {
            humans: engine
                .active
                .iter()
                .chain(engine.peers.values())
                .map(|actor| {
                    (
                        actor.reservation.id.clone(),
                        (
                            actor.reservation.world,
                            actor.reservation.player,
                            format!("{:?}", actor.life),
                        ),
                    )
                })
                .collect(),
            bots: engine
                .bot_registry
                .bots
                .iter()
                .map(|(&id, bot)| {
                    (
                        id,
                        (bot.target.runtime.map_or(0, |r| r.generation), bot.health),
                    )
                })
                .collect(),
        }
    }
}
/// Fully prepared owned lifecycle/reward/reliable-publication fields. Dropping
/// it leaves authoritative Engine unchanged. Origin/revision/round/topology and
/// native input fences prevent a different Engine or later event adopting it.
pub struct Candidate {
    state: EffectState,
    native: NativeMap,
    native_before: NativeMap,
    origin: std::sync::Arc<()>,
    revision: u64,
    round: u64,
    topology: Topology,
    now: Duration,
    audit: Vec<Value>,
    messages: Vec<OutboundMessage>,
}
impl Candidate {
    pub fn audit(&self) -> &[Value] {
        &self.audit
    }
    pub fn native_states(&self) -> &NativeMap {
        &self.native
    }
}
impl<W: Write> Engine<W> {
    /// Run a bounded callback against cloned character-effect authority and
    /// restore the original fields before returning. Queries, corrections and
    /// damage may be interleaved in native pair order inside `prepare`.
    /// The closure may mutate only character-effect fields covered by EffectState;
    /// other world/objective candidates remain owned by the outer callback.
    pub(crate) fn prepare_acquisition_character_callback<F>(
        &mut self,
        native: &NativeMap,
        now: Duration,
        prepare: F,
    ) -> Result<Candidate, &'static str>
    where
        F: FnOnce(
            &mut Self,
            &mut NativeMap,
        ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str>,
    {
        if native.len() > MAX_CHARACTER_EFFECTS {
            return Err("acquisition_character_effect_limit");
        }
        let before = EffectState::capture(self);
        let origin = std::sync::Arc::clone(&self.acquisition_effect_origin);
        let revision = self.sequence;
        let round = self.round_number;
        let topology = Topology::capture(self);
        let mut owned_native = native.clone();
        let result = prepare(self, &mut owned_native).and_then(|(audit, messages)| {
            if audit.len() > MAX_CHARACTER_EFFECTS
                || owned_native.len() > MAX_CHARACTER_EFFECTS
                || messages.len()
                    > MAX_CHARACTER_EFFECTS.saturating_mul(self.maximum_players.max(1))
            {
                return Err("acquisition_character_effect_limit");
            }
            Ok((EffectState::capture(self), audit, messages))
        });
        before.install(self);
        let (state, audit, messages) = result?;
        Ok(Candidate {
            state,
            native: owned_native,
            native_before: native.clone(),
            origin,
            revision,
            round,
            topology,
            now,
            audit,
            messages,
        })
    }
    pub fn prepare_acquisition_character_effects(
        &mut self,
        effects: &[(Target, Request)],
        native: &NativeMap,
        now: Duration,
    ) -> Result<Candidate, &'static str> {
        self.prepare_acquisition_character_effects_with_moves(&[], effects, native, now)
    }
    /// Convenience API for a pre-resolved batch. World-query callbacks that must
    /// update later cast layers after each death use the interleaved closure API.
    pub fn prepare_acquisition_character_effects_with_moves(
        &mut self,
        moves: &[(Target, replication::CharacterPosition)],
        effects: &[(Target, Request)],
        native: &NativeMap,
        now: Duration,
    ) -> Result<Candidate, &'static str> {
        if moves
            .len()
            .checked_add(effects.len())
            .is_none_or(|n| n > MAX_CHARACTER_EFFECTS)
        {
            return Err("acquisition_character_effect_limit");
        }
        self.prepare_acquisition_character_callback(native, now, |engine, native| {
            let mut audit = Vec::with_capacity(moves.len() + effects.len());
            let mut messages = Vec::new();
            for &(target, position) in moves {
                audit.push(engine.apply_acquisition_character_move(target, position, now)?);
            }
            for &(target, request) in effects {
                let state = native
                    .get_mut(&target.character_id)
                    .ok_or("acquisition_character_native_state_missing")?;
                let plan =
                    engine.plan_acquisition_character_effect(target, request, *state, now)?;
                audit.push(plan.audit().clone());
                messages.extend(engine.commit_acquisition_character_effect(plan, state)?);
            }
            Ok((audit, messages))
        })
    }
    /// Validate before the durable outer callback record. Accepted intervening
    /// events, another Engine, round changes or native-state changes are stale.
    pub fn check_acquisition_character_effects(
        &self,
        candidate: &Candidate,
        native: &NativeMap,
    ) -> Result<(), &'static str> {
        if !std::sync::Arc::ptr_eq(&self.acquisition_effect_origin, &candidate.origin)
            || self.sequence != candidate.revision
            || self.round_number != candidate.round
            || *native != candidate.native_before
            || Topology::capture(self) != candidate.topology
            || !candidate.state.same_actors(self)
        {
            return Err("stale_acquisition_character_candidate");
        }
        if self.failed {
            return Err("recorder_unavailable");
        }
        Ok(())
    }
    /// Source-internal installation for an already validated, recorded outer
    /// callback. There are deliberately no fallible checks after its record.
    pub(crate) fn install_character_effects_after_record(
        &mut self,
        candidate: Candidate,
        native: &mut NativeMap,
    ) -> Vec<OutboundMessage> {
        candidate.state.install(self);
        *native = candidate.native;
        candidate.messages
    }
    /// Apply only to an isolated owned candidate. Authoritative callers use the
    /// recording method below or check + their outer record + recorded install.
    #[cfg(test)]
    pub(crate) fn install_acquisition_character_effects(
        &mut self,
        candidate: Candidate,
        native: &mut NativeMap,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        self.check_acquisition_character_effects(&candidate, native)?;
        Ok(self.install_character_effects_after_record(candidate, native))
    }
    /// Concrete authoritative transaction: validate all fences, record complete
    /// effect/publication evidence, then install with no later fallible check.
    /// Account persistence remains the normal progression outbox protocol.
    pub fn record_acquisition_character_effects(
        &mut self,
        candidate: Candidate,
        native: &mut NativeMap,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        self.check_acquisition_character_effects(&candidate, native)?;
        self.record(
            json!({"op":"acquisition_character_effects", "effects":candidate.audit,
            "messages":candidate.messages,"timestamp_ms":candidate.now.as_millis() as u64}),
            wall,
        )?;
        Ok(self.install_character_effects_after_record(candidate, native))
    }
    /// Apply a qualified force-position result to the owned candidate. The
    /// ordinary Character field5 packet is reliable and includes its owner.
    pub(crate) fn apply_acquisition_character_move(
        &mut self,
        target: Target,
        position: replication::CharacterPosition,
        now: Duration,
    ) -> Result<Value, &'static str> {
        position.validate()?;
        let snapshot = self.acquisition_character_snapshot(target)?;
        if snapshot.life == LifeIdentity::Dead
            || snapshot.health == 0 && snapshot.reservation.is_none()
        {
            return Err("acquisition_move_target_not_living");
        }
        let fields = player_tracking::ClientCharacterFields {
            position: Some(position),
            ..Default::default()
        };
        let packet = replication::multiplayer::CharacterOwnerDelta {
            sequence: 2,
            timestamp_ms: now.as_millis() as u32,
            player: target.character_id as u16,
            fields: &fields,
        }
        .encode_with_profile(self.wire_profile)?;
        if let Some(id) = &snapshot.reservation {
            let actor = self
                .active
                .as_mut()
                .filter(|a| a.reservation.id == *id)
                .or_else(|| self.peers.get_mut(id))
                .ok_or("acquisition_character_missing")?;
            let mut fall = actor.fall_tracker.clone();
            fall.reset(
                target.generation,
                actor.tracking.accepted_position_sequence,
                position.y,
            )?;
            actor.tracking.server_position(
                position,
                player_tracking::PositionOrigin::ServerCorrection,
                now.as_millis() as u64,
            );
            actor.owner_fields.position = Some(position);
            actor.fall_tracker = fall;
        } else {
            self.bot_registry.correct_character_position(
                target.character_id as u16,
                target.generation,
                position,
                now,
            )?;
        }
        for recipient in self
            .active
            .iter_mut()
            .chain(self.peers.values_mut())
            .filter(|a| a.reservation.world == target.world && a.initial_objects.is_some())
        {
            recipient.queue_shared(packet.clone());
        }
        Ok(
            json!({"op":"acquisition_character_position", "character_id":target.character_id,
            "generation":target.generation,"world":target.world,"position":position,
            "publication_hex":protocol::encode_hex(&packet),"include_owner":true}),
        )
    }
}

#[cfg(test)]
#[path = "acquisition_character_effects_tests.rs"]
mod tests;
