//! Internal cohort admission for explicitly enabled retail workers: into a
//! forming match before any claim, and (late entry) into a running match.
use crate::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const CAPABILITY: &str = "party_reservation_v1";
/// A group may also enter a running (already claimed) match of its formation.
/// It joins one squad on its side when one has room for every member, or
/// else the fewest squads on that side; bots give up the seats it takes.
pub(crate) const LATE_ENTRY_CAPABILITY: &str = "party_late_entry_v1";
// Retain failed-reserve rollback tombstones for the whole selection, including
// when there are no actors. Never evict one and allow a late reserve through.
const MAX_COHORTS_PER_FORMATION: usize = 128;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub reservation_id: String,
    pub player_id: u16,
    pub account_id: u32,
    pub session_key: String,
    pub access_key: String,
    pub character: Character,
}

impl Member {
    pub(super) fn validate(&self) -> Result<(), &'static str> {
        validate_reservation_fields(
            &self.reservation_id,
            self.account_id,
            &self.session_key,
            &self.access_key,
            &self.character,
        )
    }

    fn reservation(&self, world_id: u32) -> Reservation {
        Reservation {
            id: self.reservation_id.clone(),
            world: world_id,
            player: self.player_id,
            account: self.account_id,
            session_key: self.session_key.clone(),
            access_key: self.access_key.clone(),
            character: self.character.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Reserved,
    Released,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub status: Status,
    pub formation_id: String,
    pub profile_id: String,
    pub cohort_id: String,
    pub world_id: u32,
    pub admission_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squad: Option<u16>,
    pub reservation_ids: Vec<String>,
}

#[derive(Clone, Copy)]
struct Binding<'a> {
    formation_id: &'a str,
    profile_id: &'a str,
    cohort_id: &'a str,
    world_id: u32,
    admission_epoch: u64,
}

impl Binding<'_> {
    fn reply(self, status: Status, squad: Option<u16>, reservation_ids: Vec<String>) -> Reply {
        Reply {
            status,
            formation_id: self.formation_id.into(),
            profile_id: self.profile_id.into(),
            cohort_id: self.cohort_id.into(),
            world_id: self.world_id,
            admission_epoch: self.admission_epoch,
            team: squad.map(|index| (index / 16) as u8),
            squad,
            reservation_ids,
        }
    }

    fn matches(self, reply: &Reply) -> bool {
        reply.formation_id == self.formation_id
            && reply.profile_id == self.profile_id
            && reply.cohort_id == self.cohort_id
            && reply.world_id == self.world_id
            && reply.admission_epoch == self.admission_epoch
    }
}

enum Cohort {
    Reserved {
        reply: Reply,
        // Store the immutable request fingerprint separately from live actor
        // records, whose selected kit/progression can change after claim.
        fingerprint: [u8; 32],
        allocation_sequence: u64,
    },
    Released(Reply),
}

impl Cohort {
    fn reply(&self) -> &Reply {
        match self {
            Self::Reserved { reply, .. } | Self::Released(reply) => reply,
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    enabled: bool,
    cohorts: BTreeMap<String, Cohort>,
}

impl State {
    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn clear(&mut self) {
        self.cohorts.clear();
    }

    fn lookup(&self, binding: Binding<'_>) -> Result<Option<&Cohort>, &'static str> {
        let entry = self.cohorts.get(binding.cohort_id);
        if let Some(entry) = entry
            && !binding.matches(entry.reply())
        {
            return Err("party_cohort_conflict");
        }
        Ok(entry)
    }

    fn check_room(&self) -> Result<(), &'static str> {
        if self.cohorts.len() >= MAX_COHORTS_PER_FORMATION {
            Err("party_cohort_history_full")
        } else {
            Ok(())
        }
    }
}

pub(super) fn validate_scope(
    formation_id: &str,
    profile_id: &str,
    cohort_id: &str,
    admission_epoch: u64,
) -> Result<(), &'static str> {
    if !identifier(formation_id, 1)
        || !identifier(profile_id, 1)
        || !identifier(cohort_id, 1)
        || admission_epoch == 0
    {
        return Err("invalid_party_reservation_scope");
    }
    Ok(())
}

impl<W: Write> Engine<W> {
    pub fn configure_party_reservations(&mut self) -> Result<(), &'static str> {
        if self.wire_profile.application_id() != 20231 || self.match_profiles.is_none() {
            return Err("party_reservations_require_retail_match_profiles");
        }
        if self.selected_match.is_some() || self.active.is_some() || !self.peers.is_empty() {
            return Err("party_reservations_require_unselected_worker");
        }
        self.party_reservations.enabled = true;
        Ok(())
    }

