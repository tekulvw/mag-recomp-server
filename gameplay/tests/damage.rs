// SYNTHETIC: all inputs in this suite are authored in code; no game catalogs or captures.
use mag_gameplay::damage::{DamageKind, ResolvedDamage};
#[test]
fn native_remaining_health_quantization_and_invalid_damage() {
    let mut d = ResolvedDamage {
        amount_hp: 0.1,
        kind: DamageKind::Bullet,
        kinetic_response: true,
        location: 3,
        attacker_id: 1,
        source_object_id: 2,
        user_data: 3,
    };
    assert_eq!(d.apply(400).unwrap().health_quarter_points, 399);
    assert_eq!(d.apply(400).unwrap().last_location_and_type, 0x93);
    for amount in [0.0, -1.0, f32::NAN, f32::INFINITY, 1001.0] {
        d.amount_hp = amount;
        assert!(d.apply(400).is_err());
    }
    d.amount_hp = 1000.0;
    assert_eq!(d.apply(400).unwrap().health_quarter_points, 0);
    assert!(d.apply(4096).is_err());
    d.location = 16;
    assert!(d.apply(400).is_err());
}
