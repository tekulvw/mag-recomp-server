use super::{BombletSample, ClusterBlast, EffectRules, Work, next_sample};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrecisionStrikeRules {
    pub delay_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuidedArtilleryRules {
    pub delay_ms: u32,
    pub damage_delay_ms: u32,
    pub effective_radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlastFalloff {
    pub outer_radius: f32,
    pub outer_scale: f32,
    pub inner_radius: f32,
    pub inner_scale: f32,
}

impl BlastFalloff {
    fn validate(self, allow_coincident: bool) -> bool {
        positive(self.outer_radius)
            && positive(self.inner_radius)
            && self.inner_radius <= self.outer_radius
            && (allow_coincident || self.inner_radius < self.outer_radius)
            && (0. ..=1.).contains(&self.outer_scale)
            && (0. ..=1.).contains(&self.inner_scale)
    }

    fn blast(self, damage: f32) -> ClusterBlast {
        ClusterBlast {
            damage,
            material_penetration: 0.,
            inner_radius: self.inner_radius,
            inner_scale: if self.inner_scale == 0. {
                0.5
            } else {
                self.inner_scale
            },
            outer_radius: self.outer_radius,
            outer_scale: self.outer_scale,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GasBombardmentRules {
    pub delay_ms: u32,
    pub damage_delay_ms: u32,
    pub duration_ms: u32,
    pub interval_ms: u32,
    pub num_bomblets: u8,
    pub bomblet_damage: f32,
    pub bomblet: BlastFalloff,
    pub gas_damage_per_second: f32,
    pub gas: BlastFalloff,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StrafingRunRules {
    pub delay_ms: u32,
    pub damage_delay_ms: u32,
    pub duration_ms: u32,
    pub interval_ms: u32,
    pub strafe_length: f32,
    pub deviation_radius: f32,
    pub bomblet_damage: f32,
    pub bomblet: BlastFalloff,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GasVolume {
    pub origin: [f32; 3],
    pub duration_ms: u32,
    pub damage_per_second: f32,
    pub falloff: BlastFalloff,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PlatoonSchedule {
    Precision {
        due_ms: u32,
    },
    Guided {
        due_ms: u32,
        radius: f32,
        consumed: bool,
    },
    Gas {
        first_ms: u32,
        stop_ms: u32,
        ordinal: u16,
        rng: u64,
        rules: GasBombardmentRules,
    },
    Strafe {
        next_pass_ms: u32,
        period_ms: u32,
        pass: u8,
        elapsed_ms: u32,
        ordinal: u32,
        endpoints: Option<[[f32; 2]; 2]>,
        rng: u64,
        rules: StrafingRunRules,
    },
}

fn positive(value: f32) -> bool {
    value > 0. && value.is_finite()
}

fn deadline(start: u32, delta: u32) -> Result<u32, &'static str> {
    start
        .checked_add(delta)
        .ok_or("invalid_command_effect_deadline")
}

impl PlatoonSchedule {
    pub(super) fn new(start: u32, rules: EffectRules) -> Result<Self, &'static str> {
        if rules.delay_ms() == 0 {
            return Err("invalid_platoon_effect_delay");
        }
        match rules {
            EffectRules::PrecisionStrike(rules) => Ok(Self::Precision {
                due_ms: deadline(start, rules.delay_ms)?,
            }),
            EffectRules::GuidedArtillery(rules) if positive(rules.effective_radius) => {
                Ok(Self::Guided {
                    due_ms: deadline(deadline(start, rules.delay_ms)?, rules.damage_delay_ms)?,
                    radius: rules.effective_radius,
                    consumed: false,
                })
            }
            EffectRules::GasBombardment(rules)
                if rules.duration_ms > 0
                    && rules.interval_ms > 0
                    && rules.num_bomblets > 0
                    && positive(rules.bomblet_damage)
                    && rules.bomblet.validate(false)
                    && positive(rules.gas_damage_per_second)
                    && rules.gas.validate(true) =>
            {
                let base = deadline(start, rules.delay_ms)?;
                let first_ms = deadline(base, rules.damage_delay_ms)?;
                let last_offset = rules
                    .interval_ms
                    .checked_mul(u32::from(rules.num_bomblets))
                    .ok_or("invalid_command_effect_deadline")?;
                deadline(first_ms, last_offset)?;
                Ok(Self::Gas {
                    first_ms,
                    stop_ms: deadline(base, rules.duration_ms)?,
                    ordinal: 0,
                    rng: u64::from(start),
                    rules,
                })
            }
            EffectRules::StrafingRun(rules)
                if rules.duration_ms > 0
                    && rules.interval_ms > 0
                    && positive(rules.strafe_length)
                    && rules.deviation_radius.is_finite()
                    && rules.deviation_radius >= 0.
                    && positive(rules.bomblet_damage)
                    && rules.bomblet.validate(false) =>
            {
                let first_ms = deadline(start, rules.damage_delay_ms)?;
                let period_ms = deadline(rules.delay_ms, rules.duration_ms)?;
                deadline(
                    deadline(deadline(first_ms, period_ms)?, period_ms)?,
                    period_ms,
                )?;
                Ok(Self::Strafe {
                    next_pass_ms: first_ms,
                    period_ms,
                    pass: 0,
                    elapsed_ms: 0,
                    ordinal: 0,
                    endpoints: None,
                    rng: u64::from(start),
                    rules,
                })
            }
            _ => Err("invalid_platoon_effect_rules"),
        }
    }

    pub(super) fn retain(&mut self, previous: Self, successful: bool) -> Result<(), &'static str> {
        match (self, previous) {
            (Self::Precision { .. }, Self::Precision { .. }) => {}
            (
                Self::Guided { consumed, .. },
                Self::Guided {
                    consumed: previous, ..
                },
            ) => {
                *consumed = !successful && previous;
            }
            (
                Self::Gas { ordinal, rng, .. },
                Self::Gas {
                    ordinal: old_ordinal,
                    rng: old_rng,
                    ..
                },
            ) => {
                if !successful {
                    *ordinal = old_ordinal;
                    *rng = old_rng;
                }
            }
            (
                Self::Strafe { endpoints, .. },
                Self::Strafe {
                    endpoints: previous,
                    ..
                },
            ) => {
                *endpoints = previous;
            }
            _ => return Err("command_effect_instance_mismatch"),
        }
        Ok(())
    }

    pub(super) fn tick(
        &mut self,
        now_ms: u32,
        delta_ms: u32,
        successful: bool,
        origin: [f32; 3],
        alternate_spawn: Option<[f32; 3]>,
    ) -> (bool, Work) {
        match self {
            Self::Precision { due_ms } if now_ms > *due_ms => (
                true,
                if successful {
                    Work::PrecisionStrike
                } else {
                    Work::Complete
                },
            ),
            Self::Guided {
                due_ms,
                radius,
                consumed,
            } if now_ms >= *due_ms => {
                // The retained consumed flag, rather than the success byte, gates DropBomb.
                let work = if *consumed {
                    Work::Complete
                } else {
                    Work::GuidedArtillery {
                        origin,
                        radius: *radius,
                    }
                };
                *consumed = true;
                (true, work)
            }
            Self::Gas {
                first_ms,
                stop_ms,
                ordinal,
                rng,
                rules,
            } => {
                let count = u16::from(rules.num_bomblets);
                let mut work = Work::Idle;
                // The slot limit is inclusive. The distinct stop check below uses >=.
                if *ordinal <= count
                    && now_ms >= *first_ms + rules.interval_ms * u32::from(*ordinal)
                {
                    // The gas field is installed on slot zero before the bomblet success check.
                    let volume = (*ordinal == 0).then_some(GasVolume {
                        origin,
                        duration_ms: rules.duration_ms,
                        damage_per_second: rules.gas_damage_per_second,
                        falloff: rules.gas,
                    });
                    let bomblet = successful.then(|| {
                        let radius = next_sample(rng) * 0.75 * rules.bomblet.inner_radius;
                        let angle = next_sample(rng) * std::f32::consts::TAU;
                        let (sin, cos) = angle.sin_cos();
                        BombletSample {
                            xz: [origin[0] + sin * radius, origin[2] + cos * radius],
                            blast: rules.bomblet.blast(rules.bomblet_damage),
                        }
                    });
                    *ordinal += 1;
                    if volume.is_some() || bomblet.is_some() {
                        work = Work::GasBombardment { volume, bomblet };
                    }
                }
                let complete = now_ms >= *stop_ms && *ordinal >= count;
                (
                    complete,
                    if complete && work == Work::Idle {
                        Work::Complete
                    } else {
                        work
                    },
                )
            }
            Self::Strafe {
                next_pass_ms,
                period_ms,
                pass,
                elapsed_ms,
                ordinal,
                endpoints,
                rng,
                rules,
            } => {
                if now_ms >= *next_pass_ms {
                    if *pass == 3 {
                        return (true, Work::Complete);
                    }
                    if let Some(spawn) = alternate_spawn {
                        *endpoints =
                            Some(strafe_endpoints(origin, spawn, *pass, rules.strafe_length));
                    }
                    *pass += 1;
                    *next_pass_ms += *period_ms;
                    *elapsed_ms = 0;
                    *ordinal = 0;
                }
                let mut work = Work::Idle;
                if *pass > 0
                    && *elapsed_ms < *period_ms
                    && *elapsed_ms
                        > rules
                            .delay_ms
                            .saturating_add(rules.interval_ms.saturating_mul(*ordinal))
                {
                    if successful {
                        let deviation = (next_sample(rng) * -2.)
                            .mul_add(rules.deviation_radius, rules.deviation_radius);
                        if let Some([start, end]) = *endpoints {
                            let fraction =
                                (*elapsed_ms - rules.delay_ms) as f32 / rules.duration_ms as f32;
                            let perpendicular =
                                normalized_xz([end[1] - start[1], start[0] - end[0]]);
                            let x = (end[0] - start[0]).mul_add(fraction, start[0]);
                            let z = (end[1] - start[1]).mul_add(fraction, start[1]);
                            work = Work::StrafingRun(BombletSample {
                                xz: [
                                    x + perpendicular[0] * deviation,
                                    z + perpendicular[1] * deviation,
                                ],
                                blast: rules.bomblet.blast(rules.bomblet_damage),
                            });
                        }
                    }
                    *ordinal += 1;
                }
                *elapsed_ms = elapsed_ms.saturating_add(delta_ms);
                (false, work)
            }
            _ => (false, Work::Idle),
        }
    }
}

fn strafe_endpoints(origin: [f32; 3], spawn: [f32; 3], pass: u8, length: f32) -> [[f32; 2]; 2] {
    let [x, y, z] = std::array::from_fn(|i| origin[i] - spawn[i]);
    let square = x * x + y * y + z * z;
    let [x, z] = if square > 1e-30 {
        let inverse = square.sqrt().recip();
        [x * inverse, z * inverse]
    } else {
        [0., 1.]
    };
    let (sin, cos) = [0.0_f32, -1.047, 1.047][usize::from(pass)].sin_cos();
    let offset_x = x.mul_add(cos, -(z * sin)) * (length * 0.5);
    let offset_z = x.mul_add(sin, z * cos) * (length * 0.5);
    [
        [origin[0] + offset_x, origin[2] + offset_z],
        [origin[0] - offset_x, origin[2] - offset_z],
    ]
}

fn normalized_xz([x, z]: [f32; 2]) -> [f32; 2] {
    let square = x * x + z * z;
    if square > 1e-30 {
        let inverse = square.sqrt().recip();
        [x * inverse, z * inverse]
    } else {
        [0., 1.]
    }
}
