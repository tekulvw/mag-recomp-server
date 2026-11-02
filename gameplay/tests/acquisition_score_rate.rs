//! Synthetic times and raw script arguments, never captured game state.
use mag_gameplay::{acquisition_control::UnboundCall, acquisition_score_rate::*};

#[test]
fn repeated_rate_calls_preserve_deadline_and_due_updates_deduct_once_strictly() {
    let request = Request::from_call(&UnboundCall {
        symbol: "Game_SetScoreBleed".into(),
        arguments: vec![1, 1, 0],
    })
    .unwrap();
    let mut clock = Rates::default();
    clock.configure(request, 100).unwrap();
    assert_eq!(clock.rates()[0].deadline_ms, 1100);
    clock.configure(request, 600).unwrap();
    assert_eq!(clock.rates()[0].deadline_ms, 1100);
    assert_eq!(clock.due(1100), [None, None]);
    assert_eq!(
        clock.due(5000),
        [Some(Deduction { team: 0, amount: 1 }), None]
    );
    assert_eq!(clock.rates()[0].deadline_ms, 2100);
    assert_eq!(
        clock.due(5000),
        [Some(Deduction { team: 0, amount: 1 }), None]
    );
    assert_eq!(clock.rates()[0].deadline_ms, 3100);
}

#[test]
fn earlier_candidate_disable_and_unsigned_wrap_follow_the_setter() {
    let mut clock = Rates::default();
    clock
        .configure(
            Request {
                frequency_ms: 2000,
                amount: 3,
                team: 1,
            },
            100,
        )
        .unwrap();
    clock
        .configure(
            Request {
                frequency_ms: 500,
                amount: 2,
                team: 1,
            },
            200,
        )
        .unwrap();
    assert_eq!(clock.rates()[1].deadline_ms, 700);
    clock
        .configure(
            Request {
                frequency_ms: 500,
                amount: 0,
                team: 1,
            },
            300,
        )
        .unwrap();
    assert_eq!(clock.rates()[1].deadline_ms, 0);
    assert_eq!(clock.due(10000), [None, None]);
    clock
        .configure(
            Request {
                frequency_ms: 10,
                amount: 1,
                team: 0,
            },
            u32::MAX - 4,
        )
        .unwrap();
    assert_eq!(clock.rates()[0].deadline_ms, 5);
    assert_eq!(clock.due(5), [None, None]);
    assert_eq!(clock.due(6), [Some(Deduction { team: 0, amount: 1 }), None]);
    let before = clock.clone();
    assert!(
        clock
            .configure(
                Request {
                    frequency_ms: 1,
                    amount: 1,
                    team: 2
                },
                0
            )
            .is_err()
    );
    assert_eq!(clock, before);
}

#[test]
fn conversion_is_explicit_bounded_and_does_not_silently_accept_unknown_calls() {
    let input = UnboundCall {
        symbol: "Game_SetScoreBleed".into(),
        arguments: vec![2, i32::MAX, 1],
    };
    assert_eq!(
        Request::from_call(&input).unwrap(),
        Request {
            frequency_ms: 2000,
            amount: 2147483648,
            team: 1
        }
    );
    for arguments in [
        vec![1, 1],
        vec![-1, 1, 0],
        vec![1, -1, 0],
        vec![1, 1, 2],
        vec![i32::MAX, 1, 0],
    ] {
        assert!(
            Request::from_call(&UnboundCall {
                symbol: input.symbol.clone(),
                arguments
            })
            .is_err()
        );
    }
    assert!(
        Request::from_call(&UnboundCall {
            symbol: "Unknown".into(),
            arguments: vec![1, 1, 0]
        })
        .is_err()
    );
}
