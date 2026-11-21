//! Private authoritative event receipts. Horizon owns XP/level/point arithmetic
//! and durable deduplication; Rust only reports committed gameplay quantities.
use crate::match_flow::Publication;
use crate::{
    Active, Engine, OutboundMessage, Request, leadership_progression::CommittedLeadership,
    protocol, replication,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Write, time::Duration};
pub const MAX_PENDING_REWARDS: usize = 256;
pub const MAX_REWARD_BATCH: usize = 32;
// Horizon accepts 128 deliveries per response. Leave one slot for the
// originating cohort's match-statistics packet.
const MAX_ACK_LEVEL_MESSAGES: usize = 127;
const MAX_REWARD_NOTIFICATIONS: usize = 128;
// One terminal callback per bounded human, with up to 128 authored objective
// vector rows (repetitions included) and one result. Wire batches stay at 32.
const MAX_TERMINAL_REWARDS: usize = crate::MAX_MATCH_PLAYERS * (128 + 1);
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
    DefensiveEmplacementsDestroyed,
    BridgesDestroyed,
    FortificationsDestroyed,
    GatesDestroyed,
    MortarsDestroyed,
    SensorNetworksDestroyed,
    AntiAircraftDestroyed,
    MotorPoolsDestroyed,
    EscortVehicleDelivered,
    MatchMajorVictory,
    MatchMinorVictory,
    MatchMajorDefeat,
    MatchMinorDefeat,
}
impl RewardKind {
    pub(crate) fn is_structural_destruction(self) -> bool {
        matches!(
            self,
            Self::DefensiveEmplacementsDestroyed
                | Self::BridgesDestroyed
                | Self::FortificationsDestroyed
                | Self::GatesDestroyed
                | Self::MortarsDestroyed
                | Self::SensorNetworksDestroyed
                | Self::AntiAircraftDestroyed
                | Self::MotorPoolsDestroyed
        )
    }
    pub(crate) fn is_extended_objective(self) -> bool {
        self.is_structural_destruction() || self == Self::EscortVehicleDelivered
    }
    pub(crate) fn is_frago_action(self, units: u32) -> bool {
        match self {
            Self::TeammateHealed | Self::RepairUnits => (1..=u32::from(u16::MAX)).contains(&units),
            Self::TeammateResuscitated => units == 1,
            _ => false,
        }
    }
    pub(crate) fn is_frago_objective(self, units: u32) -> bool {
        units == 1
            && (self.is_extended_objective()
                || matches!(
                    self,
                    Self::PrimaryObjectiveActivated
                        | Self::PrimaryObjectiveSecured
                        | Self::PrimaryObjectiveCompleted
                        | Self::SecondaryObjectiveCompleted
                        | Self::ChargePlanted
                        | Self::ChargeDefused
                ))
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
            Self::DefensiveEmplacementsDestroyed => Some(42),
            Self::BridgesDestroyed => Some(44),
            Self::FortificationsDestroyed => Some(46),
            Self::GatesDestroyed => Some(48),
            Self::MortarsDestroyed => Some(50),
            Self::SensorNetworksDestroyed => Some(52),
            Self::AntiAircraftDestroyed => Some(54),
            Self::MotorPoolsDestroyed => Some(56),
            Self::EscortVehicleDelivered => Some(59),
            _ => None,
        }
    }
    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::PrimaryObjectiveCompleted | Self::SecondaryObjectiveCompleted
        ) || self.is_match_result()
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
impl RoundParticipation {
    fn settle(
        &mut self,
        pending: &RewardEvent,
        receipt: &CommittedReward,
    ) -> Vec<protocol::StatValue> {
        let match_total = receipt.match_experience.max(self.match_experience);
        let add_units = |stat| {
            self.statistics
                .get(&stat)
                .copied()
                .unwrap_or(0)
                .saturating_add(pending.units.min(i32::MAX as u32) as i32)
        };
        let mut stats = Vec::new();
        if let Some(stat) = pending.kind.statistic() {
            stats.push((stat, add_units(stat)));
        } else if pending.kind.is_match_result() && match_total > self.match_experience {
            stats.push((4, (match_total - self.match_experience) as i32));
        }
        if pending.kind.is_structural_destruction() {
            stats.push((61, add_units(61)));
        }
        if let Some(points) = receipt.match_leadership_points {
            stats.push((7, points as i32));
        }
        if let Some(points) = receipt.match_frago_points {
            stats.push((
                3,
                (points as i32).max(self.statistics.get(&3).copied().unwrap_or(0)),
            ));
        }
        self.statistics.extend(stats.iter().copied());
        if self.match_experience != match_total {
            self.match_experience = match_total;
            stats.insert(0, (2, match_total as i32));
        }
        stats
            .into_iter()
            .map(|(stat, value)| protocol::StatValue { stat, value })
            .collect()
    }
}

