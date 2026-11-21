//! Shared retail round scores and native result publication. Values come from disk.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub(super) const MAX_PENDING_NOTIFICATIONS: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchTerminal {
    pub world_id: u32,
    pub match_id: String,
    pub reason: String,
    pub reservation_ids: Vec<String>,
}
#[derive(Clone, Debug)]
pub(crate) struct Outcome {
    pub winner: u8,
    pub major: bool,
    pub deadline: Duration,
    pub finalized: bool,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Round {
    pub scores: Option<[i16; 2]>,
    scored: BTreeSet<u16>,
    last_bleed: Option<i16>,
    pub outcome: Option<Outcome>,
}
#[derive(Clone, Copy)]
pub(crate) enum ScorePublication {
    Sabotage { objective_effect: bool },
    NativeScoreRate { before_bleed: [i16; 2] },
}

pub(crate) enum Publication {
    Application(Vec<u8>),
    Replication(Vec<u8>),
}
impl Active {
    fn queue_round_score(
        &mut self,
        team: usize,
        points: i16,
        reason: protocol::ScoreChangeReason,
        profile: wire_profile::WireProfile,
    ) -> Result<(), &'static str> {
        self.queue_round(Publication::Application(
            protocol::ScoreChange {
                team: team as u8,
                points,
                reason,
            }
            .encode_with_profile(profile)?,
        ));
        Ok(())
    }

    pub(crate) fn queue_round(&mut self, publication: Publication) {
        if self.pending_notifications.len() >= MAX_PENDING_NOTIFICATIONS {
            self.replication_backlogged = true;
        } else {
            self.pending_notifications.push_back(publication);
        }
    }
}
impl<W: Write> Engine<W> {
    pub(crate) fn round_combat_allowed(&self, now: Duration) -> bool {
        if let Some(round) = self
            .acquisition_round
            .as_ref()
            .or(self.domination_round.as_ref())
        {
            use acquisition_round::Phase;
            return match round.phase() {
                Phase::Playing => true,
                Phase::Ending { deadline_ms } => (now.as_millis() as u32) < deadline_ms,
                Phase::Finished => false,
            };
        }
        self.round
            .outcome
            .as_ref()
            .is_none_or(|end| !end.finalized && now < end.deadline)
    }

