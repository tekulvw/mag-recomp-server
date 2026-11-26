//! In-match voice routing (docs/superpowers/specs/2026-11-25-voice-relay-design.md).
//!
//! Off unless the worker runs with `--voice-routing` (the `voice_routing` switch). Gameplay never
//! sees voice audio: the DME relays it. Gameplay only tells the DME, on the world stream, which
//! relay groups each human DME client listens to and may transmit to.
//!
//! - The client's talk-channel selection (message `0x0259`, one byte) is decoded and kept per
//!   human. It is advisory: it never widens what a player may transmit to.
//! - [`route`] is one pure function from a world's memberships (side, squad placement, held
//!   leadership role and the world's platoons per side) to per-player listen and transmit sets.
//!   A later proximity source joins it through [`ProximityAudience`] without changing the frame.
//! - [`Publisher`] numbers [`VoiceGroups`] frames. A frame always carries the full table. One is
//!   sent when a subscriber that asked for them registers (first enable, reconnect or resume)
//!   and whenever any membership changes, never merely because a tick ran.
//!
//! Group numbers follow the client's own: team = side (0, 1); company = 2 + side; platoon =
//! 4 + platoon (world squad index / 4); platoon broadcast = 12 + platoon; squad = 20 + world
//! squad index (0..31). Party groups (52..82) are never served. Proximity groups (83..338) are
//! reserved for the proximity source.
use crate::leadership_passives::HeldLeadershipRole;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::RangeInclusive;

/// Hello capability: this worker can publish `voice_groups` frames to a subscriber that asks.
pub const CAPABILITY: &str = "voice_groups_v1";
/// The client's talk-channel selection.
pub const CHANNEL_SELECT: u16 = crate::voice_diagnostics::CHANNEL_SELECT;
/// The client listens to at most this many groups.
pub const MAX_LISTEN_GROUPS: usize = 8;
/// A player may transmit to at most this many groups (four static, one proximity).
pub const MAX_TRANSMIT_GROUPS: usize = 8;
pub const SQUADS: u8 = 32;
pub const SQUADS_PER_SIDE: u8 = 16;
pub const SQUADS_PER_PLATOON: u8 = 4;
pub const MAX_PLATOONS_PER_SIDE: u8 = SQUADS_PER_SIDE / SQUADS_PER_PLATOON;
pub const PARTY_GROUPS: RangeInclusive<u16> = 52..=82;
pub const PROXIMITY_GROUPS: RangeInclusive<u16> = 83..=338;

/// The talk channel a client selected, by its wire value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TalkChannel {
    None,
    Squad,
    Platoon,
    Company,
    PlatoonBroadcast,
    TeamBroadcast,
    Proximity,
    Party,
}

impl TalkChannel {
    pub fn from_wire(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::None,
            1 => Self::Squad,
            2 => Self::Platoon,
            3 => Self::Company,
            4 => Self::PlatoonBroadcast,
            5 => Self::TeamBroadcast,
            6 => Self::Proximity,
            7 => Self::Party,
            _ => return None,
        })
    }

    pub fn wire(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Squad => 1,
            Self::Platoon => 2,
            Self::Company => 3,
            Self::PlatoonBroadcast => 4,
            Self::TeamBroadcast => 5,
            Self::Proximity => 6,
            Self::Party => 7,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Squad => "squad",
            Self::Platoon => "platoon",
            Self::Company => "company",
            Self::PlatoonBroadcast => "platoon_broadcast",
            Self::TeamBroadcast => "team_broadcast",
            Self::Proximity => "proximity",
            Self::Party => "party",
        }
    }
}

pub fn team_group(side: u8) -> u16 {
    u16::from(side)
}
pub fn company_group(side: u8) -> u16 {
    2 + u16::from(side)
}
/// `platoon` is the world platoon index, world squad index / 4 (0..7).
pub fn platoon_group(platoon: u8) -> u16 {
    4 + u16::from(platoon)
}
pub fn platoon_broadcast_group(platoon: u8) -> u16 {
    12 + u16::from(platoon)
}
/// `squad` is the world squad index (0..31).
pub fn squad_group(squad: u8) -> u16 {
    20 + u16::from(squad)
}

/// What routing needs to know about one human.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceMembership {
    side: u8,
    squad: Option<u8>,
    role: HeldLeadershipRole,
}

impl VoiceMembership {
    /// A human placed in world squad `squad` (0..31); its side follows from the squad.
    pub fn placed(squad: u8, role: HeldLeadershipRole) -> Option<Self> {
        (squad < SQUADS).then_some(Self {
            side: squad / SQUADS_PER_SIDE,
            squad: Some(squad),
            role,
        })
    }