struct RewardAcknowledgement<'a> {
    pending: &'a RewardEvent,
    receipt: &'a CommittedReward,
    now: Duration,
}

struct RewardSubjectUpdate {
    reservation_id: String,
    progression: ProgressionSnapshot,
    leadership: Option<CommittedLeadership>,
    original_actor: bool,
}

#[derive(Default)]
struct CohortRewardPlan {
    subject: Option<RewardSubjectUpdate>,
    participation: Option<RoundParticipation>,
    ordered: bool,
    stat_notification: Option<Vec<u8>>,
    level_packet: Option<Vec<u8>>,
    level_publications: BTreeMap<String, Vec<u8>>,
}
impl CohortRewardPlan {
    fn install<'a>(
        mut self,
        actors: impl Iterator<Item = &'a mut Active>,
        participants: &mut BTreeMap<ParticipantKey, RoundParticipation>,
        key: &ParticipantKey,
    ) {
        if let Some(subject) = self.subject {
            for actor in actors {
                if actor.reservation.id == subject.reservation_id {
                    actor.reservation.character.progression = Some(subject.progression);
                    if let Some(leadership) = subject.leadership {
                        actor.install_committed_leadership(leadership);
                    }
                    if subject.original_actor
                        && let Some(participation) = &self.participation
                    {
                        actor.match_experience = participation.match_experience;
                        actor
                            .reward_statistics
                            .clone_from(&participation.statistics);
                    }
                    if let Some(wire) = self.stat_notification.take() {
                        actor.queue_round(Publication::Application(wire));
                    }
                }
                if let Some(wire) = self.level_publications.remove(&actor.reservation.id) {
                    actor
                        .remember(wire)
                        .expect("validated reward replication plan");
                } else if actor.initial_objects.is_some()
                    && let Some(packet) = &self.level_packet
                {
                    if self.ordered {
                        if actor.ready_for_updates {
                            actor.queue_round(Publication::Replication(packet.clone()));
                        }
                    } else {
                        actor.queue_shared(packet.clone());
                    }
                }
            }
        }
        if let Some(participation) = self.participation {
            participants.insert(key.clone(), participation);
        }
    }
}

