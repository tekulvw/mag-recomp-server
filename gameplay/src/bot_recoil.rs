//! Standing hip-fire model using externally supplied weapon parameters.
//! No weapon constants, native code or original random sequence is embedded.
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug)]
pub(super) struct Profile {
    fire: f64,
    recover: f64,
    side_fire: f64,
    side_recover: f64,
    spread_fire: f64,
    spread_recover: f64,
    move_spread: f64,
    rotate_spread: f64,
    move_recover: f64,
    cone: [f64; 2],
    pitch: [f64; 2],
    yaw: [f64; 2],
    normalization: i32,
}

fn table(value: &Value) -> Result<Map<String, Value>, &'static str> {
    let source = value.as_object().ok_or("invalid_weapon_reticule_table")?;
    if source.len() > 128 {
        return Err("excessive_weapon_reticule_fields");
    }
    if let Some(rows) = source.get("$lua_table") {
        let mut output = Map::new();
        let rows = rows
            .as_array()
            .filter(|rows| rows.len() <= 128)
            .ok_or("invalid_weapon_reticule_table")?;
        for row in rows {
            let key = row["key"].as_str().ok_or("invalid_weapon_reticule_key")?;
            let value = row.get("value").ok_or("invalid_weapon_reticule_value")?;
            // Some extracted source tables retain identical duplicate keys.
            // A conflicting duplicate requires an explicit acquisition decision.
            if output.get(key).is_some_and(|old| old != value) {
                return Err("ambiguous_weapon_reticule_key");
            }
            output.insert(key.into(), value.clone());
        }
        Ok(output)
    } else {
        Ok(source.clone())
    }
}