    /// A human on `side` without a squad yet. Leadership needs a squad, so it has none.
    pub fn unplaced(side: u8) -> Option<Self> {
        (side < 2).then_some(Self {
            side,
            squad: None,
            role: HeldLeadershipRole::None,
        })
    }

    pub fn side(self) -> u8 {
        self.side
    }
    pub fn squad(self) -> Option<u8> {
        self.squad
    }
    pub fn role(self) -> HeldLeadershipRole {
        self.role
    }
}

/// One player's listen and transmit groups, ascending and without repeats.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSets {
    pub listen: Vec<u16>,
    pub transmit: Vec<u16>,
}

/// The retail channel tiers for one player. `platoons_per_side` is the world's platoon count a
/// side (1 in 64-player modes, which have no OIC and give the platoon leader team broadcast).
pub fn static_groups(member: VoiceMembership, platoons_per_side: u8) -> GroupSets {
    let side = member.side;
    let team = team_group(side);
    let Some(squad) = member.squad else {
        // Until placed: nothing to say, and only the side's broadcast to hear.
        return GroupSets {
            listen: vec![team],
            transmit: Vec::new(),
        };
    };
    let platoon = squad / SQUADS_PER_PLATOON;
    let own_squad = squad_group(squad);
    let own_platoon = platoon_group(platoon);
    let own_broadcast = platoon_broadcast_group(platoon);
    let own_company = company_group(side);
    let mut listen = vec![own_squad, own_broadcast, team];
    let transmit = match member.role {
        HeldLeadershipRole::None => vec![own_squad],
        HeldLeadershipRole::SquadLeader => {
            listen.push(own_platoon);
            vec![own_squad, own_platoon]
        }
        HeldLeadershipRole::PlatoonLeader => {
            listen.extend([own_platoon, own_company]);
            if platoons_per_side > 1 {
                vec![own_squad, own_platoon, own_company, own_broadcast]
            } else {
                vec![own_squad, own_platoon, team]
            }
        }
        HeldLeadershipRole::OfficerInCharge => {
            listen.push(own_company);
            vec![own_squad, own_platoon, own_company, team]
        }
    };
    GroupSets {
        listen: canonical(listen),
        transmit: canonical(transmit),
    }
}

fn canonical(mut groups: Vec<u16>) -> Vec<u16> {
    groups.sort_unstable();
    groups.dedup();
    groups
}

/// Room for proximity voice: per-talker audiences computed from positions, enemies included.
/// Each talker transmits to its own proximity group; each listener hears the proximity groups
/// of the talkers near it, nearest first. Nothing produces one yet.
#[derive(Clone, Debug)]
pub struct ProximityAudience<K> {
    pub transmit: BTreeMap<K, u16>,
    pub listen: BTreeMap<K, Vec<u16>>,
}

impl<K> Default for ProximityAudience<K> {
    fn default() -> Self {
        Self {
            transmit: BTreeMap::new(),
            listen: BTreeMap::new(),
        }
    }
}

/// Every member's listen and transmit sets. Static tiers come first; proximity groups fill the
/// remaining listen places, nearest talker first, so no player listens to more than
/// [`MAX_LISTEN_GROUPS`].
pub fn route<K: Ord + Clone>(
    members: &BTreeMap<K, VoiceMembership>,
    platoons_per_side: u8,
    proximity: Option<&ProximityAudience<K>>,
) -> BTreeMap<K, GroupSets> {
    members
        .iter()
        .map(|(key, member)| {
            let mut sets = static_groups(*member, platoons_per_side);
            if let Some(proximity) = proximity {
                if let Some(&own) = proximity.transmit.get(key)
                    && PROXIMITY_GROUPS.contains(&own)
                    && sets.transmit.len() < MAX_TRANSMIT_GROUPS
                {
                    sets.transmit.push(own);
                }
                let heard = proximity.listen.get(key).into_iter().flatten().copied();
                for group in heard.filter(|group| PROXIMITY_GROUPS.contains(group)) {
                    if sets.listen.len() >= MAX_LISTEN_GROUPS {
                        break;
                    }
                    if !sets.listen.contains(&group) {
                        sets.listen.push(group);
                    }
                }
            }
            sets.listen.truncate(MAX_LISTEN_GROUPS);
            sets.listen = canonical(sets.listen);
            sets.transmit = canonical(sets.transmit);
            (key.clone(), sets)
        })
        .collect()
}

