//! Optional retail firearm assist accounting for current Character allocations.
use crate::*;
use serde::Deserialize;
use std::{collections::BTreeMap, io::Read, path::Path};

const RETAIL_APPLICATION_ID: u32 = 20231;
const MAX_RULE_BYTES: u64 = 65_536;
const CONTRIBUTORS: usize = 5;
pub(super) const STAT: u8 = 11;

/// Authored assist lifetime projected from the original retail settings.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub schema_version: u8,
    pub application_id: u32,
    pub assist_timeout_ms: u32,
}

impl Rules {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_RULE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_RULE_BYTES {
            return Err(io::Error::other("firearm_assist_rules_too_large"));
        }
        let rules: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        rules.validate().map_err(io::Error::other)?;
        Ok(rules)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1 || self.application_id != RETAIL_APPLICATION_ID {
            return Err("invalid_firearm_assist_rules");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Allocation {
    Human(u64),
    Bot(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Actor {
    index: u16,
    allocation: Allocation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Victim {
    actor: Actor,
    generation: u32,
}

impl Victim {
    fn matches<W: Write>(self, engine: &Engine<W>, world: u32, alive: bool) -> bool {
        let index = self.actor.index;
        match self.actor.allocation {
            Allocation::Human(sequence) => engine
                .active
                .iter()
                .chain(engine.peers.values())
                .find(|human| human.reservation.world == world && human.reservation.player == index)
                .is_some_and(|human| {
                    human.allocation_sequence == sequence
                        && match human.life {
                            CharacterLife::Alive {
                                generation,
                                health_quarter_points,
                            } => {
                                alive && generation == self.generation && health_quarter_points > 0
                            }
                            CharacterLife::Incapacitated { generation, .. }
                            | CharacterLife::AwaitingRespawn { generation } => {
                                !alive && generation == self.generation
                            }
                            CharacterLife::AwaitingFirstSpawn => false,
                        }
                }),
            Allocation::Bot(allocation) => {
                engine.bot_registry.bots.get(&index).is_some_and(|bot| {
                    bot.allocation_generation == allocation
                        && (bot.health > 0) == alive
                        && bot.target.runtime.is_some_and(|runtime| {
                            runtime.generation == self.generation
                                && matches!(runtime.phase, development_target::BotPhase::Dead)
                                    != alive
                        })
                })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    actor: Actor,
    expiry_ms: u128,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Ledger([Option<Entry>; CONTRIBUTORS]);

impl Ledger {
    fn remember(&mut self, actor: Actor, expiry_ms: u128) {
        let position = self
            .0
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.actor.index == actor.index))
            .or_else(|| self.0.iter().position(Option::is_none))
            .unwrap_or_else(|| {
                self.0
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, entry)| entry.unwrap().expiry_ms)
                    .unwrap()
                    .0
            });
        self.0[position] = Some(Entry { actor, expiry_ms });
    }

    fn eligible(&self, killer: Actor, now_ms: u128) -> impl Iterator<Item = Actor> + '_ {
        self.0
            .iter()
            .flatten()
            .filter(move |entry| entry.actor != killer && now_ms <= entry.expiry_ms)
            .map(|entry| entry.actor)
    }
}

#[derive(Clone, Default)]
pub(super) struct State {
    rules: Option<Rules>,
    scope: Option<(u32, u64)>,
    victims: BTreeMap<u16, (Victim, Ledger)>,
    revision: u64,
}

impl State {
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn enabled(&self) -> bool {
        self.rules.is_some()
    }

    pub(super) fn clear_round(&mut self) {
        if self.scope.take().is_some() {
            self.advance();
        }
        self.victims.clear();
    }

    pub(super) fn forget_victim(&mut self, index: u16) {
        if self.enabled() {
            self.victims.remove(&index);
            self.advance();
        }
    }

    fn advance(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

/// Live identities supplied by ordinary firearm admission. The engine resolves
/// them again against the installed Character before preparing accounting.
#[derive(Clone, Copy)]
pub(super) struct Fighter<'a> {
    pub(super) index: u16,
    pub(super) reservation: Option<&'a str>,
    pub(super) generation: u32,
}

impl<'a> Fighter<'a> {
    pub(super) fn human(actor: &'a Active) -> Self {
        Self {
            index: actor.reservation.player,
            reservation: Some(&actor.reservation.id),
            generation: actor
                .life
                .damage_health()
                .map_or(0, |(generation, _)| generation),
        }
    }

    fn current<W: Write>(self, engine: &Engine<W>, world: u32) -> Option<(Actor, u8)> {
        if usize::from(self.index) >= MAX_MATCH_PLAYERS {
            return None;
        }
        if let Some(reservation) = self.reservation {
            let human = engine
                .active
                .iter()
                .chain(engine.peers.values())
                .find(|actor| {
                    actor.reservation.world == world
                        && actor.reservation.player == self.index
                        && actor.reservation.id == reservation
                })?;
            if human.is_mounted()
                || !matches!(human.life, CharacterLife::Alive { generation, health_quarter_points }
                    if generation == self.generation && health_quarter_points > 0)
            {
                return None;
            }
            return Some((
                Actor {
                    index: self.index,
                    allocation: Allocation::Human(human.allocation_sequence),
                },
                human.team(),
            ));
        }
        let bot = engine.bot_registry.bots.get(&self.index)?;
        let runtime = bot.target.runtime?;
        if bot.health == 0
            || runtime.generation != self.generation
            || matches!(runtime.phase, development_target::BotPhase::Dead)
        {
            return None;
        }
        Some((
            Actor {
                index: self.index,
                allocation: Allocation::Bot(bot.allocation_generation),
            },
            bot.target.squad.team(),
        ))
    }
}

struct Award {
    actor: Actor,
    reservation: Option<String>,
    before: i32,
    after: i32,
}

/// Identity of the reservation that accepted and still owns a queued frag dose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FragOwner {
    allocation: u64,
    team: u8,
}

impl FragOwner {
    pub(crate) fn capture(owner: &Active) -> Self {
        Self {
            allocation: owner.allocation_sequence,
            team: owner.team(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct GroundCrew {
    gunner: Victim,
    vehicle: acquisition_ground_runtime::Key,
    spawn_counter: u16,
    driver: Option<Victim>,
}

impl GroundCrew {
    fn capture<W: Write>(engine: &Engine<W>, gunner: &Active) -> Option<Self> {
        let seat = gunner.ground_seat?;
        if !engine.ground_gunner_authority(gunner, seat.vehicle_index)
            || engine.current_ground_gunner_kind(gunner).is_none()
        {
            return None;
        }
        let vehicle = engine.ground_runtime.as_ref()?.get(seat.vehicle_index)?;
        let mut drivers = engine
            .active
            .iter()
            .chain(engine.peers.values())
            .filter(|driver| {
                driver.reservation.world == gunner.reservation.world
                    && driver.team() == gunner.team()
                    && driver.reservation.character.faction == vehicle.faction
                    && driver
                        .owner_fields
                        .state
                        .is_some_and(|state| state.state == 14)
                    && engine.ground_driver_authority(driver, seat.vehicle_index)
            });
        let driver = drivers.next().filter(|_| drivers.next().is_none());
        let identity = |actor: &Active| Victim {
            actor: Actor {
                index: actor.reservation.player,
                allocation: Allocation::Human(actor.allocation_sequence),
            },
            generation: Fighter::human(actor).generation,
        };
        Some(Self {
            gunner: identity(gunner),
            vehicle: vehicle.key,
            spawn_counter: vehicle.spawn_counter,
            driver: driver.map(identity),
        })
    }
}

enum Source {
    Firearm(Victim),
    GroundPrimary(GroundCrew),
    QueuedFrag {
        index: u16,
        owner: FragOwner,
    },
    Rocket {
        index: u16,
        allocation: u64,
        team: u8,
    },
}

impl Source {
    fn matches<W: Write>(&self, engine: &Engine<W>, world: u32) -> bool {
        match *self {
            Self::Firearm(source) => source.matches(engine, world, true),
            Self::GroundPrimary(crew) => {
                engine
                    .active
                    .iter()
                    .chain(engine.peers.values())
                    .any(|gunner| {
                        gunner.reservation.world == world
                            && gunner.reservation.player == crew.gunner.actor.index
                            && GroundCrew::capture(engine, gunner) == Some(crew)
                    })
            }
            Self::Rocket {
                index,
                allocation,
                team,
            } => engine
                .active
                .iter()
                .chain(engine.peers.values())
                .any(|actor| {
                    actor.reservation.world == world
                        && actor.reservation.player == index
                        && actor.allocation_sequence == allocation
                        && actor.team() == team
                }),
            Self::QueuedFrag { index, owner } => engine
                .active
                .iter()
                .chain(engine.peers.values())
                .any(|actor| {
                    actor.reservation.world == world
                        && actor.reservation.player == index
                        && FragOwner::capture(actor) == owner
                }),
        }
    }
}

pub(crate) const XP_CAPABILITY: &str = "assist_rewards_v1";

pub(super) struct Plan {
    scope: (u32, u64),
    revision: u64,
    source: Source,
    victim: Victim,
    after: Option<Ledger>,
    awards: Vec<Award>,
}

impl Plan {
    pub(super) fn prepare<W: Write>(
        engine: &Engine<W>,
        world: u32,
        now: Duration,
        source: Fighter<'_>,
        target: Fighter<'_>,
        collapsed: bool,
    ) -> Option<Self> {
        let (actor, team) = source.current(engine, world)?;
        let identity = Source::Firearm(Victim {
            actor,
            generation: source.generation,
        });
        Self::prepare_hit(
            engine,
            world,
            now,
            (actor, team, identity),
            target,
            collapsed,
        )
    }

    pub(super) fn prepare_ground_primary<W: Write>(
        engine: &Engine<W>,
        gunner: &Active,
        victim: &Active,
        now: Duration,
    ) -> Option<Self> {
        let crew = GroundCrew::capture(engine, gunner)?;
        Self::prepare_hit(
            engine,
            gunner.reservation.world,
            now,
            (
                crew.gunner.actor,
                gunner.team(),
                Source::GroundPrimary(crew),
            ),
            Fighter::human(victim),
            true,
        )
    }

    pub(super) fn prepare_ordinary_frag<W: Write>(
        engine: &Engine<W>,
        now: Duration,
        owner: &Active,
        binding: FragOwner,
        target: &Active,
        collapsed: bool,
    ) -> Option<Self> {
        if FragOwner::capture(owner) != binding {
            return None;
        }
        let index = owner.reservation.player;
        let actor = Actor {
            index,
            allocation: Allocation::Human(binding.allocation),
        };
        // The queue retains doses owed to peers after its owner dies or respawns.
        Self::prepare_hit(
            engine,
            owner.reservation.world,
            now,
            (
                actor,
                binding.team,
                Source::QueuedFrag {
                    index,
                    owner: binding,
                },
            ),
            Fighter::human(target),
            collapsed,
        )
    }

    pub(super) fn prepare_rocket<W: Write>(
        engine: &Engine<W>,
        source: &crate::hospital_rocket_runtime::LaunchSource,
        now: Duration,
        target: Fighter<'_>,
        collapsed: bool,
    ) -> Option<Self> {
        let identity = Source::Rocket {
            index: source.player,
            allocation: source.allocation_sequence,
            team: source.team,
        };
        if source.round != engine.round_number || !identity.matches(engine, source.world) {
            return None;
        }
        Self::prepare_hit(
            engine,
            source.world,
            now,
            (
                Actor {
                    index: source.player,
                    allocation: Allocation::Human(source.allocation_sequence),
                },
                source.team,
                identity,
            ),
            target,
            collapsed,
        )
    }

    fn prepare_hit<W: Write>(
        engine: &Engine<W>,
        world: u32,
        now: Duration,
        (killer, killer_team, source): (Actor, u8, Source),
        target: Fighter<'_>,
        collapsed: bool,
    ) -> Option<Self> {
        let rules = engine.firearm_assists.rules?;
        if engine.wire_profile.application_id() != rules.application_id
            || !engine.match_started
            || engine.match_terminal.is_some()
            || !engine.round_combat_allowed(now)
        {
            return None;
        }
        let (actor, victim_team) = target.current(engine, world)?;
        if killer.index == actor.index || (killer_team == victim_team && !collapsed) {
            return None;
        }
        let victim = Victim {
            actor,
            generation: target.generation,
        };
        let scope = (world, engine.round_number);
        let mut ledger = engine
            .firearm_assists
            .victims
            .get(&actor.index)
            .filter(|(key, _)| engine.firearm_assists.scope == Some(scope) && *key == victim)
            .map(|(_, rows)| rows.clone())
            .unwrap_or_default();
        // Duration's whole-millisecond range plus any u32 timeout fits in u128.
        let now_ms = now.as_millis();
        if killer_team != victim_team {
            let expiry = now_ms + u128::from(rules.assist_timeout_ms);
            ledger.remember(killer, expiry);
            if let Source::GroundPrimary(crew) = source
                && let Some(driver) = crew.driver
            {
                ledger.remember(driver.actor, expiry);
            }
        }
        let awards = if collapsed {
            ledger
                .eligible(killer, now_ms)
                .filter_map(|actor| Self::award(engine, world, actor))
                .collect()
        } else {
            Vec::new()
        };
        Some(Self {
            scope,
            revision: engine.firearm_assists.revision,
            source,
            victim,
            after: (!collapsed).then_some(ledger),
            awards,
        })
    }

    fn award<W: Write>(engine: &Engine<W>, world: u32, actor: Actor) -> Option<Award> {
        let (reservation, before) = match actor.allocation {
            Allocation::Human(sequence) => {
                let human = engine
                    .active
                    .iter()
                    .chain(engine.peers.values())
                    .find(|human| {
                        human.reservation.world == world
                            && human.reservation.player == actor.index
                            && human.allocation_sequence == sequence
                    })?;
                (
                    Some(human.reservation.id.clone()),
                    human.reward_statistics.get(&STAT).copied().unwrap_or(0),
                )
            }
            Allocation::Bot(allocation) => {
                let bot = engine
                    .bot_registry
                    .bots
                    .get(&actor.index)
                    .filter(|bot| bot.allocation_generation == allocation)?;
                (None, bot.firearm_kd.assists)
            }
        };
        Some(Award {
            actor,
            reservation,
            before,
            after: before.saturating_add(1),
        })
    }

    pub(super) fn humans(&self) -> impl Iterator<Item = &str> {
        self.awards
            .iter()
            .filter_map(|award| award.reservation.as_deref())
    }

    pub(super) fn rewards<W: Write>(
        &self,
        engine: &Engine<W>,
        victim_position: Option<[f32; 3]>,
        frame: crate::frago::Frame<'_>,
    ) -> Vec<crate::progression::RewardEvent> {
        self.humans()
            .filter_map(|reservation| {
                let actor = engine
                    .active
                    .iter()
                    .chain(engine.peers.values())
                    .find(|actor| actor.reservation.id == reservation)?;
                if actor.wire_profile.application_id() != 20231
                    || actor.reservation.character.assist_rewards_enabled != Some(true)
                {
                    return None;
                }
                let recipient = crate::progression::RewardRecipient::from_active(actor)?;
                let victim = self.victim;
                let identity = format!(
                    "human-assist-v1-{}-{}-{}-{}",
                    actor.reservation.id, engine.sequence, victim.actor.index, victim.generation
                );
                Some(engine.frago_combat_reward(
                    recipient.event(identity, crate::progression::RewardKind::EnemyKillAssist, 1),
                    actor,
                    victim_position,
                    frame,
                ))
            })
            .collect()
    }

    pub(super) fn audit(&self) -> Vec<Value> {
        self.awards
            .iter()
            .map(|award| {
                json!({
                    "player_id": award.actor.index,
                    "victim_player_id": self.victim.actor.index,
                    "before": award.before,
                    "after": award.after,
                })
            })
            .collect()
    }

    fn current<W: Write>(&self, engine: &Engine<W>, victim_alive: bool) -> bool {
        let (world, round) = self.scope;
        engine.round_number == round
            && engine.firearm_assists.revision == self.revision
            && engine
                .active
                .iter()
                .chain(engine.peers.values())
                .any(|actor| actor.reservation.world == world)
            && self.source.matches(engine, world)
            && self.victim.matches(engine, world, victim_alive)
    }

    pub(super) fn check_before_record<W: Write>(
        &self,
        engine: &Engine<W>,
    ) -> Result<(), &'static str> {
        if !self.current(engine, true)
            || self.awards.iter().any(|award| {
                Self::award(engine, self.scope.0, award.actor)
                    .is_none_or(|current| current.before != award.before)
            })
        {
            return Err("stale_firearm_assist_plan");
        }
        Ok(())
    }

    /// Called after the enclosing damage record and installed damage. Human
    /// awards use the event's staged ordinary statistic publication.
    pub(super) fn install<W: Write>(self, engine: &mut Engine<W>) -> Result<(), &'static str> {
        if !self.current(engine, self.after.is_some())
            || self.awards.iter().any(|award| {
                Self::award(engine, self.scope.0, award.actor).is_none_or(|current| {
                    award.reservation.is_none() && current.before != award.before
                })
            })
        {
            return Err("stale_firearm_assist_plan");
        }
        let state = &mut engine.firearm_assists;
        if state.scope != Some(self.scope) {
            state.clear_round();
            state.scope = Some(self.scope);
        }
        if let Some(after) = self.after {
            state
                .victims
                .insert(self.victim.actor.index, (self.victim, after));
            state.advance();
        } else {
            state.forget_victim(self.victim.actor.index);
        }
        for award in self.awards {
            match award.actor.allocation {
                Allocation::Human(allocation) => {
                    engine.queue_human_firearm_assist(award.actor.index, allocation, self.scope.0)
                }
                Allocation::Bot(_) => {
                    engine
                        .bot_registry
                        .bots
                        .get_mut(&award.actor.index)
                        .expect("validated current bot assist contributor")
                        .firearm_kd
                        .assists = award.after;
                }
            }
        }
        Ok(())
    }
}

impl<W: Write> Engine<W> {
    pub fn with_firearm_assist_rules(mut self, rules: Rules) -> Result<Self, &'static str> {
        rules.validate()?;
        if self.wire_profile.application_id() != rules.application_id {
            return Err("firearm_assist_rules_application_mismatch");
        }
        self.firearm_assists.clear_round();
        self.firearm_assists.rules = Some(rules);
        self.firearm_assists.advance();
        Ok(self)
    }

    /// Detached world damage candidates reach this only after their outer record.
    pub(super) fn prune_firearm_assist_victims(&mut self) {
        let Some((world, round)) = self.firearm_assists.scope else {
            return;
        };
        if round != self.round_number {
            self.firearm_assists.clear_round();
            return;
        }
        let mut victims = std::mem::take(&mut self.firearm_assists.victims);
        let original_len = victims.len();
        victims.retain(|_, (victim, _)| victim.matches(self, world, true));
        if victims.len() != original_len {
            self.firearm_assists.advance();
        }
        self.firearm_assists.victims = victims;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bot(index: u16) -> Actor {
        Actor {
            index,
            allocation: Allocation::Bot(1),
        }
    }

    #[test]
    fn lethal_sixth_contributor_replaces_first_on_expiry_tie_before_awarding() {
        let mut ledger = Ledger::default();
        for index in 1..=5 {
            ledger.remember(bot(index), 41);
        }
        ledger.remember(bot(6), 42);
        let awarded: Vec<_> = ledger
            .eligible(bot(6), 41)
            .map(|actor| actor.index)
            .collect();
        assert_eq!(awarded, [2, 3, 4, 5]);
        assert_eq!(ledger.eligible(bot(6), 42).count(), 0);
    }

    #[test]
    fn refresh_changes_expiry_without_claiming_another_slot() {
        let mut ledger = Ledger::default();
        for index in 1..=5 {
            ledger.remember(bot(index), 41);
        }
        ledger.remember(bot(1), 42);
        ledger.remember(bot(6), 42);
        let awarded: Vec<_> = ledger
            .eligible(bot(6), 41)
            .map(|actor| actor.index)
            .collect();
        assert_eq!(awarded, [1, 3, 4, 5]);
        assert_eq!(ledger.eligible(bot(6), 42).collect::<Vec<_>>(), [bot(1)]);
    }

    #[test]
    fn new_hit_from_replacement_slot_starts_its_own_credit_in_same_lane() {
        let mut ledger = Ledger::default();
        for index in 1..=5 {
            ledger.remember(bot(index), 41);
        }
        let replacement = Actor {
            index: 3,
            allocation: Allocation::Human(9),
        };
        ledger.remember(replacement, 42);
        let awarded: Vec<_> = ledger.eligible(bot(6), 41).collect();
        assert_eq!(awarded, [bot(1), bot(2), replacement, bot(4), bot(5)]);
    }
}
