//! Private authoritative event receipts. Horizon owns XP/level/point arithmetic
//! and durable deduplication; Rust only reports committed gameplay quantities.
use crate::{Active, Engine, OutboundMessage, Request, protocol, replication};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{io::Write, time::Duration};
pub const MAX_PENDING_REWARDS: usize = 256;
pub const MAX_REWARD_BATCH: usize = 32;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProgressionSnapshot {
    pub experience: u32,
    pub level: u8,
    pub skill_points: u8,
}
impl Default for ProgressionSnapshot {
    fn default() -> Self {
        Self {
            experience: 0,
            level: 1,
            skill_points: 0,
        }
    }
}
impl ProgressionSnapshot {
    pub fn validate(self) -> Result<(), &'static str> {
        // Horizon validates the disk-authored level table; this layer validates wire bounds only.
        if self.experience > i32::MAX as u32 || self.level == 0 {
            Err("invalid_progression_snapshot")
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RewardKind {
    EnemyKilled,
    EnemyFinished,
    TeammateHealed,
    TeammateResuscitated,
    RepairUnits,
    PrimaryObjectiveActivated,
    PrimaryObjectiveSecured,
    PrimaryObjectiveCompleted,
    SecondaryObjectiveCompleted,
    ChargePlanted,
    ChargeDefused,
    MatchMajorVictory,
    MatchMinorVictory,
    MatchMajorDefeat,
    MatchMinorDefeat,
}
impl RewardKind {
    pub(crate) fn is_frago_action(self, units: u32) -> bool {
        match self {
            Self::TeammateHealed | Self::RepairUnits => (1..=u32::from(u16::MAX)).contains(&units),
            Self::TeammateResuscitated => units == 1,
            _ => false,
        }
    }
    pub(crate) fn statistic(self) -> Option<u8> {
        match self {
            // Gameplay commits own kills, finishing moves (24) and revives (34).
            // Their later XP receipts must not increment those events again.
            Self::EnemyKilled | Self::EnemyFinished | Self::TeammateResuscitated => None,
            Self::PrimaryObjectiveActivated => Some(37),
            Self::PrimaryObjectiveSecured => Some(38),
            Self::PrimaryObjectiveCompleted => Some(39),
            Self::SecondaryObjectiveCompleted => Some(40),
            Self::ChargePlanted => Some(60),
            Self::ChargeDefused => Some(62),
            _ => None,
        }
    }
    pub(crate) fn is_match_result(self) -> bool {
        matches!(
            self,
            Self::MatchMajorVictory
                | Self::MatchMinorVictory
                | Self::MatchMajorDefeat
                | Self::MatchMinorDefeat
        )
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RewardEvent {
    pub match_id: String,
    pub reward_id: String,
    pub reservation_id: String,
    pub world_id: u32,
    pub player_id: u16,
    pub account_id: String,
    pub character_id: u32,
    pub kind: RewardKind,
    pub units: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leadership: Option<crate::leadership_progression::AwardContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frago: Option<bool>,
}
#[derive(Clone, Debug)]
pub(crate) struct RewardRecipient {
    pub match_id: String,
    pub reservation_id: String,
    pub world_id: u32,
    pub player_id: u16,
    pub account_id: String,
    pub character_id: u32,
}
impl RewardRecipient {
    /// Deferred effects retain ordinary credit identity, but new context must
    /// come from the same currently allocated character and round.
    pub(crate) fn matches_active(&self, active: &Active) -> bool {
        self.match_id == active.reward_match_id
            && self.reservation_id == active.reservation.id
            && self.world_id == active.reservation.world
            && self.player_id == active.reservation.player
            && active.reservation.character.account_id.as_deref() == Some(self.account_id.as_str())
            && active.reservation.character.character_id == Some(self.character_id)
    }
    pub(crate) fn from_active(active: &Active) -> Option<Self> {
        let character = &active.reservation.character;
        Some(Self {
            match_id: active.reward_match_id.clone(),
            reservation_id: active.reservation.id.clone(),
            world_id: active.reservation.world,
            player_id: active.reservation.player,
            account_id: character.account_id.clone()?,
            character_id: character.character_id?,
        })
    }
    /// Shared weapon-independent receipt identity for one represented bot life.
    /// The caller proves kill eligibility; no bot account/reservation is invented.
    pub(crate) fn bot_killed(&self, object_index: u16, generation: u32) -> RewardEvent {
        self.event(
            format!(
                "kill-{}-bot-{object_index:04x}-{generation:08x}",
                self.reservation_id
            ),
            RewardKind::EnemyKilled,
            1,
        )
    }
    /// Native death statistics count initial collapses, including a new collapse
    /// after same-object revival. Use that retained ordinal rather than treating
    /// a physical spawn generation as an entire rewardable life.
    pub(crate) fn human_killed(
        &self,
        victim: &crate::Reservation,
        generation: u32,
        collapse_ordinal: i32,
    ) -> RewardEvent {
        let identity = serde_json::json!([
            "human-collapse-v1",
            self.reservation_id,
            victim.id,
            victim.account,
            victim.character.character_id,
            generation,
            collapse_ordinal
        ]);
        self.event(identity.to_string(), RewardKind::EnemyKilled, 1)
    }
    pub(crate) fn event(&self, reward_id: String, kind: RewardKind, units: u32) -> RewardEvent {
        RewardEvent {
            match_id: self.match_id.clone(),
            reward_id: format!(
                "{:x}",
                Sha256::digest(format!("MAG-reward-v1\0{}\0{reward_id}", self.match_id).as_bytes())
            ),
            reservation_id: self.reservation_id.clone(),
            world_id: self.world_id,
            player_id: self.player_id,
            account_id: self.account_id.clone(),
            character_id: self.character_id,
            kind,
            units,
            leadership: None,
            frago: None,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct HealingSession {
    pub effect: crate::healing::HealingEffect,
    pub healer: Option<RewardRecipient>,
    pub effect_id: String,
}
impl std::ops::Deref for HealingSession {
    type Target = crate::healing::HealingEffect;
    fn deref(&self) -> &Self::Target {
        &self.effect
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommittedReward {
    pub reservation_id: String,
    pub world_id: u32,
    pub player_id: u16,
    pub progression: ProgressionSnapshot,
    pub match_experience: u32,
    pub leadership: Option<crate::leadership_runtime::LeadershipRecord>,
    pub match_leadership_points: Option<u32>,
    pub match_frago_points: Option<u32>,
}
/// A new round remains distinct even if an abandoned reservation ID is reused.
pub(crate) fn round_identity(reservation: &crate::Reservation, ordinal: u64) -> String {
    let mut digest = Sha256::new();
    digest.update(b"MAG-round-v1\0");
    digest.update(reservation.id.as_bytes());
    digest.update([0]);
    digest.update(reservation.world.to_be_bytes());
    digest.update(ordinal.to_be_bytes());
    format!("{:x}", digest.finalize())
}
/// Match history uses authenticated character identity, never a transport slot.
pub(crate) type ParticipantKey = (u32, String, u32);
#[derive(Clone, Debug, Default)]
pub(crate) struct RoundParticipation {
    pub match_experience: u32,
    pub statistics: std::collections::BTreeMap<u8, i32>,
    pub repair_credit: crate::repair_rewards::State,
}
pub(crate) fn participant_key(reservation: &crate::Reservation) -> Option<ParticipantKey> {
    Some((
        reservation.world,
        reservation.character.account_id.clone()?,
        reservation.character.character_id?,
    ))
}
impl<W: Write> Engine<W> {
    /// Retire a departed reservation id so a later Horizon release replays as
    /// acknowledged. Without match profiles the set is bounded to 512 ids.
    pub(crate) fn retire_reservation_id(&mut self, reservation_id: &str) {
        if self.released_reservations.insert(reservation_id.to_owned()) {
            self.release_order.push_back(reservation_id.to_owned());
        }
        while self.match_profiles.is_none() && self.release_order.len() > 512 {
            if let Some(old) = self.release_order.pop_front() {
                self.released_reservations.remove(&old);
            }
        }
    }
    pub(crate) fn remember_round_participant(&mut self, actor: &Active) {
        if self.match_profiles.is_some()
            && self
                .released_reservations
                .insert(actor.reservation.id.clone())
        {
            self.release_order.push_back(actor.reservation.id.clone());
        }
        if self.match_profiles.is_some()
            && let Some(receipt) = &actor.pending_loadout_selection
        {
            self.retired_loadout_selections
                .insert(receipt.selection_id.clone(), receipt.clone());
        }
        if self.reward_match_id.as_deref() == Some(&actor.reward_match_id)
            && let Some(key) = participant_key(&actor.reservation)
        {
            self.round_participants.insert(
                key,
                RoundParticipation {
                    match_experience: actor.match_experience,
                    statistics: actor.reward_statistics.clone(),
                    repair_credit: actor.repair_credit,
                },
            );
        }
    }

    pub(crate) fn pending_reward_batch(&self) -> Vec<RewardEvent> {
        self.reward_order
            .iter()
            .filter_map(|id| self.pending_rewards.get(id).cloned())
            .take(MAX_REWARD_BATCH)
            .collect()
    }
    pub(crate) fn check_reward_capacity(
        &self,
        rewards: &[RewardEvent],
    ) -> Result<(), &'static str> {
        for reward in rewards {
            if let Some(frago) = reward.frago
                && (!frago
                    || !(reward.kind == RewardKind::EnemyKilled && reward.units == 1
                        || reward.kind.is_frago_action(reward.units)))
            {
                return Err("invalid_frago_reward_context");
            }
            if let Some(context) = reward.leadership {
                context.validate()?;
                if !reward.kind.is_match_result() || reward.units != 1 {
                    return Err("leadership_reward_requires_match_result");
                }
            }
        }
        let additional = rewards
            .iter()
            .filter(|r| {
                !self.pending_rewards.contains_key(&r.reward_id)
                    && !self.committed_rewards.contains_key(&r.reward_id)
            })
            .count();
        if self.pending_rewards.len() + additional > MAX_PENDING_REWARDS {
            Err("reward_outbox_full")
        } else {
            Ok(())
        }
    }
    pub(crate) fn commit_rewards(&mut self, rewards: Vec<RewardEvent>) {
        for reward in rewards {
            if self.pending_rewards.contains_key(&reward.reward_id)
                || self.committed_rewards.contains_key(&reward.reward_id)
            {
                continue;
            }
            self.reward_order.push_back(reward.reward_id.clone());
            self.pending_rewards
                .insert(reward.reward_id.clone(), reward);
        }
    }
    pub(crate) fn attacker_is_friendly(&self, attacker: u32, world: u32, team: u8) -> bool {
        self.character_team_in_world(attacker, world) == Some(team)
    }
    pub(crate) fn acknowledge_progression(
        &mut self,
        request: Request,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let Request::ProgressionCommitted {
            reservation_id,
            world_id,
            player_id,
            reward_id,
            progression,
            match_experience,
            leadership,
            match_leadership_points,
            match_frago_points,
            ..
        } = request
        else {
            return Err("wrong_progression_operation");
        };
        progression.validate()?;
        crate::leadership_progression::validate_snapshot(leadership, match_leadership_points)?;
        crate::frago::validate_snapshot(match_frago_points, match_experience)?;
        if match_experience > i32::MAX as u32 {
            return Err("invalid_match_experience");
        }
        let receipt = CommittedReward {
            reservation_id: reservation_id.clone(),
            world_id,
            player_id,
            progression,
            match_experience,
            leadership,
            match_leadership_points,
            match_frago_points,
        };
        if let Some(previous) = self.committed_rewards.get(&reward_id) {
            return if previous == &receipt {
                Ok(Vec::new())
            } else {
                Err("reward_ack_conflict")
            };
        }
        let pending = self
            .pending_rewards
            .get(&reward_id)
            .ok_or("unknown_reward_id")?
            .clone();
        if pending.reservation_id != reservation_id
            || pending.world_id != world_id
            || pending.player_id != player_id
        {
            return Err("reward_identity_mismatch");
        }
        if pending.leadership.is_some() {
            if !pending.kind.is_match_result()
                || leadership.is_none()
                || match_leadership_points.is_none()
            {
                return Err("leadership_result_snapshot_required");
            }
        } else if match_leadership_points.is_some() {
            return Err("unexpected_match_leadership_points");
        }
        if pending.frago == Some(true) && match_frago_points.is_none() {
            return Err("frago_snapshot_required");
        }
        if pending.frago.is_none() && match_frago_points.is_some() {
            return Err("unexpected_match_frago_points");
        }
        let same_round = self.reward_match_id.as_deref() == Some(pending.match_id.as_str());
        let key = (
            pending.world_id,
            pending.account_id.clone(),
            pending.character_id,
        );
        // Account state can advance while its terminal receipt is deferred
        // into another round. Match totals remain scoped independently below.
        let actor = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|actor| participant_key(&actor.reservation).as_ref() == Some(&key));
        let actor_in_round =
            actor.is_some_and(|actor| same_round && actor.reward_match_id == pending.match_id);
        let recipient_id = actor.map(|a| a.reservation.id.clone());
        let recipient_player = actor.map_or(player_id, |a| a.reservation.player);
        let mut participation = if let Some(actor) = actor.filter(|_| actor_in_round) {
            RoundParticipation {
                match_experience: actor.match_experience,
                statistics: actor.reward_statistics.clone(),
                repair_credit: actor.repair_credit,
            }
        } else if same_round {
            self.round_participants
                .get(&key)
                .cloned()
                .unwrap_or_default()
        } else {
            RoundParticipation::default()
        };
        let match_total = match_experience.max(participation.match_experience);
        let statistic_update = if !same_round {
            None
        } else if let Some(stat) = pending.kind.statistic() {
            Some((
                stat,
                participation
                    .statistics
                    .get(&stat)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(pending.units.min(i32::MAX as u32) as i32),
            ))
        } else if pending.kind.is_match_result() && match_total > participation.match_experience {
            Some((
                4,
                (match_total - participation.match_experience).min(i32::MAX as u32) as i32,
            ))
        } else {
            None
        };
        let mut statistic_updates: Vec<_> = statistic_update.into_iter().collect();
        if let Some(points) = match_leadership_points
            && same_round
        {
            statistic_updates.push((7, points as i32));
        }
        if let Some(points) = match_frago_points
            && same_round
        {
            statistic_updates.push((
                3,
                (points as i32).max(participation.statistics.get(&3).copied().unwrap_or(0)),
            ));
        }
        let mut messages = Vec::new();
        let mut level_packet = None;
        let mut updated = None;
        if let Some(actor) = actor {
            let old = actor.reservation.character.progression.unwrap_or_default();
            let state = if progression.experience > old.experience {
                ProgressionSnapshot {
                    level: progression.level.max(old.level),
                    ..progression
                }
            } else {
                old
            };
            let mut stats = Vec::new();
            if actor_in_round {
                for &(stat, value) in &statistic_updates {
                    stats.push(protocol::StatValue { stat, value });
                }
            }
            if actor_in_round && match_total != actor.match_experience {
                stats.insert(
                    0,
                    protocol::StatValue {
                        stat: 2,
                        value: match_total as i32,
                    },
                );
            }
            if !stats.is_empty() {
                let wire = protocol::StatUpdate {
                    character_index: recipient_player,
                    stats,
                }
                .encode_with_profile(self.wire_profile)?;
                messages.push(OutboundMessage {
                    reservation_id: actor.reservation.id.clone(),
                    world_id,
                    player_id: recipient_player,
                    plugin_id: 1,
                    payload_hex: protocol::encode_hex(&wire),
                });
            }
            let leadership_update = self.committed_leadership(actor, leadership);
            let changed_leadership = leadership_update.is_some_and(|update| {
                update.record.leadership_level != actor.leadership_level
                    || update.nominations != actor.nominations
            });
            if (state.level > old.level || changed_leadership) && actor.initial_objects.is_some() {
                let mut mag_info = actor.mag_info(state.level);
                if let Some(update) = leadership_update {
                    mag_info.leadership_level = update.record.leadership_level;
                    mag_info.nominations = update.nominations;
                }
                level_packet = Some(
                    replication::CharacterLevelUpdate {
                        sequence: 1,
                        timestamp_ms: now.as_millis() as u32,
                        object_index: recipient_player,
                        mag_info,
                    }
                    .encode_with_profile(self.wire_profile)?,
                );
            }
            updated = Some((
                state,
                if actor_in_round {
                    match_total
                } else {
                    actor.match_experience
                },
                leadership_update,
            ));
        }
        // Stage the level delta in each recipient's own reliable sequence space
        // before recording the receipt. A recording failure releases neither the
        // stat notice nor the state commit; retry never needs to replay gameplay.
        let mut level_publications = Vec::new();
        if let Some(packet) = &level_packet {
            for actor in self
                .peers
                .values()
                .chain(self.active.iter())
                .filter(|a| a.initial_objects.is_some() && a.ready_for_updates)
            {
                let wire = replication::multiplayer::for_recipient_with_profile(
                    packet,
                    actor.next_sequence()?,
                    self.wire_profile,
                )?;
                messages.push(OutboundMessage {
                    reservation_id: actor.reservation.id.clone(),
                    world_id: actor.reservation.world,
                    player_id: actor.reservation.player,
                    plugin_id: 1,
                    payload_hex: protocol::encode_hex(&wire),
                });
                level_publications.push((actor.reservation.id.clone(), wire));
            }
        }
        self.record(json!({"op":"progression_committed","reward_id":reward_id,"reservation_id":reservation_id,"progression":progression,"match_experience":match_experience,"leadership":leadership,"match_leadership_points":match_leadership_points,"match_frago_points":match_frago_points,"messages":messages}),wall)?;
        if let Some((state, match_total, leadership_update)) = updated {
            let reservation_id = recipient_id.as_ref().unwrap();
            let actor = if self
                .active
                .as_ref()
                .is_some_and(|a| a.reservation.id == *reservation_id)
            {
                self.active.as_mut().unwrap()
            } else {
                self.peers.get_mut(reservation_id).unwrap()
            };
            actor.reservation.character.progression = Some(state);
            actor.match_experience = match_total;
            if let Some(update) = leadership_update {
                actor.install_committed_leadership(update);
            }
            if actor_in_round {
                for &(stat, value) in &statistic_updates {
                    actor.reward_statistics.insert(stat, value);
                }
            }
        }
        for (id, wire) in level_publications {
            let actor = if self.active.as_ref().is_some_and(|a| a.reservation.id == id) {
                self.active.as_mut().unwrap()
            } else {
                self.peers.get_mut(&id).unwrap()
            };
            actor.remember(wire)?;
        }
        if let Some(packet) = level_packet {
            for actor in self
                .peers
                .values_mut()
                .chain(self.active.iter_mut())
                .filter(|a| a.initial_objects.is_some() && !a.ready_for_updates)
            {
                actor.queue_shared(packet.clone());
            }
        }
        if same_round {
            participation.match_experience = match_total;
            for &(stat, value) in &statistic_updates {
                participation.statistics.insert(stat, value);
            }
            self.round_participants.insert(key, participation);
        }
        self.pending_rewards.remove(&reward_id);
        self.reward_order.retain(|id| id != &reward_id);
        self.committed_rewards.insert(reward_id.clone(), receipt);
        self.committed_reward_order.push_back(reward_id);
        while self.committed_reward_order.len() > MAX_PENDING_REWARDS {
            if let Some(id) = self.committed_reward_order.pop_front() {
                self.committed_rewards.remove(&id);
            }
        }
        Ok(messages)
    }
}

#[cfg(test)]
mod revive_reward_tests {
    use super::RewardKind;

    #[test]
    fn finish_and_revive_xp_receipts_preserve_event_owned_statistics() {
        for wire_kind in ["enemy_finished", "teammate_resuscitated"] {
            let kind: RewardKind = serde_json::from_value(serde_json::json!(wire_kind)).unwrap();
            assert_eq!(
                kind.statistic(),
                None,
                "XP acknowledgement must not count an event twice"
            );
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(wire_kind)
            );
            assert!(!kind.is_match_result());
        }
    }
}
