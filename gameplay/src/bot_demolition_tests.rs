//! SYNTHETIC demolition team selection: invented structures, squads and bots.
use super::*;

fn target(index: u16, x: f64, owner: u8, value: f64) -> Target {
    Target {
        index,
        center: [x, 0., 0.],
        owner,
        value,
    }
}

fn squad(squad: u16, team: u8, lane_x: f64) -> Squad {
    Squad {
        squad,
        team,
        lane: Some([lane_x, 0., 0.]),
    }
}

fn bot(id: u16, squad: u16, x: f64) -> Candidate {
    Candidate {
        id,
        generation: 1,
        squad,
        position: [x, 0., 0.],
        available: true,
    }
}

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

#[test]
fn values_rank_closing_a_spawn_over_support_over_vehicles_over_the_rest() {
    let values = Values::default();
    let spawn = Hint {
        closes_spawn_of: Some(1),
        ..Default::default()
    };
    let support = Hint {
        toggles_abilities: true,
        ..Default::default()
    };
    let opens = Hint {
        opens_spawn_of: Some(0),
        ..Default::default()
    };
    let vehicle = Hint {
        closes_vehicle_spawn_of: Some(1),
        ..Default::default()
    };
    assert_eq!(spawn.value(&values, 1, 0, true), values.spawn);
    assert_eq!(spawn.value(&values, 1, 0, false), values.idle_spawn);
    assert_eq!(support.value(&values, 1, 0, true), values.support);
    assert_eq!(opens.value(&values, 1, 0, true), values.support);
    assert_eq!(vehicle.value(&values, 1, 0, true), values.vehicle_spawn);
    assert_eq!(Hint::default().value(&values, 1, 0, true), values.other);
    // A spawn of the attacking side itself is no reason to destroy anything.
    assert_eq!(spawn.value(&values, 0, 1, true), values.other);
}

#[test]
fn an_attacking_squad_sends_its_nearest_bots_to_the_best_scoring_structure() {
    let policy = Policy {
        team_size: 2,
        ..Default::default()
    };
    let mut book = Book::default();
    let mut changes = Vec::new();
    // A spawn structure 60 m from the lane beats a gate 10 m from it:
    // 100 - 30 = 70 > 20 - 5 = 15. The defenders' own structure is ignored.
    let targets = [
        target(7, 60., 1, policy.values.spawn),
        target(8, 10., 1, policy.values.other),
        target(9, 5., 0, policy.values.spawn),
    ];
    let bots = [
        bot(1, 0, 0.),
        bot(2, 0, 50.),
        bot(3, 0, 40.),
        bot(4, 16, 60.),
    ];
    let duties = book.plan(
        ms(0),
        &targets,
        &[squad(0, 0, 0.), squad(16, 1, 60.)],
        &bots,
        &policy,
        &mut changes,
    );
    assert_eq!(duties, BTreeMap::from([(2, 7), (3, 7), (4, 9)]));
    assert!(changes.contains(&Change::Picked {
        squad: 0,
        target: 7,
        score: 70.
    }));
}

#[test]
fn structures_out_of_reach_or_without_a_positive_score_are_left_alone() {
    let policy = Policy::default();
    let mut book = Book::default();
    let far = target(7, policy.reach_m + 1., 1, 1000.);
    let worthless = target(8, 50., 1, policy.distance_weight * 50.);
    let duties = book.plan(
        ms(0),
        &[far, worthless],
        &[squad(0, 0, 0.)],
        &[bot(1, 0, 0.)],
        &policy,
        &mut Vec::new(),
    );
    assert!(duties.is_empty());
}

#[test]
fn squads_take_distinct_structures_and_members_keep_their_places() {
    let policy = Policy {
        team_size: 2,
        teams_per_side: 2,
        ..Default::default()
    };
    let mut book = Book::default();
    let targets = [target(7, 10., 1, 100.), target(8, 20., 1, 100.)];
    let squads = [squad(0, 0, 0.), squad(1, 0, 0.)];
    let mut bots = vec![
        bot(1, 0, 10.),
        bot(2, 0, 11.),
        bot(3, 0, 12.),
        bot(4, 1, 0.),
        bot(5, 1, 1.),
    ];
    let first = book.plan(ms(0), &targets, &squads, &bots, &policy, &mut Vec::new());
    assert_eq!(first, BTreeMap::from([(1, 7), (2, 7), (4, 8), (5, 8)]));
    // Bot 1 dies; its new life is not the old member, and the nearest free
    // squad mate takes the place while bot 2 keeps its own.
    bots[0].generation = 2;
    bots[2].position = [100., 0., 0.];
    let second = book.plan(ms(100), &targets, &squads, &bots, &policy, &mut Vec::new());
    assert_eq!(second.get(&2), Some(&7));
    assert_eq!(second.iter().filter(|(_, t)| **t == 7).count(), 2);
}

