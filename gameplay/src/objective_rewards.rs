//! Retail state-stat callbacks and endgame squad awards. XP arithmetic is Horizon-owned.
use crate::*;
use crate::{
    objective_catalog::NativeClass, objective_control::NativeEntry,
    progression::RewardKind as Kind, replication::ObjectiveState as State,
};

/// A callback is an entry effect, not a comparison of final world snapshots.
/// Rollbacks deliberately bypass the retail positive-stat notification.
fn action_kinds(entry: NativeEntry, category: u8) -> Vec<Kind> {
    let (old, new) = (entry.previous, entry.delta.state_actor.state);
    let structural = entry.class == NativeClass::Structural;
    let mut kinds = Vec::new();
    if structural && old == State::Activating && new == State::Activated {
        kinds.push(Kind::ChargePlanted);
    }
    if structural && old == State::Deactivating && new == State::Deactivated {
        kinds.push(Kind::ChargeDefused);
    }
    if category <= 1 {
        match (old, new) {
            (State::Activating, State::Activated) | (State::Deactivating, State::Deactivated) => {
                kinds.push(Kind::PrimaryObjectiveActivated)
            }
            (State::Activated, State::Completed) => kinds.push(Kind::PrimaryObjectiveSecured),
            (State::Retaking, State::Retaken) | (State::CancelRetaken, State::Completed)
                if !structural =>
            {
                kinds.push(Kind::PrimaryObjectiveActivated)
            }
            (State::Retaken, State::Deactivated) if !structural => {
                kinds.push(Kind::PrimaryObjectiveSecured)
            }
            _ => {}
        }
    }
    kinds
}

impl<W: Write> Engine<W> {
    /// Called inside the owner's existing character-effect rollback boundary,
    /// after common/FSP effects and before blast, script and tag changes.
    pub(crate) fn stage_native_objective_rewards(
        &mut self,
        entry: NativeEntry,
        assignment: Option<acquisition_objective_runtime::Assignment>,
        frame: frago::Frame<'_>,
    ) -> Result<Vec<Value>, &'static str> {
        let Some(rule) = self
            .objective_catalog
            .as_ref()
            .and_then(|c| {
                c.objects
                    .iter()
                    .find(|o| o.object_index == entry.delta.object_index)
            })
            .filter(|o| o.native_class == Some(entry.class))
            .and_then(|o| o.reward.as_ref())
        else {
            return Ok(vec![]);
        };
        let Some(player) = entry.reward_actor else {
            return Ok(vec![]);
        };
        // Native resolves the currently allocated packed slot here. Neither
        // death nor an old life generation prevents a current character callback.
        let Some(actor) = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|a| a.reservation.player == player && a.entered_game && a.claimed.is_some())
        else {
            return Ok(vec![]);
        };
        let Some(recipient) = progression::RewardRecipient::from_active(actor) else {
            return Ok(vec![]);
        };
        let context = self.frago_object_position(0x0c80_0000 | u32::from(entry.delta.object_index));
        let rewards: Vec<_> = action_kinds(entry, rule.category)
            .into_iter()
            .map(|kind| {
                self.frago_objective_reward(
                    recipient.event(
                        format!(
                            "nobj-{:016x}-{}-{}-{}-{}-{}-{}",
                            self.round_number,
                            entry.delta.object_index,
                            entry.delta.state_actor.change_time_ms,
                            entry.previous as u8,
                            entry.delta.state_actor.state as u8,
                            kind.statistic().expect("objective statistic"),
                            recipient.reservation_id
                        ),
                        kind,
                        1,
                    ),
                    actor,
                    context,
                    assignment,
                    frame,
                )
            })
            .collect();
        self.check_reward_capacity(&rewards)?;
        let audit = rewards
            .iter()
            .map(|reward| json!({"op":"objective_reward","reward":reward}))
            .collect();
        self.commit_rewards(rewards);
        Ok(audit)
    }

    /// Acquisition retains its existing capture timer; a late tick may queue
    /// both callbacks even though the published delta contains only Completed.
    pub(crate) fn stage_acquisition_capture_rewards(
        &mut self,
        actions: Vec<objective_control::ObjectiveRewardAction>,
        runtime: &acquisition_gameplay_engine::Runtime,
        ground: &acquisition_ground_runtime::Runtime,
    ) -> Result<Vec<Value>, &'static str> {
        let mut rewards = Vec::new();
        for action in actions {
            if !matches!(
                action.kind,
                Kind::PrimaryObjectiveActivated | Kind::PrimaryObjectiveSecured
            ) {
                continue;
            }
            let Some(actor) = self
                .active
                .iter()
                .chain(self.peers.values())
                .find(|a| action.recipient.matches_active(a))
            else {
                continue;
            };
            let assignment = runtime
                .state
                .squad(actor.squad_catalog.squad_index())
                .map(|s| s.assignment);
            rewards.push(self.frago_objective_reward(
                action.recipient.event(
                    format!(
                        "objective-{:016x}-{}-{}-{:?}",
                        self.round_number,
                        action.object_index,
                        action.started_at.as_nanos(),
                        action.kind
                    ),
                    action.kind,
                    1,
                ),
                actor,
                self.frago_object_position(0x0c80_0000 | u32::from(action.object_index)),
                assignment,
                frago::Frame {
                    ground: Some(ground),
                    ..Default::default()
                },
            ));
        }
        self.check_reward_capacity(&rewards)?;
        let audit = rewards
            .iter()
            .map(|reward| json!({"op":"objective_reward","reward":reward}))
            .collect();
        self.commit_rewards(rewards);
        Ok(audit)
    }

    /// One award for each qualifying ordered row, including repeated objectives.
    /// This runs before match-result finalization, never at local completion.
    pub(crate) fn end_objective_rewards(
        &self,
        actor: &Active,
    ) -> Result<Vec<progression::RewardEvent>, &'static str> {
        let Some(catalog) = &self.objective_catalog else {
            return Ok(vec![]);
        };
        let Some(recipient) = progression::RewardRecipient::from_active(actor) else {
            return Ok(vec![]);
        };
        let mut rewards = Vec::new();
        for (entry, (index, category)) in catalog
            .end_reward_objectives(actor.squad_catalog.squad_index())?
            .into_iter()
            .enumerate()
        {
            let kind = match category {
                0 => Kind::PrimaryObjectiveCompleted,
                2 | 3 => Kind::SecondaryObjectiveCompleted,
                _ => continue,
            };
            let state = self.objective_world.as_ref().and_then(|w| w.state(index));
            let eligible = state.is_some_and(|state| {
                if actor.team() == 0 {
                    matches!(state, State::Completed | State::Locked)
                } else {
                    matches!(
                        state,
                        State::Inactive | State::Deactivated | State::Repaired
                    )
                }
            });
            if eligible {
                rewards.push(self.frago_objective_reward(
                    recipient.event(
                        format!(
                            "end-objective-{:016x}-{}-{index}-{entry}",
                            self.round_number, actor.reservation.id
                        ),
                        kind,
                        1,
                    ),
                    actor,
                    self.frago_object_position(0x0c80_0000 | u32::from(index)),
                    self.frago_assignment(actor),
                    frago::Frame::default(),
                ));
            }
        }
        Ok(rewards)
    }
}
