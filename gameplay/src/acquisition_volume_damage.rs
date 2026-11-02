//! DamageVolume callback primitives. Pair membership/filtering stay external.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetKey {
    pub id: u64,
    pub generation: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Character,
    GroundVehicle,
    Mine,
    Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub key: TargetKey,
    pub kind: TargetKind,
    pub team: u8,
}
#[derive(Clone, Copy, Debug)]
pub struct Arguments {
    pub player_damage: f32,
    pub ground_vehicle_damage: f32,
    pub mine_damage: f32,
    pub apply_to_team: i32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MineMutation {
    Add(f32),
    ExceedThreshold,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    ActorDamage(f32),
    LethalActor,
    Mine(MineMutation),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageRequest {
    pub target: Target,
    pub effect: Effect,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MineState {
    pub accumulated: f32,
    pub threshold: f32,
    pub flags: u64,
}
/// Dispatch ONE currently resolved native penetration pair. The caller must
/// obtain the complete ordered pair list using qualified geometry/filter state,
/// resolve current identity/liveness for each pair, and preserve duplicates.
/// This function neither discovers overlaps nor applies death/explosion effects.
pub fn dispatch(
    arguments: Arguments,
    target: Target,
) -> Result<Option<DamageRequest>, &'static str> {
    if [
        arguments.player_damage,
        arguments.ground_vehicle_damage,
        arguments.mine_damage,
    ]
    .iter()
    .any(|v| !v.is_finite())
    {
        return Err("unsupported_volume_damage_number");
    }
    let value = match target.kind {
        TargetKind::Character => arguments.player_damage,
        TargetKind::GroundVehicle => arguments.ground_vehicle_damage,
        TargetKind::Mine => arguments.mine_damage,
        TargetKind::Other => return Ok(None),
    };
    if target.key.id == 0 || target.key.generation == 0 {
        return Err("invalid_volume_damage_target");
    }
    let excluded = 1_u8.wrapping_sub(arguments.apply_to_team as u8);
    if value == 0. || (target.kind != TargetKind::Mine && target.team == excluded) {
        return Ok(None);
    }
    let effect = if target.kind == TargetKind::Mine {
        Effect::Mine(if value < 0. {
            MineMutation::ExceedThreshold
        } else {
            MineMutation::Add(value)
        })
    } else if value < 0. {
        Effect::LethalActor
    } else {
        Effect::ActorDamage(value)
    };
    Ok(Some(DamageRequest { target, effect }))
}
impl MineState {
    /// Native accumulated-damage/flag mutation only. Does not synchronously
    /// explode/remove a mine; the later qualified mine Update owns that behavior.
    /// Finite admitted inputs/results are required; unsupported overflow fails.
    pub fn apply(self, mutation: MineMutation) -> Result<Self, &'static str> {
        if !self.accumulated.is_finite() || !self.threshold.is_finite() {
            return Err("unsupported_mine_state");
        }
        let increment = match mutation {
            MineMutation::Add(value) => {
                if !value.is_finite() || value < 0. {
                    return Err("unsupported_mine_damage");
                }
                if value == 0. {
                    return Ok(self);
                }
                value
            }
            MineMutation::ExceedThreshold => {
                let delta = self.threshold - self.accumulated;
                if !delta.is_finite() {
                    return Err("unsupported_mine_arithmetic");
                }
                if delta > 1. { delta + 1. } else { 2. }
            }
        };
        let accumulated = self.accumulated + increment;
        if !accumulated.is_finite() {
            return Err("unsupported_mine_arithmetic");
        }
        let flags = if self.threshold <= accumulated {
            self.flags | 0x80000000
        } else {
            self.flags
        };
        Ok(Self {
            accumulated,
            flags,
            ..self
        })
    }
}
