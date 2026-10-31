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
    development_bindings: Vec<DevelopmentBinding>,
}
impl RolePolicy {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }
    pub fn from_json(raw: &str) -> io::Result<Self> {
        let mut value: Self =
            serde_json::from_str(raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        // Schema1 is the deployed boolean form; schema2 names the squad mode.
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
            2 => {
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
        let mut seen = BTreeSet::new();
        if !mode_valid
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
/// saved leadership level: none, squad, then squad+platoon. Level3 adds the OIC
/// bit only in GameType2, which this server does not host.
pub fn mask_nominations(leadership_level: u8, bits: u8) -> u8 {
    match leadership_level {
        0 => 0,
        1 => bits & 1,
        _ => bits & 3,
    }
}
/// Squad index, new held leader (none = vacant) and audit reason.
type LeaderChange = (u16, Option<(u16, String)>, &'static str);
pub(crate) struct LeadershipRuntime {
    catalog: LeadershipCatalog,
    policy: RolePolicy,
    /// Held squad leaders: squad index -> (player index, reservation id). Native
    /// promotion only fills vacancies; a sitting leader is never displaced.
    squad_leaders: BTreeMap<u16, (u16, String)>,
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
            mounted: self.mounted_seat.is_some(),
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
        });
        Ok(self)
    }
    fn native_election(&self) -> Option<&NativeElection> {
        self.leadership.as_ref().and_then(|r| r.policy.native())
    }
    pub(crate) fn native_leader_election(&self) -> bool {
        self.native_election().is_some()
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
            Some(r) if r.policy.native().is_some() => {
                r.squad_leaders.get(&squad).map_or(u16::MAX, |(p, _)| *p)
            }
            _ => members[0],
        }
    }
    /// Native vacancy maintenance, run at every roster publication and match
    /// start. A departed, transferred or unnominated leader is removed
    /// (`RemoveMember`/`ChangeSquadLeaderNomination`); while the match plays,
    /// each vacancy is filled by `MagSimSquad::FindAvailableLeader` (0x0094ac38):
    /// member slots in order, nomination bit1, no held role, score
    /// level*levelWeight+matchesSinceLastCommand*mslcWeight, strict greater.
    /// Bots carry no nominations and are never candidates.
    pub(crate) fn reconcile_squad_leaders(&mut self) -> Result<(), &'static str> {
        let Some(native) = self.native_election() else {
            return Ok(());
        };
        let (level_weight, mslc_weight) = (native.squad_level_weight, native.squad_mslc_weight);
        let actors: Vec<&Active> = self
            .active
            .iter()
            .chain(self.peers.values())
            .filter(|a| a.initial_objects.is_some())
            .collect();
        let held = &self.leadership.as_ref().unwrap().squad_leaders;
        let mut changes: Vec<LeaderChange> = Vec::new();
        for (squad, (player, id)) in held {
            let keep = actors.iter().any(|a| {
                a.reservation.player == *player
                    && a.reservation.id == *id
                    && a.squad_catalog.squad_index() == *squad
                    && a.nominations & 1 != 0
            });
            if !keep {
                changes.push((*squad, None, "removed"));
            }
        }
        if self.match_started {
            let squads: BTreeSet<u16> = actors
                .iter()
                .map(|a| a.squad_catalog.squad_index())
                .collect();
            for squad in squads {
                if held.contains_key(&squad) && !changes.iter().any(|(s, _, _)| *s == squad) {
                    continue;
                }
                let mut best: Option<(&Active, f32)> = None;
                for slot in self.squad_members(squad, None) {
                    let Some(a) = actors.iter().find(|a| {
                        a.reservation.player == slot && a.squad_catalog.squad_index() == squad
                    }) else {
                        continue;
                    };
                    let record = a
                        .reservation
                        .character
                        .leadership
                        .unwrap_or(native.default_record);
                    let level = a
                        .reservation
                        .character
                        .progression
                        .unwrap_or_default()
                        .level;
                    let score = (f64::from(level) * f64::from(level_weight)
                        + f64::from(record.matches_since_last_command) * f64::from(mslc_weight))
                        as f32;
                    // A development binding fixes this identity's role; native
                    // candidates must hold no role at all.
                    let unbound = self
                        .leadership
                        .as_ref()
                        .is_some_and(|r| r.policy.binding(a).is_none());
                    if a.nominations & 1 != 0 && unbound && best.is_none_or(|(_, b)| b < score) {
                        best = Some((a, score));
                    }
                }
                if let Some((a, _)) = best {
                    changes.retain(|(s, _, _)| *s != squad);
                    let elected = (a.reservation.player, a.reservation.id.clone());
                    changes.push((squad, Some(elected), "elected"));
                }
            }
        }
        if changes.is_empty() {
            return Ok(());
        }
        let audit: Vec<Value> = changes
            .iter()
            .map(|(squad, leader, reason)| {
                json!({"squad_index":squad,"reason":reason,
                    "leader_index":leader.as_ref().map_or(u16::MAX, |l| l.0),
                    "reservation_id":leader.as_ref().map(|l| l.1.clone())})
            })
            .collect();
        self.record(
            json!({"op":"squad_leaders_changed","changes":audit,"match_started":self.match_started}),
            self.last_wall_secs,
        )?;
        let held = &mut self.leadership.as_mut().unwrap().squad_leaders;
        for (squad, leader, _) in changes {
            match leader {
                Some(leader) => held.insert(squad, leader),
                None => held.remove(&squad),
            };
        }
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
                let held =
                    match runtime.policy.squad_leader {
                        Some(SquadLeaderMode::FirstMember) => {
                            self.squad_members(squad, None)[0] == a.reservation.player
                        }
                        Some(SquadLeaderMode::Native) => runtime
                            .squad_leaders
                            .get(&squad)
                            .is_some_and(|(player, id)| {
                                *player == a.reservation.player && *id == a.reservation.id
                            }),
                        _ => false,
                    };
                if held {
                    HeldLeadershipRole::SquadLeader
                } else {
                    HeldLeadershipRole::None
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
