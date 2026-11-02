use mag_gameplay::acquisition_damage_metadata::*;

#[test]
fn crushed_character_request_retains_native_self_attribution_and_raw_amount() {
    let m = Metadata::from_symbols(
        "DamageType.ExternalKinetic",
        "DamageSourceType.eDamageSourceTypeCrushed",
    )
    .unwrap();
    let r = m.character_lethal(0x03800007, [-3., 4., 0.], 9).unwrap();
    assert_eq!(r.amount_hp, 1000.); // Raw input to normal damage; no force-zero shortcut.
    assert_eq!(r.damage_type, 5);
    assert_eq!(r.source_type, 10);
    assert_eq!(r.attacker, 0x03800007);
    assert_eq!(r.source_index, -1);
    assert_eq!(r.source_object, u32::MAX);
    assert_eq!(r.location, 0);
    assert_eq!(r.user_data, 0);
    assert_eq!(r.armor_reduction, [0., 0.]);
    assert_eq!(r.from_direction, [-3., 4., 0.]); // Copied victim vector, not normalized.
    assert!(r.clear_invulnerability);
    assert_eq!(r.restore_invulnerability, Some(1)); // Native bool normalization, not previous raw9.
    let unprotected = m.character_lethal(0x03800007, [0.; 3], 0).unwrap();
    assert!(!unprotected.clear_invulnerability);
    assert_eq!(unprotected.restore_invulnerability, None);
}

#[test]
fn ground_request_uses_unknown_attacker_and_vaporize_without_claiming_immediate_destruction() {
    let m = Metadata::from_symbols(
        "DamageType.ExternalKinetic",
        "DamageSourceType.eDamageSourceTypeCrushed",
    )
    .unwrap();
    assert_eq!(
        m.ground_lethal(),
        GroundLethal {
            attacker: u32::MAX,
            vaporize: true
        }
    );
    // Ground model IsDead/SetHealth and later lifecycle are handled elsewhere.
    assert!(
        Metadata::from_symbols("Unknown", "DamageSourceType.eDamageSourceTypeCrushed").is_err()
    );
    assert!(Metadata::from_symbols("DamageType.ExternalKinetic", "Unknown").is_err());
    assert!(m.character_lethal(u32::MAX, [0.; 3], 0).is_err());
    assert!(
        m.character_lethal(0x03800007, [f32::NAN, 0., 0.], 0)
            .is_err()
    );
}
