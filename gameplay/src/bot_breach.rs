//! Attacking bots break a lane's defences before its capture.
//!
//! Specified in docs/bot-breach-order.md. This is an original policy for this
//! server's bots, not a decoded behaviour: retail has no bots, and neither
//! Acquisition's nor Domination's rules lock a capture behind the defenders'
//! structures. The structures are physical and tactical obstacles: gates and
//! barricades block routes, bunkers are the defenders' forward spawns and
//! turrets, and anti-air keeps the attackers' drop zones closed.
//!
//! - A lane is what one group of attacking squads captures now. In
//!   Acquisition it is one attacking platoon's pair of vehicle chains, seen
//!   from the platoon's delivery point. In Domination it is the point
//!   attacking squads share as their native primary, seen from those squads'
//!   home spawns.
//! - A lane's defences are the defenders' demolition structures within
//!   `reach_m` of its capture and nearer the attackers' side than it:
//!   obstacles (gates, barricades) nearest the lane's captures, bunkers whose
//!   forward spawn belongs to a defender squad of the lane, and anti-air
//!   whose destruction opens drop zones of the lane's attacking squads or
//!   switches command abilities on. Mortars and sensors (which only switch
//!   the defenders' abilities off) and motor pools are support and not part
//!   of it.
//! - Defences are worked outermost first. While some stand and the capture is
//!   open to the attackers, the n-th attacking squad of the lane (by squad
//!   index, among squads with bots, after `attacker_reserve_squads` that stay
//!   on the capture) works the n-th standing defence instead. Defender squads
//!   of the lane spread over the defences the attackers work, except the last
//!   `defender_reserve_squads`, which stay on the capture.
//! - A defence counts as broken once destroyed, so a repair does not send the
//!   lane back, or once attacking squads have worked it for `give_up_ms`
//!   without destroying it.
//! - A human leader's FRAGO still comes first (bot_squad_orders). Demolition
//!   teams (bot_demolition) leave the defences attacking squads work to them.
//!
//! Each bot tick is O(lanes x structures + squads) distance comparisons; the
//! plan is rebuilt only when the lanes change.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    /// Attacking squads break their lane's defences before its capture.
    /// `false` keeps the lane objective (capture first), as before.
    pub enabled: bool,
    /// Match game types it runs in (1 is Acquisition, 2 is Domination).
    pub game_types: Vec<u8>,
    /// How long attacking squads work one defence before passing it by.
    pub give_up_ms: u64,
    /// Attacking squads per lane that stay on the capture meanwhile.
    pub attacker_reserve_squads: u16,
    /// Defender squads per lane that stay on the capture meanwhile.
    pub defender_reserve_squads: u16,
    /// Only structures this close (horizontally) to an Acquisition lane's
    /// vehicles are its defences.
    pub acquisition_reach_m: f64,
    /// Only structures this close (horizontally) to a Domination point are
    /// its defences.
    pub domination_reach_m: f64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            game_types: vec![1, 2],
            give_up_ms: 300_000,
            attacker_reserve_squads: 0,
            defender_reserve_squads: 1,
            acquisition_reach_m: 300.,
            domination_reach_m: 100.,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let reach = |r: f64| r.is_finite() && (1. ..=2048.).contains(&r);
        if self.game_types.len() > 8
            || self.game_types.iter().any(|&t| t > 7)
            || !(10_000..=3_600_000).contains(&self.give_up_ms)
            || self.attacker_reserve_squads > 16
            || self.defender_reserve_squads > 16
            || !reach(self.acquisition_reach_m)
            || !reach(self.domination_reach_m)
        {
            return Err("invalid_bot_breach_policy");
        }
        Ok(())
    }
}

/// A demolition structure as the plan reads its authored links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// No links: its own geometry (a gate, a barricade).
    Obstacle,
    /// The forward spawn of the owners' squad point with this index (a bunker).
    OwnerSpawn(u16),
    /// Destroying it opens these spawn points of the other side (anti-air).
    AntiAir(Vec<u16>),
    /// Mortar, sensor, motor pool or vehicle source: not a lane defence.
    Support,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Obstacle,
    Bunker,
    AntiAir,
}
impl Kind {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Obstacle => "obstacle",
            Self::Bunker => "bunker",
            Self::AntiAir => "anti_air",
        }
    }
}

