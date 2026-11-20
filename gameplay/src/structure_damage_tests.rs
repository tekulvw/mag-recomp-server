use super::{StructureDamage, StructureDamageOutcome};

#[test]
fn raw_damage_accumulates_through_exact_threshold_once() {
    let mut receiver = StructureDamage::new(4, 60.0).unwrap();
    assert_eq!(
        receiver.apply(4, 15.0, true, true),
        StructureDamageOutcome::Accumulated
    );
    assert_eq!(receiver.accumulated_damage(), 15.0);
    assert!(!receiver.is_destroyed());
    assert_eq!(
        receiver.apply(4, 45.0, true, true),
        StructureDamageOutcome::Destroyed
    );
    assert!(receiver.is_destroyed());
    assert_eq!(
        receiver.apply(4, 10.0, true, true),
        StructureDamageOutcome::Ignored
    );
    assert_eq!(receiver.accumulated_damage(), 60.0);
}

#[test]
fn airstrike_only_receiver_requires_source_four_active_receiver_and_active_owner() {
    let mut receiver = StructureDamage::new(5, 40.0).unwrap();
    for (source, active, owner_is_active) in [(2, true, true), (4, false, true), (4, true, false)] {
        assert_eq!(
            receiver.apply(source, 60.0, active, owner_is_active),
            StructureDamageOutcome::Ignored
        );
    }
    assert_eq!(receiver.accumulated_damage(), 0.0);
    assert_eq!(
        receiver.apply(4, 60.0, true, true),
        StructureDamageOutcome::Destroyed
    );
    assert_eq!(receiver.accumulated_damage(), 60.0);
}

#[test]
fn invalid_thresholds_and_damage_preserve_valid_runtime_state() {
    for kind in [0, 1, 2, 3, 6] {
        assert!(StructureDamage::new(kind, 10.0).is_err());
    }
    for threshold in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
        assert!(StructureDamage::new(4, threshold).is_err());
    }
    let mut receiver = StructureDamage::new(4, 10.0).unwrap();
    for amount in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
        assert_eq!(
            receiver.apply(4, amount, true, true),
            StructureDamageOutcome::Ignored
        );
    }
    assert_eq!(receiver.accumulated_damage(), 0.0);
}
