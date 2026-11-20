//! Qualified objective targeting for the Precision Strike command.
use crate::{
    Deserialize, Engine, Write,
    objective_control::{NativeClass, ObjectiveWorld},
    structure_damage::Source,
};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrecisionTarget {
    pub object_index: u16,
    pub team: u8,
}

fn objective_index(target_id: u32) -> Option<u16> {
    (target_id >> 16 == 0x0c80).then_some(target_id as u16)
}

fn target_allowed(
    targets: &[PrecisionTarget],
    world: &ObjectiveWorld,
    target_id: u32,
    team: u8,
) -> bool {
    let Some(index) = objective_index(target_id) else {
        return false;
    };
    team < 2
        && targets
            .iter()
            .any(|row| row.object_index == index && row.team != team)
        && world.state(index).is_some()
        && world
            .native_class(index)
            .is_none_or(|class| class == NativeClass::Structural)
}

impl<W: Write> Engine<W> {
    pub(crate) fn precision_target_allowed(
        &self,
        target_id: u32,
        team: u8,
    ) -> Result<bool, &'static str> {
        let runtime = self
            .active_abilities
            .as_ref()
            .ok_or("active_ability_runtime_unavailable")?;
        let world = self
            .objective_world
            .as_ref()
            .ok_or("objective_world_unavailable")?;
        Ok(target_allowed(
            &runtime.setup.precision_targets,
            world,
            target_id,
            team,
        ))
    }

    /// The sealed activation target is re-resolved in the current candidate.
    /// The objective update consumes its desired state later in the same pass.
    pub(crate) fn stage_precision_strike(
        &self,
        world: &mut ObjectiveWorld,
        target_id: u32,
        source: Option<Source>,
    ) -> Result<bool, &'static str> {
        let Some(index) = objective_index(target_id) else {
            return Ok(false);
        };
        world.stage_precision_strike(index, source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{objective_catalog::ObjectiveCatalog, objective_control::ObjectiveWorld};

    #[test]
    fn qualified_target_requires_exact_family_live_slot_and_enemy_team() {
        let json = crate::objective_control::trigger_box_tests::native_catalog();
        let world = ObjectiveWorld::new(&ObjectiveCatalog::from_json(&json.to_string()).unwrap());
        let targets = [
            PrecisionTarget {
                object_index: 10,
                team: 1,
            },
            PrecisionTarget {
                object_index: 11,
                team: 1,
            },
            PrecisionTarget {
                object_index: 12,
                team: 1,
            },
        ];
        assert!(target_allowed(&targets, &world, 0x0c80_000b, 0));
        assert!(!target_allowed(&targets, &world, 0x0c80_000b, 1));
        assert!(!target_allowed(&targets, &world, 0x0c81_000b, 0));
        assert!(!target_allowed(&targets, &world, 0x0380_000b, 0));
        assert!(!target_allowed(&targets, &world, 0x0c80_000a, 0));
        assert!(!target_allowed(&targets, &world, 0x0c80_000c, 0));
    }
}