    pub(super) fn handle_party_request(
        &mut self,
        request: &Request,
        now: Duration,
        wall: u64,
    ) -> Option<Result<Reply, &'static str>> {
        let (binding, members) = match request {
            Request::ReserveParty {
                formation_id,
                profile_id,
                cohort_id,
                world_id,
                admission_epoch,
                members,
                ..
            } => (
                Binding {
                    formation_id,
                    profile_id,
                    cohort_id,
                    world_id: *world_id,
                    admission_epoch: *admission_epoch,
                },
                Some(members.as_slice()),
            ),
            Request::ReleaseParty {
                formation_id,
                profile_id,
                cohort_id,
                world_id,
                admission_epoch,
                ..
            } => (
                Binding {
                    formation_id,
                    profile_id,
                    cohort_id,
                    world_id: *world_id,
                    admission_epoch: *admission_epoch,
                },
                None,
            ),
            _ => return None,
        };
        Some(self.party_operation(binding, members, now, wall))
    }

    fn party_operation(
        &mut self,
        binding: Binding<'_>,
        members: Option<&[Member]>,
        now: Duration,
        wall: u64,
    ) -> Result<Reply, &'static str> {
        if self.failed {
            return Err("recorder_unavailable");
        }
        if !self.party_reservations.enabled() || self.wire_profile.application_id() != 20231 {
            return Err("party_reservations_disabled");
        }
        // Normal empty-world cleanup retires its selection. Preserve exact
        // rollback acknowledgements until another selection actually commits.
        if self.selected_match.is_none()
            && self.retired_formations.contains(binding.formation_id)
            && let Some(Cohort::Released(reply)) = self.party_reservations.lookup(binding)?
        {
            return if members.is_none() {
                Ok(reply.clone())
            } else {
                Err("party_cohort_released")
            };
        }
        self.check_party_scope(binding)?;
        match members {
            Some(members) => self.reserve_party(binding, members, now, wall),
            None => self.release_party(binding, wall),
        }
    }

    fn check_party_scope(&self, binding: Binding<'_>) -> Result<(), &'static str> {
        if self.match_terminal.is_some() && self.selected_match_is(binding) {
            return Err("party_match_ending");
        }
        if self.match_terminal.is_some()
            || self.round_number > binding.admission_epoch
            || !self.selected_match_is(binding)
        {
            return Err("party_formation_stale");
        }
        if self
            .active
            .iter()
            .chain(self.peers.values())
            .any(|actor| actor.reservation.world != binding.world_id)
            || self
                .results_cohorts
                .values()
                .any(|cohort| cohort.terminal.world_id != binding.world_id)
        {
            return Err("party_world_conflict");
        }
        Ok(())
    }

    fn selected_match_is(&self, binding: Binding<'_>) -> bool {
        self.selected_match.as_ref().is_some_and(|selected| {
            selected.formation_id == binding.formation_id
                && selected.profile_id == binding.profile_id
                && selected.admission_epoch == Some(binding.admission_epoch)
        })
    }

    fn party_member(&self, id: &str, world_id: u32, allocation: u64) -> Option<&Active> {
        self.active
            .as_ref()
            .filter(|actor| actor.reservation.id == id)
            .or_else(|| self.peers.get(id))
            .filter(|actor| {
                actor.reservation.world == world_id && actor.allocation_sequence == allocation
            })
    }

    fn reserve_party(
        &mut self,
        binding: Binding<'_>,
        members: &[Member],
        now: Duration,
        wall: u64,
    ) -> Result<Reply, &'static str> {
        let canonical = serde_json::to_vec(members).map_err(|_| "invalid_party_members")?;
        let fingerprint = Sha256::digest(canonical).into();
        match self.party_reservations.lookup(binding)? {
            Some(Cohort::Released(_)) => return Err("party_cohort_released"),
            Some(Cohort::Reserved {
                reply,
                fingerprint: accepted,
                allocation_sequence,
            }) => {
                if *accepted != fingerprint {
                    return Err("party_cohort_conflict");
                }
                if self.round_number != binding.admission_epoch
                    || reply.reservation_ids.iter().any(|id| {
                        self.party_member(id, binding.world_id, *allocation_sequence)
                            .is_none_or(|actor| {
                                now.saturating_sub(actor.last_event) >= IDLE_TIMEOUT
                            })
                    })
                {
                    return Err("party_cohort_stale");
                }
                return Ok(reply.clone());
            }
            None => {}
        }
        self.party_reservations.check_room()?;
        let (squad, late, prepared) = self.prepare_party(binding, members)?;
        let reply = binding.reply(
            Status::Reserved,
            Some(squad),
            members
                .iter()
                .map(|member| member.reservation_id.clone())
                .collect(),
        );
        let accounts: Vec<_> = members
            .iter()
            .map(|member| json!({"account_id":member.account_id,"player_id":member.player_id}))
            .collect();
        let squads: BTreeSet<u16> = prepared
            .iter()
            .map(|admission| admission.squad_catalog.squad_index())
            .collect();
        self.record(
            json!({"op":"party_reserved","party_reservation":reply,"members":accounts,
                "late_entry":late,"squads":squads}),
            wall,
        )?;
        let previous = self.active.take();
        for admission in prepared {
            self.install_reserved(admission, now);
            let actor = self
                .active
                .take()
                .expect("prepared party admission installed");
            self.peers.insert(actor.reservation.id.clone(), actor);
        }
        self.active = previous;
        self.party_reservations.cohorts.insert(
            binding.cohort_id.into(),
            Cohort::Reserved {
                reply: reply.clone(),
                fingerprint,
                allocation_sequence: self.sequence,
            },
        );
        if late {
            let players: Vec<u16> = members.iter().map(|member| member.player_id).collect();
            // The cohort is durable and installed. Bot seats are derived world
            // state: if this eager displacement stops early, the next bot tick
            // reconciles the same squads and team, so the reply stays committed.
            let _ = self.displace_bots_for_party(&players, &squads, squad / 16, now, wall);
        }
        Ok(reply)
    }

    /// Retire bots that occupy a late group's player indices, overfill its
    /// squads or push its side past the team capacity. Humans own their seats.
    fn displace_bots_for_party(
        &mut self,
        players: &[u16],
        squads: &BTreeSet<u16>,
        team: u16,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        if self.bot_registry.bots.is_empty() {
            return Ok(());
        }
        for &player in players {
            self.retire_bot_for_human(player, now, wall)?;
        }
        let policy = self.match_profile();
        let (squad_capacity, team_capacity) = (policy.squad_capacity, policy.team_capacity);
        for &squad in squads {
            while self.squad_reservation_count(squad) + self.bot_registry.members(squad).count()
                > squad_capacity
            {
                let Some(id) = self.bot_registry.members(squad).max() else {
                    break;
                };
                self.retire_bot_for_human(id, now, wall)?;
            }
        }
        let team = team as u8;
        loop {
            let humans = self
                .active
                .iter()
                .chain(self.peers.values())
                .filter(|actor| actor.team() == team)
                .count();
            let bots: Vec<u16> = self
                .bot_registry
                .bots
                .iter()
                .filter(|(_, bot)| bot.target.squad.team() == team)
                .map(|(&id, _)| id)
                .collect();
            if humans + bots.len() <= team_capacity {
                return Ok(());
            }
            let Some(&id) = bots.last() else {
                return Ok(());
            };
            self.retire_bot_for_human(id, now, wall)?;
        }
    }

    fn prepare_party(
        &self,
        binding: Binding<'_>,
        members: &[Member],
    ) -> Result<(u16, bool, Vec<admission::PreparedAdmission>), &'static str> {
        self.validate_objective_configuration(self.wire_profile)?;
        let policy = self.match_profile();
        if !policy.auto_assign_teams {
            return Err("party_profile_unavailable");
        }
        let current: Vec<_> = self.active.iter().chain(self.peers.values()).collect();
        // Once any member of the formation has claimed, the match is running:
        // a group enters late, beside the humans and bots already playing.
        let late = current.iter().any(|actor| actor.claimed.is_some());
        let round_number = if current.is_empty() {
            self.round_number
                .checked_add(1)
                .ok_or("round_number_exhausted")?
        } else {
            self.round_number
        };
        if round_number != binding.admission_epoch {
            return Err("party_formation_stale");
        }
        if current.len() + members.len() > self.maximum_players {
            return Err("party_no_room");
        }
        let mut reservations = BTreeSet::new();
        let mut players = BTreeSet::new();
        let mut accounts = BTreeSet::new();
        let faction = members[0].character.faction;
        if faction != policy.attacking_faction && faction != policy.defending_faction {
            return Err("party_faction_mismatch");
        }
        let mut candidates = Vec::with_capacity(members.len());
        for member in members {
            if usize::from(member.player_id) >= self.maximum_players {
                return Err("invalid_player_slot");
            }
            if member.character.faction != faction {
                return Err("party_faction_mismatch");
            }
            if member.character.universe_id != 26 {
                return Err("unsupported_match_character");
            }
            if self.released_reservations.contains(&member.reservation_id)
                || self.results_owner(&member.reservation_id).is_some()
            {
                return Err("reservation_id_retired");
            }
            if !reservations.insert(member.reservation_id.as_str())
                || !players.insert(member.player_id)
                || !accounts.insert(member.account_id)
                || current.iter().any(|actor| {
                    actor.reservation.id == member.reservation_id
                        || actor.reservation.player == member.player_id
                        || actor.reservation.account == member.account_id
                })
            {
                return Err("party_member_conflict");
            }
            let modifiers = self.validate_admission_character(&member.character)?;
            candidates.push((member.reservation(binding.world_id), modifiers));
        }
        let catalogs = match self.allocate_squad_for(faction, members.len()) {
            Ok(catalog) => vec![catalog; members.len()],
            Err("match_full") if late => self.allocate_split_party(faction, members.len())?,
            Err("match_full") => return Err("party_no_room"),
            Err("unsupported_match_character") => return Err("party_faction_mismatch"),
            Err(code) => return Err(code),
        };
        let mut prepared = Vec::with_capacity(members.len());
        for ((candidate, modifiers), mut squad_catalog) in candidates.into_iter().zip(catalogs) {
            let shared_spawn = self
                .squad_spawn_for(squad_catalog.squad_index())
                .ok_or("missing_squad_spawn")?;
            squad_catalog.set_runtime_default(shared_spawn)?;
            prepared.push(admission::PreparedAdmission {
                candidate,
                modifiers,
                squad_catalog,
                shared_spawn,
                round_number,
            });
        }
        Ok((prepared[0].squad_catalog.squad_index(), late, prepared))
    }

    /// Late-entry fallback when no single squad on the group's side has room
    /// for all of it: keep the group on its side in the fewest squads, filling
    /// the emptiest squad first (the leader's). Bots never count as occupants.
    pub(crate) fn allocate_split_party(
        &self,
        faction: u8,
        members: usize,
    ) -> Result<Vec<spawn_catalog::SpawnCatalog>, &'static str> {
        let catalog = self
            .spawn_catalog
            .as_ref()
            .ok_or("spawn_catalog_unavailable")?;
        let policy = catalog.match_profile();
        let team = if faction == policy.attacking_faction {
            0
        } else {
            1
        };
        let humans = self
            .active
            .iter()
            .chain(self.peers.values())
            .filter(|actor| actor.team() == team)
            .count();
        if humans + members > self.human_team_capacity() {
            return Err("party_no_room");
        }
        let mut vacancies: Vec<_> = catalog
            .map_pool()
            .squads
            .iter()
            .filter(|binding| binding.team == team)
            .map(|binding| {
                let free = policy
                    .squad_capacity
                    .saturating_sub(self.squad_reservation_count(binding.index));
                (std::cmp::Reverse(free), binding.index)
            })
            .filter(|(free, _)| free.0 > 0)
            .collect();
        vacancies.sort_unstable();
        let mut catalogs = Vec::with_capacity(members);
        for (std::cmp::Reverse(free), index) in vacancies {
            let squad = catalog.for_squad(index)?;
            for _ in 0..free.min(members - catalogs.len()) {
                catalogs.push(squad.clone());
            }
            if catalogs.len() == members {
                return Ok(catalogs);
            }
        }
        Err("party_no_room")
    }

    fn release_party(&mut self, binding: Binding<'_>, wall: u64) -> Result<Reply, &'static str> {
        let reservations = match self.party_reservations.lookup(binding)? {
            Some(Cohort::Released(reply)) => return Ok(reply.clone()),
            Some(Cohort::Reserved {
                reply,
                allocation_sequence,
                ..
            }) => {
                for id in &reply.reservation_ids {
                    let actor = self
                        .party_member(id, binding.world_id, *allocation_sequence)
                        .ok_or("party_cohort_stale")?;
                    if actor.claimed.is_some() {
                        return Err("party_already_claimed");
                    }
                }
                reply.reservation_ids.clone()
            }
            None => {
                self.party_reservations.check_room()?;
                Vec::new()
            }
        };
        let reply = binding.reply(Status::Released, None, reservations);
        self.record(
            json!({"op":"party_released","party_reservation":reply}),
            wall,
        )?;
        for id in &reply.reservation_ids {
            if self
                .active
                .as_ref()
                .is_some_and(|actor| actor.reservation.id == *id)
            {
                self.active = None;
            } else {
                self.peers.remove(id);
            }
            self.retire_reservation_id(id);
        }
        self.party_reservations
            .cohorts
            .insert(binding.cohort_id.into(), Cohort::Released(reply.clone()));
        if !reply.reservation_ids.is_empty() && self.active.is_none() && self.peers.is_empty() {
            self.reset_empty_gameplay_world();
        }
        Ok(reply)
    }
}
