use mag_gameplay::{
    active_abilities::{Ability, CommandRole, Rules, Side},
    active_ability_state::{
        NativeAbilityData, NativeAbilityScope, NativeAbilityUpdate, NativeActivationOutcome,
        NativeCooldowns, NativeDetectionUpdate, native_ability_mask, native_command_availability,
        native_shadow_modifier_percent,
    },
    wire_profile::WireProfileCatalog,
};

fn rules() -> Rules {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "application_id": 20231,
        "rules_sha256": "",
        "source_archive_sha256": "",
        "global_source_member": "",
        "global_source_sha256": "",
        "command_global_cooldown_ms": 20000,
        "strategic_global_cooldown_ms": 20000,
        "authority": { "effects": false, "target_validation": false, "ability_replication": false },
        "modes": [
            {"players":64,"side":"attacker","squad":[],"platoon":[],"strategic":[]},
            {"players":128,"side":"attacker","squad":["ClusterBomb","Sensor"],"platoon":[],"strategic":[]},
            {"players":128,"side":"defender","squad":["GuidedArtillery"],"platoon":[],"strategic":[]},
            {"players":256,"side":"defender","squad":["ClusterBomb","GuidedArtillery"],"platoon":[],"strategic":[]}
        ],
        "commands": [
            {"ability":"ClusterBomb","source_member":"","source_sha256":"","cooldown_ms":300000,"delay_ms":9000},
            {"ability":"Sensor","source_member":"","source_sha256":"","cooldown_ms":180000,"delay_ms":11000}
        ],
        "strategic": []
    })).unwrap()
}

#[test]
fn native_availability_uses_the_switch_and_authored_side_columns() {
    let rules = rules();
    let defender_squad =
        |mode| native_command_availability(&rules, mode, Side::Defender, CommandRole::SquadLeader);
    for mode in [1, 3] {
        assert_eq!(
            defender_squad(mode).unwrap(),
            &[Ability::ClusterBomb, Ability::Sensor]
        );
    }
    assert_eq!(
        defender_squad(2).unwrap(),
        &[Ability::ClusterBomb, Ability::GuidedArtillery]
    );
    assert!(defender_squad(4).unwrap().is_empty());
    assert!(defender_squad(u16::MAX).unwrap().is_empty());
    assert_eq!(
        native_command_availability(&rules, 0, Side::Attacker, CommandRole::SquadLeader),
        Some([].as_slice())
    );
}

#[test]
fn cooldowns_share_platoon_global_and_preserve_partial_activation() {
    use Ability::{ClusterBomb, Sensor};
    use NativeAbilityScope::{Platoon, Squad};
    let mut state = NativeCooldowns::new(&rules()).unwrap();
    assert_eq!(
        state.remaining_ms(Squad(16), ClusterBomb, 0, 0.).unwrap(),
        20_000
    );
    assert_eq!(
        state
            .remaining_ms(Squad(16), ClusterBomb, 19_999, 0.)
            .unwrap(),
        1
    );
    assert_eq!(
        state
            .remaining_ms(Squad(16), ClusterBomb, 20_000, 0.)
            .unwrap(),
        0
    );
    state
        .record(
            Squad(16),
            ClusterBomb,
            22_000,
            NativeActivationOutcome::FactoryFailed,
        )
        .unwrap();
    assert_eq!(
        state
            .remaining_ms(Squad(16), ClusterBomb, 22_000, -0.05)
            .unwrap(),
        285_000
    );
    assert_eq!(
        state.remaining_ms(Squad(16), Sensor, 22_000, 0.).unwrap(),
        0
    );
    assert_eq!(
        state
            .remaining_ms(Squad(16), ClusterBomb, 22_000, -2.)
            .unwrap(),
        u32::MAX
    );
    state
        .record(
            Squad(16),
            Sensor,
            23_000,
            NativeActivationOutcome::AcceptedFailure,
        )
        .unwrap();
    assert_eq!(
        state.remaining_ms(Squad(17), Sensor, 24_000, 0.).unwrap(),
        19_000
    );
    assert_eq!(
        state.remaining_ms(Platoon(4), Sensor, 24_000, 0.).unwrap(),
        19_000
    );
    assert_eq!(
        state.remaining_ms(Squad(20), Sensor, 24_000, 0.).unwrap(),
        0
    );
    assert_eq!(
        state.remaining_ms(Squad(16), Sensor, 24_000, 0.).unwrap(),
        179_000
    );
    assert!(state.remaining_ms(Squad(32), Sensor, 24_000, 0.).is_err());
    assert!(
        state
            .remaining_ms(Squad(16), Sensor, 24_000, f32::NAN)
            .is_err()
    );
}

