//! Indexed hospital packets must update the same vehicle named by character state.
use super::*;

fn profile() -> crate::wire_profile::WireProfile {
    crate::wire_profile::WireProfileCatalog::embedded()
        .profile(20231)
        .unwrap()
}

#[test]
fn indexed_hospital_mount_keeps_character_and_vehicle_identity_together() {
    for index in [1, 15] {
        let packet = AlphaSeatTransition {
            sequence: 5,
            timestamp_ms: 100,
            action: AlphaSeatAction::Mount {
                seat: 12,
                client_index: 7,
            },
        }
        .encode_for_bunker_with_profile(
            index,
            7,
            Some(AlphaVehicleOwnership {
                owner: 7,
                turret_owner: u16::MAX,
            }),
            profile(),
        )
        .unwrap();
        let mut reader = BitReader::with_profile(&packet, profile());
        reader.position = profile().header_bits();
        assert_eq!(reader.read(1).unwrap(), 1);
        assert_eq!(reader.read(16).unwrap(), 7);
        for field in 0..22 {
            let present = reader.read(1).unwrap();
            assert_eq!(present, u32::from(field == 4));
            if field == 4 {
                assert_eq!(reader.read(8).unwrap(), 14);
                assert_eq!(reader.read(8).unwrap(), 13);
                assert_eq!(reader.read(8).unwrap(), u32::from(index));
                assert_eq!(reader.read(8).unwrap(), 12);
            }
        }
        assert_eq!(reader.read(1).unwrap(), 0);
        assert_eq!(reader.read(1).unwrap(), 1);
        assert_eq!(reader.read(16).unwrap(), u32::from(index));
        for field in 0..25 {
            assert_eq!(reader.read(1).unwrap(), u32::from([1, 6].contains(&field)));
            if field == 1 {
                assert_eq!(reader.read(16).unwrap(), 7);
            }
            if field == 6 {
                assert_eq!(reader.read(16).unwrap(), u32::from(u16::MAX));
            }
        }
        assert_eq!(reader.read(1).unwrap(), 0);
    }
}

#[test]
fn indexed_hospital_respawn_accepts_authored_seats_and_preserves_legacy_bounds() {
    let mut respawn = CharacterRespawn::at_position(
        100,
        CharacterPosition {
            x: 1.,
            y: 2.,
            z: 3.,
        },
    );
    respawn.sequence = 5;
    respawn.object_index = 7;
    respawn.state = CharacterState {
        state: 14,
        stance: 0,
        user_data: [13, 15, 12],
    };
    let ownership = AlphaVehicleOwnership {
        owner: 7,
        turret_owner: u16::MAX,
    };
    let packet = respawn
        .encode_in_bunker_with_profile(None, 7, ownership, profile())
        .unwrap();
    let update = ObjectUpdate::decode_with_profile(&packet, profile()).unwrap();
    assert_eq!(update.header.object_type_mask, (1 << 3) | (1 << 13));
    assert!(
        respawn
            .encode_in_alpha_with_ownership_with_profile(None, 7, ownership, profile())
            .is_err()
    );
    respawn.state.user_data[1] = 16;
    assert!(
        respawn
            .encode_in_bunker_with_profile(None, 7, ownership, profile())
            .is_err()
    );
    respawn.state.user_data = [13, 1, 17];
    assert!(
        respawn
            .encode_in_bunker_with_profile(None, 7, ownership, profile())
            .is_err()
    );
}
