//! Opt-in server leadership policy and transactional inherited modifier planning.
//! Held roles are match state; authenticated account/character keys only select
//! explicit development bindings. No persistent progression or wire role grant.
use crate::leadership_passives::{
    HeldLeadershipRole, LeadershipCatalog, PassiveParticipant, PassiveState,
};
use crate::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DevelopmentBinding {
    account_id: u32,
    character_id: u32,
    native_role: u8,
}
/// Saved character leadership fields used by native squad promotion. Horizon
/// forwards them when available; otherwise the policy's explicit default applies.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LeadershipRecord {
    pub leadership_level: u8,
    pub nominations: u8,
    pub matches_since_last_command: u8,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SquadLeaderMode {
    FirstMember,
    Native,
    None,
}
/// Retail `mag_squad_matchmaking_properties.zpr` squad weights, projected and
/// checked by `tools/project-retail-leadership-role-policy.py`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeElection {
    source_sha256: String,
    squad_level_weight: f32,
    squad_mslc_weight: f32,
    default_record: LeadershipRecord,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatoonElection {
    level_weight: f32,
    mslc_weight: f32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RolePolicy {
    schema_version: u32,
    application_id: u32,
    #[serde(default)]
    first_member_squad_leader: Option<bool>,
    #[serde(default)]
    squad_leader: Option<SquadLeaderMode>,
    #[serde(default)]
    native_election: Option<NativeElection>,
    #[serde(default)]
    platoon_election: Option<PlatoonElection>,
    development_bindings: Vec<DevelopmentBinding>,
}
impl RolePolicy {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }
    pub fn from_json(raw: &str) -> io::Result<Self> {
        let mut value: Self =
            serde_json::from_str(raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        // Schema 1 uses the legacy boolean. Schema 2 names the squad mode.
        let mode_valid = match value.schema_version {
            1 => {
                let schema2_fields = value.squad_leader.is_some();
                let first = value.first_member_squad_leader.take();
                value.squad_leader = first.map(|first| {
                    if first {
                        SquadLeaderMode::FirstMember
                    } else {
                        SquadLeaderMode::None
                    }
                });
                first.is_some() && !schema2_fields && value.native_election.is_none()
            }
            2 | 3 => {
                value.first_member_squad_leader.is_none()
                    && match (value.squad_leader, &value.native_election) {
                        (Some(SquadLeaderMode::Native), Some(n)) => {
                            n.squad_level_weight.is_finite()
                                && n.squad_mslc_weight.is_finite()
                                && n.source_sha256.len() == 64
                                && n.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                        }
                        (Some(SquadLeaderMode::Native), None) => false,
                        (Some(_), None) => true,
                        _ => false,
                    }
            }
            _ => false,
        };
        let platoon_valid = match (value.schema_version, value.platoon_election) {
            (3, Some(weights)) => {
                value.native().is_some()
                    && weights.level_weight.is_finite()
                    && weights.level_weight >= 0.0
                    && weights.mslc_weight.is_finite()
                    && weights.mslc_weight >= 0.0
            }
            (1 | 2, None) => true,
            _ => false,
        };
        let mut seen = BTreeSet::new();
        if !mode_valid
            || !platoon_valid
            || value.application_id != 20231
            || value.development_bindings.iter().any(|b| {
                b.account_id == 0
                    || b.character_id == 0
                    || HeldLeadershipRole::from_native_role(b.native_role).is_err()
                    || !seen.insert((b.account_id, b.character_id))
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid leadership role policy",
            ));
        }
        Ok(value)
    }
    pub fn has_development_bindings(&self) -> bool {
        !self.development_bindings.is_empty()
    }
    /// Registry-backed worlds use the native vacancy policy.
    pub fn supports_match_profiles(&self) -> bool {
        self.native().is_some() && !self.has_development_bindings()
    }
    pub fn supports_platoon_election(&self) -> bool {
        self.platoon_election.is_some()
    }
    fn native(&self) -> Option<&NativeElection> {
        self.native_election
            .as_ref()
            .filter(|_| self.squad_leader == Some(SquadLeaderMode::Native))
    }
    fn binding(&self, active: &Active) -> Option<HeldLeadershipRole> {
        let character = active.reservation.character.character_id?;
        self.development_bindings
            .iter()
            .find(|b| b.account_id == active.reservation.account && b.character_id == character)
            .and_then(|b| HeldLeadershipRole::from_native_role(b.native_role).ok())
    }
}
/// Retail `MagSimCharacter::SetNominations` (0x0093b7b8) masks requested bits by
/// saved leadership level: none, squad, then squad and platoon. The worker
/// supports these two election scopes. OIC nomination remains outside its policy.
pub fn mask_nominations(leadership_level: u8, bits: u8) -> u8 {
    match leadership_level {
        0 => 0,
        1 => bits & 1,
        _ => bits & 3,
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Leader {
    player: u16,
    reservation: String,
    world: u32,
}
impl Leader {
    fn from_actor(actor: &Active) -> Self {
        Self {
            player: actor.reservation.player,
            reservation: actor.reservation.id.clone(),
            world: actor.reservation.world,
        }
    }

    fn matches(&self, actor: &Active, nomination: u8) -> bool {
        actor.initial_objects.is_some()
            && actor.nominations & nomination != 0
            && self.player == actor.reservation.player
            && self.reservation == actor.reservation.id
            && self.world == actor.reservation.world
    }
}

pub(crate) struct LeadershipRuntime {
    catalog: LeadershipCatalog,
    policy: RolePolicy,
    squad_leaders: BTreeMap<u16, Leader>,
    platoon_leaders: BTreeMap<u16, Leader>,
}
impl LeadershipRuntime {
    pub(crate) fn supports_match_profiles(&self) -> bool {
        self.policy.supports_match_profiles()
    }
    pub(crate) fn supports_platoon_election(&self) -> bool {
        self.policy.supports_platoon_election()
    }
    pub(crate) fn clear_held_roles(&mut self) {
        self.squad_leaders.clear();
        self.platoon_leaders.clear();
    }
    fn elect<'a>(
        &self,
        candidates: impl Iterator<Item = &'a Active>,
        native: &NativeElection,
        nomination: u8,
        weights: (f32, f32),
    ) -> Option<&'a Active> {
        let mut best = None;
        for actor in candidates {
            if actor.nominations & nomination == 0 || self.policy.binding(actor).is_some() {
                continue;
            }
            let record = actor
                .reservation
                .character
                .leadership
                .unwrap_or(native.default_record);
            let level = actor
                .reservation
                .character
                .progression
                .unwrap_or_default()
                .level;
            let score = f32::from(record.matches_since_last_command)
                .mul_add(weights.1, f32::from(level) * weights.0);
            if best.is_none_or(|(_, best_score)| best_score < score) {
                best = Some((actor, score));
            }
        }
        best.map(|(actor, _)| actor)
    }

    fn native_role(&self, actor: &Active) -> HeldLeadershipRole {
        let squad = actor.squad_catalog.squad_index();
        if self
            .platoon_leaders
            .get(&(squad / 4))
            .is_some_and(|leader| leader.matches(actor, 2))
        {
            HeldLeadershipRole::PlatoonLeader
        } else if self
            .squad_leaders
            .get(&squad)
            .is_some_and(|leader| leader.matches(actor, 1))
        {
            HeldLeadershipRole::SquadLeader
        } else {
            HeldLeadershipRole::None
        }
    }
}