struct Bits<'a>(&'a [u8], usize);

impl Bits<'_> {
    fn read(&mut self, n: usize) -> u32 {
        let mut value = 0;
        for bit in self.1..self.1 + n {
            value = value * 2 + u32::from((self.0[bit / 8] >> (7 - bit % 8)) & 1);
        }
        self.1 += n;
        value
    }
}

#[test]
fn ability_payload_is_177_unaligned_bits_on_both_native_scopes() {
    let profile =
        WireProfileCatalog::from_json(include_str!("../data/synthetic-wire-profiles.json"))
            .unwrap()
            .profile(20231)
            .unwrap();
    let data = NativeAbilityData {
        position: [-1.0, 2.5, 4.0],
        squad_index: 16,
        activate_time_ms: 0x12345678,
        enabled_at_start: false,
        target_id: 0x87654321,
    };
    for (scope, count, data_field, active_mask_field) in [
        (NativeAbilityScope::Squad(16), 26, 18, 22),
        (NativeAbilityScope::Platoon(4), 13, 4, 10),
    ] {
        let mut update = NativeAbilityUpdate::new(scope);
        update.data[2] = Some(data);
        update.active_mask = Some(4);
        let bytes = update.encode(7, 500, profile).unwrap();
        let mut bits = Bits(&bytes, profile.header_bits());
        assert_eq!(bits.read(1), 1);
        assert_eq!(bits.read(16), if count == 26 { 16 } else { 4 });
        for field in 0..count {
            assert_eq!(
                bits.read(1),
                u32::from(field == data_field || field == active_mask_field)
            );
            if field == data_field {
                let start = bits.1;
                for v in data.position {
                    assert_eq!(bits.read(32), v.to_bits());
                }
                assert_eq!(bits.read(16), 16);
                assert_eq!(bits.read(32), 0x12345678);
                assert_eq!(bits.read(1), 0);
                assert_eq!(bits.read(32), 0x87654321);
                assert_eq!(bits.1 - start, 177);
            } else if field == active_mask_field {
                assert_eq!(bits.read(16), 4);
            }
        }
        assert_eq!(bits.read(1), 0);
        assert_eq!(bits.1.div_ceil(8), bytes.len());
    }
}

#[test]
fn detection_codec_places_only_the_requested_character_booleans() {
    let profile =
        WireProfileCatalog::from_json(include_str!("../data/synthetic-wire-profiles.json"))
            .unwrap()
            .profile(20231)
            .unwrap();
    let bytes = NativeDetectionUpdate {
        character_index: 4,
        globally_detected: Some(false),
        undetected_via_satellite_jammer: Some(true),
    }
    .encode(7, 500, profile)
    .unwrap();
    let mut bits = Bits(&bytes, profile.header_bits());
    assert_eq!(bits.read(1), 1);
    assert_eq!(bits.read(16), 4);
    for field in 0..22 {
        assert_eq!(bits.read(1), u32::from(field == 10 || field == 21));
        if field == 10 || field == 21 {
            assert_eq!(bits.read(1), u32::from(field == 21));
        }
    }
    assert_eq!(bits.read(1), 0);
    assert_eq!(bits.1.div_ceil(8), bytes.len());
}

