use super::*;

fn launch() -> Motion<u8> {
    Motion::launch(
        [1., 1., 1.],
        [10., 0., 0.],
        1_000,
        Limits {
            armed_ms: 200,
            lifetime_ms: 10_000,
            grace_ms: 2_000,
            linear_damping: 0.1,
            restitution: 0.3,
            maximum_speed_mps: 50.,
            maximum_duration_ms: 12_000,
        },
    )
    .unwrap()
}

fn contact(key: u8, fraction: f64, normal: Option<[f64; 3]>) -> Contact<u8, u8> {
    Contact {
        key,
        fraction,
        normal,
        obstacle: key,
    }
}

#[test]
fn ballistic_gravity_damping_and_advance_are_record_owned() {
    let motion = launch();
    let Advance::Flying(candidate) = motion
        .advance(1_020, |start, end| {
            assert_eq!(start, [1., 1., 1.]);
            assert!((end[0] - 1.2).abs() < 1e-12);
            assert!((end[1] - (1. - 0.5 * 9.81 * 0.02_f64.powi(2))).abs() < 1e-12);
            Ok(None::<Contact<u8, u8>>)
        })
        .unwrap()
    else {
        panic!("expected flight")
    };
    assert_eq!(motion.advanced_at_ms(), 1_000);
    assert_eq!(motion.position(), [1., 1., 1.]);
    assert_eq!(candidate.advanced_at_ms(), 1_020);
    assert!((candidate.velocity()[0] - 9.98).abs() < 1e-12);
    assert!(candidate.velocity()[1] < 0.);
}

#[test]
fn ballistic_prearm_bounces_and_later_new_contact_ends_at_inclusive_arm() {
    let motion = launch();
    let Advance::Flying(bounced) = motion
        .advance(1_020, |_, _| Ok(Some(contact(1, 0.5, Some([-1., 0., 0.])))))
        .unwrap()
    else {
        panic!("early contact must preserve flight")
    };
    assert!((bounced.velocity()[0] + 2.994).abs() < 1e-12);
    assert!(bounced.position()[0] < 1.1);
    let Advance::Terminal { next, terminal } = bounced
        .advance(1_200, |_, _| Ok(Some(contact(2, 0.5, Some([1., 0., 0.])))))
        .unwrap()
    else {
        panic!("outer-update arm equality admits new contact")
    };
    assert_eq!(terminal.reason, TerminalReason::Contact(2));
    assert_eq!(terminal.position, next.position());
    assert!((terminal.elapsed_ms - 30.).abs() < 1e-12);
    assert!(matches!(
        next.advance(1_201, |_, _| Ok(None::<Contact<u8, u8>>)),
        Ok(Advance::AlreadyTerminal)
    ));
}

#[test]
fn ballistic_initial_overlap_and_unresolved_normal_never_turn_into_armed_contact() {
    let motion = launch();
    let Advance::Flying(blocked) = motion
        .advance(1_020, |_, _| Ok(Some(contact(1, 0.5, None))))
        .unwrap()
    else {
        panic!("unresolved early normal must preserve flight")
    };
    let Advance::Flying(still_blocked) = blocked
        .advance(1_200, |_, _| Ok(Some(contact(1, 0., Some([0., 1., 0.])))))
        .unwrap()
    else {
        panic!("unchanged overlap is not a new contact event")
    };
    assert_eq!(still_blocked.advanced_at_ms(), 1_200);

    let Advance::Flying(_) = still_blocked
        .advance(1_220, |_, _| Ok(Some(contact(1, 0.5, Some([0., 1., 0.])))))
        .unwrap()
    else {
        panic!("same unresolved contact cannot detonate without separation")
    };
}

#[test]
fn ballistic_work_is_capped_and_age_terminal_wins_only_at_its_deadline() {
    let mut motion = launch();
    let mut calls = 0;
    loop {
        let before = calls;
        match motion
            .advance(13_000, |_, _| {
                calls += 1;
                Ok(None::<Contact<u8, u8>>)
            })
            .unwrap()
        {
            Advance::Flying(next) => {
                assert!(next.advanced_at_ms() > motion.advanced_at_ms());
                assert!(calls - before <= 16);
                motion = next;
            }
            Advance::Terminal { next, terminal } => {
                assert_eq!(terminal.reason, TerminalReason::AgeExpired);
                assert_eq!(terminal.elapsed_ms, 12_000.);
                assert_eq!(next.advanced_at_ms(), 13_000);
                break;
            }
            Advance::AlreadyTerminal => panic!("one terminal expected"),
        }
    }
}

#[test]
fn ballistic_same_feature_can_detonate_only_after_a_clear_separating_sweep() {
    let source = launch();
    let Advance::Flying(blocked) = source
        .advance(1_020, |_, _| Ok(Some(contact(7, 0.5, Some([-1., 0., 0.])))))
        .unwrap()
    else {
        panic!("expected early bounce");
    };
    let Advance::Flying(clear) = blocked
        .advance(1_040, |start, end| {
            assert!(end[0] < start[0]);
            Ok(None::<Contact<u8, u8>>)
        })
        .unwrap()
    else {
        panic!("expected separating flight");
    };
    let Advance::Terminal { terminal, .. } = clear
        .advance(1_200, |_, _| Ok(Some(contact(7, 0.5, Some([1., 0., 0.])))))
        .unwrap()
    else {
        panic!("new armed contact with the same feature should be eligible");
    };
    assert_eq!(terminal.reason, TerminalReason::Contact(7));
}

#[test]
fn ballistic_invalid_inputs_and_query_refusal_leave_the_candidate_untouched() {
    let source = launch();
    let before = source.clone();
    assert!(matches!(
        source.advance(999, |_, _| Ok(None::<Contact<u8, u8>>)),
        Err("ballistic_time_reversed")
    ));
    let mut calls = 0;
    assert!(matches!(
        source.advance(1_041, |_, _| {
            calls += 1;
            if calls == 2 {
                Err("test_query_budget")
            } else {
                Ok(None::<Contact<u8, u8>>)
            }
        }),
        Err("test_query_budget")
    ));
    assert_eq!(source, before);
    assert!(matches!(
        source.advance(1_020, |_, _| Ok(Some(contact(4, f64::NAN, None)))),
        Err("invalid_ballistic_contact_fraction")
    ));
    let invalid = Limits {
        restitution: 1.01,
        ..source.limits
    };
    assert!(matches!(
        Motion::<u8>::launch([0.; 3], [1., 0., 0.], 0, invalid),
        Err("invalid_ballistic_limits")
    ));
}
