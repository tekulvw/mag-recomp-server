//! Frozen end-result authority and saved leadership snapshot application.
use crate::{leadership_passives::HeldLeadershipRole, leadership_runtime::LeadershipRecord, *};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub leadership_level: u8,
    pub nominations: u8,
    pub matches_since_last_command: u8,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AwardContext {
    pub role: u8,
    pub baseline: Baseline,
}
impl AwardContext {
    pub(crate) fn validate(self) -> Result<(), &'static str> {
        HeldLeadershipRole::from_native_role(self.role).map(|_| ())
    }
}
impl<W: Write> Engine<W> {
    pub(crate) fn leadership_reward_context(&self, actor: &Active) -> Option<AwardContext> {
        if !actor
            .reservation
            .character
            .leadership_progression_enabled
            .unwrap_or(false)
        {
            return None;
        }
        let baseline = self.leadership_record(&actor.reservation.character)?;
        let role = match self.held_leadership_role(actor)? {
            HeldLeadershipRole::None => 0,
            HeldLeadershipRole::SquadLeader => 1,
            HeldLeadershipRole::PlatoonLeader => 2,
            HeldLeadershipRole::OfficerInCharge => 4,
        };
        Some(AwardContext {
            role,
            baseline: Baseline {
                leadership_level: baseline.leadership_level,
                nominations: baseline.nominations,
                matches_since_last_command: baseline.matches_since_last_command,
            },
        })
    }
}

pub(crate) fn validate_snapshot(
    leadership: Option<LeadershipRecord>,
    match_points: Option<u32>,
) -> Result<(), &'static str> {
    let points = leadership.and_then(|record| record.leadership_points);
    if leadership.is_some() && points.is_none() {
        return Err("leadership_snapshot_points_required");
    }
    if let Some(gained) = match_points
        && (gained > i32::MAX as u32 || points.is_none_or(|total| gained > total))
    {
        return Err("invalid_match_leadership_points");
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct CommittedLeadership {
    pub record: LeadershipRecord,
    pub nominations: u8,
}
impl<W: Write> Engine<W> {
    pub(crate) fn committed_leadership(
        &self,
        actor: &Active,
        snapshot: Option<LeadershipRecord>,
    ) -> Option<CommittedLeadership> {
        let mut record = snapshot?;
        let points = record.leadership_points?;
        if let Some(prior) = actor
            .reservation
            .character
            .leadership
            .and_then(|r| r.leadership_points)
        {
            if points < prior {
                return None;
            }
            // At an equal balance, a later character-level settlement may
            // already have unlocked rank. A greater raw point word must keep
            // its native result, including the signed high-bit boundary.
            if points == prior {
                record.leadership_level = record.leadership_level.max(actor.leadership_level);
            }
        }
        Some(CommittedLeadership {
            nominations: self.match_nominations(record, actor.nominations),
            record,
        })
    }
}
impl Active {
    pub(crate) fn install_committed_leadership(&mut self, update: CommittedLeadership) {
        self.reservation.character.leadership = Some(update.record);
        self.leadership_level = update.record.leadership_level;
        self.nominations = update.nominations;
    }
}