#[test]
fn published_masks_and_shadow_use_native_integer_fields() {
    assert_eq!(native_ability_mask(Ability::ClusterBomb), Ok(1));
    assert_eq!(native_ability_mask(Ability::Sensor), Ok(4));
    assert_eq!(native_ability_mask(Ability::GuidedArtillery), Ok(32));
    assert!(native_ability_mask(Ability::None).is_err());
    assert_eq!(native_shadow_modifier_percent(-0.05), Ok(-5));
    assert_eq!(native_shadow_modifier_percent(0.059), Ok(5));
    assert_eq!(native_shadow_modifier_percent(1.30), Ok(-126));

    let profile =
        WireProfileCatalog::from_json(include_str!("../data/synthetic-wire-profiles.json"))
            .unwrap()
            .profile(20231)
            .unwrap();
    let mut update = NativeAbilityUpdate::new(NativeAbilityScope::Platoon(4));
    update.active = Some(true);
    update.owner_index = Some(0x105);
    update.leader_index = Some(u16::MAX);
    update.enabled_mask = Some(u16::MAX);
    update.available_mask = Some(5);
    update.shadow_modifier_percent = Some(-5);
    let bytes = update.encode(2, 500, profile).unwrap();
    let mut bits = Bits(&bytes, profile.header_bits());
    assert_eq!(bits.read(1), 1);
    assert_eq!(bits.read(16), 4);
    for field in 0..13 {
        match field {
            0 => {
                assert_eq!(bits.read(2), 3);
            }
            1 | 8 | 9 | 11 => {
                assert_eq!(bits.read(1), 1);
                assert_eq!(
                    bits.read(16),
                    match field {
                        1 => 0x105,
                        11 => 5,
                        _ => 0xffff,
                    }
                );
            }
            12 => {
                assert_eq!(bits.read(1), 1);
                assert_eq!(bits.read(8), 0xfb);
            }
            _ => {
                assert_eq!(bits.read(1), 0);
            }
        }
    }
    assert_eq!(bits.read(1), 0);
}

#[test]
fn detection_batch_encodes_consecutive_list_entries_under_one_header() {
    let profile =
        WireProfileCatalog::from_json(include_str!("../data/synthetic-wire-profiles.json"))
            .unwrap()
            .profile(20231)
            .unwrap();
    let first = NativeDetectionUpdate {
        character_index: 4,
        globally_detected: Some(true),
        undetected_via_satellite_jammer: None,
    };
    let second = NativeDetectionUpdate {
        character_index: 22,
        globally_detected: None,
        undetected_via_satellite_jammer: Some(false),
    };
    let bytes = NativeDetectionUpdate::encode_batch(&[first, second], 7, 500, profile).unwrap();
    let mut bits = Bits(&bytes, profile.header_bits());
    for (index, field, value) in [(4, 10, 1), (22, 21, 0)] {
        assert_eq!(bits.read(1), 1);
        assert_eq!(bits.read(16), index);
        for slot in 0..22 {
            assert_eq!(bits.read(1), u32::from(slot == field));
            if slot == field {
                assert_eq!(bits.read(1), value);
            }
        }
    }
    assert_eq!(bits.read(1), 0);
    assert_eq!(bits.1.div_ceil(8), bytes.len());
    assert!(NativeDetectionUpdate::encode_batch(&[], 7, 500, profile).is_err());
    assert!(NativeDetectionUpdate::encode_batch(&[first; 49], 7, 500, profile).is_err());
}

#[test]
fn ability_batches_share_a_single_header_and_reject_mixed_scope_or_overflow() {
    let profile =
        WireProfileCatalog::from_json(include_str!("../data/synthetic-wire-profiles.json"))
            .unwrap()
            .profile(20231)
            .unwrap();
    for (first, second, field_count, mask_field) in [
        (
            NativeAbilityScope::Squad(2),
            NativeAbilityScope::Squad(3),
            26,
            22,
        ),
        (
            NativeAbilityScope::Platoon(2),
            NativeAbilityScope::Platoon(3),
            13,
            10,
        ),
    ] {
        let first = NativeAbilityUpdate::new(first);
        let mut second = NativeAbilityUpdate::new(second);
        second.active_mask = Some(5);
        let bytes = NativeAbilityUpdate::encode_batch(&[first, second], 5, 10, profile).unwrap();
        let mut bits = Bits(&bytes, profile.header_bits());
        for index in 2..4 {
            assert_eq!(bits.read(1), 1);
            assert_eq!(bits.read(16), index);
            for field in 0..field_count {
                let present = index == 3 && field == mask_field;
                assert_eq!(bits.read(1), u32::from(present));
                if present {
                    assert_eq!(bits.read(16), 5);
                }
            }
        }
        assert_eq!(bits.read(1), 0);
        assert_eq!(bits.1.div_ceil(8), bytes.len());
    }
    let squad = NativeAbilityUpdate::new(NativeAbilityScope::Squad(2));
    let platoon = NativeAbilityUpdate::new(NativeAbilityScope::Platoon(2));
    assert!(NativeAbilityUpdate::encode_batch(&[squad.clone(), platoon], 5, 10, profile).is_err());
    assert!(NativeAbilityUpdate::encode_batch(&vec![squad; 9], 5, 10, profile).is_err());
}
