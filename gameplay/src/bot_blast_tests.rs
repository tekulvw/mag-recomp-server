//! SYNTHETIC blast zones: invented centres and radii.
use super::*;

fn zone() -> Zone {
    Zone {
        objective: 3,
        center: [0., 0., 0.],
        radius: 10.,
        attackers: 0,
    }
}

#[test]
fn a_zone_contains_points_within_its_horizontal_radius() {
    let z = zone();
    assert!(z.contains([9.9, 50., 0.]), "height does not matter");
    assert!(!z.contains([10., 0., 0.]));
    assert!(!z.contains([8., 0., 8.]));
}

#[test]
fn a_route_from_outside_enters_a_zone_only_when_a_leg_passes_inside() {
    let z = zone();
    // Straight through the middle.
    assert!(z.entered_by(&[[-20., 0., 0.], [20., 0., 0.]]));
    // Around it.
    assert!(!z.entered_by(&[
        [-20., 0., 0.],
        [-20., 0., 15.],
        [20., 0., 15.],
        [20., 0., 0.]
    ]));
    // Ending inside.
    assert!(z.entered_by(&[[-20., 0., 0.], [-5., 0., 0.]]));
    // Starting inside is leaving, not entering.
    assert!(!z.entered_by(&[[0., 0., 0.], [-20., 0., 0.]]));
    assert!(!z.entered_by(&[]));
}

#[test]
fn the_attackers_guard_ring_is_pushed_outside_the_zone() {
    let policy = crate::bot_armed_objectives::ArmedPolicy::default();
    let zones = Zones::from([(3, zone())]);
    let pushed = Engine::<Vec<u8>>::armed_policy_outside_blast(&zones, 3, &policy);
    let [inner, outer] = pushed.guard_radius_m;
    assert!(inner > zone().radius);
    assert!(
        (outer - inner - (policy.guard_radius_m[1] - policy.guard_radius_m[0])).abs() < 1e-9,
        "the ring keeps its width"
    );
    assert_eq!(
        pushed.converge_radius_m, policy.converge_radius_m,
        "defenders keep theirs"
    );
    let other = Engine::<Vec<u8>>::armed_policy_outside_blast(&zones, 4, &policy);
    assert_eq!(other, policy, "another objective's ring is unchanged");
}

#[test]
fn policy_bounds_are_checked() {
    assert!(Policy::default().validate().is_ok());
    assert!(
        Policy {
            margin_m: -1.,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}
