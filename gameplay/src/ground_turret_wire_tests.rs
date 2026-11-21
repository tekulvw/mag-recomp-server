use super::*;
use crate::{
    hospital_turret_wire::{Fields, InventoryDefinition, Pitch, Yaw},
    replication::InventoryItem,
    wire_profile::{WireProfile, WireProfileCatalog},
};

fn profile() -> WireProfile {
    WireProfileCatalog::embedded().profile(20231).unwrap()
}

fn field_row(bits: &mut BitWriter, index: u16, fields: &[(usize, u32, usize)]) {
    bits.write(1, 1).unwrap();
    bits.write(index.into(), 16).unwrap();
    for field in 0..30 {
        let value = fields.iter().find(|(present, _, _)| *present == field);
        bits.write(u32::from(value.is_some()), 1).unwrap();
        if let Some((_, value, width)) = value {
            bits.write(*value, *width).unwrap();
        }
    }
}

fn equipped() -> InventoryItem {
    InventoryItem {
        inventory_update_id: 0xa3,
        gear_slot: 0,
        ammo_pool: 0,
        ammo_count: 0x5a,
        reloading: true,
    }
}

#[test]
fn ground_gunner_wire_keeps_one_family_list_for_both_roles() {
    let mut bits = BitWriter::with_profile(profile());
    let primary = (0xa3 << 16) | (0x5a << 1) | 1;
    field_row(
        &mut bits,
        31,
        &[
            (8, 0x400, 11),
            (9, 0x201, 10),
            (11, primary, 24),
            (21, 0x8a, 8),
            (27, 1, 1),
            (28, 0x240, 10),
        ],
    );
    field_row(&mut bits, 3, &[]);
    bits.write(0, 1).unwrap();
    bits.write(0b101101, 6).unwrap(); // Beginning of the following selected family.
    let wire = bits.finish();
    let mut reader = BitReader::with_profile(&wire, profile());
    let rows = decode_owner_updates(&mut reader).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].dynamics.index, 31);
    assert_eq!(rows[0].dynamics.brake, Some(true));
    assert_eq!(rows[0].dynamics.throttle, Some(-0.25));
    assert_eq!(
        rows[0].turret.yaw.unwrap().radians().to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(rows[0].turret.pitch.unwrap().packed(), 0x201);
    assert_eq!(rows[0].turret.primary.unwrap().ammo_count, 0x5a);
    assert_eq!(rows[0].turret.overheat_info, Some(0x8a));
    assert_eq!(rows[1].dynamics.index, 3);
    assert!(!rows[1].dynamics.has_changes() && !rows[1].turret.has_changes());
    assert_eq!(reader.read(6).unwrap(), 0b101101);

    let mut reader = BitReader::with_profile(&wire, profile());
    assert_eq!(
        OwnerDynamics::decode_list(&mut reader).unwrap_err(),
        "unsupported_client_ground_vehicle_field"
    );
}

#[test]
fn ground_gunner_wire_rejects_server_fields_duplicates_and_unknown_indices() {
    let decode = |indices: &[u16], fields: &[(usize, u32, usize)]| {
        let mut bits = BitWriter::with_profile(profile());
        for &index in indices {
            field_row(&mut bits, index, fields);
        }
        bits.write(0, 1).unwrap();
        decode_owner_updates(&mut BitReader::with_profile(&bits.finish(), profile()))
    };
    for field in [0, 1, 6, 10, 13, 18, 22, 24] {
        assert_eq!(
            decode(&[0], &[(field, 0, 0)]).unwrap_err(),
            "unsupported_client_ground_vehicle_field"
        );
    }
    assert_eq!(
        decode(&[31, 31], &[]).unwrap_err(),
        "duplicate_ground_vehicle_owner_update"
    );
    assert_eq!(
        decode(&[32], &[]).unwrap_err(),
        "ground_vehicle_owner_index_out_of_range"
    );
}

