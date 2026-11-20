//! Round-owned command effect scheduling. World observations and publication
//! stay with the caller that owns the transaction and combat state.

use crate::active_abilities::{Ability, Side};
use crate::active_ability_sensor::{
    DEFENDER_SQUAD_SLOTS, Detector, SensorRules, SquadScan, Surveillance,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClusterRules {
    pub delay_ms: u32,
    pub damage_delay_ms: u32,
    pub interval_ms: u32,
    pub count: u8,
    pub scatter_radius: f32,
    pub damage: f32,
    pub inner_radius: f32,
    pub inner_scale: f32,
    pub outer_radius: f32,
    pub outer_scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectRules {
    Cluster(ClusterRules),
    Sensor(SensorRules),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterBlast {
    pub damage: f32,
    pub material_penetration: f32,
    pub inner_radius: f32,
    pub inner_scale: f32,
    pub outer_radius: f32,
    pub outer_scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterSample {
    pub ordinal: u8,
    pub xz: [f32; 2],
    pub blast: ClusterBlast,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Work {
    Idle,
    ClusterSample(ClusterSample),
    ScanSquad(SquadScan),
    ClearSurveillance,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Schedule {
    Cluster {
        first_ms: u32,
        interval_ms: u32,
        count: u8,
        ordinal: u8,
        rng: u64,
        scatter_radius: f32,
        blast: ClusterBlast,
    },
    AttackerSensor {
        available_after_ms: u32,
        expire_after_ms: u32,
        detector: Detector,
        slot: u16,
    },
    DefenderSensor {
        expire_after_ms: u32,
        detector: Detector,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effect {
    now_ms: u32,
    side: Side,
    successful: bool,
    origin: [f32; 3],
    schedule: Schedule,
    complete: bool,
}

impl Effect {
    pub fn new(
        ability: Ability,
        side: Side,
        activate_ms: u32,
        successful: bool,
        accepted_xyz: [f32; 3],
        rules: EffectRules,
    ) -> Result<Self, &'static str> {
        if !accepted_xyz.into_iter().all(f32::is_finite) {
            return Err("invalid_command_effect_origin");
        }
        let deadline = |base: u32, delay: u32| {
            base.checked_add(delay)
                .ok_or("invalid_command_effect_deadline")
        };
        let schedule = match (ability, rules) {
            (Ability::ClusterBomb, EffectRules::Cluster(rules)) => {
                if rules.count == 0
                    || rules.interval_ms == 0
                    || rules.delay_ms == 0
                    || ![
                        rules.scatter_radius,
                        rules.damage,
                        rules.inner_radius,
                        rules.outer_radius,
                    ]
                    .into_iter()
                    .all(|value| value.is_finite() && value > 0.)
                    || rules.inner_radius >= rules.outer_radius
                    || ![0., 0.5].contains(&rules.inner_scale)
                    || rules.outer_scale != 0.
                {
                    return Err("invalid_cluster_bomb_rules");
                }
                let first_ms = deadline(
                    deadline(activate_ms, rules.delay_ms)?,
                    rules.damage_delay_ms,
                )?;
                let last_offset = u32::from(rules.count - 1)
                    .checked_mul(rules.interval_ms)
                    .ok_or("invalid_command_effect_deadline")?;
                deadline(first_ms, last_offset)?;
                Schedule::Cluster {
                    first_ms,
                    interval_ms: rules.interval_ms,
                    count: rules.count,
                    ordinal: 0,
                    rng: u64::from(activate_ms),
                    scatter_radius: rules.scatter_radius,
                    blast: ClusterBlast {
                        damage: rules.damage,
                        material_penetration: 0.,
                        inner_radius: rules.inner_radius,
                        inner_scale: 0.5,
                        outer_radius: rules.outer_radius,
                        outer_scale: rules.outer_scale,
                    },
                }
            }
            (Ability::Sensor, EffectRules::Sensor(rules)) => {
                if rules.delay_ms == 0 || rules.duration_ms == 0 {
                    return Err("invalid_command_sensor_duration");
                }
                let attacker = Detector::new(rules.attacker_radius, rules.speed_threshold)?;
                let defender = Detector::new(rules.defender_radius, rules.speed_threshold)?;
                match side {
                    Side::Attacker => {
                        let available_after_ms = deadline(activate_ms, rules.delay_ms)?;
                        Schedule::AttackerSensor {
                            available_after_ms,
                            expire_after_ms: deadline(available_after_ms, rules.duration_ms)?,
                            detector: attacker,
                            slot: DEFENDER_SQUAD_SLOTS.start,
                        }
                    }
                    Side::Defender => Schedule::DefenderSensor {
                        expire_after_ms: deadline(activate_ms, rules.duration_ms)?,
                        detector: defender,
                    },
                }
            }
            _ => return Err("unsupported_command_effect"),
        };
        Ok(Self {
            now_ms: activate_ms,
            side,
            successful,
            origin: accepted_xyz,
            schedule,
            complete: false,
        })
    }

    /// Start another activation on the same registered native ability instance.
    /// Failed Cluster activation leaves its old bomblet state intact, and
    /// Sensor retains its attacker squad cursor across all activations.
    pub fn rearm(
        &self,
        ability: Ability,
        side: Side,
        activate_ms: u32,
        successful: bool,
        accepted_xyz: [f32; 3],
        rules: EffectRules,
    ) -> Result<Self, &'static str> {
        if activate_ms < self.now_ms {
            return Err("command_effect_time_regressed");
        }
        if side != self.side {
            return Err("command_effect_instance_mismatch");
        }
        let mut next = Self::new(ability, side, activate_ms, successful, accepted_xyz, rules)?;
        match (self.schedule, &mut next.schedule) {
            (Schedule::Cluster { .. }, Schedule::Cluster { .. }) if successful => {}
            (
                Schedule::Cluster {
                    ordinal: previous,
                    rng: previous_rng,
                    ..
                },
                Schedule::Cluster { ordinal, rng, .. },
            ) => {
                *ordinal = previous;
                *rng = previous_rng;
            }
            (
                Schedule::AttackerSensor { slot: previous, .. },
                Schedule::AttackerSensor { slot, .. },
            ) => *slot = previous,
            (Schedule::DefenderSensor { .. }, Schedule::DefenderSensor { .. }) => {}
            _ => return Err("command_effect_instance_mismatch"),
        }
        Ok(next)
    }

    /// Prepare once alongside the recorded activation transaction.
    pub fn start_work(&self) -> Option<Surveillance> {
        match self.schedule {
            Schedule::DefenderSensor { detector, .. } if self.successful && !self.complete => {
                Some(Surveillance { detector })
            }
            _ => None,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Prepare one native manager update from the retained state. The caller
    /// installs the returned state only after the corresponding work commits.
    pub fn prepare_tick(&self, now_ms: u32) -> Result<(Self, Work), &'static str> {
        if now_ms < self.now_ms {
            return Err("command_effect_time_regressed");
        }
        let mut next = Self { now_ms, ..*self };
        if self.complete {
            return Ok((next, Work::Complete));
        }
        let work = match &mut next.schedule {
            Schedule::Cluster { count, ordinal, .. } if *ordinal >= *count => {
                next.complete = true;
                Work::Complete
            }
            Schedule::Cluster {
                first_ms,
                interval_ms,
                count,
                ordinal,
                rng,
                scatter_radius,
                blast,
            } if now_ms >= *first_ms + *interval_ms * u32::from(*ordinal) => {
                let work = if self.successful {
                    let radius = next_sample(rng) * *scatter_radius;
                    let angle = next_sample(rng) * std::f32::consts::TAU;
                    let [x, _, z] = self.origin;
                    let (sin, cos) = angle.sin_cos();
                    Work::ClusterSample(ClusterSample {
                        ordinal: *ordinal,
                        xz: [x + sin * radius, z + cos * radius],
                        blast: *blast,
                    })
                } else {
                    Work::Idle
                };
                *ordinal += 1;
                if *ordinal == *count {
                    next.complete = true;
                    if !self.successful {
                        Work::Complete
                    } else {
                        work
                    }
                } else {
                    work
                }
            }
            Schedule::AttackerSensor {
                expire_after_ms, ..
            } if now_ms > *expire_after_ms => {
                next.complete = true;
                Work::Complete
            }
            Schedule::AttackerSensor {
                available_after_ms,
                detector,
                slot,
                ..
            } if self.successful && now_ms > *available_after_ms => {
                let work = Work::ScanSquad(SquadScan {
                    slot: *slot,
                    origin: self.origin,
                    detector: *detector,
                });
                *slot += 1;
                if *slot == DEFENDER_SQUAD_SLOTS.end {
                    *slot = DEFENDER_SQUAD_SLOTS.start;
                }
                work
            }
            Schedule::DefenderSensor {
                expire_after_ms, ..
            } if now_ms > *expire_after_ms => {
                next.complete = true;
                if self.successful {
                    Work::ClearSurveillance
                } else {
                    Work::Complete
                }
            }
            _ => Work::Idle,
        };
        Ok((next, work))
    }
}

fn next_sample(state: &mut u64) -> f32 {
    *state = state.wrapping_mul(16_807).wrapping_add(1) % u64::from(u32::MAX);
    (*state as f32) * (1. / 4_294_967_296.)
}
