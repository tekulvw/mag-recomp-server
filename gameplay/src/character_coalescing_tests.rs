use super::*;
use crate::player_tracking::ClientCharacterFields;
use crate::replication::multiplayer::CharacterOwnerDelta;
use crate::replication::{CharacterDamage, CharacterPosition, CharacterState, InventoryItem};

fn retail() -> WireProfile {
    crate::wire_profile::WireProfileCatalog::embedded()
        .profile(20231)
        .unwrap()
}

fn movement(player: u16, timestamp_ms: u32, x: f32) -> Vec<u8> {
    let fields = ClientCharacterFields {
        position: Some(CharacterPosition { x, y: -3.5, z: 40. }),
        yaw: Some(512),
        ..Default::default()
    };
    CharacterOwnerDelta {
        sequence: 2,
        timestamp_ms,
        player,
        fields: &fields,
    }
    .encode_with_profile(retail())
    .unwrap()
}

/// Every field a delta row may carry, including landing damage.
fn full(player: u16, timestamp_ms: u32) -> Vec<u8> {
    let item = InventoryItem {
        inventory_update_id: 3,
        gear_slot: 17,
        ammo_pool: 2,
        ammo_count: 29,
        reloading: true,
    };
    let fields = ClientCharacterFields {
        state: Some(CharacterState {
            state: 7,
            stance: 2,
            user_data: [1, 2, 255],
        }),
        position: Some(CharacterPosition {
            x: 1023.96875,
            y: -0.03125,
            z: 0.,
        }),
        pitch: Some(200),
        yaw: Some(1023),
        primary: Some(item),
        secondary: Some(InventoryItem {
            reloading: false,
            ..item
        }),
        primary_equipped: Some(true),
        ..Default::default()
    };
    CharacterOwnerDelta {
        sequence: 2,
        timestamp_ms,
        player,
        fields: &fields,
    }
    .encode_with_damage(
        retail(),
        Some(CharacterDamage {
            last_location_and_type: 0x50,
            health_quarter_points: 4095,
            conditions: 15,
            last_attacker: 0x0380_0000 | u32::from(player),
            last_source_object: u32::MAX,
            last_user_data: 0xdead_beef,
        }),
    )
    .unwrap()
}

fn bit(bytes: &[u8], position: usize) -> u8 {
    (bytes[position / 8] >> (7 - position % 8)) & 1
}

/// The joined update is one header, then each source row's bits verbatim,
/// then one terminator and zero padding.
fn assert_joined(joined: &[u8], sources: &[Vec<u8>], sequence: u32) {
    let profile = retail();
    let header = ObjectUpdate::decode_with_profile(joined, profile)
        .unwrap()
        .header;
    let first = ObjectUpdate::decode_with_profile(&sources[0], profile)
        .unwrap()
        .header;
    assert_eq!(header, UpdateHeader { sequence, ..first });
    let mut at = profile.header_bits();
    for source in sources {
        let row = CharacterRow::measure(source, profile).unwrap();
        for offset in 0..row.bits {
            assert_eq!(
                bit(joined, at + offset),
                bit(source, row.start + offset),
                "row bit {offset}"
            );
        }
        at += row.bits;
    }
    assert_eq!(bit(joined, at), 0, "list terminator");
    assert_eq!(joined.len(), (at + 1).div_ceil(8), "only padding follows");
    for padding in at + 1..joined.len() * 8 {
        assert_eq!(bit(joined, padding), 0);
    }
}

#[test]
fn measures_single_character_rows_exactly() {
    let profile = retail();
    for wire in [movement(40, 900, 12.5), full(255, 900)] {
        let row = CharacterRow::measure(&wire, profile).expect("measured row");
        assert_eq!(row.start, profile.header_bits());
        assert_eq!(row.timestamp_ms, 900);
        assert_eq!(CharacterRow::update_bytes(row.bits(), profile), wire.len());
    }
    let row = CharacterRow::measure(&movement(40, 900, 12.5), profile).unwrap();
    assert_eq!(row.object, 40);
    assert_eq!(row.bits(), 1 + 16 + 22 + 49 + 10);
    let row = CharacterRow::measure(&full(255, 900), profile).unwrap();
    assert_eq!(row.object, 255);
    assert_eq!(
        row.bits(),
        1 + 16 + 22 + 32 + 49 + 8 + 10 + 120 + 24 + 24 + 1
    );
}

