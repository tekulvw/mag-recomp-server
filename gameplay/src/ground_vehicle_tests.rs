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
        initial_state: 2,
        passenger_exit_candidates: Vec::new(),
        passenger_seats: Vec::new(),
        maximum_occupancy: None,
        maximum_mount_distance_m: None,
        maximum_mount_position_age_ms: None,
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
        turret_policies: Default::default(),
        raw_sha256: String::new(),
        format: "mag_ground_vehicles".into(),
        schema_version: 1,
        source: Source {
            spawn_catalog_sha256: "0".repeat(64),
            native_elf_sha256: "a".repeat(64),
            native_binding_sha256: "b".repeat(64),
            vehicle_properties_sha256: "c".repeat(64),
            creation_scope: None,
        },
        vehicles: (0..32).map(vehicle).collect(),
        passenger_spawns: Vec::new(),
        vehicle_sources: Vec::new(),
        driver_seats: Vec::new(),
        seat_visibility: None,
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

#[test]
fn requested_free_vehicle_slot_can_be_created_in_hidden_state_zero() {
    let mut v = vehicle(4);
    v.initial_state = 0;
    let wire = encode(2, 0, &[v]).unwrap();
    let profile = crate::wire_profile::WireProfileCatalog::embedded()
        .profile(20231)
        .unwrap();
    let mut bits = BitReader::with_profile(&wire, profile);
    bits.position = profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(4));
    for field in 0..=5 {
        let present = bits.read(1).unwrap();
        match field {
            0 => {
                assert_eq!(present, 1);
                assert_eq!(bits.read(1), Ok(1));
            }
            1 => {
                bits.read(16).unwrap();
            }
            2 => assert_eq!(present, 0),
            3 => {
                bits.read(32).unwrap();
            }
            4 => {
                bits.read(8).unwrap();
            }
            5 => assert_eq!(bits.read(8), Ok(0)),
            _ => unreachable!(),
        }
    }
    let mut bad = vehicle(0);
    bad.initial_state = 0;
    assert!(encode(2, 0, &[bad]).is_err()); // Four initial static slots stay reserved.
}

#[test]
fn retail_ground_owner_decodes_sparse_dynamics_from_fixture_bits() {
    use crate::replication::{BitReader, BitWriter};
    let profile = crate::wire_profile::WireProfileCatalog::embedded()
        .profile(20231)
        .unwrap();
    let mut bits = BitWriter::with_profile(profile);
    for (value, width) in [
        (1, 1),
        (4, 16),
        (0, 25),
        (1, 1),
        (321, 16),
        (1, 1),
        (48, 16),
        (257, 16),
        (1, 1),
        (0, 11),
        (0, 11),
        (0, 11),
        (256, 11),
        (1, 1),
        (0, 1),
        (1, 1),
        (1, 1),
        (192, 9),
        (1, 1),
        (1, 1),
        (1, 1),
        (7, 16),
        (0, 27),
        (1, 1),
        (1, 1),
        (0, 2),
        (0, 1),
        (1, 1),
    ] {
        bits.write(value, width).unwrap();
    }
    let wire = bits.finish();
    let mut reader = BitReader::with_profile(&wire, profile);
    let reports = OwnerDynamics::decode_list(&mut reader).unwrap();
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].index, 4);
    let position = reports[0].position.unwrap();
    assert_eq!(
        [position.x, position.y, position.z],
        [10.03125, -1.5, 8.03125]
    );
    assert_eq!(reports[0].rotation_xyzw, Some([0., 0., 0., 1.]));
    assert_eq!(reports[0].brake, Some(false));
    assert_eq!(reports[0].throttle, Some(-0.75));
    assert_eq!(reports[0].smoke_screen, Some(true));
    assert_eq!(
        reports[1],
        OwnerDynamics {
            index: 7,
            brake: Some(true),
            ..OwnerDynamics::default()
        }
    );
    assert_eq!(reader.read(1), Ok(1));
    assert!(OwnerDynamics::decode_list(&mut BitReader::with_profile(&wire[..5], profile)).is_err());
}

#[test]
fn retail_ground_owner_rejects_server_owned_or_duplicate_fields() {
    use crate::replication::{BitReader, BitWriter};
    let profile = crate::wire_profile::WireProfileCatalog::embedded()
        .profile(20231)
        .unwrap();
    for (values, expected) in [
        (
            vec![(1, 1), (4, 16), (1, 1), (1, 1), (0, 29), (0, 1)],
            "unsupported_client_ground_vehicle_field",
        ),
        (
            vec![(1, 1), (32, 16)],
            "ground_vehicle_owner_index_out_of_range",
        ),
        (
            vec![
                (1, 1),
                (4, 16),
                (0, 27),
                (1, 1),
                (1, 1),
                (0, 2),
                (1, 1),
                (4, 16),
            ],
            "duplicate_ground_vehicle_owner_update",
        ),
    ] {
        let mut bits = BitWriter::with_profile(profile);
        for (value, width) in values {
            bits.write(value, width).unwrap();
        }
        let wire = bits.finish();
        assert_eq!(
            OwnerDynamics::decode_list(&mut BitReader::with_profile(&wire, profile)),
            Err(expected)
        );
    }
}