fn leadership_changes(
    scope_key: &str,
    current: &BTreeMap<u16, Leader>,
    next: &BTreeMap<u16, Leader>,
) -> Vec<Value> {
    current
        .keys()
        .chain(next.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|index| {
            let leader = next.get(&index);
            (current.get(&index) != leader).then(|| {
                json!({
                    (scope_key): index,
                    "reason": if leader.is_some() {"elected"} else {"removed"},
                    "leader_index":leader.map_or(u16::MAX, |l| l.player),
                    "reservation_id":leader.map(|l| &l.reservation)
                })
            })
        })
        .collect()
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct CachedLeadership {
    scope: (u32, u32, u16),
    pub(crate) state: PassiveState,
}
struct PlannedRecipient {
    cache: CachedLeadership,
    effective: skill_catalog::OwnedModifiers,
    role: HeldLeadershipRole,
    old_level: u8,
}
#[derive(Default)]
pub(crate) struct LeadershipPlan {
    recipients: BTreeMap<String, PlannedRecipient>,
}
impl LeadershipPlan {
    pub(crate) fn modifiers(&self, active: &Active) -> skill_catalog::OwnedModifiers {
        self.recipients
            .get(&active.reservation.id)
            .map_or(active.modifiers, |p| p.effective)
    }
    pub(crate) fn audit(&self) -> Vec<Value> {
        self.recipients.iter().filter(|(_,p)|p.old_level!=p.cache.state.level()).map(|(id,p)|json!({"reservation_id":id,"held_role_ability_level":p.role.ability_level(),"old_inherited_level":p.old_level,"new_inherited_level":p.cache.state.level()})).collect()
    }
}
impl Active {
    fn leadership_scope(&self) -> (u32, u32, u16) {
        let generation = match self.life {
            CharacterLife::Alive { generation, .. }
            | CharacterLife::Incapacitated { generation, .. }
            | CharacterLife::AwaitingRespawn { generation } => generation,
            CharacterLife::AwaitingFirstSpawn => 0,
        };
        (
            self.reservation.world,
            generation,
            self.squad_catalog.squad_index(),
        )
    }
    fn leadership_participant(&self, role: HeldLeadershipRole) -> Option<PassiveParticipant> {
        let position = self.tracking.sample?.position;
        Some(PassiveParticipant {
            character_id: 0x03800000 | u32::from(self.reservation.player),
            team: self.team(),
            squad: self.squad_catalog.squad_index(),
            position: [position.x, position.y, position.z],
            held_role: role,
            state_code: if self.entered_game && self.ready_for_updates {
                self.owner_fields.state.map_or(0, |s| s.state)
            } else {
                0
            },
            damage_model_dead: !self.life.is_alive(),
            locally_dead: false,
            damage_conditions: self.last_damage.conditions,
            mounted: self.is_mounted(),
        })
    }
}
impl<W: Write> Engine<W> {
    /// Explicit opt-in. Bindings keyed by authenticated numeric account and saved
    /// character identity require development control before installation.
    pub fn with_leadership_passives(
        mut self,
        catalog: LeadershipCatalog,
        policy: RolePolicy,
    ) -> Result<Self, &'static str> {
        if self.match_profiles.is_some() && !policy.supports_match_profiles() {
            return Err("match_profiles_unsupported_world_extras");
        }
        if self.active.is_some() || !self.peers.is_empty() {
            return Err("leadership_configuration_locked");
        }
        if policy.has_development_bindings() && !self.development_control {
            return Err("leadership_bindings_require_development_control");
        }
        self.leadership = Some(LeadershipRuntime {
            catalog,
            policy,
            squad_leaders: BTreeMap::new(),
            platoon_leaders: BTreeMap::new(),
        });
        Ok(self)
    }
    fn native_election(&self) -> Option<&NativeElection> {
        self.leadership.as_ref().and_then(|r| r.policy.native())
    }
    pub(crate) fn native_leader_election(&self) -> bool {
        self.native_election().is_some()
    }
    pub(crate) fn command_leadership_role(
        &self,
        actor: &Active,
    ) -> Option<active_abilities::CommandRole> {
        match self.leadership.as_ref()?.native_role(actor) {
            HeldLeadershipRole::SquadLeader => Some(active_abilities::CommandRole::SquadLeader),
            HeldLeadershipRole::PlatoonLeader => Some(active_abilities::CommandRole::PlatoonLeader),
            _ => None,
        }
    }
    /// The saved record's fields, or the policy default when Horizon supplied none.
    pub(crate) fn leadership_record(&self, character: &Character) -> Option<LeadershipRecord> {
        let native = self.native_election()?;
        Some(character.leadership.unwrap_or(native.default_record))
    }
    /// Retail `MagSimCharacter::Initialize` applies the saved nominations through
    /// SetNominations. Without native election the replicated value stays zero.
    pub(crate) fn initial_nominations(&self, character: &Character) -> u8 {
        self.leadership_record(character)
            .map_or(0, |r| mask_nominations(r.leadership_level, r.nominations))
    }
    /// Masked nominations for a ChangePlayerNominations request, if native.
    pub(crate) fn requested_nominations(&self, active: &Active, bits: u8) -> Option<u8> {
        self.leadership_record(&active.reservation.character)
            .map(|r| mask_nominations(r.leadership_level, bits))
    }
    /// Squad field10. Native mode publishes the held leader or 0xffff; every
    /// other configuration keeps the existing first-member wire value.
    pub(crate) fn squad_leader_index(&self, squad: u16, members: &[u16; 8]) -> u16 {
        match &self.leadership {
            Some(r) if r.policy.native().is_some() => r
                .squad_leaders
                .get(&squad)
                .map_or(u16::MAX, |leader| leader.player),
            _ => members[0],
        }
    }
    /// Platoon field 8 publishes the elected character slot or the vacancy sentinel.
    pub(crate) fn platoon_leader_index(&self, platoon: u16) -> u16 {
        self.leadership
            .as_ref()
            .and_then(|runtime| runtime.platoon_leaders.get(&platoon))
            .map_or(u16::MAX, |leader| leader.player)
    }

    /// Fill platoon vacancies before squad vacancies. Platoon promotion may
    /// replace the winner's squad authority, so stage and record both together.
    pub(crate) fn reconcile_squad_leaders(&mut self) -> Result<(), &'static str> {
        let Some(runtime) = self.leadership.as_ref() else {
            return Ok(());
        };
        let Some(native) = runtime.policy.native() else {
            return Ok(());
        };
        let actors: Vec<&Active> = self
            .active
            .iter()
            .chain(self.peers.values())
            .filter(|a| a.initial_objects.is_some())
            .collect();
        let squads: BTreeMap<_, _> = actors
            .iter()
            .map(|a| a.squad_catalog.squad_index())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|squad| (squad, self.squad_members(squad, None)))
            .collect();

        let mut platoon_leaders = runtime.platoon_leaders.clone();
        platoon_leaders.retain(|platoon, leader| {
            actors.iter().any(|actor| {
                actor.squad_catalog.squad_index() / 4 == *platoon && leader.matches(actor, 2)
            })
        });
        let mut squad_leaders = runtime.squad_leaders.clone();
        squad_leaders.retain(|squad, leader| {
            actors.iter().any(|actor| {
                actor.squad_catalog.squad_index() == *squad && leader.matches(actor, 1)
            })
        });

        let members = |squad| {
            let actors = &actors;
            squads
                .get(&squad)
                .into_iter()
                .flatten()
                .filter_map(move |player| {
                    actors.iter().copied().find(|a| {
                        a.reservation.player == *player && a.squad_catalog.squad_index() == squad
                    })
                })
        };

        if self.match_started {
            if let Some(weights) = runtime.policy.platoon_election {
                let occupied_platoons: BTreeSet<u16> =
                    squads.keys().map(|squad| squad / 4).collect();
                for platoon in occupied_platoons {
                    if platoon_leaders.contains_key(&platoon) {
                        continue;
                    }
                    let candidates = (platoon * 4..platoon * 4 + 4).flat_map(members);
                    if let Some(winner) = runtime.elect(
                        candidates,
                        native,
                        2,
                        (weights.level_weight, weights.mslc_weight),
                    ) {
                        let squad = winner.squad_catalog.squad_index();
                        if squad_leaders
                            .get(&squad)
                            .is_some_and(|leader| leader.matches(winner, 1))
                        {
                            squad_leaders.remove(&squad);
                        }
                        platoon_leaders.insert(platoon, Leader::from_actor(winner));
                    }
                }
            }

            for &squad in squads.keys() {
                if squad_leaders.contains_key(&squad) {
                    continue;
                }
                let candidates = members(squad).filter(|actor| {
                    !platoon_leaders
                        .get(&(squad / 4))
                        .is_some_and(|leader| leader.matches(actor, 2))
                });
                if let Some(winner) = runtime.elect(
                    candidates,
                    native,
                    1,
                    (native.squad_level_weight, native.squad_mslc_weight),
                ) {
                    squad_leaders.insert(squad, Leader::from_actor(winner));
                }
            }
        }

        let changes = leadership_changes("squad_index", &runtime.squad_leaders, &squad_leaders);
        let platoon_changes =
            leadership_changes("platoon_index", &runtime.platoon_leaders, &platoon_leaders);
        if !platoon_changes.is_empty() {
            self.record(
                json!({"op":"leadership_roles_changed","squad_changes":changes,
                    "platoon_changes":platoon_changes,"match_started":self.match_started}),
                self.last_wall_secs,
            )?;
        } else if !changes.is_empty() {
            self.record(
                json!({"op":"squad_leaders_changed","changes":changes,"match_started":self.match_started}),
                self.last_wall_secs,
            )?;
        }
        let runtime = self.leadership.as_mut().unwrap();
        runtime.squad_leaders = squad_leaders;
        runtime.platoon_leaders = platoon_leaders;
        Ok(())
    }
    pub(crate) fn plan_leadership(&self, now: Duration) -> Result<LeadershipPlan, &'static str> {
        let Some(runtime) = &self.leadership else {
            return Ok(LeadershipPlan::default());
        };
        let now_ms = u64::try_from(now.as_millis()).map_err(|_| "leadership_clock_overflow")?;
        let actors: Vec<_> = self
            .active
            .iter()
            .chain(self.peers.values())
            .filter(|a| a.wire_profile.application_id() == 20231)
            .collect();
        let role_for = |a: &Active| {
            runtime.policy.binding(a).unwrap_or_else(|| {
                let squad = a.squad_catalog.squad_index();
                match runtime.policy.squad_leader {
                    Some(SquadLeaderMode::FirstMember)
                        if self.squad_members(squad, None)[0] == a.reservation.player =>
                    {
                        HeldLeadershipRole::SquadLeader
                    }
                    Some(SquadLeaderMode::Native) => runtime.native_role(a),
                    _ => HeldLeadershipRole::None,
                }
            })
        };
        let providers: Vec<_> = actors
            .iter()
            .filter_map(|a| {
                a.leadership_participant(role_for(a))
                    .map(|p| (a.reservation.world, p))
            })
            .collect();
        let mut result = LeadershipPlan::default();
        for a in actors {
            let scope = a.leadership_scope();
            let mut state = a
                .leadership
                .filter(|c| c.scope == scope)
                .map_or_else(PassiveState::default, |c| c.state);
            let old_level = a.leadership.map_or(0, |c| c.state.level());
            let role = role_for(a);
            if let Some(recipient) = a.leadership_participant(role) {
                let scoped: Vec<_> = providers
                    .iter()
                    .filter(|(world, _)| *world == a.reservation.world)
                    .map(|(_, p)| *p)
                    .collect();
                state.refresh(&runtime.catalog, &recipient, &scoped, now_ms, 0.)?;
            } else {
                state.clear();
            }
            let effective = a
                .modifiers
                .with_leadership(&runtime.catalog.modifiers(state.level())?)?;
            result.recipients.insert(
                a.reservation.id.clone(),
                PlannedRecipient {
                    cache: CachedLeadership { scope, state },
                    effective,
                    role,
                    old_level,
                },
            );
        }
        Ok(result)
    }
    pub(crate) fn commit_leadership(&mut self, plan: LeadershipPlan) {
        for a in self.active.iter_mut().chain(self.peers.values_mut()) {
            if let Some(p) = plan.recipients.get(&a.reservation.id)
                && p.cache.scope == a.leadership_scope()
            {
                a.leadership = Some(p.cache);
            }
        }
    }
}