#[test]
fn a_team_ends_when_its_structure_is_armed_and_gives_up_after_its_patience() {
    let policy = Policy {
        rest_ms: 0,
        ..Default::default()
    };
    let mut book = Book::default();
    let squads = [squad(0, 0, 0.)];
    let bots = [bot(1, 0, 0.)];
    let targets = [target(7, 10., 1, 100.)];
    assert_eq!(
        book.plan(ms(0), &targets, &squads, &bots, &policy, &mut Vec::new()),
        BTreeMap::from([(1, 7)])
    );
    // Armed (no longer a target): the team ends.
    let mut changes = Vec::new();
    assert!(
        book.plan(ms(10), &[], &squads, &bots, &policy, &mut changes)
            .is_empty()
    );
    assert!(changes.contains(&Change::Ended {
        squad: 0,
        target: 7,
        reason: "unavailable"
    }));
    // Back to a target, picked again; never armed in time, it is given up
    // and not picked again until the retry delay passes.
    book.plan(ms(20), &targets, &squads, &bots, &policy, &mut Vec::new());
    let late = ms(20 + policy.give_up_ms);
    let mut changes = Vec::new();
    assert!(
        book.plan(late, &targets, &squads, &bots, &policy, &mut changes)
            .is_empty()
    );
    assert!(changes.contains(&Change::Ended {
        squad: 0,
        target: 7,
        reason: "gave_up"
    }));
    let after = late + ms(policy.retry_ms);
    assert_eq!(
        book.plan(after, &targets, &squads, &bots, &policy, &mut Vec::new()),
        BTreeMap::from([(1, 7)])
    );
}

#[test]
fn unavailable_bots_do_not_join_and_a_disabled_book_drops_every_team() {
    let policy = Policy::default();
    let mut book = Book::default();
    let mut busy = bot(1, 0, 0.);
    busy.available = false;
    let targets = [target(7, 10., 1, 100.)];
    let duties = book.plan(
        ms(0),
        &targets,
        &[squad(0, 0, 0.)],
        &[busy, bot(2, 0, 30.)],
        &policy,
        &mut Vec::new(),
    );
    assert_eq!(duties, BTreeMap::from([(2, 7)]));
    let mut changes = Vec::new();
    book.clear(&mut changes);
    assert!(changes.contains(&Change::Left {
        squad: 0,
        target: 7,
        bot: 2
    }));
}

#[test]
fn policy_bounds_are_checked() {
    assert!(Policy::default().validate().is_ok());
    for bad in [
        Policy {
            team_size: 9,
            ..Default::default()
        },
        Policy {
            squads_per_target: 0,
            ..Default::default()
        },
        Policy {
            reach_m: f64::NAN,
            ..Default::default()
        },
        Policy {
            give_up_ms: 10,
            ..Default::default()
        },
    ] {
        assert!(bad.validate().is_err());
    }
}

#[test]
fn a_side_keeps_a_bounded_number_of_teams_and_rests_after_one_ends() {
    let policy = Policy {
        teams_per_side: 1,
        give_up_ms: 600_000,
        ..Default::default()
    };
    let mut book = Book::default();
    let targets = [target(7, 10., 1, 100.), target(8, 20., 1, 100.)];
    let squads = [squad(0, 0, 0.), squad(1, 0, 0.), squad(16, 1, 0.)];
    let bots = [bot(1, 0, 0.), bot(2, 1, 0.), bot(3, 16, 0.)];
    let enemy = target(9, 5., 0, 100.);
    let mut all = targets.to_vec();
    all.push(enemy);
    let first = book.plan(ms(0), &all, &squads, &bots, &policy, &mut Vec::new());
    assert_eq!(
        first,
        BTreeMap::from([(1, 7), (3, 9)]),
        "one team for each side"
    );
    // Structure 7 falls: the attacking side rests, the other side keeps its team.
    let rest = book.plan(
        ms(10),
        &[targets[1], enemy],
        &squads,
        &bots,
        &policy,
        &mut Vec::new(),
    );
    assert_eq!(rest, BTreeMap::from([(3, 9)]));
    let later = ms(10 + policy.rest_ms);
    let next = book.plan(
        later,
        &[targets[1], enemy],
        &squads,
        &bots,
        &policy,
        &mut Vec::new(),
    );
    assert_eq!(next.len(), 2);
    assert!(next.get(&1) == Some(&8) || next.get(&2) == Some(&8));
}
