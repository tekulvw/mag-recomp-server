use crate::{
    active_ability_state::NativeAbilityData,
    replication::{BitReader, ObjectUpdate},
    strategic_replication::CompanyUpdate,
    wire_profile::WireProfileCatalog,
};

#[test]
fn company_field_order_and_ability_index_match_native_type_eleven() {
    let profile =
        WireProfileCatalog::from_json(include_str!("../data/synthetic-wire-profiles.json"))
            .unwrap()
            .profile(20231)
            .unwrap();
    let mut update = CompanyUpdate::new(1);
    update.owner_index = Some(0x105);
    update.active = Some(true);
    update.leader_index = Some(0x20);
    update.active_mask = Some(1 << 7);
    update.available_mask = Some(0xe1);
    update.shadow_modifier_percent = Some(-3);
    update.data[7] = Some(NativeAbilityData {
        activate_time_ms: 500,
        enabled_at_start: true,
        ..Default::default()
    });
    let wire = CompanyUpdate::encode_batch(&[update], 2, 502, profile).unwrap();
    let decoded = ObjectUpdate::decode_with_profile(&wire, profile).unwrap();
    assert_eq!(decoded.header.object_type_mask, 1 << 11);
    let mut bits = BitReader::with_profile(decoded.wire, profile);
    bits.position = profile.header_bits();
    assert_eq!(bits.read(1).unwrap(), 1);
    assert_eq!(bits.read(16).unwrap(), 1);
    assert_eq!(bits.read(2).unwrap(), 3);
    assert_eq!(bits.read(1).unwrap(), 1);
    assert_eq!(bits.read(16).unwrap(), 0x105);
    assert_eq!(bits.read(7).unwrap(), 0);
    assert_eq!(bits.read(1).unwrap(), 1);
    for (width, expected) in [
        (32, 0),
        (32, 0),
        (32, 0),
        (16, 0),
        (32, 500),
        (1, 1),
        (32, u32::MAX),
    ] {
        assert_eq!(bits.read(width).unwrap(), expected);
    }
    assert_eq!(bits.read(1).unwrap(), 0);
    for (width, expected) in [(16, 0x20), (16, 1 << 7), (16, 0xe1), (8, 253)] {
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(bits.read(width).unwrap(), expected);
    }
    assert_eq!(bits.read(1).unwrap(), 0);
}
