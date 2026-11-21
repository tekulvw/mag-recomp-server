//! Round-local counters on registry bot allocations.
use super::*;

// Original retail CharacterStatType ordinals.
const KILLS: u8 = 10;
const DEATHS: u8 = 12;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Totals {
    pub(super) kills: i32,
    pub(super) assists: i32,
    pub(super) deaths: i32,
}

impl Totals {
    pub(super) fn encode(
        self,
        index: u16,
        profile: wire_profile::WireProfile,
        include_assists: bool,
    ) -> Result<Vec<u8>, &'static str> {
        let mut stats = vec![protocol::StatValue {
            stat: KILLS,
            value: self.kills,
        }];
        if include_assists {
            stats.push(protocol::StatValue {
                stat: firearm_assists::STAT,
                value: self.assists,
            });
        }
        stats.push(protocol::StatValue {
            stat: DEATHS,
            value: self.deaths,
        });
        protocol::StatUpdate {
            character_index: index,
            stats,
        }
        .encode_with_profile(profile)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Life {
    pub(super) index: u16,
    pub(super) generation: u32,
    pub(super) team: u8,
    pub(super) health: u16,
}

struct Change {
    life: Life,
    allocation_generation: u32,
    is_death: bool,
    before: Totals,
    after: Totals,
}

pub(super) struct Plan {
    world: u32,
    round: u64,
    changes: Vec<Change>,
}

impl Plan {
    pub(super) fn prepare<W: Write>(
        engine: &Engine<W>,
        world: u32,
        killer: Option<Life>,
        victim: Option<Life>,
    ) -> Result<Self, &'static str> {
        if engine.wire_profile.application_id() != 20231
            || engine.match_terminal.is_some()
            || !Self::current_world(engine, world)
            || killer.zip(victim).is_some_and(|(killer, victim)| {
                killer.index == victim.index || killer.team == victim.team
            })
        {
            return Err("invalid_bot_firearm_kd_scope");
        }
        let mut changes = Vec::new();
        for (life, is_kill) in [(killer, true), (victim, false)] {
            let Some(life) = life else { continue };
            let bot = engine
                .bot_registry
                .bots
                .get(&life.index)
                .filter(|bot| {
                    bot.health != 0
                        && bot.health == life.health
                        && bot.target.squad.team() == life.team
                        && bot.target.runtime.is_some_and(|runtime| {
                            runtime.generation == life.generation
                                && !matches!(runtime.phase, development_target::BotPhase::Dead)
                        })
                })
                .ok_or("stale_bot_firearm_kd_life")?;
            let before = bot.firearm_kd;
            let after = Totals {
                kills: before.kills.saturating_add(i32::from(is_kill)),
                assists: before.assists,
                deaths: before.deaths.saturating_add(i32::from(!is_kill)),
            };
            // Validate the bounded absolute row before the firearm commits.
            after.encode(
                life.index,
                engine.wire_profile,
                engine.firearm_assists.enabled(),
            )?;
            changes.push(Change {
                life,
                allocation_generation: bot.allocation_generation,
                is_death: !is_kill,
                before,
                after,
            });
        }
        Ok(Self {
            world,
            round: engine.round_number,
            changes,
        })
    }

    pub(super) fn audit(&self) -> Vec<serde_json::Value> {
        self.changes.iter().map(|change| json!({
            "object_index":change.life.index,"allocation_generation":change.allocation_generation,
            "life_generation":change.life.generation,
            "kills_before":change.before.kills,"kills_after":change.after.kills,
            "deaths_before":change.before.deaths,"deaths_after":change.after.deaths,
        })).collect()
    }

    pub(super) fn install<W: Write>(self, engine: &mut Engine<W>) -> Result<(), &'static str> {
        if self.round != engine.round_number
            || !Self::current_world(engine, self.world)
            || self.changes.iter().any(|change| {
                engine
                    .bot_registry
                    .bots
                    .get(&change.life.index)
                    .is_none_or(|bot| {
                        bot.allocation_generation != change.allocation_generation
                            || bot.firearm_kd != change.before
                            || bot.target.squad.team() != change.life.team
                            || bot.health
                                != if change.is_death {
                                    0
                                } else {
                                    change.life.health
                                }
                            || bot.target.runtime.is_none_or(|runtime| {
                                runtime.generation != change.life.generation
                                    || matches!(runtime.phase, development_target::BotPhase::Dead)
                                        != change.is_death
                            })
                    })
            })
        {
            return Err("stale_bot_firearm_kd_plan");
        }
        for change in self.changes {
            engine
                .bot_registry
                .bots
                .get_mut(&change.life.index)
                .unwrap()
                .firearm_kd = change.after;
        }
        Ok(())
    }

    fn current_world<W: Write>(engine: &Engine<W>, world: u32) -> bool {
        engine
            .active
            .iter()
            .chain(engine.peers.values())
            .next()
            .is_some_and(|actor| actor.reservation.world == world)
    }
}