    pub(crate) fn tick_sabotage_round(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let Some(epoch) = self
            .match_epoch
            .or_else(|| self.active.as_ref().and_then(|a| a.match_clock_started))
        else {
            return Ok(Vec::new());
        };
        let catalog = self
            .objective_catalog
            .as_ref()
            .ok_or("objective_catalog_unavailable")?;
        let rules = catalog
            .match_rules
            .as_ref()
            .ok_or("match_rules_unavailable")?;
        let profile = self.match_profile();
        let mut next = self.round.clone();
        if next.scores.is_none() {
            next.scores = Some(profile.maximum_points);
            // A deliberately pre-completed fixture does not replay earlier effects.
            next.scored.extend(
                catalog
                    .objects
                    .iter()
                    .filter(|o| o.initial_state == 7)
                    .map(|o| o.object_index),
            );
        }
        let mut scores = next.scores.unwrap();
        let mut clock = self.score_clock;
        let mut effects = Vec::new();
        if next.outcome.is_none() {
            if let Some(world) = &self.objective_world {
                for object in &catalog.objects {
                    if world.state(object.object_index)
                        == Some(replication::ObjectiveState::Completed)
                        && !next.scored.contains(&object.object_index)
                    {
                        next.scored.insert(object.object_index);
                        if let Some(scoring) = &object.scoring {
                            let team = usize::from(scoring.owner_team);
                            scores[team] = scores[team]
                                .saturating_sub(scoring.activation_score as i16)
                                .max(0);
                            if scoring.activation_score > 0 {
                                effects.push(json!({"object_index":object.object_index,"team":team,"amount":scoring.activation_score,"reason":3}));
                            }
                        }
                    }
                }
            }
            if let Some(bleed) = &catalog.score_bleed {
                let paused = self
                    .objective_world
                    .as_ref()
                    .and_then(|w| w.state(bleed.pause_objective))
                    .is_some_and(|s| {
                        if bleed.pause_states.is_empty() {
                            s as u8 == bleed.pause_state
                        } else {
                            bleed.pause_states.contains(&(s as u8))
                        }
                    });
                clock = clock.advance(epoch, now, paused, bleed.interval_ms);
                let base = clock.points(
                    profile.maximum_points[usize::from(bleed.team)],
                    bleed.interval_ms,
                    bleed.points,
                );
                let lost = next
                    .last_bleed
                    .unwrap_or(profile.maximum_points[usize::from(bleed.team)])
                    .saturating_sub(base)
                    .max(0);
                scores[usize::from(bleed.team)] =
                    scores[usize::from(bleed.team)].saturating_sub(lost).max(0);
                next.last_bleed = Some(base);
            }
            if scores.contains(&0) {
                let winner = u8::from(scores[0] == 0);
                let major = i32::from(scores[usize::from(winner)]) * 100
                    >= i32::from(profile.maximum_points[usize::from(winner)])
                        * i32::from(rules.major_victory_percent);
                next.outcome = Some(Outcome {
                    winner,
                    major,
                    deadline: now.saturating_add(Duration::from_millis(
                        rules
                            .local_end_game_delay_ms
                            .unwrap_or(rules.end_game_delay_ms)
                            .into(),
                    )),
                    finalized: false,
                });
            }
        }
        if let Some(outcome) = &mut next.outcome
            && now >= outcome.deadline
        {
            outcome.finalized = true;
        }
        next.scores = Some(scores);
        let beginning = self.round.outcome.is_none() && next.outcome.is_some();
        let mut rewards = Vec::new();
        if beginning {
            let end = next.outcome.as_ref().unwrap();
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
                rewards.push(self.match_result_reward(actor, &recipient, end));
            }
        }
        let objective_effect = !effects.is_empty();
        self.commit_round_update(next, rewards, effects, now, wall)?;
        self.score_clock = clock;
        self.publish_round_update(now, ScorePublication::Sabotage { objective_effect })
    }

    pub(crate) fn match_result_reward(
        &self,
        actor: &Active,
        recipient: &progression::RewardRecipient,
        end: &Outcome,
    ) -> progression::RewardEvent {
        let kind = match (actor.team() == end.winner, end.major) {
            (true, true) => progression::RewardKind::MatchMajorVictory,
            (true, false) => progression::RewardKind::MatchMinorVictory,
            (false, true) => progression::RewardKind::MatchMajorDefeat,
            (false, false) => progression::RewardKind::MatchMinorDefeat,
        };
        let mut reward = recipient.event(
            format!(
                "match-result-{:016x}-{}",
                self.round_number, actor.reservation.id
            ),
            kind,
            1,
        );
        reward.leadership = self.leadership_reward_context(actor);
        reward
    }

    pub(crate) fn commit_round_update(
        &mut self,
        next: Round,
        rewards: Vec<progression::RewardEvent>,
        effects: Vec<Value>,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let start = next
            .outcome
            .as_ref()
            .filter(|_| self.round.outcome.is_none());
        let finishing = next.outcome.as_ref().is_some_and(|o| o.finalized)
            && !self.round.outcome.as_ref().is_some_and(|o| o.finalized);
        let notice = if let Some(outcome) = start {
            let world = self
                .active
                .as_ref()
                .or_else(|| self.peers.values().next())
                .ok_or("no_round_members")?
                .reservation
                .world;
            Some(MatchTerminal {
                world_id: world,
                match_id: format!("world-{world}-round-{:016x}", self.round_number),
                reason: if outcome.winner == 0 {
                    "attacker_victory"
                } else {
                    "defender_victory"
                }
                .to_owned(),
                reservation_ids: self
                    .peers
                    .values()
                    .chain(self.active.iter())
                    .map(|a| a.reservation.id.clone())
                    .collect(),
            })
        } else {
            None
        };
        if notice.is_some() {
            self.check_terminal_reward_capacity(&rewards)?;
        } else {
            self.check_reward_capacity(&rewards)?;
        }
        let changed = self.round.scores != next.scores
            || !effects.is_empty()
            || notice.is_some()
            || finishing;
        if changed {
            self.record(json!({"op":"match_round_tick","rewards":rewards,"scores":next.scores,"objective_effects":effects,"timestamp_ms":now.as_millis() as u32,
                "winner":next.outcome.as_ref().map(|o|o.winner),"major_victory":next.outcome.as_ref().map(|o|o.major),
                "end_time_ms":next.outcome.as_ref().map(|o|o.deadline.as_millis() as u32),"postgame":next.outcome.as_ref().is_some_and(|o|o.finalized)}),wall)?;
        }
        if let Some(notice) = notice {
            self.record(json!({"op":"match_terminal","match_terminal":notice}), wall)?;
            self.match_terminal = Some(notice);
            for actor in self.peers.values_mut().chain(self.active.iter_mut()) {
                actor.pending_spawn_deadline = None;
            }
        }
        self.round = next;
        self.commit_rewards(rewards);
        Ok(())
    }

    pub(crate) fn publish_round_update(
        &mut self,
        now: Duration,
        publication: ScorePublication,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let settled = !self
            .pending_rewards
            .values()
            .any(|r| Some(r.match_id.as_str()) == self.reward_match_id.as_deref());
        queue_round_publications(
            &self.round,
            self.wire_profile,
            self.peers
                .values_mut()
                .chain(self.active.iter_mut())
                .collect(),
            settled,
            now,
            publication,
        )?;
        Ok(Vec::new())
    }
    /// Application notices and phase deltas share one bounded FIFO, so stats cannot
    /// overtake their end/phase messages. Only replication packets enter NAK history.
    pub(super) fn drain_notifications(
        &mut self,
        mut messages: Vec<OutboundMessage>,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let limit = if self.maximum_players > 8 { 128 } else { 32 };
        let mut bytes: usize = messages.iter().map(|m| m.payload_hex.len() / 2).sum();
        let mut actors: Vec<_> = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter(|a| {
                a.ready_for_updates
                    && a.pending_shared.is_empty()
                    && !a.pending_notifications.is_empty()
            })
            .collect();
        actors.sort_unstable_by_key(|a| a.reservation.player);
        if !actors.is_empty() {
            let start = actors.partition_point(|a| a.reservation.player < self.shared_drain_cursor);
            let count = actors.len();
            actors.rotate_left(start % count);
        }
        let mut offsets = vec![0; actors.len()];
        let mut sequences = actors
            .iter()
            .map(|a| a.next_sequence())
            .collect::<Result<Vec<_>, _>>()?;
        let mut plans = Vec::new();
        loop {
            let mut progress = false;
            for (i, a) in actors.iter().enumerate() {
                if messages.len() + plans.len() >= limit {
                    break;
                }
                let Some(publication) = a.pending_notifications.get(offsets[i]) else {
                    continue;
                };
                let (wire, replicated) = match publication {
                    Publication::Application(w) => (w.clone(), false),
                    Publication::Replication(w) => (
                        replication::multiplayer::for_recipient_with_profile(
                            w,
                            sequences[i],
                            self.wire_profile,
                        )?,
                        true,
                    ),
                };
                if bytes + wire.len() > 24_000 {
                    continue;
                }
                bytes += wire.len();
                offsets[i] += 1;
                progress = true;
                if replicated {
                    sequences[i] = sequences[i]
                        .checked_add(1)
                        .ok_or("replication_sequence_exhausted")?;
                }
                plans.push((a.reservation.id.clone(), wire, replicated));
            }
            if !progress || messages.len() + plans.len() >= limit {
                break;
            }
        }
        let publications: Vec<_> = plans
            .iter()
            .map(|(id, w, _)| {
                let a = self
                    .active
                    .as_ref()
                    .filter(|a| a.reservation.id == *id)
                    .or_else(|| self.peers.get(id))
                    .unwrap();
                OutboundMessage {
                    reservation_id: id.clone(),
                    world_id: a.reservation.world,
                    player_id: a.reservation.player,
                    plugin_id: 1,
                    payload_hex: protocol::encode_hex(w),
                }
            })
            .collect();
        if !publications.is_empty() {
            self.record(
                json!({"op":"match_publications","messages":publications}),
                wall,
            )?;
        }
        for (id, w, replicated) in plans {
            let a = if self.active.as_ref().is_some_and(|a| a.reservation.id == id) {
                self.active.as_mut().unwrap()
            } else {
                self.peers.get_mut(&id).unwrap()
            };
            if replicated {
                a.remember(w)?;
            }
            a.pending_notifications.pop_front();
            self.shared_drain_cursor = a.reservation.player.wrapping_add(1);
        }
        messages.extend(publications);
        Ok(messages)
    }
}

