//! Invented current pair resolutions; no claimed native collision membership.
use mag_gameplay::acquisition_volume_damage::*;
fn target(kind: TargetKind, team: u8) -> Target {
    Target {
        key: TargetKey {
            id: 9,
            generation: 1,
        },
        kind,
        team,
    }
}
#[test]
fn actor_team_filter_ground_only_and_mine_no_team_filter_are_distinct() {
    let args = Arguments {
        player_damage: -1.,
        ground_vehicle_damage: 5.,
        mine_damage: -1.,
        apply_to_team: -1,
    };
    assert!(
        dispatch(args, target(TargetKind::Character, 2))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        dispatch(args, target(TargetKind::Character, 0))
            .unwrap()
            .unwrap()
            .effect,
        Effect::LethalActor
    );
    assert_eq!(
        dispatch(args, target(TargetKind::GroundVehicle, 1))
            .unwrap()
            .unwrap()
            .effect,
        Effect::ActorDamage(5.)
    );
    assert!(
        dispatch(args, target(TargetKind::Other, 0))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        dispatch(args, target(TargetKind::Mine, 2))
            .unwrap()
            .unwrap()
            .effect,
        Effect::Mine(MineMutation::ExceedThreshold)
    );
    let zeros = Arguments {
        player_damage: 0.,
        ground_vehicle_damage: 0.,
        mine_damage: 0.,
        apply_to_team: 0,
    };
    for kind in [
        TargetKind::Character,
        TargetKind::GroundVehicle,
        TargetKind::Mine,
        TargetKind::Other,
    ] {
        assert!(dispatch(zeros, target(kind, 0)).unwrap().is_none());
    }
}
#[test]
fn duplicate_mine_pairs_repeat_accumulation_and_do_not_explode_or_remove() {
    let args = Arguments {
        player_damage: 0.,
        ground_vehicle_damage: 0.,
        mine_damage: -1.,
        apply_to_team: 0,
    };
    let mut state = MineState {
        accumulated: 0.,
        threshold: 5.,
        flags: 4,
    };
    for _ in 0..2 {
        let request = dispatch(args, target(TargetKind::Mine, 1))
            .unwrap()
            .unwrap();
        let Effect::Mine(mutation) = request.effect else {
            panic!("mine effect")
        };
        state = state.apply(mutation).unwrap();
    }
    assert_eq!(state.accumulated, 8.);
    assert_eq!(state.flags, 4 | 0x80000000);
    assert_eq!(state.apply(MineMutation::Add(0.)).unwrap(), state);
    assert_eq!(
        MineState {
            accumulated: 10.,
            threshold: 10.5,
            flags: 0
        }
        .apply(MineMutation::ExceedThreshold)
        .unwrap()
        .accumulated,
        12.
    );
}
#[test]
fn positive_mine_damage_threshold_and_unsupported_arithmetic_are_explicit() {
    let state = MineState {
        accumulated: 1.,
        threshold: 5.,
        flags: 8,
    };
    assert_eq!(
        state.apply(MineMutation::Add(2.)).unwrap(),
        MineState {
            accumulated: 3.,
            ..state
        }
    );
    assert_eq!(
        state.apply(MineMutation::Add(4.)).unwrap().flags,
        8 | 0x80000000
    );
    assert!(state.apply(MineMutation::Add(f32::NAN)).is_err());
    assert!(state.apply(MineMutation::Add(-1.)).is_err());
    assert!(
        MineState {
            accumulated: f32::MAX,
            threshold: f32::MAX,
            flags: 0
        }
        .apply(MineMutation::Add(f32::MAX))
        .is_err()
    );
    let args = Arguments {
        player_damage: f32::INFINITY,
        ground_vehicle_damage: 0.,
        mine_damage: 0.,
        apply_to_team: 0,
    };
    assert!(dispatch(args, target(TargetKind::Character, 0)).is_err());
}
