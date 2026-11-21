//! Qualified objective targeting for the Precision Strike command.
use crate::{
    Deserialize, Engine, Write,
    objective_control::{NativeClass, ObjectiveWorld, PrecisionRewardActor},
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
    pub(crate) fn current_objective_source_team(&self, player: u16, world: u32) -> Option<u8> {
        if let Some(actor) =
            self.active.iter().chain(self.peers.values()).find(|actor| {
                actor.reservation.player == player && actor.reservation.world == world
            })
        {
            return (actor.entered_game && actor.claimed.is_some()).then(|| actor.team());
        }
        self.bot_registry
            .bots
            .get(&player)
            .filter(|_| self.accepts_subscription_world(world))
            .map(|bot| bot.target.squad.team())
    }

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
        targets: &[PrecisionTarget],
        target_id: u32,
        source: Option<Source>,
        source_player: u16,
        source_world: u32,
    ) -> Result<bool, &'static str> {
        let Some(index) = objective_index(target_id) else {
            return Ok(false);
        };
        let reward_actor = targets
            .iter()
            .find(|target| target.object_index == index)
            .zip(self.current_objective_source_team(source_player, source_world))
            .map(|(target, team)| {
                if target.team == team {
                    PrecisionRewardActor::Defender(source_player)
                } else {
                    PrecisionRewardActor::Attacker(source_player)
                }
            });
        world.stage_precision_strike_with_caller(index, source, reward_actor, Some(source_player))
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
