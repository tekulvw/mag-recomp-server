//! SYNTHETIC descent arithmetic. No captured map data.
use super::*;

fn landing() -> replication::CharacterPosition {
    replication::CharacterPosition {
        x: 5.,
        y: 10.,
        z: -3.,
    }
}

#[test]
fn a_descent_comes_straight_down_at_a_steady_speed() {
    let start = Duration::from_secs(7);
    let motion = bot_physics::Motion::new([5., 10., -3.]);
    let descent = Descent::plan(landing(), 42., start, motion).unwrap();
    assert_eq!(descent.lands, start + Duration::from_secs(5));
    assert_eq!(descent.position(start).y, 42.);
    let halfway = descent.position(start + Duration::from_millis(2500));
    assert!((halfway.y - 26.).abs() < 1e-4, "{halfway:?}");
    assert_eq!((halfway.x, halfway.z), (5., -3.));
    assert_eq!(descent.position(start + Duration::from_secs(9)), landing());
    assert!(!descent.landed(start + Duration::from_millis(4999)));
    assert!(descent.landed(start + Duration::from_secs(5)));
    let airborne = descent.airborne_motion();
    assert!(!airborne.grounded);
    assert_eq!(airborne.position, motion.position);
}

#[test]
fn a_drop_zone_not_above_the_landing_spot_has_no_descent() {
    let motion = bot_physics::Motion::new([5., 10., -3.]);
    for top in [10., 10.9, 4., f32::NAN] {
        assert!(
            Descent::plan(landing(), top, Duration::ZERO, motion).is_none(),
            "{top}"
        );
    }
}
