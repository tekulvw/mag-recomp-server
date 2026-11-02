//! Pure incapacitation decisions driven by an explicit external rules file.
use serde::Deserialize;
use std::{io, path::Path, time::Duration};

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub schema_version: u32,
    pub incapacitated_damage_threshold_hp: f32,
    pub incapacitated_duration_seconds: f32,
    pub excluded_character_states: Vec<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Downed {
    pub deadline: Duration,
    pub accumulated_damage_hp: f32,
}

impl Rules {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let value = serde_json::from_reader(io::BufReader::new(std::fs::File::open(path)?))?;
        Self::from_value(value)
    }

    pub fn from_value(value: serde_json::Value) -> io::Result<Self> {
        let rules: Self = serde_json::from_value(value)?;
        rules
            .validate()
            .map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason))?;
        Ok(rules)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1 {
            return Err("unsupported_incapacitation_rules_schema");
        }
        if !positive_finite(self.incapacitated_damage_threshold_hp)
            || !positive_finite(self.incapacitated_duration_seconds)
        {
            return Err("invalid_incapacitation_rules");
        }
        let states: std::collections::BTreeSet<_> = self.excluded_character_states.iter().collect();
        if states.len() != self.excluded_character_states.len() {
            return Err("duplicate_excluded_character_state");
        }
        Ok(())
    }

    /// Decide admission after an initial lethal hit using its final adjusted HP
    /// damage. Notification/statistics for that hit are separate from admission.
    /// `now` and the returned deadline use the caller's same monotonic epoch.
    pub fn admit(
        &self,
        recent_damage_hp: f32,
        character_state: u16,
        duration_modifier: f32,
        now: Duration,
    ) -> Result<Option<Downed>, &'static str> {
        self.validate()?;
        if !nonnegative_finite(recent_damage_hp) || !duration_modifier.is_finite() {
            return Err("invalid_incapacitation_input");
        }
        if recent_damage_hp >= self.incapacitated_damage_threshold_hp
            || self.excluded_character_states.contains(&character_state)
        {
            return Ok(None);
        }
        let base = self.incapacitated_duration_seconds;
        let seconds = base.mul_add(duration_modifier, base);
        if !seconds.is_finite() {
            return Err("invalid_incapacitation_duration");
        }
        if seconds <= 0.0 {
            return Ok(None);
        }
        let duration =
            Duration::try_from_secs_f32(seconds).map_err(|_| "invalid_incapacitation_duration")?;
        let deadline = now
            .checked_add(duration)
            .ok_or("incapacitation_deadline_overflow")?;
        Ok(Some(Downed {
            deadline,
            accumulated_damage_hp: 0.0,
        }))
    }
}

impl Downed {
    /// Add the final adjusted incapacitated HP damage. `true` requests a finish;
    /// the caller must consume the downed life state before emitting that event.
    /// Invalid amounts leave the accumulator unchanged.
    pub fn apply_damage(&mut self, rules: &Rules, amount_hp: f32) -> Result<bool, &'static str> {
        rules.validate()?;
        if !nonnegative_finite(self.accumulated_damage_hp) || !nonnegative_finite(amount_hp) {
            return Err("invalid_incapacitated_damage");
        }
        let accumulated = self.accumulated_damage_hp + amount_hp;
        if !accumulated.is_finite() {
            return Err("incapacitated_damage_overflow");
        }
        self.accumulated_damage_hp = accumulated;
        Ok(accumulated >= rules.incapacitated_damage_threshold_hp)
    }

    pub fn is_expired(self, now: Duration) -> bool {
        now >= self.deadline
    }
}

/// Restore source maximum HP times a positive modifier, cap to the patient's
/// maximum HP, then truncate the resulting health to quarter-HP wire units.
/// A sub-quarter result is zero; callers must require an actual HP increase
/// before emitting a successful resuscitation event.
pub fn resuscitated_health_quarter_points(
    source_max_hp: f32,
    patient_max_hp: f32,
    modifier: f32,
) -> Result<u16, &'static str> {
    if !positive_finite(source_max_hp)
        || !positive_finite(patient_max_hp)
        || !positive_finite(modifier)
    {
        return Err("invalid_resuscitation_input");
    }
    let restored_hp = source_max_hp * modifier;
    if !restored_hp.is_finite() {
        return Err("resuscitation_health_overflow");
    }
    let quarters = (restored_hp.min(patient_max_hp) * 4.0).trunc();
    if !quarters.is_finite() || quarters >= 4096.0 {
        return Err("unrepresentable_resuscitation_health");
    }
    Ok(quarters as u16)
}