pub(crate) struct StructureInput {
    pub objective: u16,
    pub owner_team: u8,
    pub position: [f32; 3],
    pub role: Role,
}

pub(crate) struct LaneInput {
    /// The lane's captures with their places, in stage order.
    pub captures: Vec<(u16, [f32; 3])>,
    /// Where the lane's attackers come from.
    pub origin: [f32; 3],
    /// The side that owns the captures.
    pub defender: u8,
}

pub(crate) struct SquadInput {
    pub spawn_points: Vec<u16>,
    /// The squad's lane objectives (any of them names its lane).
    pub authored: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Defence {
    pub objective: u16,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lane {
    pub captures: Vec<u16>,
    pub defender: u8,
    /// Outermost first.
    pub defences: Vec<Defence>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Plan {
    pub lanes: Vec<Lane>,
}

fn flat(a: [f32; 3], b: [f32; 3]) -> f64 {
    f64::from(a[0] - b[0]).hypot(f64::from(a[2] - b[2]))
}

/// Each lane's defences, from authored data only.
pub(crate) fn plan(
    lanes: &[LaneInput],
    structures: &[StructureInput],
    squads: &[SquadInput],
    reach_m: f64,
) -> Plan {
    let lane_of_objectives = |authored: &[u16]| {
        authored.iter().find_map(|objective| {
            lanes
                .iter()
                .position(|lane| lane.captures.iter().any(|(c, _)| c == objective))
        })
    };
    let lane_of_spawn = |spawn: u16| {
        squads
            .iter()
            .filter(|s| s.spawn_points.contains(&spawn))
            .find_map(|s| lane_of_objectives(&s.authored))
    };
    let nearest = |position: [f32; 3]| {
        lanes
            .iter()
            .enumerate()
            .flat_map(|(i, lane)| lane.captures.iter().map(move |(_, at)| (i, *at)))
            .min_by(|a, b| flat(a.1, position).total_cmp(&flat(b.1, position)))
            .map(|(i, _)| i)
    };
    let mut defences: Vec<Vec<(f64, Defence)>> = vec![Vec::new(); lanes.len()];
    for s in structures {
        let (kind, lane) = match &s.role {
            Role::Support => continue,
            Role::Obstacle => (Kind::Obstacle, nearest(s.position)),
            Role::OwnerSpawn(spawn) => (
                Kind::Bunker,
                lane_of_spawn(*spawn).or_else(|| nearest(s.position)),
            ),
            Role::AntiAir(spawns) => (
                Kind::AntiAir,
                spawns
                    .iter()
                    .find_map(|&spawn| lane_of_spawn(spawn))
                    .or_else(|| nearest(s.position)),
            ),
        };
        let Some(i) = lane else { continue };
        let lane = &lanes[i];
        if s.owner_team != lane.defender {
            continue;
        }
        let first = lane
            .captures
            .iter()
            .map(|(_, at)| flat(*at, lane.origin))
            .fold(f64::INFINITY, f64::min);
        let near = lane
            .captures
            .iter()
            .map(|(_, at)| flat(*at, s.position))
            .fold(f64::INFINITY, f64::min);
        let d = flat(s.position, lane.origin);
        if d < first && near <= reach_m {
            defences[i].push((
                d,
                Defence {
                    objective: s.objective,
                    kind,
                },
            ));
        }
    }
    Plan {
        lanes: lanes
            .iter()
            .zip(defences)
            .map(|(lane, mut defences)| {
                defences
                    .sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.objective.cmp(&b.1.objective)));
                Lane {
                    captures: lane.captures.iter().map(|(c, _)| *c).collect(),
                    defender: lane.defender,
                    defences: defences.into_iter().map(|(_, d)| d).collect(),
                }
            })
            .collect(),
    }
}

/// A squad with bots and the objective its mode assigns it now, when the
/// attackers may capture that objective now.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SquadLane {
    pub squad: u16,
    pub team: u8,
    pub objective: u16,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Reserves {
    pub attackers: u16,
    pub defenders: u16,
}

/// The defence each squad works instead of its lane objective. `open` says
/// whether a defence still stands and is not yet counted as broken.
pub(crate) fn assign(
    plan: &Plan,
    squads: &[SquadLane],
    open: impl Fn(u16) -> bool,
    reserves: Reserves,
) -> BTreeMap<u16, u16> {
    let mut chosen = BTreeMap::new();
    for lane in &plan.lanes {
        let standing: Vec<u16> = lane
            .defences
            .iter()
            .map(|d| d.objective)
            .filter(|&o| open(o))
            .collect();
        if standing.is_empty() {
            continue;
        }
        let mut members: Vec<&SquadLane> = squads
            .iter()
            .filter(|s| lane.captures.contains(&s.objective))
            .collect();
        members.sort_by_key(|s| s.squad);
        let (attackers, defenders): (Vec<&SquadLane>, Vec<&SquadLane>) =
            members.into_iter().partition(|s| s.team != lane.defender);
        let working = attackers
            .len()
            .saturating_sub(usize::from(reserves.attackers));
        for (rank, s) in attackers.iter().take(working).enumerate() {
            chosen.insert(s.squad, standing[rank % standing.len()]);
        }
        let front = &standing[..standing.len().min(working.max(1))];
        let working = defenders
            .len()
            .saturating_sub(usize::from(reserves.defenders));
        for (rank, s) in defenders.iter().take(working).enumerate() {
            chosen.insert(s.squad, front[rank % front.len()]);
        }
    }
    chosen
}

/// Per-match progress through the lanes' defences.
#[derive(Default)]
pub(crate) struct Book {
    plan_key: Option<Vec<u64>>,
    plan: Option<std::sync::Arc<Plan>>,
    /// Defences counted as broken this match, and why.
    broken: BTreeMap<u16, &'static str>,
    /// Since when attacking squads work each defence.
    worked_since: BTreeMap<u16, Duration>,
    /// Squad -> (its lane objective, the defence it works instead).
    assigned: BTreeMap<u16, (u16, u16)>,
}
impl Book {
    /// The objective a squad works instead of `lane`, if any.
    pub(crate) fn instead_of(&self, squad: u16, lane: u16) -> Option<u16> {
        self.assigned
            .get(&squad)
            .filter(|(base, _)| *base == lane)
            .map(|&(_, target)| target)
    }
    /// The defences attacking squads work now.
    pub(crate) fn attacked(&self) -> BTreeSet<u16> {
        self.worked_since.keys().copied().collect()
    }
    fn reset(&mut self) {
        self.plan_key = None;
        self.plan = None;
        self.broken.clear();
        self.worked_since.clear();
        self.assigned.clear();
    }
}

fn standing(state: Option<replication::ObjectiveState>) -> bool {
    use replication::ObjectiveState::*;
    matches!(
        state,
        Some(Deactivated | Activating | Activated | Deactivating | Repaired)
    )
}

impl<W: Write> Engine<W> {
    /// The lane objective the mode assigns a squad, before any defence.
    pub(crate) fn bot_mode_lane(&self, squad: u16) -> Option<u16> {
        if self.domination_gameplay.is_some() {
            self.initial_objective_index(squad)
                .ok()
                .filter(|&index| index != u16::MAX)
        } else {
            self.objective_catalog
                .as_ref()
                .zip(self.objective_world.as_ref())
                .and_then(|(c, w)| crate::bot_objectives::lane_assignment(c, w, squad))
        }
    }