/// Platoons a side, from the world's size: four squads a platoon.
pub fn platoons_per_side(team_capacity: usize, squad_capacity: usize) -> u8 {
    let platoon = squad_capacity.max(1) * usize::from(SQUADS_PER_PLATOON);
    team_capacity
        .div_ceil(platoon)
        .clamp(1, usize::from(MAX_PLATOONS_PER_SIDE)) as u8
}

/// One human DME client's entry in a `voice_groups` frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceGroupsMember {
    pub reservation_id: String,
    pub player_id: u16,
    pub side: u8,
    pub listen: Vec<u16>,
    pub transmit: Vec<u16>,
}

/// The `voice_groups` world-stream frame: the full relay table at one revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceGroups {
    pub revision: u64,
    pub members: Vec<VoiceGroupsMember>,
}

/// Numbers and deduplicates frames for the current subscriber.
#[derive(Debug, Default)]
pub struct Publisher {
    subscribed: bool,
    snapshot_due: bool,
    revision: u64,
    published: Option<Vec<VoiceGroupsMember>>,
}

impl Publisher {
    /// A subscriber registered. One that asked for frames gets a full one at a new revision.
    pub fn subscribe(&mut self, wants_frames: bool) {
        self.subscribed = wants_frames;
        self.snapshot_due = wants_frames;
    }

    pub fn subscribed(&self) -> bool {
        self.subscribed
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The frame to attach to the next world frame, if any: when a snapshot is due or the
    /// table differs from the last one published.
    pub fn next(&mut self, members: Vec<VoiceGroupsMember>) -> Option<VoiceGroups> {
        if !self.subscribed || (!self.snapshot_due && self.published.as_ref() == Some(&members)) {
            return None;
        }
        self.snapshot_due = false;
        self.revision += 1;
        self.published = Some(members.clone());
        Some(VoiceGroups {
            revision: self.revision,
            members,
        })
    }
}

/// Planning context for one decoded `0x0259`.
pub(crate) fn talk_channel_summary(
    channel: TalkChannel,
    squad_index: Option<u16>,
    role: HeldLeadershipRole,
) -> serde_json::Value {
    serde_json::json!({"type":"talk_channel","channel":channel.wire(),"channel_name":channel.name(),
        "squad_index":squad_index,"held_role_ability_level":role.ability_level()})
}

impl<W: std::io::Write> crate::Engine<W> {
    /// Explicit opt-in (`--voice-routing`): decode the talk channel, advertise [`CAPABILITY`]
    /// and publish `voice_groups` to a subscriber that asks for them.
    pub fn with_voice_routing(mut self) -> Self {
        self.voice = Some(Publisher::default());
        self
    }

    pub fn voice_routing_enabled(&self) -> bool {
        self.voice.is_some()
    }

    /// Called when a world subscriber registers, with whether it asked for `voice_groups`.
    pub fn subscribe_voice_groups(&mut self, wants_frames: bool) {
        if let Some(publisher) = self.voice.as_mut() {
            publisher.subscribe(wants_frames);
        }
    }

    fn voice_membership(&self, actor: &crate::Active) -> Option<VoiceMembership> {
        if actor.initial_objects.is_none() {
            return VoiceMembership::unplaced(actor.team());
        }
        let squad = u8::try_from(actor.squad_catalog.squad_index()).ok()?;
        VoiceMembership::placed(squad, self.current_held_role(actor))
    }

    /// The table for every human in the world, ordered by DME client index.
    pub(crate) fn voice_members(&self) -> Vec<VoiceGroupsMember> {
        let actors: BTreeMap<(u16, String), &crate::Active> = self
            .active
            .iter()
            .chain(self.peers.values())
            .map(|actor| {
                (
                    (actor.reservation.player, actor.reservation.id.clone()),
                    actor,
                )
            })
            .collect();
        let memberships: BTreeMap<(u16, String), VoiceMembership> = actors
            .iter()
            .filter_map(|(key, actor)| Some((key.clone(), self.voice_membership(actor)?)))
            .collect();
        let profile = self.match_profile();
        let platoons = platoons_per_side(profile.team_capacity, profile.squad_capacity);
        route(&memberships, platoons, None)
            .into_iter()
            .map(|((player_id, reservation_id), sets)| VoiceGroupsMember {
                side: memberships[&(player_id, reservation_id.clone())].side(),
                reservation_id,
                player_id,
                listen: sets.listen,
                transmit: sets.transmit,
            })
            .collect()
    }