#[test]
fn leaves_other_shapes_unmeasured() {
    let profile = retail();
    let wire = movement(40, 900, 12.5);
    // A field the delta row never carries (field 18, the level row).
    let level = crate::replication::CharacterLevelUpdate {
        sequence: 2,
        timestamp_ms: 900,
        object_index: 40,
        mag_info: crate::replication::MagCharacterInfo {
            nominations: 0,
            level: 5,
            leadership_level: 0,
            skill_specialty: 0,
            head_id: 0,
            voice_id: 0,
        },
    }
    .encode_with_profile(profile)
    .unwrap();
    assert_eq!(CharacterRow::measure(&level, profile), None);
    // Two Characters already, or trailing bits after the terminator.
    let two = join(
        &[&wire, &movement(41, 900, 1.)],
        &[
            CharacterRow::measure(&wire, profile).unwrap(),
            CharacterRow::measure(&movement(41, 900, 1.), profile).unwrap(),
        ],
        2,
        profile,
    )
    .unwrap();
    assert_eq!(CharacterRow::measure(&two, profile), None);
    let mut padded = wire.clone();
    padded.push(0);
    assert_eq!(CharacterRow::measure(&padded, profile), None);
    let mut dirty = wire.clone();
    *dirty.last_mut().unwrap() |= 1;
    assert_eq!(CharacterRow::measure(&dirty, profile), None);
    // Client-reported (NAK or client index) and empty updates.
    let mut nak = wire.clone();
    nak[9] = 1; // low byte of the NAK sequence (bits 48..80)
    assert_eq!(CharacterRow::measure(&nak, profile), None);
    let empty = ObjectUpdate {
        header: UpdateHeader {
            sequence: 2,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: 900,
        },
        wire: &[],
    }
    .encode_empty_with_profile(profile)
    .unwrap();
    assert_eq!(CharacterRow::measure(&empty, profile), None);
}

#[test]
fn joins_rows_verbatim_under_one_header() {
    let profile = retail();
    let sources: Vec<_> = (0..40u16)
        .map(|i| {
            if i % 7 == 3 {
                full(32 + i, 4_250)
            } else {
                movement(32 + i, 4_250, f32::from(i) * 3.25)
            }
        })
        .collect();
    let rows: Vec<_> = sources
        .iter()
        .map(|wire| CharacterRow::measure(wire, profile).unwrap())
        .collect();
    let wires: Vec<&[u8]> = sources.iter().map(Vec::as_slice).collect();
    let joined = join(&wires, &rows, 77, profile).unwrap();
    assert_joined(&joined, &sources, 77);
    let separate: usize = sources.iter().map(Vec::len).sum();
    assert!(
        joined.len() * 2 < separate,
        "{} joined vs {separate} separate bytes",
        joined.len()
    );
}

#[test]
fn join_refuses_mixed_ticks_repeats_and_oversize() {
    let profile = retail();
    let a = movement(40, 900, 1.);
    let b = movement(41, 950, 1.);
    let measure = |wire: &[u8]| CharacterRow::measure(wire, profile).unwrap();
    assert!(join(&[&a, &b], &[measure(&a), measure(&b)], 2, profile).is_err());
    assert!(join(&[&a, &a], &[measure(&a), measure(&a)], 2, profile).is_err());
    assert!(join(&[&a], &[measure(&a)], 0, profile).is_err());
    let many: Vec<_> = (0..256u16).map(|i| full(i, 900)).collect();
    let rows: Vec<_> = many.iter().map(|wire| measure(wire)).collect();
    let wires: Vec<&[u8]> = many.iter().map(Vec::as_slice).collect();
    assert_eq!(
        join(&wires, &rows, 2, profile),
        Err("replication_packet_too_long")
    );
    // The largest prefix that fits is exactly the byte limit's worth.
    let fit = (1..=rows.len())
        .take_while(|&n| {
            CharacterRow::update_bytes(rows[..n].iter().map(|r| r.bits()).sum(), profile)
                <= MAX_UPDATE_BYTES
        })
        .last()
        .unwrap();
    let joined = join(&wires[..fit], &rows[..fit], 2, profile).unwrap();
    assert!(joined.len() <= MAX_UPDATE_BYTES);
    assert_joined(&joined, &many[..fit], 2);
}
