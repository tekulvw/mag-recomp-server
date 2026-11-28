//! SYNTHETIC lane defences: invented lanes, squads, structures
//! and places. No retail map, index or position is used.
use super::*;

/// Two lanes 200 m apart. Attackers deliver at z = 0, the defenders' vehicles
/// stand at z = 400. Lane 0 is x = 0 (captures 10, 11), lane 1 is x = 200
/// (captures 20, 21). Team 1 defends.
fn lanes() -> Vec<LaneInput> {
    [(0., [10, 11]), (200., [20, 21])]
        .into_iter()
        .map(|(x, captures)| LaneInput {
            captures: captures.map(|c| (c, [x, 0., 400.])).to_vec(),
            origin: [x, 0., 0.],
            defender: 1,
        })
        .collect()
}

fn structure(objective: u16, x: f32, z: f32, role: Role) -> StructureInput {
    StructureInput {
        objective,
        owner_team: 1,
        position: [x, 0., z],
        role,
    }
}

/// Attacking squads 0..3 (lane 0) and 4..7 (lane 1) own drop points 100+squad;
/// defending squads 16..19 (lane 0) and 20..23 (lane 1) own bunker points
/// 200+squad.
fn squads() -> Vec<SquadInput> {
    (0..8)
        .map(|s| SquadInput {
            spawn_points: vec![100 + s],
            authored: if s < 4 {
                vec![10, 12, 11]
            } else {
                vec![20, 22, 21]
            },
        })
        .chain((16..24).map(|s| SquadInput {
            spawn_points: vec![200 + s],
            authored: if s < 20 {
                vec![13, 10, 11]
            } else {
                vec![23, 20, 21]
            },
        }))
        .collect()
}

fn defences(plan: &Plan, lane: usize) -> Vec<(u16, Kind)> {
    plan.lanes[lane]
        .defences
        .iter()
        .map(|d| (d.objective, d.kind))
        .collect()
}

#[test]
fn a_lane_breaks_its_gates_bunkers_and_anti_air_outermost_first() {
    let structures = [
        structure(30, 0., 150., Role::Obstacle),
        // A bunker of a lane-0 defender squad, nearer lane 1's vehicles:
        // its owning squad, not its place, decides the lane.
        structure(31, 120., 100., Role::OwnerSpawn(216)),
        // Anti-air opening lane-1 attackers' drop zones.
        structure(32, 90., 200., Role::AntiAir(vec![105, 106])),
        structure(33, 5., 350., Role::Obstacle),
        // Support structures are never lane defences.
        structure(34, 0., 120., Role::Support),
        // Behind the vehicles (a motor pool's place): not before them.
        structure(35, 0., 450., Role::Obstacle),
        structure(36, 200., 50., Role::Obstacle),
    ];
    let plan = plan(&lanes(), &structures, &squads(), 1000.);
    assert_eq!(plan.lanes[0].captures, vec![10, 11]);
    assert_eq!(
        defences(&plan, 0),
        vec![
            (30, Kind::Obstacle),
            (31, Kind::Bunker),
            (33, Kind::Obstacle)
        ]
    );
    assert_eq!(
        defences(&plan, 1),
        vec![(36, Kind::Obstacle), (32, Kind::AntiAir)]
    );
}

#[test]
fn attackers_own_structures_and_unlinked_spawns_follow_ownership_and_place() {
    let mut own = structure(40, 0., 100., Role::Obstacle);
    own.owner_team = 0;
    let structures = [
        own,
        // A spawn no squad owns falls back to the nearest lane.
        structure(41, 190., 100., Role::OwnerSpawn(999)),
    ];
    let plan = plan(&lanes(), &structures, &squads(), 1000.);
    assert!(plan.lanes[0].defences.is_empty());
    assert_eq!(defences(&plan, 1), vec![(41, Kind::Bunker)]);
}

fn lane_plan(defences: &[u16]) -> Plan {
    Plan {
        lanes: vec![Lane {
            captures: vec![10, 11],
            defender: 1,
            defences: defences
                .iter()
                .map(|&objective| Defence {
                    objective,
                    kind: Kind::Obstacle,
                })
                .collect(),
        }],
    }
}

const R: Reserves = Reserves {
    attackers: 0,
    defenders: 1,
    point_attackers: 0,
};

fn squad(squad: u16, team: u8, objective: u16) -> SquadLane {
    SquadLane {
        squad,
        team,
        objective,
    }
}

