//! Synthetic scores and clocks for the Acquisition round contract.
use mag_gameplay::acquisition_round::{Phase, ResultNotice, Round, Rules, ScoreIntent, Transition};
use mag_gameplay::acquisition_score_rate::Request;

fn round(starting_points: [i16; 2]) -> Round {
    Round::new(Rules {
        starting_points,
        major_percent: 50,
        end_delay_ms: 2000,
    })
}

fn rate(amount: u32) -> Request {
    Request {
        team: 0,
        frequency_ms: 1000,
        amount,
    }
}

#[test]
fn bleed_waits_past_deadline_and_deducts_once_per_update() {
    let mut round = round([1200, 1000]);
    round.rates_mut().configure(rate(3), 100).unwrap();
    assert_eq!(round.update_bleed(1100), [None, None]);
    assert_eq!(round.points(), [1200, 1000]);
    assert_eq!(round.update_bleed(5500), [Some(1197), None]);
    assert_eq!(round.update_bleed(5500), [Some(1194), None]);
    round.rates_mut().configure(rate(0), 5500).unwrap();
    assert_eq!(round.update_bleed(6000), [None, None]);
}

#[test]
fn score_changes_wrap_clamp_and_refuse_revival() {
    let mut round = round([i16::MAX, 1000]);
    assert_eq!(
        round.apply_score(ScoreIntent { team: 0, delta: 0 }),
        Ok(None)
    );
    assert_eq!(
        round.apply_score(ScoreIntent { team: 0, delta: 1 }),
        Ok(Some(0))
    );
    assert_eq!(
        round.apply_score(ScoreIntent {
            team: 0,
            delta: 100
        }),
        Ok(None)
    );
    assert!(
        round
            .apply_score(ScoreIntent { team: 2, delta: -1 })
            .is_err()
    );
    assert_eq!(round.points(), [0, 1000]);
}

#[test]
fn simultaneous_bleed_depletion_chooses_team_one_in_the_same_update() {
    let mut round = round([3, 3]);
    round.rates_mut().configure(rate(3), 100).unwrap();
    round
        .rates_mut()
        .configure(Request { team: 1, ..rate(3) }, 100)
        .unwrap();
    assert_eq!(round.update_bleed(1101), [Some(0), Some(0)]);
    assert_eq!(
        round.update_end(1101),
        Some(Transition::EndGame(ResultNotice {
            winner: 1,
            major: false
        }))
    );
    assert_eq!(round.phase(), Phase::Ending { deadline_ms: 3101 });
    assert_eq!(round.update_end(3100), None);
    assert_eq!(round.update_end(3101), Some(Transition::Finished));
    assert_eq!(round.update_end(3102), None);
    assert_eq!(round.phase(), Phase::Finished);
}

#[test]
fn time_zero_is_the_epoch_sentinel_and_exact_half_is_major() {
    let mut round = round([1200, 1000]);
    round
        .apply_score(ScoreIntent {
            team: 0,
            delta: -600,
        })
        .unwrap();
    round
        .apply_score(ScoreIntent {
            team: 1,
            delta: -1000,
        })
        .unwrap();
    assert_eq!(round.update_end(0), None);
    assert_eq!(round.phase(), Phase::Playing);
    assert_eq!(
        round.update_end(1),
        Some(Transition::EndGame(ResultNotice {
            winner: 0,
            major: true
        }))
    );
}

#[test]
fn end_delay_uses_unsigned_wrapping_addition_and_reset_discards_round_state() {
    let mut round = round([1200, 1000]);
    round.rates_mut().configure(rate(1), 100).unwrap();
    round
        .apply_score(ScoreIntent {
            team: 1,
            delta: -1000,
        })
        .unwrap();
    assert!(matches!(
        round.update_end(u32::MAX - 1999),
        Some(Transition::EndGame(_))
    ));
    assert_eq!(round.phase(), Phase::Ending { deadline_ms: 0 });
    assert_eq!(round.update_end(1), Some(Transition::Finished));
    round.reset();
    assert_eq!(round.phase(), Phase::Playing);
    assert_eq!(round.points(), [1200, 1000]);
    assert_eq!(round.update_bleed(9999), [None, None]);
}

#[test]
fn bleed_narrows_amount_to_signed_sixteen_bits_before_subtraction() {
    let mut round = round([1200, 1000]);
    round.rates_mut().configure(rate(65535), 100).unwrap();
    assert_eq!(round.update_bleed(1101), [Some(1201), None]);
}