impl Profile {
    pub(super) fn from_weapon(parameters: &Value) -> Result<Self, &'static str> {
        let p = table(&parameters["reticule_params_stand"])?;
        let number = |name: &str| {
            p.get(name)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && v.abs() <= 3600.)
                .ok_or("invalid_weapon_reticule_number")
        };
        let nonnegative = |name: &str| {
            number(name).and_then(|v| {
                if v >= 0. {
                    Ok(v)
                } else {
                    Err("negative_weapon_reticule_rate")
                }
            })
        };
        let pair = |name: &str| -> Result<[f64; 2], &'static str> {
            let a = p
                .get(name)
                .and_then(Value::as_array)
                .filter(|a| a.len() == 2)
                .ok_or("invalid_weapon_reticule_bounds")?;
            let lo = a[0]
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or("invalid_weapon_reticule_bounds")?;
            let hi = a[1]
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or("invalid_weapon_reticule_bounds")?;
            if lo > hi || lo < -180. || hi > 180. {
                return Err("invalid_weapon_reticule_bounds");
            }
            Ok([lo, hi])
        };
        if number("KnockAdd")? != 0. {
            return Err("unsupported_weapon_knock_add");
        }
        if p.get("WidthCombineOp").and_then(Value::as_str) != Some("kWCO_Add") {
            return Err("unsupported_weapon_width_combination");
        }
        if nonnegative("KnockReturn")? == 0. {
            return Err("unsupported_weapon_pitch_transfer");
        }
        let cone = pair("cone_width_range")?;
        let pitch = pair("pitch_offset_range")?;
        let yaw = pair("yaw_offset_range")?;
        if cone[0] < 0.
            || cone[1] >= 90.
            || pitch[0] > 0.
            || pitch[1] < 0.
            || yaw[0] > 0.
            || yaw[1] < 0.
        {
            return Err("invalid_weapon_reticule_bounds");
        }
        let normalization = p
            .get("SpreadNormalization")
            .and_then(Value::as_i64)
            .filter(|n| (-8..=8).contains(n))
            .ok_or("unsupported_weapon_spread_normalization")? as i32;
        Ok(Self {
            fire: nonnegative("KnockFire")?,
            recover: nonnegative("KnockReturn")?,
            side_fire: nonnegative("SideKnockFire")?,
            side_recover: nonnegative("SideKnockReturn")?,
            spread_fire: nonnegative("SpreadFire")?,
            spread_recover: nonnegative("SpreadReturn")?,
            move_spread: nonnegative("SpreadMove")?,
            rotate_spread: nonnegative("SpreadRotate")?,
            move_recover: nonnegative("MoveSpreadReturn")?,
            cone,
            pitch,
            yaw,
            normalization,
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    pub(super) pitch_degrees: f64,
    pub(super) yaw_degrees: f64,
    firing_width: f64,
    movement_width: f64,
}
fn toward_zero(value: f64, amount: f64) -> f64 {
    if value > 0. {
        (value - amount).max(0.)
    } else {
        (value + amount).min(0.)
    }
}
pub(super) fn random(seed: u64) -> u64 {
    let mut x = seed.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}
fn unit(seed: u64) -> f64 {
    (random(seed) >> 11) as f64 / ((1u64 << 53) as f64)
}
impl State {
    pub(super) fn new(p: Profile) -> Self {
        Self {
            firing_width: p.cone[0],
            movement_width: p.cone[0],
            ..Self::default()
        }
    }
    pub(super) fn tick(
        &mut self,
        p: Profile,
        dt: f64,
        trigger_held: bool,
        movement: f64,
        rotation: f64,
    ) {
        self.pitch_degrees =
            toward_zero(self.pitch_degrees, p.recover * dt).clamp(p.pitch[0], p.pitch[1]);
        // Automatic sustained fire suppresses width and side contraction.
        if !trigger_held {
            self.yaw_degrees =
                toward_zero(self.yaw_degrees, p.side_recover * dt).clamp(p.yaw[0], p.yaw[1]);
            self.firing_width =
                (self.firing_width - p.spread_recover * dt).clamp(p.cone[0], p.cone[1]);
        }
        let desired = (p.cone[0]
            + (p.move_spread * movement.clamp(0., 1.) + p.rotate_spread * rotation.clamp(0., 1.))
                * (p.cone[1] - p.cone[0]))
            .clamp(p.cone[0], p.cone[1]);
        self.movement_width = desired
            .max(self.movement_width - p.move_recover * dt)
            .clamp(p.cone[0], p.cone[1]);
    }
    pub(super) fn fire(&mut self, p: Profile, seed: u64) {
        self.pitch_degrees = (self.pitch_degrees + p.fire).clamp(p.pitch[0], p.pitch[1]);
        let sign = if random(seed) & 1 == 0 { 1. } else { -1. };
        self.yaw_degrees = (self.yaw_degrees + sign * p.side_fire).clamp(p.yaw[0], p.yaw[1]);
        self.firing_width = (self.firing_width + p.spread_fire).clamp(p.cone[0], p.cone[1]);
    }
    pub(super) fn width_degrees(&self, p: Profile) -> f64 {
        (self.firing_width + self.movement_width - p.cone[0]).clamp(p.cone[0], p.cone[1])
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn endpoint(
        &self,
        p: Profile,
        from: [f64; 3],
        yaw: f64,
        pitch: f64,
        distance: f64,
        compensation: u16,
        seed: u64,
    ) -> [f64; 3] {
        let residual = 1. - f64::from(compensation) / 1000.;
        let yaw = yaw + (self.yaw_degrees * residual).to_radians();
        let pitch = (pitch + (self.pitch_degrees * residual).to_radians())
            .clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
        let mut radius = unit(seed).sqrt();
        for _ in 0..p.normalization.unsigned_abs() {
            radius = if p.normalization >= 0 {
                radius * radius
            } else {
                radius.sqrt()
            };
        }
        let lateral = self.width_degrees(p).to_radians().sin() * radius;
        let angle = unit(seed.wrapping_add(1)) * std::f64::consts::TAU;
        let forward = [
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        let right = [yaw.cos(), 0., -yaw.sin()];
        let up = [
            -yaw.sin() * pitch.sin(),
            pitch.cos(),
            -yaw.cos() * pitch.sin(),
        ];
        let longitudinal = (1. - lateral * lateral).max(0.).sqrt();
        std::array::from_fn(|i| {
            from[i]
                + distance
                    * (forward[i] * longitudinal
                        + right[i] * lateral * angle.cos()
                        + up[i] * lateral * angle.sin())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Value {
        serde_json::json!({"reticule_params_stand":{
            "KnockAdd":0,"KnockFire":1.25,"KnockReturn":2.,"SideKnockFire":0.4,"SideKnockReturn":3.,
            "SpreadFire":0.4,"SpreadReturn":1.,"SpreadMove":0.5,"SpreadRotate":0.,"MoveSpreadReturn":1.,
            "cone_width_range":[0.5,3.5],"pitch_offset_range":[0.,5.],"yaw_offset_range":[-2.,2.],
            "SpreadNormalization":1,"WidthCombineOp":"kWCO_Add"}})
    }
    #[test]
    fn sustained_shots_build_recoil_and_contraction_recovers_after_release() {
        let p = Profile::from_weapon(&fixture()).unwrap();
        let mut s = State::new(p);
        assert_eq!(s.pitch_degrees, 0.);
        s.fire(p, 7);
        let yaw = s.yaw_degrees;
        let width = s.width_degrees(p);
        s.tick(p, 0.1, true, 0., 0.);
        assert_eq!(s.yaw_degrees, yaw);
        assert_eq!(s.width_degrees(p), width);
        assert!(s.pitch_degrees > 0.);
        for i in 0..20 {
            s.fire(p, i);
        }
        assert_eq!(s.pitch_degrees, 5.);
        assert_eq!(s.width_degrees(p), 3.5);
        s.tick(p, 10., false, 0., 0.);
        assert_eq!(s.pitch_degrees, 0.);
        assert_eq!(s.yaw_degrees, 0.);
        assert_eq!(s.width_degrees(p), 0.5);
        assert_eq!(State::new(p).pitch_degrees, 0.);
    }
    #[test]
    fn authored_cone_changes_actual_shot_ray_and_compensation_does_not_remove_spread() {
        let p = Profile::from_weapon(&fixture()).unwrap();
        let mut s = State::new(p);
        s.fire(p, 3);
        let first = s.endpoint(p, [0.; 3], 0., 0., 100., 0, 42);
        let compensated = s.endpoint(p, [0.; 3], 0., 0., 100., 1000, 42);
        assert_ne!(first, compensated);
        assert_ne!(compensated, [0., 0., 100.]);
        assert_eq!(compensated, s.endpoint(p, [0.; 3], 0., 0., 100., 1000, 42));
        assert!((compensated.iter().map(|v| v * v).sum::<f64>().sqrt() - 100.).abs() < 1e-10);
    }
    #[test]
    fn duplicate_source_tables_require_equal_values_and_unknown_models_fail() {
        let mut value = fixture();
        let object = value["reticule_params_stand"].as_object().unwrap();
        let mut rows: Vec<_> = object
            .iter()
            .map(|(k, v)| serde_json::json!({"key":k,"value":v}))
            .collect();
        rows.push(serde_json::json!({"key":"SpreadNormalization","value":1}));
        value["reticule_params_stand"] = serde_json::json!({"$lua_table":rows});
        assert!(Profile::from_weapon(&value).is_ok());
        value["reticule_params_stand"]["$lua_table"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"key":"KnockFire","value":9}));
        assert!(Profile::from_weapon(&value).is_err());
        let mut missing = fixture();
        missing["reticule_params_stand"]
            .as_object_mut()
            .unwrap()
            .remove("KnockFire");
        assert!(Profile::from_weapon(&missing).is_err());
        let mut unsupported = fixture();
        unsupported["reticule_params_stand"]["KnockAdd"] = Value::from(1);
        assert!(Profile::from_weapon(&unsupported).is_err());
    }
}