#[test]
fn attacking_squads_spread_over_standing_defences_in_order() {
    let plan = lane_plan(&[30, 31, 32]);
    let squads = [
        squad(3, 0, 10),
        squad(0, 0, 10),
        squad(1, 0, 10),
        squad(2, 0, 10),
    ];
    let chosen = assign(&plan, &squads, |_| true, R);
    assert_eq!(chosen, BTreeMap::from([(0, 30), (1, 31), (2, 32), (3, 30)]));
    // A broken defence moves the squads on to the next ones.
    let chosen = assign(&plan, &squads, |o| o != 30, R);
    assert_eq!(chosen, BTreeMap::from([(0, 31), (1, 32), (2, 31), (3, 32)]));
}

#[test]
fn the_vehicle_comes_once_every_defence_is_broken() {
    let plan = lane_plan(&[30, 31]);
    let squads = [squad(0, 0, 10), squad(16, 1, 10)];
    assert!(assign(&plan, &squads, |_| false, R).is_empty());
    // A delivery (arrival) or the lane's next stage is not held back either:
    // only squads on one of the lane's captures are.
    let squads = [squad(0, 0, 12)];
    assert!(assign(&plan, &squads, |_| true, R).is_empty());
}

#[test]
fn defenders_hold_the_threatened_defences_and_keep_a_reserve_on_the_vehicle() {
    let plan = lane_plan(&[30, 31, 32, 33]);
    let squads = [
        squad(0, 0, 10),
        squad(1, 0, 10),
        squad(16, 1, 10),
        squad(17, 1, 10),
        squad(18, 1, 10),
        squad(19, 1, 10),
    ];
    let chosen = assign(&plan, &squads, |_| true, R);
    // Attackers work 30 and 31; defenders hold those two, 19 stays home.
    assert_eq!(chosen[&0], 30);
    assert_eq!(chosen[&1], 31);
    assert_eq!(chosen[&16], 30);
    assert_eq!(chosen[&17], 31);
    assert_eq!(chosen[&18], 30);
    assert!(!chosen.contains_key(&19));
    // A single defender squad is the reserve.
    let chosen = assign(&plan, &[squad(0, 0, 10), squad(16, 1, 10)], |_| true, R);
    assert!(!chosen.contains_key(&16));
}

#[test]
fn a_book_only_replaces_the_lane_objective_it_was_computed_for() {
    let mut book = Book::default();
    book.assigned.insert(0, (10, 30));
    assert_eq!(book.instead_of(0, 10), Some(30));
    // The lane moved on (vehicle captured, now delivering): no defence.
    assert_eq!(book.instead_of(0, 12), None);
    assert_eq!(book.instead_of(1, 10), None);
}

#[test]
fn reach_bounds_a_lanes_defences_and_reserves_keep_squads_on_the_capture() {
    let structures = [
        structure(30, 0., 150., Role::Obstacle),
        structure(31, 0., 350., Role::Obstacle),
    ];
    // Only 31 is within 100 m of the vehicles at z = 400.
    let near = plan(&lanes(), &structures, &squads(), 100.);
    assert_eq!(defences(&near, 0), vec![(31, Kind::Obstacle)]);
    let plan = lane_plan(&[30, 31]);
    let squads = [squad(0, 0, 10), squad(1, 0, 10), squad(16, 1, 10)];
    let reserves = Reserves {
        attackers: 1,
        defenders: 0,
        point_attackers: 0,
    };
    let chosen = assign(&plan, &squads, |_| true, reserves);
    // Squad 1 holds the capture; the defenders face the one defence worked.
    assert_eq!(chosen, BTreeMap::from([(0, 30), (16, 30)]));
    // The assault's point squads stay too, but never the lane's last squad.
    let point = Reserves {
        attackers: 0,
        defenders: 0,
        point_attackers: 1,
    };
    assert_eq!(
        assign(&plan, &squads, |_| true, point),
        BTreeMap::from([(0, 30), (16, 30)])
    );
    let alone = [squad(0, 0, 10), squad(16, 1, 10)];
    assert_eq!(
        assign(&plan, &alone, |_| true, point),
        BTreeMap::from([(0, 30), (16, 30)])
    );
}

#[test]
fn policy_defaults_and_bounds() {
    let p: Policy = serde_json::from_str("{}").unwrap();
    assert_eq!(p, Policy::default());
    assert!(p.enabled);
    p.validate().unwrap();
    assert_eq!(p.game_types, vec![1, 2]);
    let p: Policy = serde_json::from_str(r#"{"give_up_ms": 5}"#).unwrap();
    assert!(p.validate().is_err());
    assert!(serde_json::from_str::<Policy>(r#"{"other": 1}"#).is_err());
}