fn reward_message(recipient: &Active, wire: &[u8]) -> OutboundMessage {
    OutboundMessage {
        reservation_id: recipient.reservation.id.clone(),
        world_id: recipient.reservation.world,
        player_id: recipient.reservation.player,
        plugin_id: 1,
        payload_hex: protocol::encode_hex(wire),
    }
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
        if (self.match_profiles.is_some() || self.match_terminal.is_some())
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
        self.check_reward_capacity_for(rewards, false)
    }
    pub(crate) fn check_terminal_reward_capacity(
        &self,
        rewards: &[RewardEvent],
    ) -> Result<(), &'static str> {
        if rewards
            .iter()
            .any(|r| !r.kind.is_terminal() || r.units != 1)
        {
            return Err("invalid_terminal_reward");
        }
        self.check_reward_capacity_for(rewards, true)
    }
    fn check_reward_capacity_for(
        &self,
        rewards: &[RewardEvent],
        terminal: bool,
    ) -> Result<(), &'static str> {
        for reward in rewards {
            if let Some(frago) = reward.frago
                && (!frago
                    || !(reward.kind == RewardKind::EnemyKilled && reward.units == 1
                        || reward.kind.is_frago_action(reward.units)
                        || reward.kind.is_frago_objective(reward.units)))
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
        let pending_terminal = self
            .pending_rewards
            .values()
            .filter(|r| r.kind.is_terminal())
            .count();
        let pending_ordinary = self.pending_rewards.len() - pending_terminal;
        let ordinary = pending_ordinary + if terminal { 0 } else { additional };
        let terminal = pending_terminal + if terminal { additional } else { 0 };
        if ordinary > MAX_PENDING_REWARDS
            || terminal > MAX_TERMINAL_REWARDS
            || self.pending_rewards.len() + additional > MAX_PENDING_REWARDS + MAX_TERMINAL_REWARDS
        {
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
    fn plan_cohort_reward<'a>(
        &self,
        actors: impl Iterator<Item = &'a Active> + Clone,
        participation: Option<RoundParticipation>,
        acknowledgement: &RewardAcknowledgement<'_>,
        game_type: i8,
        ordered: bool,
        messages: &mut Vec<OutboundMessage>,
    ) -> Result<CohortRewardPlan, &'static str> {
        let RewardAcknowledgement {
            pending,
            receipt,
            now,
        } = *acknowledgement;
        let subject = actors.clone().find(|actor| {
            let character = &actor.reservation.character;
            character.account_id.as_deref() == Some(pending.account_id.as_str())
                && character.character_id == Some(pending.character_id)
        });
        let original_actor = subject.filter(|actor| {
            actor.reward_match_id == pending.match_id && actor.reservation.world == pending.world_id
        });
        let mut plan = CohortRewardPlan {
            participation,
            ordered,
            ..Default::default()
        };
        if let Some(participation) = &mut plan.participation {
            if let Some(actor) = original_actor {
                *participation = RoundParticipation {
                    match_experience: actor.match_experience,
                    statistics: actor.reward_statistics.clone(),
                    repair_credit: actor.repair_credit,
                };
            }
            let stats = participation.settle(pending, receipt);
            if let Some(actor) = original_actor
                && !stats.is_empty()
                && (!ordered || (actor.initial_objects.is_some() && actor.ready_for_updates))
            {
                let wire = protocol::StatUpdate {
                    character_index: actor.reservation.player,
                    stats,
                }
                .encode_with_profile(actor.wire_profile)?;
                if ordered {
                    plan.stat_notification = Some(wire);
                } else {
                    messages.push(reward_message(actor, &wire));
                }
            }
        }
        let Some(actor) = subject else {
            return Ok(plan);
        };
        let old = actor.reservation.character.progression.unwrap_or_default();
        let progression = if receipt.progression.experience > old.experience {
            ProgressionSnapshot {
                level: receipt.progression.level.max(old.level),
                ..receipt.progression
            }
        } else {
            old
        };
        let leadership = self.committed_leadership(
            actor,
            receipt.leadership,
            receipt.progression.experience,
            game_type,
        );
        let changed_leadership = leadership.is_some_and(|update| {
            update.record.leadership_level != actor.leadership_level
                || update.nominations != actor.nominations
        });
        if (progression.level > old.level || changed_leadership) && actor.initial_objects.is_some()
        {
            let mut mag_info = actor.mag_info(progression.level);
            if let Some(update) = leadership {
                mag_info.leadership_level = update.record.leadership_level;
                mag_info.nominations = update.nominations;
            }
            // These avatars have independent object indices, wire profiles and
            // reliable sequences even when they represent the same character.
            let update = replication::CharacterLevelUpdate {
                sequence: 1,
                timestamp_ms: now.as_millis() as u32,
                object_index: actor.reservation.player,
                mag_info,
            };
            plan.level_packet = Some(update.encode_with_profile(actor.wire_profile)?);
            for recipient in actors
                .clone()
                .filter(|a| {
                    !ordered
                        && a.initial_objects.is_some()
                        && a.ready_for_updates
                        && a.pending_shared.is_empty()
                })
                .take(MAX_ACK_LEVEL_MESSAGES.saturating_sub(messages.len()))
            {
                let wire = replication::CharacterLevelUpdate {
                    sequence: recipient.next_sequence()?,
                    ..update.clone()
                }
                .encode_with_profile(recipient.wire_profile)?;
                messages.push(reward_message(recipient, &wire));
                plan.level_publications
                    .insert(recipient.reservation.id.clone(), wire);
            }
        }
        // The archived result stream owns both End and its following stats.
        // If it has not reserved space for End or these deltas yet, retain the
        // pending receipt for a later poll after the stream drains.
        if ordered {
            for recipient in actors.filter(|a| a.initial_objects.is_some() && a.ready_for_updates) {
                let notifications = usize::from(plan.level_packet.is_some())
                    + usize::from(
                        std::ptr::eq(recipient, actor) && plan.stat_notification.is_some(),
                    );
                if notifications != 0 {
                    if !recipient.end_announced {
                        return Err("reward_end_not_queued");
                    }
                    if recipient.pending_notifications.len() + notifications
                        > MAX_REWARD_NOTIFICATIONS
                    {
                        return Err("reward_notifications_full");
                    }
                }
            }
        }
        plan.subject = Some(RewardSubjectUpdate {
            reservation_id: actor.reservation.id.clone(),
            progression,
            leadership,
            original_actor: original_actor.is_some(),
        });
        Ok(plan)
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
        let key = (
            pending.world_id,
            pending.account_id.clone(),
            pending.character_id,
        );
        let participation = |match_id: Option<&str>, participants: &BTreeMap<_, _>| {
            (match_id == Some(pending.match_id.as_str()))
                .then(|| participants.get(&key).cloned().unwrap_or_default())
        };
        let acknowledgement = RewardAcknowledgement {
            pending: &pending,
            receipt: &receipt,
            now,
        };
        let mut messages = Vec::new();
        let live = self.plan_cohort_reward(
            self.peers.values().chain(self.active.iter()),
            participation(self.reward_match_id.as_deref(), &self.round_participants),
            &acknowledgement,
            self.match_profile().game_type,
            false,
            &mut messages,
        )?;
        let mut retained = Vec::new();
        for (match_id, cohort) in &self.results_cohorts {
            let plan = self.plan_cohort_reward(
                cohort.actors.values(),
                participation(Some(match_id), &cohort.participants),
                &acknowledgement,
                cohort.game_type,
                self.world_tick_enabled,
                &mut messages,
            )?;
            if plan.subject.is_some() || plan.participation.is_some() {
                retained.push((match_id.clone(), plan));
            }
        }
        self.record(json!({"op":"progression_committed","reward_id":reward_id,"reservation_id":reservation_id,"progression":progression,"match_experience":match_experience,"leadership":leadership,"match_leadership_points":match_leadership_points,"match_frago_points":match_frago_points,"messages":messages}),wall)?;
        live.install(
            self.peers.values_mut().chain(self.active.iter_mut()),
            &mut self.round_participants,
            &key,
        );
        for (match_id, plan) in retained {
            let cohort = self.results_cohorts.get_mut(&match_id).unwrap();
            plan.install(cohort.actors.values_mut(), &mut cohort.participants, &key);
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

#[cfg(test)]
#[path = "match_cohort_reward_tests.rs"]
mod match_cohort_reward_tests;
