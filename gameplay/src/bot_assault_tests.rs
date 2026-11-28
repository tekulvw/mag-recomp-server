//! SYNTHETIC assault planning: invented points, stages, squads and positions.
use super::*;
use crate::replication::ObjectiveState::{self, *};

fn states(pairs: &[(u16, ObjectiveState)]) -> impl Fn(u16) -> Option<ObjectiveState> + '_ {
    move |point| pairs.iter().find(|(p, _)| *p == point).map(|(_, s)| *s)
}

fn on(squad: u16, point: u16) -> OnPoint {
    OnPoint { squad, point }
}

#[test]
fn defaults_validate_and_bad_rings_are_refused() {
    assert!(Policy::default().validate().is_ok());
    for bad in [
        Policy {
            hold_radius_m: [8., 4.],
            ..Policy::default()
        },
        Policy {
            hold_radius_m: [0., 4.],
            ..Policy::default()
        },
        Policy {
            rally_back_m: 5.,
            ..Policy::default()
        },
        Policy {
            rally_share: 1.5,
            ..Policy::default()
        },
        Policy {
            game_types: vec![9],
            ..Policy::default()
        },
    ] {
        assert_eq!(bad.validate(), Err("invalid_bot_assault_policy"));
    }
    // Rallies may be switched off.
    assert!(
        Policy {
            rally_back_m: 0.,
            ..Policy::default()
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn secured_and_contested_states_cover_a_point_until_it_is_lost() {
    for s in [Completed, Retaking, Retaken, CancelRetaken] {
        assert!(secured(s) && !contested(s), "{s:?}");
    }
    for s in [Deactivated, Activating, Activated, Deactivating] {
        assert!(contested(s) && !secured(s), "{s:?}");
    }
    for s in [Inactive, Locked] {
        assert!(!contested(s) && !secured(s), "{s:?}");
    }
}

#[test]
fn a_secured_point_keeps_its_hold_squads_and_sends_the_rest_to_its_partner() {
    let groups = vec![vec![10, 20]];
    let pairs = [(10, Completed), (20, Activated)];
    // Squads 3 and 1 hold 10, squad 5 attacks 20.
    let squads = [on(3, 10), on(1, 10), on(5, 20)];
    let moved = reinforcements(&groups, states(&pairs), &squads, 1);
    // The lowest squad index stays; the other reinforces 20.
    assert_eq!(moved, BTreeMap::from([(3, 20)]));
    // With two hold squads nobody moves.
    assert!(reinforcements(&groups, states(&pairs), &squads, 2).is_empty());
    // Zero hold squads: both leave.
    assert_eq!(
        reinforcements(&groups, states(&pairs), &squads, 0),
        BTreeMap::from([(1, 20), (3, 20)])
    );
}

#[test]
fn being_retaken_still_counts_as_secured_but_a_lost_point_calls_squads_home() {
    let groups = vec![vec![10, 20]];
    let squads = [on(1, 10), on(2, 10), on(5, 20)];
    for held in [Retaking, Retaken, CancelRetaken] {
        let pairs = [(10, held), (20, Deactivated)];
        assert_eq!(
            reinforcements(&groups, states(&pairs), &squads, 1),
            BTreeMap::from([(2, 20)])
        );
    }
    // Lost again (or never taken): every squad works its own point.
    let pairs = [(10, Deactivated), (20, Deactivated)];
    assert!(reinforcements(&groups, states(&pairs), &squads, 1).is_empty());
    // Both secured (the stage locks next): nothing open to reinforce.
    let pairs = [(10, Completed), (20, Completed)];
    assert!(reinforcements(&groups, states(&pairs), &squads, 1).is_empty());
    // A locked or inactive stage is not open either.
    let pairs = [(10, Locked), (20, Inactive)];
    assert!(reinforcements(&groups, states(&pairs), &squads, 1).is_empty());
}

#[test]
fn reinforcements_spread_over_the_least_worked_open_points() {
    let groups = vec![vec![1, 2, 3]];
    let pairs = [(1, Completed), (2, Deactivated), (3, Deactivated)];
    // Point 2 already has a squad, point 3 none.
    let squads = [on(0, 1), on(4, 1), on(6, 1), on(7, 2)];
    let moved = reinforcements(&groups, states(&pairs), &squads, 1);
    // Squad 4 goes to the empty point 3; squad 6 then breaks the 1-1 tie
    // towards the lower index.
    assert_eq!(moved, BTreeMap::from([(4, 3), (6, 2)]));
    // Squads on other stages are untouched.
    let other = vec![vec![1, 2, 3], vec![8, 9]];
    let pairs = [
        (1, Completed),
        (2, Deactivated),
        (3, Deactivated),
        (8, Deactivated),
        (9, Deactivated),
    ];
    let squads = [on(0, 1), on(4, 1), on(10, 8), on(11, 8)];
    assert_eq!(
        reinforcements(&other, states(&pairs), &squads, 1),
        BTreeMap::from([(4, 2)])
    );
}

#[test]
fn ring_places_stay_inside_the_ring_and_spread_by_slot() {
    let center = [100., 20., -50.];
    let ring = [2.5, 7.];
    let mut seen = Vec::new();
    for slot in 0..32 {
        for attempt in 0..8 {
            let p = ring_point(center, ring, slot, attempt);
            let d = flat(p, center);
            assert!((ring[0] - 1e-9..=ring[1] + 1e-9).contains(&d), "{d}");
            assert_eq!(p[1], 20.5);
        }
        seen.push(ring_point(center, ring, slot, 0));
    }
    // Different slots do not share a place.
    for (i, a) in seen.iter().enumerate() {
        for b in &seen[i + 1..] {
            assert!(flat(*a, *b) > 0.1);
        }
    }
}

#[test]
fn the_rally_place_lies_towards_home_and_never_beyond_it() {
    let center = [0., 0., 0.];
    let p = rally_point(center, [100., 3., 0.], 35.);
    assert!((p[0] - 35.).abs() < 1e-9 && p[2].abs() < 1e-9);
    // Home closer than the rally distance: rally at home.
    let p = rally_point(center, [0., 0., 20.], 35.);
    assert!((p[2] - 20.).abs() < 1e-9);
}

fn input(squad: u16, point: Option<u16>) -> WaveInput {
    WaveInput {
        squad,
        point,
        center: [0., 0., 0.],
        rally: [40., 0., 0.],
    }
}

fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

#[test]
fn a_squad_far_from_its_point_gathers_then_pushes_together() {
    let policy = Policy::default();
    let mut waves = BTreeMap::new();
    let far = vec![[120., 0., 0.]; 4];
    let bots = BTreeMap::from([(1, far)]);
    let changes = advance_waves(&mut waves, &[input(1, Some(7))], &bots, &policy, at(0));
    assert_eq!(
        changes,
        vec![(
            1,
            Some(Wave::Rally {
                point: 7,
                since: at(0)
            })
        )]
    );
    // Two of four at the rally place: below 60%, still gathering, no change.
    let bots = BTreeMap::from([(
        1,
        vec![[40., 0., 0.], [41., 0., 2.], [120., 0., 0.], [120., 0., 0.]],
    )]);
    assert!(advance_waves(&mut waves, &[input(1, Some(7))], &bots, &policy, at(5000)).is_empty());
    // Three of four: push.
    let bots = BTreeMap::from([(
        1,
        vec![[40., 0., 0.], [41., 0., 2.], [45., 0., -3.], [120., 0., 0.]],
    )]);
    assert_eq!(
        advance_waves(&mut waves, &[input(1, Some(7))], &bots, &policy, at(6000)),
        vec![(1, Some(Wave::Push { point: 7 }))]
    );
    // Pushing bots inside the rally distance keep the push, even as others
    // fall back or respawn far away.
    let bots = BTreeMap::from([(1, vec![[5., 0., 0.], [120., 0., 0.]])]);
    assert!(advance_waves(&mut waves, &[input(1, Some(7))], &bots, &policy, at(9000)).is_empty());
    // The push is wiped out: the next wave gathers again.
    let bots = BTreeMap::from([(1, vec![[120., 0., 0.]])]);
    assert_eq!(
        advance_waves(&mut waves, &[input(1, Some(7))], &bots, &policy, at(10000)),
        vec![(
            1,
            Some(Wave::Rally {
                point: 7,
                since: at(10000)
            })
        )]
    );
}

#[test]
fn a_rally_ends_after_its_wait_and_squads_already_forward_push_at_once() {
    let policy = Policy::default();
    let mut waves = BTreeMap::new();
    let bots = BTreeMap::from([(1, vec![[120., 0., 0.]]), (2, vec![[10., 0., 0.]])]);
    let inputs = [input(1, Some(7)), input(2, Some(7))];
    let changes = advance_waves(&mut waves, &inputs, &bots, &policy, at(0));
    assert_eq!(changes.len(), 2);
    assert_eq!(waves[&2], Wave::Push { point: 7 });
    assert!(advance_waves(&mut waves, &inputs, &bots, &policy, at(19_999)).is_empty());
    assert_eq!(
        advance_waves(&mut waves, &inputs, &bots, &policy, at(20_000)),
        vec![(1, Some(Wave::Push { point: 7 }))]
    );
}

#[test]
fn waves_end_when_the_point_is_no_longer_open_or_rallies_are_off() {
    let mut waves = BTreeMap::new();
    let bots = BTreeMap::from([(1, vec![[120., 0., 0.]])]);
    let policy = Policy::default();
    advance_waves(&mut waves, &[input(1, Some(7))], &bots, &policy, at(0));
    // The point is being taken or held: no wave, bots go in.
    assert_eq!(
        advance_waves(&mut waves, &[input(1, None)], &bots, &policy, at(100)),
        vec![(1, None)]
    );
    assert!(waves.is_empty());
    // A squad that is gone drops its wave too.
    advance_waves(&mut waves, &[input(1, Some(7))], &bots, &policy, at(200));
    assert_eq!(
        advance_waves(&mut waves, &[], &bots, &policy, at(300)),
        vec![(1, None)]
    );
    let off = Policy {
        rally_back_m: 0.,
        ..Policy::default()
    };
    assert!(advance_waves(&mut waves, &[input(1, Some(7))], &bots, &off, at(400)).is_empty());
    assert!(waves.is_empty());
}

#[test]
fn a_squad_without_living_bots_keeps_its_wave() {
    let policy = Policy::default();
    let mut waves = BTreeMap::from([(1, Wave::Push { point: 7 })]);
    let empty = BTreeMap::new();
    assert!(advance_waves(&mut waves, &[input(1, Some(7))], &empty, &policy, at(0)).is_empty());
    assert_eq!(waves[&1], Wave::Push { point: 7 });
}