    /// Whether the side that does not own `objective` may capture it now.
    fn breach_capturable(&self, objective: u16) -> Option<u8> {
        let (catalog, world) = self
            .objective_catalog
            .as_ref()
            .zip(self.objective_world.as_ref())?;
        let object = catalog
            .objects
            .iter()
            .find(|o| o.object_index == objective)?;
        let owner = object.interaction.as_ref()?.owner_team;
        let attacker = 1 - owner.min(1);
        let state = world.state(objective)?;
        matches!(
            crate::bot_objectives::operator_action(object, state, attacker, world),
            Some((protocol::ObjectiveInteractionKind::Activate, _))
        )
        .then_some(owner)
    }

    fn breach_structures(&self, roles: Vec<(u16, Role)>) -> Vec<StructureInput> {
        let Some(catalog) = self.objective_catalog.as_ref() else {
            return Vec::new();
        };
        roles
            .into_iter()
            .filter_map(|(objective, role)| {
                let rule = catalog
                    .objects
                    .iter()
                    .find(|o| o.object_index == objective)?
                    .demolition
                    .as_ref()?;
                Some(StructureInput {
                    objective,
                    owner_team: rule.owner_team,
                    position: rule.position,
                    role,
                })
            })
            .collect()
    }

    /// Acquisition lanes: one per attacking platoon, from the controller's
    /// chains, seen from the platoon's first delivery point.
    fn acquisition_breach_plan(&self, reach_m: f64) -> Option<Plan> {
        let runtime = self.acquisition_gameplay.as_ref()?;
        let setup = &runtime.setup;
        let structural = setup.structural.as_ref()?;
        let catalog = self.objective_catalog.as_ref()?;
        let mut lanes: Vec<(u8, LaneInput)> = Vec::new();
        for chain in setup.controller.chains() {
            let (capture, _) = catalog.binding(chain.capture_token)?;
            let rule = catalog
                .objects
                .iter()
                .find(|o| o.object_index == capture)?
                .interaction
                .as_ref()?;
            let origin = setup.arrival_triggers.get(&chain.arrival_token)?.centre;
            match lanes.iter_mut().find(|(p, _)| *p == chain.platoon) {
                Some((_, lane)) => lane.captures.push((capture, rule.position)),
                None => lanes.push((
                    chain.platoon,
                    LaneInput {
                        captures: vec![(capture, rule.position)],
                        origin,
                        defender: rule.owner_team,
                    },
                )),
            }
        }
        let structures = self.breach_structures(structural.bot_stage_roles());
        let squads: Vec<SquadInput> = structural
            .squad_spawn_points()
            .into_iter()
            .map(|(squad, spawn_points)| SquadInput {
                spawn_points,
                authored: catalog.authored_objectives(squad).unwrap_or_default(),
            })
            .collect();
        let lanes: Vec<LaneInput> = lanes.into_iter().map(|(_, lane)| lane).collect();
        Some(plan(&lanes, &structures, &squads, reach_m))
    }

