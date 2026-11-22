//! Model the retail static-provider Heal6 percentage application.
use crate::replication::CharacterDamage;

pub(super) fn apply(
    mut before: CharacterDamage,
    maximum_hp: f32,
    percent: f32,
) -> Option<CharacterDamage> {
    let current_hp = f32::from(before.health_quarter_points) * 0.25;
    let amount = (maximum_hp * (percent / 100.0)).min(maximum_hp - current_hp);
    if amount <= 0.0 || before.conditions & 2 != 0 {
        return None;
    }
    // A positive sub-quarter application still writes type6. Preserve the
    // previous hit location, conditions, and attribution from the model.
    before.health_quarter_points =
        (((current_hp + amount).min(maximum_hp) * 4.0).trunc() as u16) & 0x0fff;
    before.last_location_and_type = 0x60 | (before.last_location_and_type & 0x0f);
    Some(before)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(health_quarter_points: u16, conditions: u8) -> CharacterDamage {
        CharacterDamage {
            last_location_and_type: 0xa3,
            health_quarter_points,
            conditions,
            last_attacker: 0x0380_0001,
            last_source_object: u32::MAX,
            last_user_data: 29,
        }
    }

    #[test]
    fn static_supply_native_percentage_heal_caps_and_retains_hit_attribution() {
        for (maximum, before, expected) in [(100., 250, 290), (120., 250, 298), (120., 476, 480)] {
            let before = model(before, 1);
            let after = apply(before, maximum, 10.).unwrap();
            assert_eq!(after.health_quarter_points, expected);
            assert_eq!(after.last_location_and_type, 0x63);
            assert_eq!(after.conditions, before.conditions);
            assert_eq!(after.last_attacker, before.last_attacker);
            assert_eq!(after.last_source_object, before.last_source_object);
            assert_eq!(after.last_user_data, before.last_user_data);
        }
    }

    #[test]
    fn static_supply_subquarter_heal_is_a_write_but_full_and_incap_are_not() {
        let after = apply(model(250, 0), 100., 0.1).unwrap();
        assert_eq!(after.health_quarter_points, 250);
        assert_eq!(after.last_location_and_type, 0x63);
        assert!(apply(model(400, 0), 100., 10.).is_none());
        assert!(apply(model(250, 2), 100., 10.).is_none());
        assert!(apply(model(250, 0), 100., 0.).is_none());
    }
}
