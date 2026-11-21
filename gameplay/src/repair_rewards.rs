//! Accepted repair work is distinct from HP restored and fractional XP.
use crate::*;

pub(crate) const CAPABILITY: &str = "repair_rewards_v1";

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub(crate) struct State {
    pub carry: f32,
    pub sequence: u64,
}
impl State {
    fn advance(self, modifier: f32) -> Result<(Self, u32), &'static str> {
        let raw_work = 1.0_f32 + modifier;
        let total = self.carry + raw_work;
        if !modifier.is_finite()
            || !self.carry.is_finite()
            || !total.is_finite()
            || total >= 65_536.0
        {
            return Err("unrepresentable_repair_work");
        }
        let units = if total >= 1.0 {
            total.trunc() as u32
        } else {
            0
        };
        Ok((
            Self {
                carry: total - units as f32,
                sequence: self
                    .sequence
                    .checked_add(1)
                    .ok_or("repair_credit_sequence_exhausted")?,
            },
            units,
        ))
    }
}

pub(crate) struct Plan {
    reservation_id: String,
    state: State,
    units: u32,
    statistic: i32,
    reward: Option<progression::RewardEvent>,
    messages: Vec<OutboundMessage>,
}
impl Plan {
    pub(crate) fn audit(&self) -> Value {
        json!({"reservation_id":self.reservation_id,"repair_credit":self.state,
            "units":self.units,"stat35":self.statistic,"reward":self.reward,"messages":self.messages})
    }
}
impl<W: Write> Engine<W> {
    pub(crate) fn plan_repair_credit(
        &self,
        actor: &Active,
        modifier: f32,
        context_position: Option<[f32; 3]>,
    ) -> Result<Plan, &'static str> {
        let (state, units) = actor.repair_credit.advance(modifier)?;
        let statistic = actor
            .reward_statistics
            .get(&35)
            .copied()
            .unwrap_or(0)
            .saturating_add(units as i32);
        let reward = if units > 0
            && actor.wire_profile.application_id() == 20231
            && actor
                .reservation
                .character
                .repair_rewards_enabled
                .unwrap_or(false)
        {
            progression::RewardRecipient::from_active(actor).map(|recipient| {
                self.frago_action_reward(
                    recipient.event(
                        format!("repair-{}-{:016x}", actor.reservation.id, state.sequence),
                        progression::RewardKind::RepairUnits,
                        units,
                    ),
                    actor,
                    context_position,
                    frago::Frame::default(),
                )
            })
        } else {
            None
        };
        self.check_reward_capacity(reward.as_slice())?;
        let mut messages = Vec::new();
        if units > 0 {
            let packet = protocol::StatUpdate {
                character_index: actor.reservation.player,
                stats: vec![protocol::StatValue {
                    stat: 35,
                    value: statistic,
                }],
            }
            .encode_with_profile(actor.wire_profile)?;
            for recipient in self.active.iter().chain(self.peers.values()).filter(|r| {
                r.reservation.world == actor.reservation.world
                    && r.ready_for_updates
                    && r.initial_objects.is_some()
            }) {
                messages.push(OutboundMessage {
                    reservation_id: recipient.reservation.id.clone(),
                    world_id: recipient.reservation.world,
                    player_id: recipient.reservation.player,
                    plugin_id: 1,
                    payload_hex: protocol::encode_hex(&packet),
                });
            }
        }
        Ok(Plan {
            reservation_id: actor.reservation.id.clone(),
            state,
            units,
            statistic,
            reward,
            messages,
        })
    }
    pub(crate) fn install_repair_credit(&mut self, plan: Plan) -> Vec<OutboundMessage> {
        let actor = self
            .active
            .iter_mut()
            .chain(self.peers.values_mut())
            .find(|a| a.reservation.id == plan.reservation_id)
            .expect("owned recorded repair recipient");
        actor.repair_credit = plan.state;
        if plan.units > 0 {
            actor.reward_statistics.insert(35, plan.statistic);
        }
        self.commit_rewards(plan.reward.into_iter().collect());
        plan.messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn work_carry_crosses_inclusively_and_remains_separate_from_xp() {
        let (first, units) = State::default().advance(-0.75).unwrap();
        assert_eq!((first.carry, units), (0.25, 0));
        let (second, units) = first.advance(-0.25).unwrap();
        assert_eq!((second.carry, units), (0., 1));
        let (third, units) = second.advance(1.5).unwrap();
        assert_eq!((third.carry, units, third.sequence), (0.5, 2, 3));
    }
    #[test]
    fn invalid_or_unrepresentable_work_cannot_advance_state() {
        for modifier in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 65536.] {
            assert!(State::default().advance(modifier).is_err());
        }
        assert!(
            State {
                carry: 0.,
                sequence: u64::MAX
            }
            .advance(0.)
            .is_err()
        );
    }
}