    /// Domination lanes: one per point attacking squads with bots share as
    /// their primary while they may capture it, seen from those squads' home
    /// spawns.
    fn domination_breach_lanes(&self, squads: &BTreeMap<u16, u8>) -> Vec<LaneInput> {
        let (Some(catalog), Some(spawns)) = (&self.objective_catalog, &self.spawn_catalog) else {
            return Vec::new();
        };
        let mut groups: BTreeMap<u16, (u8, Vec<[f32; 3]>)> = BTreeMap::new();
        for (&squad, &team) in squads {
            let Some(point) = self.bot_mode_lane(squad) else {
                continue;
            };
            let Some(owner) = self.breach_capturable(point).filter(|&o| o != team) else {
                continue;
            };
            let home = spawns
                .default_for_squad(squad)
                .and_then(|index| spawns.point(index))
                .map(|p| [p.position.x, p.position.y, p.position.z]);
            let entry = groups.entry(point).or_insert((owner, Vec::new()));
            entry.1.extend(home);
        }
        groups
            .into_iter()
            .filter_map(|(point, (defender, homes))| {
                let at = catalog
                    .objects
                    .iter()
                    .find(|o| o.object_index == point)?
                    .interaction
                    .as_ref()?
                    .position;
                (!homes.is_empty()).then(|| {
                    let n = homes.len() as f32;
                    let origin = homes.iter().fold([0.; 3], |a, h| {
                        [a[0] + h[0] / n, a[1] + h[1] / n, a[2] + h[2] / n]
                    });
                    LaneInput {
                        captures: vec![(point, at)],
                        origin,
                        defender,
                    }
                })
            })
            .collect()
    }

