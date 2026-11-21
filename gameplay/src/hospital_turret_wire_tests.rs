use super::*;
use crate::replication::{ObjectUpdate, UpdateHeader};
use crate::wire_profile::WireProfileCatalog;

fn profile() -> WireProfile {
    WireProfileCatalog::embedded().profile(20231).unwrap()
}

fn from_hex(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn read_body(bytes: &[u8]) -> BitReader<'_> {
    let mut bits = BitReader::with_profile(bytes, profile());
    bits.position = 148;
    bits
}

fn make_header(type_mask: u32) -> BitWriter {
    let mut bits = BitWriter::with_profile(profile());
    UpdateHeader {
        sequence: 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: type_mask,
        timestamp_ms: 0,
    }
    .encode(&mut bits)
    .unwrap();
    bits
}

fn append_row(bits: &mut BitWriter, index: u16, field: Option<usize>) {
    bits.write(1, 1).unwrap();
    bits.write(index.into(), 16).unwrap();
    for slot in 0..25 {
        let present = field == Some(slot);
        bits.write(u32::from(present), 1).unwrap();
        if present && slot == 9 {
            bits.write(0, 10).unwrap();
        }
    }
}

fn item_tuple(item: InventoryItem) -> (u8, u8, u8, u8, bool) {
    (
        item.inventory_update_id,
        item.gear_slot,
        item.ammo_pool,
        item.ammo_count,
        item.reloading,
    )
}