/// Queue result publications identically for the live round and retained results.
pub(crate) fn queue_round_publications(
    round: &Round,
    profile: wire_profile::WireProfile,
    mut actors: Vec<&mut Active>,
    rewards_settled: bool,
    now: Duration,
    publication: ScorePublication,
) -> Result<(), &'static str> {
    let scores = round.scores.ok_or("round_scores_unavailable")?;
    let needs_stats = round.outcome.is_some()
        && rewards_settled
        && actors
            .iter()
            .any(|a| a.ready_for_updates && !a.results_announced);
    let final_stats: Vec<_> = actors
        .iter()
        .filter(|a| needs_stats && a.claimed.is_some())
        .map(|a| {
            protocol::StatUpdate {
                character_index: a.reservation.player,
                // Reward ACKs already include all earned XP in the durable
                // account snapshot. Publish that total as stat1 without
                // adding match XP again; stat2 remains the round total.
                stats: a
                    .reservation
                    .character
                    .progression
                    .into_iter()
                    .map(|state| protocol::StatValue {
                        stat: 1,
                        value: state.experience.min(i32::MAX as u32) as i32,
                    })
                    .chain(std::iter::once(protocol::StatValue {
                        stat: 2,
                        value: a.match_experience.min(i32::MAX as u32) as i32,
                    }))
                    .chain(
                        a.reward_statistics
                            .iter()
                            .map(|(&stat, &value)| protocol::StatValue { stat, value }),
                    )
                    .collect(),
            }
            .encode_with_profile(profile)
        })
        .collect::<Result<_, _>>()?;
    let outcome = round.outcome.clone();
    for recipient in actors
        .iter_mut()
        .filter(|a| a.initial_objects.is_some() && a.ready_for_updates)
    {
        use protocol::ScoreChangeReason::{Initial, PrimaryObjective, TimedBleed};
        let score_changed = recipient.round_scores != Some(scores)
            || matches!(publication,ScorePublication::NativeScoreRate {before_bleed} if recipient.round_scores!=Some(before_bleed));
        if score_changed && recipient.pending_notifications.len() + 4 > 128 {
            continue;
        }
        match (publication, recipient.round_scores) {
            (ScorePublication::NativeScoreRate { before_bleed }, Some(previous)) => {
                // Objective entries commit before this score-bleed update.
                // Preserve both notices if the same team changed twice.
                for (team, points) in before_bleed.into_iter().enumerate() {
                    if previous[team] != points {
                        recipient.queue_round_score(team, points, PrimaryObjective, profile)?;
                    }
                }
                for (team, points) in scores.into_iter().enumerate() {
                    if before_bleed[team] != points {
                        recipient.queue_round_score(team, points, TimedBleed, profile)?;
                    }
                }
            }
            _ if recipient.round_scores != Some(scores) => {
                let reason = match (publication, recipient.round_scores) {
                    (
                        ScorePublication::Sabotage {
                            objective_effect: true,
                        },
                        _,
                    ) => PrimaryObjective,
                    (_, None) => Initial,
                    _ => TimedBleed,
                };
                for (team, points) in scores.into_iter().enumerate() {
                    recipient.queue_round_score(team, points, reason, profile)?;
                }
            }
            _ => {}
        }
        recipient.round_scores = Some(scores);
        recipient.last_clock_points = scores[0];
        if let Some(end) = &outcome {
            if !recipient.end_announced {
                if recipient.pending_notifications.len() + 2 > 128 {
                    continue;
                }
                recipient.queue_round(Publication::Replication(
                    replication::multiplayer::MatchPhaseUpdate {
                        sequence: 1,
                        timestamp_ms: now.as_millis() as u32,
                        phase: replication::MatchPhase::EndCountdown,
                    }
                    .encode_with_profile(profile)?,
                ));
                recipient.queue_round(Publication::Application(
                    protocol::EndOfGame {
                        winning_team: end.winner,
                        major_victory: end.major,
                        end_time_ms: end.deadline.as_millis() as u32,
                    }
                    .encode_with_profile(profile)?,
                ));
                recipient.end_announced = true;
            }
            if rewards_settled && !recipient.results_announced {
                if recipient.pending_notifications.len() + final_stats.len() + 1 > 128 {
                    continue;
                }
                for stats in &final_stats {
                    recipient.queue_round(Publication::Application(stats.clone()));
                }
                recipient.queue_round(Publication::Application(
                    protocol::CharacterStatsComplete.encode(),
                ));
                recipient.results_announced = true;
            }
            if end.finalized && recipient.results_announced && !recipient.postgame_announced {
                if recipient.pending_notifications.len() >= 128 {
                    continue;
                }
                recipient.queue_round(Publication::Replication(
                    replication::multiplayer::MatchPhaseUpdate {
                        sequence: 1,
                        timestamp_ms: now.as_millis() as u32,
                        phase: replication::MatchPhase::PostGame,
                    }
                    .encode_with_profile(profile)?,
                ));
                recipient.postgame_announced = true;
            }
        }
    }
    Ok(())
}