    fn domination_breach_plan(&self, lanes: &[LaneInput], reach_m: f64) -> Option<Plan> {
        let world = self.domination_gameplay.as_ref()?;
        let catalog = self.objective_catalog.as_ref()?;
        let spawns = self.spawn_catalog.as_ref()?;
        let hints = world.runtime.structure_hints();
        let roles: Vec<(u16, Role)> = catalog
            .objects
            .iter()
            .filter(|o| o.demolition.is_some())
            .map(|o| {
                let owner = o.demolition.as_ref().map_or(0, |r| r.owner_team);
                let hint = hints.get(&o.object_index).copied().unwrap_or_default();
                let role = if hint.closes_spawn_of == Some(owner) {
                    hint.spawn_point.map_or(Role::Obstacle, Role::OwnerSpawn)
                } else if hint.opens_spawn_of.is_some_and(|t| t != owner) || hint.enables_abilities
                {
                    Role::AntiAir(Vec::new())
                } else if hint.toggles_abilities || hint.closes_vehicle_spawn_of.is_some() {
                    Role::Support
                } else {
                    Role::Obstacle
                };
                (o.object_index, role)
            })
            .collect();
        let structures = self.breach_structures(roles);
        let squads: Vec<SquadInput> = catalog
            .squad_indices()
            .map(|squad| SquadInput {
                spawn_points: spawns
                    .available_for_squad(squad)
                    .map(<[u16]>::to_vec)
                    .unwrap_or_default(),
                authored: self.bot_mode_lane(squad).into_iter().collect(),
            })
            .collect();
        Some(plan(lanes, &structures, &squads, reach_m))
    }

