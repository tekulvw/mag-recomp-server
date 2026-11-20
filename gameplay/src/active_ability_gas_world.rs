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

#[derive(Clone)]
pub(crate) struct GasSource {
    pub attacker: u32,
    pub world: u32,
    pub team: u8,
    pub reservation: String,
}

const GAS_INTERVAL: Duration = Duration::from_millis(500);
const MAX_GAS_VOLUMES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pulse {
    Waiting,
    Damage,
    Expired,
}

#[derive(Clone)]
struct PulseTiming {
    expires: Duration,
    last_update: Duration,
    accumulated: Duration,
}

impl PulseTiming {
    fn new(now: Duration, duration_ms: u32) -> Self {
        Self {
            expires: now.saturating_add(Duration::from_millis(u64::from(duration_ms))),
            last_update: now,
            accumulated: Duration::ZERO,
        }
    }

    fn advance(&mut self, now: Duration) -> Pulse {
        if now > self.expires {
            return Pulse::Expired;
        }
        self.accumulated += now.saturating_sub(self.last_update);
        self.last_update = self.last_update.max(now);
        if self.accumulated < GAS_INTERVAL {
            return Pulse::Waiting;
        }
        self.accumulated -= GAS_INTERVAL;
        Pulse::Damage
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
    reservation: Option<String>,
    expires: Duration,
}

#[derive(Clone, Default)]
pub(crate) struct GasWorld {
    clouds: VecDeque<Cloud>,
    cooldowns: BTreeMap<(u32, u32), Cooldown>,
}

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

    fn cooling_down(&self, target: Target, reservation: Option<&str>, now: Duration) -> bool {
        self.cooldowns
            .get(&(target.world, target.character_id))
            .is_some_and(|cooldown| {
                cooldown.generation == target.generation
                    && cooldown.reservation.as_deref() == reservation
                    && now < cooldown.expires
            })
    }

    fn mark_applied(&mut self, target: Target, reservation: Option<&str>, now: Duration) {
        self.cooldowns.insert(
            (target.world, target.character_id),
            Cooldown {
                generation: target.generation,
                reservation: reservation.map(str::to_owned),
                expires: now.saturating_add(GAS_INTERVAL),
            },
        );
    }

    /// Run the persistent SimDmgVolume hold before registering volumes from
    /// this update's new ability work. The outer candidate owns this copy.
    pub(crate) fn stage<W: Write>(
        &mut self,
        engine: &mut Engine<W>,
        native: &mut BTreeMap<u32, NativeState>,
        now: Duration,
        collision: BlastCollision<'_>,
    ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str> {
        self.cooldowns.retain(|_, cooldown| now < cooldown.expires);
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
        let mut audit = Vec::new();
        let mut messages = Vec::new();
        for (volume, source) in pulses {
            let source_is_present =
                engine
                    .active
                    .iter()
                    .chain(engine.peers.values())
                    .any(|actor| {
                        actor.reservation.world == source.world
                            && (0x0380_0000 | u32::from(actor.reservation.player))
                                == source.attacker
                            && actor.reservation.id == source.reservation
                    });
            let attacker = if source_is_present {
                source.attacker
            } else {
                u32::MAX
            };
            let observations = engine.collect_gas_pulse_characters(volume, collision)?;
            audit.push(json!({
                "op": "gas_bombardment_pulse", "world": source.world, "attacker": attacker,
                "origin": volume.origin, "pairs": observations.len(),
            }));
            let previous_team = engine.active_ability_damage_team;
            engine.active_ability_damage_team =
                (attacker != u32::MAX).then_some((attacker, source.world, source.team));
            let result = (|| {
                for pair in observations {
                    let BlastTarget::Character(target) = pair.target else {
                        return Err("invalid_gas_volume_target");
                    };
                    if target.world != source.world {
                        return Err("invalid_gas_volume_world");
                    }
                    let reservation = engine
                        .active
                        .iter()
                        .chain(engine.peers.values())
                        .find(|actor| {
                            actor.reservation.world == target.world
                                && actor.reservation.player == target.character_id as u16
                        })
                        .map(|actor| actor.reservation.id.clone());
                    if self.cooling_down(target, reservation.as_deref(), now) {
                        continue;
                    }
                    let displacement: [f32; 3] = std::array::from_fn(|axis| {
                        pair.sample_position[axis] - volume.origin[axis]
                    });
                    let [x, y, z] = displacement;
                    let scale =
                        native_command_falloff(volume.falloff, ((x * x + y * y) + z * z).sqrt())?;
                    let state = native
                        .get_mut(&target.character_id)
                        .ok_or("gas_volume_native_state_missing")?;
                    let plan = engine.plan_acquisition_character_effect(
                        target,
                        Request::AirStrikeGas {
                            raw_hp: (volume.damage_per_second * 0.5) * scale,
                            from_direction: displacement,
                            attacker,
                        },
                        *state,
                        now,
                    )?;
                    let applied = plan.outcome() == Outcome::Applied;
                    audit.push(plan.audit().clone());
                    messages.extend(engine.commit_acquisition_character_effect(plan, state)?);
                    if applied {
                        self.mark_applied(target, reservation.as_deref(), now);
                    }
                }
                Ok(())
            })();
            engine.active_ability_damage_team = previous_team;
            result?;
        }
        Ok((audit, messages))
    }
}

#[cfg(test)]
mod tests;
