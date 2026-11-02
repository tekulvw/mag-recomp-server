//! Invented retail type1 creations, never original packets or catalog records.
use super::*;
use crate::replication::{BitReader, ObjectUpdate};

fn vehicle(index: u16) -> Creation {
    Creation {
        index,
        spawn_point_index: index,
        spawn_point_token: 1000 + u32::from(index),
        vehicle_type_token: 0x12345678,
        faction: 1,
        team: 0,
        modifier: 0,
        position: CharacterPosition {
            x: 100.25,
            y: -2.5,
            z: 300.75,
        },
        rotation: [0., 0., 0., 1.],
        health_quarter_points: 600,
    }
}

#[test]
fn retail_ground_vehicle_creation_has_independent_transform_and_shared_damage() {
    let wire = encode(9, 123, &[vehicle(2)]).unwrap();
    let profile = crate::wire_profile::WireProfileCatalog::embedded()
        .profile(20231)
        .unwrap();
    let header = ObjectUpdate::decode_with_profile(&wire, profile)
        .unwrap()
        .header;
    assert_eq!(header.object_type_mask, 1 << 1);
    assert_eq!(header.sequence, 9);
    let mut bits = BitReader::with_profile(&wire, profile);
    bits.position = profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(2));
    for field in 0..30 {
        let present = [0, 1, 3, 4, 5, 6, 7, 18, 19, 20, 25, 26].contains(&field);
        assert_eq!(bits.read(1).unwrap(), u32::from(present), "field{field}");
        match field {
            0 => assert_eq!(bits.read(1), Ok(1)),
            1 => assert_eq!(bits.read(16), Ok(0x105)),
            3 => assert_eq!(bits.read(32), Ok(0x12345678)),
            4 => assert_eq!(bits.read(8), Ok(1)),
            5 => assert_eq!(bits.read(8), Ok(2)),
            6 | 7 => assert_eq!(bits.read(16), Ok(65535)),
            18 => {
                assert_eq!(bits.read(8), Ok(0));
                assert_eq!(bits.read(16), Ok(600));
                assert_eq!(bits.read(32), Ok(u32::MAX));
                assert_eq!(bits.read(32), Ok(u32::MAX));
                assert_eq!(bits.read(32), Ok(0));
            }
            19 => assert_eq!(bits.read(8), Ok(0)),
            20 => assert_eq!(bits.read(16), Ok(2)),
            25 => {
                assert_eq!(bits.read(16), Ok(3208));
                assert_eq!(bits.read(17), Ok((1 << 16) | 80));
                assert_eq!(bits.read(16), Ok(9624));
            }
            26 => {
                for value in [0, 0, 0, 256] {
                    assert_eq!(bits.read(11), Ok(value));
                }
            }
            _ => (),
        }
    }
    assert_eq!(bits.read(1), Ok(0));
}

#[test]
fn retail_ground_vehicle_creation_rejects_invalid_identity_transform_and_packet_size() {
    let v = vehicle(0);
    assert_eq!(
        encode(2, 0, &[v.clone(), v.clone()]).unwrap_err(),
        "duplicate_ground_vehicle"
    );
    for bad in [
        Creation {
            index: 32,
            ..v.clone()
        },
        Creation {
            faction: 3,
            ..v.clone()
        },
        Creation {
            team: 2,
            ..v.clone()
        },
        Creation {
            modifier: 64,
            ..v.clone()
        },
        Creation {
            rotation: [0.; 4],
            ..v.clone()
        },
        Creation {
            rotation: [f32::NAN, 0., 0., 1.],
            ..v.clone()
        },
        Creation {
            health_quarter_points: 4096,
            ..v.clone()
        },
        Creation {
            position: CharacterPosition {
                x: -1.,
                ..v.position
            },
            ..v.clone()
        },
    ] {
        assert!(encode(2, 0, &[bad]).is_err());
    }
    let many = (0..32).map(vehicle).collect::<Vec<_>>();
    assert!(
        encode(2, 0, &many).is_err(),
        "transport budget remains enforced"
    );
    assert!(encode(2, 0, &many[..16]).is_ok());
}

#[test]
fn retail_ground_vehicle_batches_preserve_separate_network_indices_and_budget() {
    let world = Catalog {
        format: "mag_ground_vehicles".into(),
        schema_version: 1,
        source: Source {
            spawn_catalog_sha256: "0".repeat(64),
            native_elf_sha256: "a".repeat(64),
            native_binding_sha256: "b".repeat(64),
            vehicle_properties_sha256: "c".repeat(64),
        },
        vehicles: (0..32).map(vehicle).collect(),
    };
    assert_eq!(world.batch_count(), 2);
    for batch in 0..2 {
        let wire = world.batch(batch, 2 + batch as u32, 0).unwrap();
        assert!(wire.len() <= 1397);
        let profile = crate::wire_profile::WireProfileCatalog::embedded()
            .profile(20231)
            .unwrap();
        let mut bits = BitReader::with_profile(&wire, profile);
        bits.position = profile.header_bits();
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(16), Ok(batch as u32 * 16));
    }
    assert!(world.batch(2, 4, 0).is_err());
    assert!(world.batch(usize::MAX, 4, 0).is_err());
    assert_eq!(world.vehicles[19].object_id(), 0x01000013);
}

#[test]
fn retail_ground_vehicle_quaternion_keeps_xyzw_sign_and_fraction_order() {
    let mut v = vehicle(0);
    v.rotation = [0., -0.6, 0.8, 0.];
    let wire = encode(2, 0, &[v]).unwrap();
    let profile = crate::wire_profile::WireProfileCatalog::embedded()
        .profile(20231)
        .unwrap();
    let mut bits = BitReader::with_profile(&wire, profile);
    bits.position = profile.header_bits() + 17;
    for width in [
        1, 16, 0, 32, 8, 8, 16, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 120, 8, 16, 0, 0, 0, 0, 49,
    ] {
        assert_eq!(bits.read(1).unwrap(), u32::from(width != 0));
        let mut remaining = width;
        while remaining > 0 {
            let step = remaining.min(32);
            bits.read(step).unwrap();
            remaining -= step;
        }
    }
    assert_eq!(bits.read(1), Ok(1));
    for expected in [0, 1024 | 153, 204, 0] {
        assert_eq!(bits.read(11), Ok(expected));
    }
}