#[test]
fn decodes_independent_retail_mixed_list_without_consuming_the_next_family() {
    // Original layout: retail header 148 bits. Family 13 has three rows.
    // Row 4 updates all five owner fields, row 7 has no fields, and row 15
    // sets both angles. The following family 16 list is explicitly empty.
    let wire = from_hex(concat!(
        "00080102030400000000000012000010001fe",
        "8002006005376ac077356181601d1100070000004003c03d170000000",
    ));
    let update = ObjectUpdate::decode_with_profile(&wire, profile()).unwrap();
    assert_eq!(update.header.object_type_mask, 0x12000);
    let mut bits = read_body(&wire);
    let rows = decode_objects(&mut bits).unwrap();
    assert_eq!(
        rows.iter().map(|row| row.index).collect::<Vec<_>>(),
        [4, 7, 15]
    );

    let first = rows[0].fields;
    assert_eq!(first.yaw.unwrap().packed(), 0x400);
    assert_eq!(first.yaw.unwrap().radians().to_bits(), (-0.0_f32).to_bits());
    assert_eq!(first.pitch.unwrap().packed(), 0x137);
    assert_eq!(first.pitch.unwrap().radians(), 311.0 / 256.0);
    assert_eq!(item_tuple(first.primary.unwrap()), (0xab, 0, 0, 238, false));
    assert_eq!(item_tuple(first.secondary.unwrap()), (0xab, 1, 2, 5, true));
    assert_eq!(first.overheat_info, Some(0xd1));

    let empty = rows[1].fields;
    assert!(empty.yaw.is_none());
    assert!(empty.pitch.is_none());
    assert!(empty.primary.is_none());
    assert!(empty.secondary.is_none());
    assert!(empty.overheat_info.is_none());

    let last = rows[2].fields;
    assert_eq!(last.yaw.unwrap().packed(), 0x745);
    assert_eq!(last.yaw.unwrap().radians(), -837.0 / 128.0);
    assert_eq!(last.pitch.unwrap().packed(), 0x200);
    assert_eq!(
        last.pitch.unwrap().radians().to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(bits.position, 373);
    assert_eq!(bits.read(1).unwrap(), 0, "family 16 terminator");
    assert_eq!(bits.read(2).unwrap(), 0, "final packet padding");
}

#[test]
fn encodes_independent_retail_definition_with_a_hole_and_exact_sparse_equipment() {
    let version = 0xad;
    let primary = InventoryItem {
        inventory_update_id: version,
        gear_slot: 0,
        ammo_pool: 0,
        ammo_count: 250,
        reloading: false,
    };
    let secondary = InventoryItem {
        inventory_update_id: 0x81,
        gear_slot: 5,
        ammo_pool: 3,
        ammo_count: 2,
        reloading: true,
    };
    let rows = [
        ServerUpdate {
            index: 2,
            definition: Some(InventoryDefinition {
                version,
                items: [Some(0xd67), None, Some(0x9c4)],
            }),
            fields: Fields {
                yaw: Some(Yaw::new(0x400).unwrap()),
                primary: Some(primary),
                ..Fields::default()
            },
        },
        ServerUpdate {
            index: 15,
            definition: None,
            fields: Fields {
                pitch: Some(Pitch::new(0x280).unwrap()),
                secondary: Some(secondary),
                overheat_info: Some(0xb3),
                ..Fields::default()
            },
        },
    ];
    // Two synthetic indices and a present/absent/present definition. The data
    // starts at bit 148 and ends at bit 345 before final zero padding.
    let expected = from_hex(concat!(
        "0008090ab00c0000000000000200011223344",
        "80010060035bd6767135a03e80008007803401812e0500d9800",
    ));
    assert_eq!(
        encode_updates(0x090ab00c, 0x11223344, &rows, profile()).unwrap(),
        [expected]
    );
}

#[test]
fn rejects_malformed_rows_and_server_only_fields_before_the_owner_boundary() {
    for field in [0, 1, 2, 6, 7, 10, 13, 18, 19, 20, 22, 24] {
        let mut wire = make_header(1 << 13);
        append_row(&mut wire, 0, Some(field));
        wire.write(0, 1).unwrap();
        let data = wire.finish();
        assert_eq!(
            decode_objects(&mut read_body(&data)).unwrap_err(),
            "unsupported_client_turret_field",
            "field {field}",
        );
    }

    let mut duplicate = make_header(1 << 13);
    append_row(&mut duplicate, 15, None);
    append_row(&mut duplicate, 15, None);
    duplicate.write(0, 1).unwrap();
    assert!(decode_objects(&mut read_body(&duplicate.finish())).is_err());

    let mut out_of_range = make_header(1 << 13);
    append_row(&mut out_of_range, 16, None);
    out_of_range.write(0, 1).unwrap();
    assert!(decode_objects(&mut read_body(&out_of_range.finish())).is_err());

    let mut truncated = make_header(1 << 13);
    append_row(&mut truncated, 0, Some(9));
    let data = truncated.finish();
    assert_eq!(
        data.len(),
        25,
        "the final row has no padding to hide a missing terminator"
    );
    assert_eq!(
        decode_objects(&mut read_body(&data)).unwrap_err(),
        "truncated_replication_bits",
    );

    let mut empty = make_header(1 << 13);
    empty.write(0, 1).unwrap();
    empty.write(0x81, 8).unwrap();
    let data = empty.finish();
    let mut bits = read_body(&data);
    assert!(decode_objects(&mut bits).unwrap().is_empty());
    assert_eq!(
        bits.position, 149,
        "the outer decoder owns any trailing bits"
    );
    assert_eq!(bits.read(8).unwrap(), 0x81);
}

#[test]
fn enforces_retail_profile_and_allocation_and_field_bounds() {
    let prototype = WireProfile::prototype();
    assert!(decode_objects(&mut BitReader::with_profile(&[0], prototype)).is_err());
    assert!(encode_updates(1, 0, &[], prototype).is_err());
    assert!(encode_updates(0, 0, &[], profile()).is_err());
    assert!(encode_updates(1, 0, &[], profile()).unwrap().is_empty());

    let empty_row = ServerUpdate {
        index: 2,
        definition: None,
        fields: Fields::default(),
    };
    assert!(encode_updates(1, 0, &[empty_row, empty_row], profile()).is_err());
    let wrong_index = ServerUpdate {
        index: 16,
        ..empty_row
    };
    assert!(encode_updates(1, 0, &[wrong_index], profile()).is_err());
    let wrong_item = ServerUpdate {
        definition: Some(InventoryDefinition {
            version: 1,
            items: [Some(4096), None, None],
        }),
        ..empty_row
    };
    assert!(encode_updates(1, 0, &[wrong_item], profile()).is_err());
    assert!(Yaw::new(2048).is_err());
    assert!(Pitch::new(1024).is_err());
}

#[test]
fn all_sixteen_fully_populated_rows_fit_one_bounded_packet() {
    let item = InventoryItem {
        inventory_update_id: u8::MAX,
        gear_slot: 31,
        ammo_pool: 3,
        ammo_count: u8::MAX,
        reloading: true,
    };
    let rows: Vec<_> = (0..16)
        .map(|index| ServerUpdate {
            index,
            definition: Some(InventoryDefinition {
                version: 255,
                items: [Some(4095); 3],
            }),
            fields: Fields {
                yaw: Some(Yaw::new(2047).unwrap()),
                pitch: Some(Pitch::new(1023).unwrap()),
                primary: Some(item),
                secondary: Some(item),
                overheat_info: Some(255),
            },
        })
        .collect();
    let packets = encode_updates(u32::MAX, 0, &rows, profile()).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].len(), 351);
    let more: Vec<_> = rows.into_iter().chain([rows_last()]).collect();
    assert!(encode_updates(1, 0, &more, profile()).is_err());
}

fn rows_last() -> ServerUpdate {
    ServerUpdate {
        index: 15,
        definition: None,
        fields: Fields::default(),
    }
}