    /// Bring the lanes' defence progress and each squad's defence up to date
    /// for this bot tick. Records `bot_breach` when they change.
    pub(super) fn refresh_breach(&mut self, now: Duration, wall: u64) -> Result<(), &'static str> {
        let policy = self
            .bot_registry
            .config
            .as_ref()
            .map(|c| c.breach.clone())
            .unwrap_or_default();
        let game_type = i16::from(self.match_profile().game_type);
        let playing = self.match_started && self.match_terminal.is_none();
        let enabled = policy.enabled
            && playing
            && policy.game_types.iter().any(|&t| i16::from(t) == game_type)
            && (self.acquisition_gameplay.is_some() || self.domination_gameplay.is_some());
        if !enabled {
            self.bot_registry.director.breach.reset();
            return Ok(());
        }
        let mut squads: BTreeMap<u16, u8> = BTreeMap::new();
        for bot in self.bot_registry.bots.values() {
            squads.insert(bot.target.squad.squad_index(), bot.target.squad.team());
        }
        // Rebuild the plan only when its inputs change: once per match in
        // Acquisition, whenever the attackers' points change in Domination.
        let (key, lanes) = if let Some(runtime) = &self.acquisition_gameplay {
            (vec![std::sync::Arc::as_ptr(&runtime.setup) as u64], None)
        } else {
            let lanes = self.domination_breach_lanes(&squads);
            let key = lanes
                .iter()
                .flat_map(|l| {
                    [u64::from(l.captures[0].0)]
                        .into_iter()
                        .chain(l.origin.map(|v| u64::from(v.to_bits())))
                })
                .collect();
            (key, Some(lanes))
        };
        if self.bot_registry.director.breach.plan_key.as_ref() != Some(&key) {
            let plan = match &lanes {
                None => self.acquisition_breach_plan(policy.acquisition_reach_m),
                Some(lanes) => self.domination_breach_plan(lanes, policy.domination_reach_m),
            }
            .map(std::sync::Arc::new);
            if let Some(plan) = &plan {
                let lanes: Vec<Value> = plan
                    .lanes
                    .iter()
                    .map(|l| {
                        json!({"captures":l.captures,"defender":l.defender,
                        "defences":l.defences.iter().map(|d| json!([d.objective, d.kind.code()]))
                            .collect::<Vec<_>>()})
                    })
                    .collect();
                self.record(
                    json!({"op":"bot_breach_plan","timestamp_ms":now.as_millis() as u64,
                        "lanes":lanes}),
                    wall,
                )?;
            }
            let book = &mut self.bot_registry.director.breach;
            book.plan_key = Some(key);
            book.plan = plan;
        }
        let Some(plan) = self.bot_registry.director.breach.plan.clone() else {
            return Ok(());
        };
        let lanes: Vec<SquadLane> = squads
            .iter()
            .filter_map(|(&squad, &team)| {
                let objective = self.bot_mode_lane(squad)?;
                self.breach_capturable(objective)?;
                Some(SquadLane {
                    squad,
                    team,
                    objective,
                })
            })
            .collect();
        let Some(world) = self.objective_world.as_ref() else {
            return Ok(());
        };
        // Domination attackers keep squads on the point while others break
        // its defences (bot_assault).
        let assault_reserve = self
            .bot_assault_policy()
            .map_or(0, |a| a.breach_reserve_squads);
        let mut events = Vec::new();
        let book = &mut self.bot_registry.director.breach;
        for lane in &plan.lanes {
            for d in &lane.defences {
                if world.state(d.objective) == Some(replication::ObjectiveState::Completed)
                    && !book.broken.contains_key(&d.objective)
                {
                    book.broken.insert(d.objective, "destroyed");
                    events.push(
                        json!({"objective":d.objective,"kind":d.kind.code(),"reason":"destroyed"}),
                    );
                }
            }
        }
        let open = |o: u16| !book.broken.contains_key(&o) && standing(world.state(o));
        let assigned = assign(
            &plan,
            &lanes,
            open,
            Reserves {
                attackers: policy.attacker_reserve_squads.max(assault_reserve),
                defenders: policy.defender_reserve_squads,
            },
        );
        // Defences attacking squads work now; a defence worked too long is passed by.
        let worked: BTreeSet<u16> = lanes
            .iter()
            .filter(|s| {
                plan.lanes
                    .iter()
                    .any(|l| l.captures.contains(&s.objective) && l.defender != s.team)
            })
            .filter_map(|s| assigned.get(&s.squad).copied())
            .collect();
        book.worked_since.retain(|o, _| worked.contains(o));
        let give_up = Duration::from_millis(policy.give_up_ms);
        for &o in &worked {
            let since = *book.worked_since.entry(o).or_insert(now);
            if now.saturating_sub(since) >= give_up {
                book.broken.insert(o, "gave_up");
                book.worked_since.remove(&o);
                let kind = plan
                    .lanes
                    .iter()
                    .flat_map(|l| &l.defences)
                    .find(|d| d.objective == o)
                    .map(|d| d.kind.code());
                events.push(json!({"objective":o,"kind":kind,"reason":"gave_up"}));
            }
        }
        let next: BTreeMap<u16, (u16, u16)> = lanes
            .iter()
            .filter_map(|s| Some((s.squad, (s.objective, *assigned.get(&s.squad)?))))
            .collect();
        let changed = next != book.assigned;
        book.assigned = next;
        if changed || !events.is_empty() {
            let squads: Vec<Value> = book
                .assigned
                .iter()
                .map(|(&squad, &(lane, defence))| {
                    json!({"squad_index":squad,"lane_objective":lane,"objective":defence})
                })
                .collect();
            let lanes: Vec<Value> = plan
                .lanes
                .iter()
                .map(|l| {
                    json!({"captures":l.captures,
                        "standing":l.defences.iter().map(|d| d.objective)
                            .filter(|&o| !book.broken.contains_key(&o) && standing(world.state(o)))
                            .collect::<Vec<_>>()})
                })
                .collect();
            self.record(
                json!({"op":"bot_breach","timestamp_ms":now.as_millis() as u64,
                    "broken":events,"lanes":lanes,"squads":squads}),
                wall,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "bot_breach_tests.rs"]
mod tests;