#[test]
fn ground_gunner_wire_interleaves_with_character_hospital_and_projectile_lists() {
    use crate::owner_update::TypedOwnerUpdateFields;

    let mut bits = BitWriter::with_profile(profile());
    UpdateHeader {
        sequence: 9,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: (1 << 1) | (1 << 3) | (1 << 13) | (1 << 16),
        timestamp_ms: 42,
    }
    .encode(&mut bits)
    .unwrap();
    field_row(&mut bits, 31, &[(8, 0x302, 11), (27, 1, 1)]);
    field_row(&mut bits, 2, &[]);
    bits.write(0, 1).unwrap();
    // Owned Character: 22 fields, pitch 6.
    bits.write(1, 1).unwrap();
    bits.write(7, 16).unwrap();
    bits.write(1, 7).unwrap();
    bits.write(0x84, 8).unwrap();
    bits.write(0, 16).unwrap();
    // Hospital: independent family 13 index 15, with pitch 9.
    bits.write(1, 1).unwrap();
    bits.write(15, 16).unwrap();
    bits.write(1, 10).unwrap();
    bits.write(0x203, 10).unwrap();
    bits.write(0, 16).unwrap();
    // Owner-matched slot 0 Projectile: 8 fields, state 3.
    bits.write(1, 1).unwrap();
    bits.write(7, 16).unwrap();
    bits.write(1, 4).unwrap();
    bits.write(2, 8).unwrap();
    bits.write(0, 5).unwrap();

    let mut wire = bits.finish();
    let update = ObjectUpdate::decode_with_profile(&wire, profile()).unwrap();
    let parsed = TypedOwnerUpdateFields::decode_checked_with_profile(
        &update,
        7,
        profile(),
        true,
        true,
        true,
    )
    .unwrap();
    assert_eq!(parsed.ground_vehicles.len(), 2);
    assert_eq!(parsed.ground_vehicles[0].brake, Some(true));
    assert_eq!(parsed.ground_turrets.len(), 1);
    assert_eq!(parsed.ground_turrets[0].dynamics.index, 31);
    assert_eq!(parsed.ground_turrets[0].turret.yaw.unwrap().packed(), 0x302);
    assert_eq!(parsed.character.unwrap().pitch, Some(0x84));
    assert_eq!(parsed.hospital_turrets[0].index, 15);
    assert_eq!(
        parsed.hospital_turrets[0].fields.pitch.unwrap().packed(),
        0x203
    );
    assert_eq!(parsed.projectiles[0].fields.state, Some(2));
    assert_eq!(
        TypedOwnerUpdateFields::decode_checked_with_profile(
            &update,
            7,
            profile(),
            true,
            true,
            false
        )
        .unwrap_err()
        .code(),
        "unsupported_client_ground_vehicle_field"
    );

    wire.push(0);
    let padded = ObjectUpdate::decode_with_profile(&wire, profile()).unwrap();
    assert_eq!(
        TypedOwnerUpdateFields::decode_checked_with_profile(
            &padded,
            7,
            profile(),
            true,
            true,
            true
        )
        .unwrap_err()
        .code(),
        "trailing_client_update_bits"
    );
}

#[test]
fn ground_gunner_wire_writes_independent_authorities_and_sparse_definition() {
    let row = TurretUpdate {
        index: 12,
        driver_owner: Some(SERVER_OWNER),
        turret_owner: Some(23),
        definition: Some(InventoryDefinition {
            version: 0xa3,
            items: [Some(0xd65), None, None],
        }),
        fields: Fields {
            yaw: Some(Yaw::new(0x400).unwrap()),
            pitch: Some(Pitch::new(0x201).unwrap()),
            primary: Some(equipped()),
            overheat_info: Some(0x8a),
            ..Fields::default()
        },
    };
    let mut raw = BitWriter::with_profile(profile());
    UpdateHeader {
        sequence: 9,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 1 << 1,
        timestamp_ms: 42,
    }
    .encode(&mut raw)
    .unwrap();
    // Authored layout: 8-bit version, 1-bit-present + 12-bit gear, two holes.
    let definition = (0xa3 << 15) | (1 << 14) | (0xd65 << 2);
    let primary = (0xa3 << 16) | (0x5a << 1) | 1;
    field_row(
        &mut raw,
        12,
        &[
            (1, 0x105, 16),
            (6, 23, 16),
            (8, 0x400, 11),
            (9, 0x201, 10),
            (10, definition, 23),
            (11, primary, 24),
            (21, 0x8a, 8),
        ],
    );
    raw.write(0, 1).unwrap();
    assert_eq!(
        TurretUpdate::encode_rows(9, 42, &[row], profile()).unwrap(),
        [raw.finish()]
    );
    let full = TurretUpdate {
        definition: Some(InventoryDefinition {
            version: 0xa3,
            items: [Some(0xd65), Some(0xd66), Some(0xd67)],
        }),
        fields: Fields {
            secondary: Some(equipped()),
            ..row.fields
        },
        ..row
    };
    let rows = std::array::from_fn::<_, 32, _>(|index| TurretUpdate {
        index: index as u16,
        ..full
    });
    let packet = TurretUpdate::encode_rows(9, 42, &rows, profile()).unwrap();
    assert_eq!(packet.len(), 1);
    assert_eq!(packet[0].len(), 831);
    assert!(TurretUpdate::encode_rows(9, 42, &[row, row], profile()).is_err());
    assert!(
        TurretUpdate::encode_rows(9, 42, &[TurretUpdate { index: 32, ..row }], profile()).is_err()
    );
}
