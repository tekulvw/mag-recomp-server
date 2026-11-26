//! SYNTHETIC squad order selection: invented squads, lanes and objectives.
use super::*;
use std::collections::BTreeSet;

fn squad(squad: u16, team: u8, lane: Option<u16>) -> SquadInput {
    SquadInput {
        squad,
        team,
        human_order: false,
        tag: u32::MAX,
        lane,
        position: None,
    }
}

fn ordered(mut s: SquadInput, objective: u16) -> SquadInput {
    s.human_order = true;
    s.tag = objective_tag(Some(objective));
    s
}

/// Objectives 0..10 on a line, x = 10 * index; `closed` are completed.
fn pick(squads: &[SquadInput], closed: &[u16]) -> BTreeMap<u16, Choice> {
    let closed: BTreeSet<u16> = closed.iter().copied().collect();
    choose(
        squads,
        true,
        |index, _| index < 10 && !closed.contains(&index),
        |index| (index < 10).then(|| [f64::from(index) * 10., 0., 0.]),
    )
}

/// The lane function the Acquisition and Sabotage runtimes use: the first
/// authored objective that is not inactive, completed or locked.
fn lane(authored: &[u16], closed: &[u16]) -> Option<u16> {
    authored.iter().copied().find(|i| !closed.contains(i))
}

#[test]
fn squads_follow_their_lane_in_order_and_skip_completed_objectives() {
    let authored = [0, 1, 2, 3];
    let mut closed = Vec::new();
    for expected in authored {
        let choice = pick(&[squad(0, 0, lane(&authored, &closed))], &closed);
        assert_eq!(choice[&0], Some((expected, Source::Lane)));
        closed.push(expected);
    }
    // Every lane objective completed and no other squad working: nothing.
    assert_eq!(
        pick(&[squad(0, 0, lane(&authored, &closed))], &closed)[&0],
        None
    );
}

#[test]
fn a_human_frago_comes_before_the_lane_even_outside_it() {
    let choice = pick(&[ordered(squad(0, 0, Some(1)), 7)], &[]);
    assert_eq!(choice[&0], Some((7, Source::Order)));
}

#[test]
fn a_completed_frago_returns_the_squad_to_its_next_lane_objective() {
    // The FRAGO was the lane's first objective; once it completes the lane
    // runtime moves on and so does the squad.
    let authored = [3, 4, 5];
    let closed = [3];
    let choice = pick(
        &[ordered(squad(0, 0, lane(&authored, &closed)), 3)],
        &closed,
    );
    assert_eq!(choice[&0], Some((4, Source::Lane)));
}

#[test]
fn unreachable_or_non_objective_tags_are_not_followed() {
    // Objective 12 has no place; a vehicle tag is not an objective.
    let mut vehicle = squad(1, 0, Some(2));
    vehicle.human_order = true;
    vehicle.tag = 0x0100_0004;
    let choice = pick(&[ordered(squad(0, 0, Some(1)), 12), vehicle], &[]);
    assert_eq!(choice[&0], Some((1, Source::Lane)));
    assert_eq!(choice[&1], Some((2, Source::Lane)));
}

#[test]
fn only_a_human_leaders_tag_overrides_the_lane() {
    // A squad without a human order carries the server's own tag, which
    // follows its lane; a stale one never pulls the squad away.
    let mut automatic = squad(0, 0, Some(2));
    automatic.tag = objective_tag(Some(5));
    assert_eq!(pick(&[automatic], &[])[&0], Some((2, Source::Lane)));
    let choice = choose(
        &[ordered(squad(0, 0, Some(2)), 5)],
        false,
        |_, _| true,
        |i| Some([f64::from(i), 0., 0.]),
    );
    assert_eq!(choice[&0], Some((2, Source::Lane)), "follow_orders off");
}

#[test]
fn an_exhausted_lane_helps_the_nearest_objective_of_its_own_team() {
    let mut idle = squad(2, 0, None);
    idle.position = Some([62., 0., 0.]);
    let squads = [
        squad(0, 0, Some(1)),
        ordered(squad(1, 0, Some(1)), 7),
        idle,
        // The other team's objective is closer but not the idle squad's.
        squad(16, 1, Some(6)),
    ];
    let choice = pick(&squads, &[]);
    assert_eq!(choice[&2], Some((7, Source::Nearest)));
    assert_eq!(choice[&16], Some((6, Source::Lane)));
    // Without a position the lowest objective index wins the tie.
    let squads = [
        squad(0, 0, Some(4)),
        squad(1, 0, Some(3)),
        squad(2, 0, None),
    ];
    assert_eq!(pick(&squads, &[])[&2], Some((3, Source::Nearest)));
}

#[test]
fn automatic_tags_follow_the_squads_objective_and_clear_with_it() {
    assert_eq!(
        automatic_tag(u32::MAX, Some((3, Source::Lane))),
        Some(0x0c80_0003)
    );
    assert_eq!(automatic_tag(0x0c80_0003, Some((3, Source::Lane))), None);
    assert_eq!(
        automatic_tag(0x0c80_0003, Some((4, Source::Nearest))),
        Some(0x0c80_0004)
    );
    assert_eq!(automatic_tag(0x0c80_0004, None), Some(u32::MAX));
    assert_eq!(automatic_tag(u32::MAX, None), None);
    assert_eq!(tagged_objective(0x0c80_ffff), None);
    assert_eq!(tagged_objective(0x0d80_0001), None);
}

#[test]
fn a_new_human_leader_inherits_the_server_tag_until_setting_their_own() {
    let mut book = Book::default();
    assert!(!book.human_order(0, false, u32::MAX));
    // No server tag yet: a leader's (empty) order is theirs.
    assert!(book.human_order(0, true, u32::MAX));
    book.set(0, 0x0c80_0003);
    // Elected over the server's tag: still the server's to move.
    assert!(!book.human_order(0, true, 0x0c80_0003));
    // The leader tags something else: from now on the order is theirs,
    // even if they later tag the server's old objective again.
    assert!(book.human_order(0, true, 0x0c80_0007));
    assert!(book.human_order(0, true, 0x0c80_0003));
}

#[test]
fn policy_defaults_on_and_rejects_unknown_fields() {
    let policy: Policy = serde_json::from_str("{}").unwrap();
    assert_eq!(policy, Policy::default());
    assert!(policy.follow_orders && policy.automatic_orders);
    let off: Policy =
        serde_json::from_str(r#"{"follow_orders":false,"automatic_orders":false}"#).unwrap();
    assert!(!off.follow_orders && !off.automatic_orders);
    assert!(serde_json::from_str::<Policy>(r#"{"follow":true}"#).is_err());
}

#[test]
fn a_refused_automatic_tag_is_not_asked_for_again_until_one_applies() {
    let mut book = Book::default();
    book.refuse(3, 0x0c80_0005);
    assert!(book.refused(3, 0x0c80_0005));
    assert!(!book.refused(3, 0x0c80_0006));
    assert!(!book.refused(4, 0x0c80_0005));
    book.set(3, 0x0c80_0006);
    assert!(!book.refused(3, 0x0c80_0005));
}