    /// The `voice_groups` frame for this world frame, if one is due.
    pub(crate) fn next_voice_groups(&mut self) -> Option<VoiceGroups> {
        if !self.voice.as_ref()?.subscribed() {
            return None;
        }
        let members = self.voice_members();
        self.voice.as_mut()?.next(members)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HeldLeadershipRole::{OfficerInCharge, PlatoonLeader, SquadLeader};
    const MEMBER: HeldLeadershipRole = HeldLeadershipRole::None;

    fn sets(squad: u8, role: HeldLeadershipRole, platoons: u8) -> GroupSets {
        static_groups(VoiceMembership::placed(squad, role).unwrap(), platoons)
    }

    #[test]
    fn talk_channel_wire_values_round_trip() {
        for value in 0..=7 {
            assert_eq!(TalkChannel::from_wire(value).unwrap().wire(), value);
        }
        assert_eq!(TalkChannel::from_wire(1), Some(TalkChannel::Squad));
        assert_eq!(TalkChannel::from_wire(5), Some(TalkChannel::TeamBroadcast));
        assert_eq!(TalkChannel::from_wire(8), None);
        assert_eq!(TalkChannel::from_wire(255), None);
    }

    #[test]
    fn group_numbers_follow_side_platoon_and_squad() {
        assert_eq!((team_group(0), team_group(1)), (0, 1));
        assert_eq!((company_group(0), company_group(1)), (2, 3));
        assert_eq!((platoon_group(0), platoon_group(7)), (4, 11));
        assert_eq!(
            (platoon_broadcast_group(0), platoon_broadcast_group(7)),
            (12, 19)
        );
        assert_eq!((squad_group(0), squad_group(31)), (20, 51));
        assert!(PARTY_GROUPS.contains(&52) && !PARTY_GROUPS.contains(&squad_group(31)));
        assert!(VoiceMembership::placed(32, MEMBER).is_none());
        assert!(VoiceMembership::unplaced(2).is_none());
        assert_eq!(VoiceMembership::placed(17, MEMBER).unwrap().side(), 1);
    }

    #[test]
    fn member_hears_squad_platoon_broadcast_and_team() {
        // Side 1, squad 21 is platoon 5.
        assert_eq!(
            sets(21, MEMBER, 4),
            GroupSets {
                listen: vec![1, 17, 41],
                transmit: vec![41],
            }
        );
    }

    #[test]
    fn squad_leader_adds_own_platoon() {
        assert_eq!(
            sets(6, SquadLeader, 4),
            GroupSets {
                listen: vec![0, 5, 13, 26],
                transmit: vec![5, 26],
            }
        );
    }

    #[test]
    fn platoon_leader_with_several_platoons_reaches_company_and_broadcast() {
        assert_eq!(
            sets(6, PlatoonLeader, 2),
            GroupSets {
                listen: vec![0, 2, 5, 13, 26],
                transmit: vec![2, 5, 13, 26],
            }
        );
    }

    #[test]
    fn platoon_leader_with_one_platoon_a_side_reaches_the_team() {
        assert_eq!(
            sets(17, PlatoonLeader, 1),
            GroupSets {
                listen: vec![1, 3, 8, 16, 37],
                transmit: vec![1, 8, 37],
            }
        );
    }

    #[test]
    fn oic_hears_company_and_reaches_the_team() {
        assert_eq!(
            sets(0, OfficerInCharge, 4),
            GroupSets {
                listen: vec![0, 2, 12, 20],
                transmit: vec![0, 2, 4, 20],
            }
        );
    }

    #[test]
    fn without_a_squad_only_the_team_is_heard_and_nothing_sent() {
        assert_eq!(
            static_groups(VoiceMembership::unplaced(1).unwrap(), 4),
            GroupSets {
                listen: vec![1],
                transmit: vec![],
            }
        );
    }

    #[test]
    fn every_role_stays_within_the_listen_limit_and_never_names_a_party_group() {
        for squad in 0..SQUADS {
            for role in [MEMBER, SquadLeader, PlatoonLeader, OfficerInCharge] {
                for platoons in 1..=4 {
                    let s = sets(squad, role, platoons);
                    assert!(s.listen.len() <= 5 && s.transmit.len() <= 4);
                    let side = squad / 16;
                    for group in s.listen.iter().chain(&s.transmit) {
                        assert!(*group <= 51, "group {group}");
                        // Every static group belongs to the player's own side.
                        let group_side = match *group {
                            0 | 1 => *group as u8,
                            2 | 3 => (*group - 2) as u8,
                            4..=11 => ((*group - 4) / 4) as u8,
                            12..=19 => ((*group - 12) / 4) as u8,
                            _ => ((*group - 20) / 16) as u8,
                        };
                        assert_eq!(group_side, side);
                    }
                    // A player always hears what it may say, except platoon chat for the
                    // OIC, whose platoon leaders hear it.
                    for group in &s.transmit {
                        assert!(
                            s.listen.contains(group)
                                || (role == OfficerInCharge && *group == platoon_group(squad / 4))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn proximity_fills_listen_places_nearest_first_up_to_eight() {
        let members: BTreeMap<u16, VoiceMembership> = [
            (1, VoiceMembership::placed(0, MEMBER).unwrap()),
            (2, VoiceMembership::placed(20, MEMBER).unwrap()),
        ]
        .into();
        let audience = ProximityAudience {
            transmit: [(1, 84), (2, 85)].into(),
            // Six talkers near player 1, nearest first; one is a party group and is ignored.
            listen: [(1, vec![90, 60, 85, 91, 92, 93, 94, 95])].into(),
        };
        let routed = route(&members, 4, Some(&audience));
        let first = &routed[&1];
        assert_eq!(first.listen.len(), MAX_LISTEN_GROUPS);
        // Static three, then the five nearest proximity groups; 94 and 95 lose.
        assert_eq!(first.listen, vec![0, 12, 20, 85, 90, 91, 92, 93]);
        assert_eq!(first.transmit, vec![20, 84]);
        // Enemies may hear proximity: player 2 (side 1) transmits to 85, heard by player 1.
        assert_eq!(routed[&2].transmit, vec![40, 85]);
        assert_eq!(
            route(&members, 4, None)[&1],
            sets(0, MEMBER, 4),
            "without a proximity source routing is the static tiers"
        );
    }

    #[test]
    fn platoons_per_side_follow_the_world_size() {
        assert_eq!(platoons_per_side(32, 8), 1);
        assert_eq!(platoons_per_side(64, 8), 2);
        assert_eq!(platoons_per_side(128, 8), 4);
        assert_eq!(platoons_per_side(8, 8), 1);
        assert_eq!(platoons_per_side(500, 1), 4);
    }

    fn member(player_id: u16, listen: Vec<u16>) -> VoiceGroupsMember {
        VoiceGroupsMember {
            reservation_id: format!("synthetic-reservation-{player_id:02}"),
            player_id,
            side: 0,
            listen,
            transmit: vec![],
        }
    }

    #[test]
    fn publisher_sends_a_full_frame_on_subscribe_and_new_revisions_only_on_change() {
        let mut publisher = Publisher::default();
        let table = vec![member(1, vec![0])];
        assert_eq!(publisher.next(table.clone()), None, "not subscribed");
        publisher.subscribe(false);
        assert_eq!(publisher.next(table.clone()), None, "did not ask");
        publisher.subscribe(true);
        let first = publisher.next(table.clone()).unwrap();
        assert_eq!((first.revision, first.members.clone()), (1, table.clone()));
        assert_eq!(publisher.next(table.clone()), None, "unchanged");
        let placed = vec![member(1, vec![0, 12, 20])];
        assert_eq!(publisher.next(placed.clone()).unwrap().revision, 2);
        assert_eq!(publisher.next(placed.clone()), None);
        // A leave is a change too.
        assert_eq!(publisher.next(vec![]).unwrap().revision, 3);
        // A resumed or new subscription gets the full table again at a new revision.
        publisher.subscribe(true);
        let resumed = publisher.next(vec![]).unwrap();
        assert_eq!((resumed.revision, resumed.members.len()), (4, 0));
        assert_eq!(publisher.revision(), 4);
    }

    #[test]
    fn frame_serializes_with_the_documented_fields() {
        let frame = VoiceGroups {
            revision: 3,
            members: vec![VoiceGroupsMember {
                reservation_id: "synthetic-reservation-01".into(),
                player_id: 4,
                side: 1,
                listen: vec![1, 17, 41],
                transmit: vec![41],
            }],
        };
        let value = serde_json::to_value(&frame).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"revision":3,"members":[{"reservation_id":"synthetic-reservation-01",
                "player_id":4,"side":1,"listen":[1,17,41],"transmit":[41]}]})
        );
        assert_eq!(serde_json::from_value::<VoiceGroups>(value).unwrap(), frame);
    }
}
