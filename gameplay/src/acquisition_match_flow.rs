//! Connect the owned Acquisition and Domination rounds to the retail result publisher.
use super::*;
use acquisition_round::{Phase, Transition};
use match_flow::{Outcome, ScorePublication};
use protocol::ScoreChangeReason::TimedBleed;

#[derive(Clone, Copy)]
pub(crate) enum OwnedRound {
    Acquisition,
    Domination,
}

impl<W: Write> Engine<W> {
    pub(crate) fn tick_owned_round(
        &mut self,
        mode: OwnedRound,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if self.match_epoch.is_none()
            && self
                .active
                .as_ref()
                .is_none_or(|actor| actor.match_clock_started.is_none())
        {
            return Ok(Vec::new());
        }
        let mut round = match mode {
            OwnedRound::Acquisition => self.acquisition_round.clone(),
            OwnedRound::Domination => self.domination_round.clone(),
        }
        .ok_or("owned_round_unavailable")?;
        let now_ms = now.as_millis() as u32;
        let mut next = self.round.clone();
        let before_bleed = round.points();
        let bleed = if round.phase() == Phase::Playing {
            round.update_bleed(now_ms)
        } else {
            [None; 2]
        };
        let mut rewards = Vec::new();
        match round.update_end(now_ms) {
            Some(Transition::EndGame(result)) => {
                let Phase::Ending { deadline_ms } = round.phase() else {
                    return Err("owned_round_missing_end_deadline");
                };
                let outcome = Outcome {
                    winner: result.winner,
                    major: result.major,
                    deadline: Duration::from_millis(deadline_ms.into()),
                    finalized: false,
                };
                for actor in self
                    .peers
                    .values()
                    .chain(self.active.iter())
                    .filter(|a| a.entered_game && a.claimed.is_some())
                {
                    let Some(recipient) = progression::RewardRecipient::from_active(actor) else {
                        continue;
                    };
                    rewards.extend(self.end_objective_rewards(actor)?);
                    rewards.push(self.match_result_reward(actor, &recipient, &outcome));
                }
                next.outcome = Some(outcome);
            }
            Some(Transition::Finished) => {
                next.outcome
                    .as_mut()
                    .ok_or("owned_round_missing_outcome")?
                    .finalized = true;
            }
            None => {}
        }
        next.scores = Some(round.points());
        let mut effects = Vec::new();
        for (team, points) in bleed.into_iter().enumerate() {
            if let Some(points) = points {
                effects.push(json!({"team":team,"points":points,"reason":TimedBleed as u8}));
            }
        }
        self.commit_round_update(next, rewards, effects, now, wall)?;
        match mode {
            OwnedRound::Acquisition => self.acquisition_round = Some(round),
            OwnedRound::Domination => self.domination_round = Some(round),
        }
        self.publish_round_update(now, ScorePublication::NativeScoreRate { before_bleed })
    }
}

#[cfg(test)]
mod tests {
    use crate::acquisition_round::{Phase, Round, Rules, ScoreIntent};
    use crate::acquisition_score_rate::Request;
    use crate::spawn_progression::tests::{engine, ready, reserve};
    use crate::*;
    use protocol::ScoreChangeReason::{Initial, PrimaryObjective, TimedBleed};

    fn score_bytes(
        engine: &Engine<Vec<u8>>,
        team: u8,
        points: i16,
        reason: protocol::ScoreChangeReason,
    ) -> String {
        protocol::encode_hex(
            &protocol::ScoreChange {
                team,
                points,
                reason,
            }
            .encode_with_profile(engine.wire_profile)
            .unwrap(),
        )
    }

