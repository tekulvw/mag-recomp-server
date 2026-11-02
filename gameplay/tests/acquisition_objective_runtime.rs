use mag_gameplay::{
    acquisition_control::ObjectiveOperation as Op, acquisition_objective_runtime::*,
};

fn objective(index: u16, state: i32, attacker: bool, order: i32) -> Order {
    Order {
        objective: Some(Objective::new(0x0c800000 | u32::from(index), state, attacker).unwrap()),
        order,
    }
}

#[test]
fn operation_guards_use_current_while_duplicate_detection_uses_target() {
    let state = State {
        current: 0,
        target: 1,
    };
    assert_eq!(state.request(Op::Enable), None);
    assert_eq!(State { target: 9, ..state }.request(Op::Enable), Some(1));
    assert_eq!(state.request(Op::Disable), None);
    assert_eq!(
        State {
            current: 7,
            target: 0
        }
        .request(Op::Disable),
        None
    );
    assert_eq!(
        State {
            current: 7,
            target: 1
        }
        .request(Op::Disable),
        Some(0)
    );
    assert_eq!(
        State {
            current: 9,
            target: 0
        }
        .request(Op::Lock),
        None
    );
    assert_eq!(
        State {
            current: 7,
            target: 9
        }
        .request(Op::Lock),
        None
    );
    assert_eq!(
        State {
            current: 7,
            target: 0
        }
        .request(Op::Lock),
        Some(9)
    );
    assert_eq!(state.current, 0); // Requests never commit current state or run entry.
}

#[test]
fn native_order_table_covers_all_states_roles_and_invalid_modes() {
    for current in -2..=14 {
        for attacker in [false, true] {
            for mode in -1..=2 {
                let take_group = [1, 2, 3, 4, 6].contains(&current);
                let hold_group = [5, 7, 10, 11, 12].contains(&current);
                let expected = match (attacker, mode) {
                    (true, 0) | (false, 1) => take_group,
                    (true, 1) | (false, 0) => hold_group,
                    _ => false,
                };
                assert_eq!(
                    objective(5, current, attacker, mode).is_valid(),
                    expected,
                    "state={current} attacker={attacker} mode={mode}"
                );
            }
        }
    }
    assert!(
        !Order {
            objective: None,
            order: 0
        }
        .is_valid()
    );
    assert!(Objective::new(0x01000005, 1, true).is_err());
    assert!(Objective::new(0x0c80ffff, 1, true).is_err());
}

#[test]
fn ordered_primary_and_frago_are_independent_and_manual_tags_win() {
    let normal = [
        objective(2, 0, true, 0),
        Order {
            objective: None,
            order: 0,
        },
        objective(6, 1, true, 0),
        objective(4, 1, true, 0),
        objective(6, 1, true, 0),
    ];
    let frago = [objective(8, 7, true, 1), objective(9, 7, true, 1)];
    assert_eq!(first_valid(&normal), Some(0x0c800006));
    let inputs = Refresh {
        gate_value: 0,
        spawn_resolved: true,
        normal: Some(&normal),
        frago: &frago,
        manual_tag: u32::MAX,
    };
    let mut a = Assignment {
        primary_index: 3,
        tagged_object: u32::MAX,
    };
    assert_eq!(
        a.refresh(inputs),
        RefreshResult {
            skipped: false,
            primary_changed: true,
            tag_written: true
        }
    );
    assert_eq!(
        a,
        Assignment {
            primary_index: 6,
            tagged_object: 0x0c800008
        }
    );
    assert!(!a.refresh(inputs).primary_changed);
    assert!(
        a.refresh(Refresh {
            manual_tag: 0x17800019,
            ..inputs
        })
        .tag_written
    );
    assert_eq!(a.tagged_object, 0x17800019);
    a.refresh(Refresh {
        normal: Some(&[]),
        manual_tag: u32::MAX,
        ..inputs
    });
    assert_eq!(a.primary_index, u16::MAX);
    assert_eq!(a.tagged_object, 0x0c800008); // FRAGO never replaces primary.
    a.refresh(Refresh {
        normal: Some(&[]),
        frago: &[],
        ..inputs
    });
    assert_eq!(a.tagged_object, u32::MAX);
}

#[test]
fn native_early_returns_preserve_both_assignments() {
    let before = Assignment {
        primary_index: 3,
        tagged_object: 0x17800019,
    };
    let normal = [objective(6, 1, true, 0)];
    let inputs = Refresh {
        gate_value: 0,
        spawn_resolved: true,
        normal: Some(&normal),
        frago: &normal,
        manual_tag: u32::MAX,
    };
    for input in [
        Refresh {
            gate_value: 3,
            ..inputs
        },
        Refresh {
            spawn_resolved: false,
            ..inputs
        },
        Refresh {
            normal: None,
            ..inputs
        },
    ] {
        let mut after = before;
        assert_eq!(
            after.refresh(input),
            RefreshResult {
                skipped: true,
                primary_changed: false,
                tag_written: false
            }
        );
        assert_eq!(after, before);
    }
}
