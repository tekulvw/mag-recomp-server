//! Retail persistent gas pulses and per-character damage-volume admission.
use crate::{
    Engine, OutboundMessage,
    acquisition_character_effects::{NativeState, Outcome, Request, Target},
    active_ability_effects::GasVolume,
    domination_structural_world::{BlastCollision, BlastTarget, native_command_falloff},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    io::Write,
    time::Duration,
};

#[derive(Clone, Debug)]
pub(crate) struct GasSource {
    pub allocation: Option<u64>,
    pub attacker: u32,
    pub world: u32,
    pub team: u8,
    pub reservation: String,
}

const GAS_INTERVAL: Duration = Duration::from_millis(500);
const MAX_GAS_VOLUMES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pulse {
    Waiting,
    Damage,
    Expired,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PulseTiming {
    expires: Duration,
    damage_at: Duration,
    interval: Duration,
    last_update: Duration,
    accumulated: Duration,
}

impl PulseTiming {
    fn new(now: Duration, duration_ms: u32) -> Self {
        Self::grenade(now, duration_ms, Duration::ZERO, GAS_INTERVAL)
    }

    pub(crate) fn grenade(
        now: Duration,
        duration_ms: u32,
        warmup: Duration,
        interval: Duration,
    ) -> Self {
        Self {
            expires: now.saturating_add(Duration::from_millis(u64::from(duration_ms))),
            damage_at: now.saturating_add(warmup),
            interval,
            last_update: now,
            accumulated: Duration::ZERO,
        }
    }

    pub(crate) fn advance(&mut self, now: Duration) -> Pulse {
        if now > self.expires {
            return Pulse::Expired;
        }
        self.accumulated += now.saturating_sub(self.last_update);
        self.last_update = self.last_update.max(now);
        if self.accumulated < self.interval {
            return Pulse::Waiting;
        }
        self.accumulated -= self.interval;
        if now < self.damage_at {
            Pulse::Waiting
        } else {
            Pulse::Damage
        }
    }
}

#[derive(Clone)]
struct Cloud {
    volume: GasVolume,
    source: GasSource,
    timing: PulseTiming,
}

#[derive(Clone)]
struct Cooldown {
    generation: u32,
    allocation: TargetAllocation,
    expires: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TargetAllocation {
    Human(u64, String),
    Bot(u32),
}

#[derive(Clone, Default)]
pub(crate) struct GasWorld {
    clouds: VecDeque<Cloud>,
}

#[derive(Clone, Default)]
pub(crate) struct GasCooldowns(BTreeMap<(u64, u32, u32), Cooldown>);

impl GasWorld {
    pub(crate) fn register(
        &mut self,
        volume: GasVolume,
        source: GasSource,
        now: Duration,
    ) -> Result<(), &'static str> {
        if !volume.origin.iter().all(|axis| axis.is_finite())
            || !volume.damage_per_second.is_finite()
            || volume.damage_per_second <= 0.
            || volume.duration_ms == 0
            || source.team > 1
        {
            return Err("invalid_gas_volume");
        }
        native_command_falloff(volume.falloff, 0.)?;
        if self.clouds.len() == MAX_GAS_VOLUMES {
            self.clouds.pop_front();
        }
        self.clouds.push_back(Cloud {
            volume,
            source,
            timing: PulseTiming::new(now, volume.duration_ms),
        });
        Ok(())
    }
}

impl GasCooldowns {
    fn cooling_down(
        &self,
        round: u64,
        target: Target,
        allocation: &TargetAllocation,
        now: Duration,
    ) -> bool {
        self.0
            .get(&(round, target.world, target.character_id))
            .is_some_and(|cooldown| {
                cooldown.generation == target.generation
                    && cooldown.allocation == *allocation
                    && now < cooldown.expires
            })
    }

    fn mark_applied(
        &mut self,
        round: u64,
        target: Target,
        allocation: TargetAllocation,
        now: Duration,
    ) {
        self.0.insert(
            (round, target.world, target.character_id),
            Cooldown {
                generation: target.generation,
                allocation,
                expires: now.saturating_add(GAS_INTERVAL),
            },
        );
    }
}

impl GasWorld {
    /// Run the persistent SimDmgVolume hold before registering volumes from
    /// this update's new ability work. The outer candidate owns this copy.
    pub(crate) fn stage<W: Write>(
        &mut self,
        engine: &mut Engine<W>,
        native: &mut BTreeMap<u32, NativeState>,
        now: Duration,
        collision: BlastCollision<'_>,
    ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str> {
        let mut pulses = Vec::new();
        self.clouds
            .retain_mut(|cloud| match cloud.timing.advance(now) {
                Pulse::Damage => {
                    pulses.push((cloud.volume, cloud.source.clone()));
                    true
                }
                Pulse::Waiting => true,
                Pulse::Expired => false,
            });
        engine.stage_gas_pulses(
            pulses.into_iter().map(|(volume, source)| GasPulse {
                origin: volume.origin,
                falloff: volume.falloff,
                raw_hp: volume.damage_per_second * 0.5,
                source,
                operation: "gas_bombardment_pulse",
            }),
            native,
            now,
            collision,
        )
    }
}

pub(crate) struct GasPulse {
    pub origin: [f32; 3],
    pub falloff: crate::active_ability_effects::BlastFalloff,
    pub raw_hp: f32,
    pub source: GasSource,
    pub operation: &'static str,
}

impl<W: Write> Engine<W> {
    pub(crate) fn stage_gas_pulses(
        &mut self,
        pulses: impl IntoIterator<Item = GasPulse>,
        native: &mut BTreeMap<u32, NativeState>,
        now: Duration,
        collision: BlastCollision<'_>,
    ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str> {
        self.gas_cooldowns
            .0
            .retain(|_, cooldown| now < cooldown.expires);
        let mut audit = Vec::new();
        let mut messages = Vec::new();
        for GasPulse {
            origin,
            falloff,
            raw_hp,
            source,
            operation,
        } in pulses
        {
            let source_is_present = self.active.iter().chain(self.peers.values()).any(|actor| {
                actor.reservation.world == source.world
                    && (0x0380_0000 | u32::from(actor.reservation.player)) == source.attacker
                    && actor.reservation.id == source.reservation
                    && source
                        .allocation
                        .is_none_or(|allocation| actor.allocation_sequence == allocation)
            });
            let attacker = if source_is_present {
                source.attacker
            } else {
                u32::MAX
            };
            let observations =
                self.collect_gas_pulse_characters(origin, falloff, raw_hp, collision)?;
            audit.push(json!({
                "op": operation, "world": source.world, "attacker": attacker,
                "origin": origin, "pairs": observations.len(),
            }));
            let previous_team = self.active_ability_damage_team;
            self.active_ability_damage_team =
                (attacker != u32::MAX).then_some((attacker, source.world, source.team));
            let result = (|| {
                for pair in observations {
                    let BlastTarget::Character(target) = pair.target else {
                        return Err("invalid_gas_volume_target");
                    };
                    if target.world != source.world {
                        return Err("invalid_gas_volume_world");
                    }
                    let allocation = self
                        .active
                        .iter()
                        .chain(self.peers.values())
                        .find(|actor| {
                            actor.reservation.world == target.world
                                && actor.reservation.player == target.character_id as u16
                        })
                        .map(|actor| {
                            TargetAllocation::Human(
                                actor.allocation_sequence,
                                actor.reservation.id.clone(),
                            )
                        })
                        .or_else(|| {
                            self.bot_registry
                                .bots
                                .get(&(target.character_id as u16))
                                .map(|bot| TargetAllocation::Bot(bot.allocation_generation))
                        })
                        .ok_or("gas_volume_target_allocation_missing")?;
                    if self
                        .gas_cooldowns
                        .cooling_down(self.round_number, target, &allocation, now)
                    {
                        continue;
                    }
                    let displacement: [f32; 3] =
                        std::array::from_fn(|axis| pair.sample_position[axis] - origin[axis]);
                    let [x, y, z] = displacement;
                    let scale = native_command_falloff(falloff, ((x * x + y * y) + z * z).sqrt())?;
                    let state = native
                        .get_mut(&target.character_id)
                        .ok_or("gas_volume_native_state_missing")?;
                    let plan = self.plan_acquisition_character_effect(
                        target,
                        Request::AirStrikeGas {
                            raw_hp: raw_hp * scale,
                            from_direction: displacement,
                            attacker,
                        },
                        *state,
                        now,
                    )?;
                    let applied = plan.outcome() == Outcome::Applied;
                    audit.push(plan.audit().clone());
                    messages.extend(self.commit_acquisition_character_effect(plan, state)?);
                    if applied {
                        self.gas_cooldowns
                            .mark_applied(self.round_number, target, allocation, now);
                    }
                }
                Ok(())
            })();
            self.active_ability_damage_team = previous_team;
            result?;
        }
        Ok((audit, messages))
    }
}

#[cfg(test)]
mod tests;