    fn reserve_ready(engine: &mut Engine<Vec<u8>>, player: u16) {
        reserve(engine, player);
        for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
            ready(actor);
        }
    }

    #[test]
    fn acquisition_round_publishes_initial_and_changed_scores_then_ends_once() {
        let mut engine = engine();
        reserve_ready(&mut engine, 1);
        engine.match_epoch = Some(Duration::from_millis(100));
        engine.acquisition_round = Some(Round::new(Rules {
            starting_points: [1200, 1000],
            major_percent: 50,
            end_delay_ms: 2000,
        }));
        engine
            .tick_match_clock(Duration::from_millis(100), 1)
            .unwrap();
        let initial = engine.drain_notifications(Vec::new(), 1).unwrap();
        for (team, points) in [(0, 1200), (1, 1000)] {
            let expected = score_bytes(&engine, team, points, Initial);
            assert!(initial.iter().any(|notice| notice.payload_hex == expected));
        }
        let round = engine.acquisition_round.as_mut().unwrap();
        round
            .rates_mut()
            .configure(
                Request {
                    team: 0,
                    amount: 1,
                    frequency_ms: 1000,
                },
                100,
            )
            .unwrap();
        round
            .apply_score(ScoreIntent {
                team: 1,
                delta: -600,
            })
            .unwrap();
        engine
            .tick_match_clock(Duration::from_millis(1101), 2)
            .unwrap();
        let delivery = engine.drain_notifications(Vec::new(), 2).unwrap();
        let changes: Vec<_> = delivery
            .iter()
            .filter_map(|notice| {
                notice
                    .payload_hex
                    .starts_with("0207")
                    .then_some(notice.payload_hex.as_str())
            })
            .collect();
        assert_eq!(
            changes,
            [
                score_bytes(&engine, 1, 400, PrimaryObjective),
                score_bytes(&engine, 0, 1199, TimedBleed),
            ]
        );
        assert!(engine.match_terminal.is_none());
        engine
            .acquisition_round
            .as_mut()
            .unwrap()
            .apply_score(ScoreIntent {
                team: 1,
                delta: -400,
            })
            .unwrap();
        engine
            .tick_match_clock(Duration::from_millis(1200), 3)
            .unwrap();
        assert_eq!(engine.round.scores, Some([1199, 0]));
        assert_eq!(
            engine.match_terminal.as_ref().unwrap().reason,
            "attacker_victory"
        );
        let expected_end = protocol::encode_hex(
            &protocol::EndOfGame {
                winning_team: 0,
                major_victory: true,
                end_time_ms: 3200,
            }
            .encode_with_profile(engine.wire_profile)
            .unwrap(),
        );
        let end = engine.drain_notifications(Vec::new(), 3).unwrap();
        assert_eq!(
            end.iter()
                .filter(|notice| notice.payload_hex == expected_end)
                .count(),
            1
        );
        engine
            .tick_match_clock(Duration::from_millis(3199), 4)
            .unwrap();
        assert_eq!(engine.round.scores, Some([1199, 0]));
        assert!(!engine.round.outcome.as_ref().unwrap().finalized);
        engine
            .tick_match_clock(Duration::from_millis(3200), 4)
            .unwrap();
        assert_eq!(
            engine.acquisition_round.as_ref().unwrap().phase(),
            Phase::Finished
        );
        assert!(engine.round.outcome.as_ref().unwrap().finalized);
        let postgame = engine.drain_notifications(Vec::new(), 4).unwrap();
        assert!(
            !postgame
                .iter()
                .any(|notice| notice.payload_hex == expected_end)
        );
    }

    #[test]
    fn release_requeue_starts_fresh_and_can_finish_for_defenders() {
        let mut engine = engine();
        reserve_ready(&mut engine, 1);
        engine.match_epoch = Some(Duration::from_millis(1));
        engine.acquisition_round = Some(Round::new(Rules {
            starting_points: [1200, 1000],
            major_percent: 50,
            end_delay_ms: 2000,
        }));
        let round = engine.acquisition_round.as_mut().unwrap();
        round
            .rates_mut()
            .configure(
                Request {
                    team: 0,
                    amount: 1,
                    frequency_ms: 1000,
                },
                1,
            )
            .unwrap();
        round
            .apply_score(ScoreIntent {
                team: 1,
                delta: -1000,
            })
            .unwrap();
        engine
            .tick_match_clock(Duration::from_millis(2), 1)
            .unwrap();
        let first = engine.match_terminal.as_ref().unwrap().match_id.clone();
        engine
            .tick_match_clock(Duration::from_millis(2002), 2)
            .unwrap();
        assert!(engine.round.outcome.as_ref().unwrap().finalized);
        let release = engine.handle(
            crate::Request::Release {
                v: BRIDGE_VERSION,
                request_id: "release-finished-round".into(),
                reservation_id: "synthetic-reservation-0001".into(),
            },
            Duration::from_millis(2003),
            2,
        );
        assert!(release.ok, "{:?}", release.error);
        reserve_ready(&mut engine, 2);
        assert!(engine.match_terminal.is_none());
        assert_eq!(engine.round.scores, None);
        assert_eq!(
            engine.acquisition_round.as_ref().unwrap().phase(),
            Phase::Playing
        );
        engine.match_epoch = Some(Duration::from_millis(2004));
        engine
            .tick_match_clock(Duration::from_millis(3000), 3)
            .unwrap();
        assert_eq!(engine.round.scores, Some([1200, 1000]));
        let expired = engine.handle(
            crate::Request::Event {
                v: BRIDGE_VERSION,
                request_id: "old-round-event".into(),
                reservation_id: "synthetic-reservation-0001".into(),
                world_id: 1,
                player_id: 1,
                transport: Transport::Tcp,
                direction: Direction::ClientToServer,
                rt_type: 61,
                plugin_id: Some(1),
                payload_hex: "021c03800000".into(),
            },
            Duration::from_millis(3001),
            3,
        );
        assert!(!expired.ok);
        assert_eq!(engine.round.scores, Some([1200, 1000]));
        let round = engine.acquisition_round.as_mut().unwrap();
        round
            .rates_mut()
            .configure(
                Request {
                    team: 0,
                    amount: 1200,
                    frequency_ms: 1000,
                },
                3000,
            )
            .unwrap();
        engine
            .tick_match_clock(Duration::from_millis(4000), 4)
            .unwrap();
        assert!(engine.match_terminal.is_none());
        engine
            .tick_match_clock(Duration::from_millis(4001), 4)
            .unwrap();
        let terminal = engine.match_terminal.as_ref().unwrap();
        assert_ne!(terminal.match_id, first);
        assert_eq!(terminal.reason, "defender_victory");
        assert_eq!(terminal.reservation_ids, ["synthetic-reservation-0002"]);
    }

    #[test]
    fn recording_failure_leaves_terminal_and_bleed_candidate_uncommitted() {
        let mut engine = engine();
        reserve_ready(&mut engine, 1);
        engine.match_epoch = Some(Duration::from_millis(100));
        let mut round = Round::new(Rules {
            starting_points: [1, 1000],
            major_percent: 50,
            end_delay_ms: 2000,
        });
        round
            .rates_mut()
            .configure(
                Request {
                    team: 0,
                    amount: 1,
                    frequency_ms: 1000,
                },
                100,
            )
            .unwrap();
        engine.acquisition_round = Some(round.clone());
        engine.max_log = engine.written;
        assert_eq!(
            engine
                .tick_match_clock(Duration::from_millis(1101), 2)
                .err(),
            Some("log_limit_reached")
        );
        assert_eq!(engine.acquisition_round, Some(round));
        assert!(engine.round.outcome.is_none());
        assert!(engine.round.scores.is_none());
        assert!(engine.match_terminal.is_none());
        assert!(engine.pending_rewards.is_empty());
    }
}