fn positive_finite(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

fn nonnegative_finite(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Synthetic tuning deliberately independent of private game resources.
    fn rules() -> Rules {
        Rules {
            schema_version: 1,
            incapacitated_damage_threshold_hp: 48.0,
            incapacitated_duration_seconds: 12.0,
            excluded_character_states: vec![6, 9],
        }
    }

    #[test]
    fn rules_load_only_explicit_valid_schema() {
        let path = std::env::temp_dir().join(format!(
            "mag-incap-rules-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            json!({"schema_version":1,"incapacitated_damage_threshold_hp":48.0,
            "incapacitated_duration_seconds":12.0,"excluded_character_states":[6,9]})
            .to_string(),
        )
        .unwrap();
        let loaded = Rules::load(&path);
        std::fs::remove_file(path).unwrap();
        assert_eq!(loaded.unwrap(), rules());
        assert!(Rules::from_value(json!({"schema_version":1})).is_err());
        assert!(
            Rules::from_value(
                json!({"schema_version":1,"incapacitated_damage_threshold_hp":48.0,
            "incapacitated_duration_seconds":12.0,"excluded_character_states":[],"unknown":true})
            )
            .is_err()
        );
    }

    #[test]
    fn rules_reject_unsupported_schema_duplicate_states_and_invalid_tuning() {
        let mut bad = rules();
        bad.schema_version = 2;
        assert!(bad.validate().is_err());
        bad = rules();
        bad.excluded_character_states.push(6);
        assert!(bad.validate().is_err());
        for amount in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            bad = rules();
            bad.incapacitated_damage_threshold_hp = amount;
            assert!(bad.validate().is_err());
            bad = rules();
            bad.incapacitated_duration_seconds = amount;
            assert!(bad.validate().is_err());
        }
    }

    #[test]
    fn admission_uses_adjusted_damage_strict_threshold_and_excluded_state() {
        let rules = rules();
        let downed = rules
            .admit(47.75, 1, 0.5, Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(downed.deadline, Duration::from_secs(23));
        assert_eq!(downed.accumulated_damage_hp, 0.0);
        assert!(rules.admit(48.0, 1, 0.0, Duration::ZERO).unwrap().is_none());
        assert!(rules.admit(49.0, 1, 0.0, Duration::ZERO).unwrap().is_none());
        assert!(rules.admit(1.0, 6, 0.0, Duration::ZERO).unwrap().is_none());
    }

    #[test]
    fn admission_requires_finite_positive_duration_and_representable_deadline() {
        let rules = rules();
        assert_eq!(
            rules
                .admit(1.0, 1, -0.5, Duration::ZERO)
                .unwrap()
                .unwrap()
                .deadline,
            Duration::from_secs(6)
        );
        assert!(rules.admit(1.0, 1, -1.0, Duration::ZERO).unwrap().is_none());
        assert!(rules.admit(1.0, 1, -2.0, Duration::ZERO).unwrap().is_none());
        for invalid in [f32::NAN, f32::INFINITY] {
            assert!(rules.admit(invalid, 1, 0.0, Duration::ZERO).is_err());
            assert!(rules.admit(1.0, 1, invalid, Duration::ZERO).is_err());
        }
        assert!(rules.admit(-1.0, 1, 0.0, Duration::ZERO).is_err());
        assert!(rules.admit(1.0, 1, f32::MAX, Duration::ZERO).is_err());
        assert!(rules.admit(1.0, 1, 0.0, Duration::MAX).is_err());
    }

    #[test]
    fn downed_damage_accumulates_separately_and_finishes_at_equality() {
        let mut downed = rules()
            .admit(47.0, 1, 0.0, Duration::ZERO)
            .unwrap()
            .unwrap();
        assert!(!downed.apply_damage(&rules(), 17.0).unwrap());
        assert_eq!(downed.accumulated_damage_hp, 17.0);
        assert!(downed.apply_damage(&rules(), 31.0).unwrap());
        assert_eq!(downed.accumulated_damage_hp, 48.0);
    }

    #[test]
    fn invalid_downed_damage_does_not_mutate_accumulator() {
        let mut downed = Downed {
            deadline: Duration::from_secs(4),
            accumulated_damage_hp: 2.0,
        };
        for amount in [-1.0, f32::NAN, f32::INFINITY] {
            assert!(downed.apply_damage(&rules(), amount).is_err());
            assert_eq!(downed.accumulated_damage_hp, 2.0);
        }
        downed.accumulated_damage_hp = f32::MAX;
        assert!(downed.apply_damage(&rules(), f32::MAX).is_err());
        assert_eq!(downed.accumulated_damage_hp, f32::MAX);
    }

    #[test]
    fn expiry_is_due_at_deadline() {
        let downed = Downed {
            deadline: Duration::from_secs(4),
            accumulated_damage_hp: 2.0,
        };
        assert!(!downed.is_expired(Duration::from_millis(3999)));
        assert!(downed.is_expired(Duration::from_secs(4)));
        assert!(downed.is_expired(Duration::from_secs(5)));
    }

    #[test]
    fn revival_uses_source_maximum_caps_patient_and_truncates_quarters() {
        assert_eq!(
            resuscitated_health_quarter_points(80.0, 200.0, 0.333).unwrap(),
            106
        );
        assert_eq!(
            resuscitated_health_quarter_points(80.0, 50.0, 2.0).unwrap(),
            200
        );
        assert_eq!(
            resuscitated_health_quarter_points(80.0, 200.0, 0.001).unwrap(),
            0
        );
    }

    #[test]
    fn revival_rejects_invalid_inputs_and_unrepresentable_health() {
        for value in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(resuscitated_health_quarter_points(value, 200.0, 0.5).is_err());
            assert!(resuscitated_health_quarter_points(80.0, value, 0.5).is_err());
            assert!(resuscitated_health_quarter_points(80.0, 200.0, value).is_err());
        }
        assert!(resuscitated_health_quarter_points(f32::MAX, 200.0, 2.0).is_err());
        assert!(resuscitated_health_quarter_points(2000.0, 2000.0, 1.0).is_err());
    }
}
