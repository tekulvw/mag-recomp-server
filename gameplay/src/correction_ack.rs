//! Owner field correction acknowledgements. Native wire8 / retail wire9.
//! This ACK is independent of ordinary replication delivery acknowledgements.
use crate::wire_profile::WireProfile;

pub(crate) fn quantized(
    p: crate::replication::CharacterPosition,
) -> Result<crate::replication::CharacterPosition, &'static str> {
    p.validate()?;
    let q = |v: f32| {
        let a = v.abs();
        let i = a as u32;
        let f = ((a - i as f32) * 32.) as u32;
        let out = (i * 32 + f) as f32 / 32.;
        if v < 0. { -out } else { out }
    };
    Ok(crate::replication::CharacterPosition {
        x: q(p.x),
        y: q(p.y),
        z: q(p.z),
    })
}

/// Restrict binding to a completely decoded owner Character packet. Unknown or
/// mixed object payloads are never guessed from bit offsets.
pub(crate) fn owner_position_packet(
    wire: &[u8],
    owner: u16,
    profile: WireProfile,
) -> Option<(u32, crate::replication::CharacterPosition)> {
    let update = crate::replication::ObjectUpdate::decode_with_profile(wire, profile).ok()?;
    if update.header.object_type_mask != 1 << 3
        || update.header.client_index != 0
        || update.header.nak_sequence != 0
    {
        return None;
    }
    let fields = crate::player_tracking::ClientCharacterFields::decode_for_owner_with_profile(
        &update, owner, profile,
    )
    .ok()?;
    Some((update.header.sequence, fields.position?))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub server_sequence: u32,
    pub client_sequence: u32,
    pub object_type: u8,
    pub object_index: u16,
    pub field_index: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorrectionAck {
    pub items: Vec<Item>,
}
impl CorrectionAck {
    pub fn decode(bytes: &[u8], profile: WireProfile) -> Result<Self, &'static str> {
        if bytes.len() < 3
            || u16::from_be_bytes([bytes[0], bytes[1]]) != profile.wire_message_type(8)
        {
            return Err("invalid_correction_ack_header");
        }
        let count = usize::from(bytes[2]);
        if count > 7 || bytes.len() != 3 + 12 * count {
            return Err("invalid_correction_ack_length");
        }
        let items = bytes[3..]
            .as_chunks::<12>()
            .0
            .iter()
            .map(|b| Item {
                server_sequence: u32::from_be_bytes(b[0..4].try_into().unwrap()),
                client_sequence: u32::from_be_bytes(b[4..8].try_into().unwrap()),
                object_type: b[8],
                object_index: u16::from_be_bytes([b[9], b[10]]),
                field_index: b[11],
            })
            .collect();
        Ok(Self { items })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_native_item_order_decodes_without_padding() {
        let wire = [0, 8, 1, 0, 0, 0, 17, 0, 0, 0, 29, 3, 0, 7, 5];
        assert_eq!(
            CorrectionAck::decode(&wire, WireProfile::prototype())
                .unwrap()
                .items,
            vec![Item {
                server_sequence: 17,
                client_sequence: 29,
                object_type: 3,
                object_index: 7,
                field_index: 5
            }]
        );
    }
    #[test]
    fn full_payload_length_and_seven_item_bound_are_required() {
        for wire in [
            &[0, 8][..],
            &[0, 8, 8][..],
            &[0, 8, 1][..],
            &[0, 8, 0, 0][..],
        ] {
            assert!(CorrectionAck::decode(wire, WireProfile::prototype()).is_err());
        }
        assert_eq!(
            CorrectionAck::decode(&[0, 8, 0], WireProfile::prototype())
                .unwrap()
                .items
                .len(),
            0
        );
    }
    #[test]
    fn forced_position_locks_owner_position_acceptance() {
        let mut tracking = crate::player_tracking::PlayerTracking::default();
        tracking.server_position(
            crate::replication::CharacterPosition {
                x: 10.,
                y: 1.,
                z: 20.,
            },
            crate::player_tracking::PositionOrigin::ServerCorrection,
            4,
        );
        assert!(
            !tracking.accepts(100),
            "old owner reports must not undo a push"
        );
    }
    fn p(x: f32) -> crate::replication::CharacterPosition {
        crate::replication::CharacterPosition { x, y: 1., z: 20. }
    }
    #[test]
    fn real_owner_report_is_queued_without_moving_forced_character() {
        let mut engine = crate::human_kill_reward_tests::fixture();
        let active = engine.active.as_mut().unwrap();
        active.tracking.server_position(
            p(10.),
            crate::player_tracking::PositionOrigin::ServerCorrection,
            1,
        );
        let player = active.reservation.player;
        let fields = crate::player_tracking::ClientCharacterFields {
            position: Some(p(9.)),
            ..Default::default()
        };
        let packet = crate::replication::multiplayer::CharacterOwnerDelta {
            sequence: 10,
            timestamp_ms: 10,
            player,
            fields: &fields,
        }
        .encode_with_profile(engine.wire_profile)
        .unwrap();
        crate::spawn_progression::dispatch(&mut engine, &packet, 10).unwrap();
        let tracking = &engine.active.as_ref().unwrap().tracking;
        assert_eq!(tracking.sample.unwrap().position, p(10.));
        assert_eq!(
            tracking
                .pending_correction
                .as_ref()
                .unwrap()
                .queued
                .unwrap()
                .client_sequence,
            Some(10)
        );
    }
    fn report(
        engine: &mut crate::Engine<Vec<u8>>,
        sequence: u32,
        position: crate::replication::CharacterPosition,
    ) {
        let fields = crate::player_tracking::ClientCharacterFields {
            position: Some(position),
            ..Default::default()
        };
        let wire = crate::replication::multiplayer::CharacterOwnerDelta {
            sequence,
            timestamp_ms: sequence,
            player: engine.active.as_ref().unwrap().reservation.player,
            fields: &fields,
        }
        .encode_with_profile(engine.wire_profile)
        .unwrap();
        crate::spawn_progression::dispatch(engine, &wire, u64::from(sequence)).unwrap();
    }
    fn ack(engine: &mut crate::Engine<Vec<u8>>, server: u32, client: u32, index: u16, field: u8) {
        let mut wire = engine
            .wire_profile
            .wire_message_type(8)
            .to_be_bytes()
            .to_vec();
        wire.push(1);
        wire.extend(server.to_be_bytes());
        wire.extend(client.to_be_bytes());
        wire.push(3);
        wire.extend(index.to_be_bytes());
        wire.push(field);
        crate::spawn_progression::dispatch(engine, &wire, 50).unwrap();
    }
    fn locked() -> crate::Engine<Vec<u8>> {
        let mut engine = crate::human_kill_reward_tests::fixture();
        let active = engine.active.as_mut().unwrap();
        active.tracking.server_position(
            p(10.),
            crate::player_tracking::PositionOrigin::ServerCorrection,
            1,
        );
        assert!(active.tracking.bind_correction(77, p(10.)));
        engine
    }
    #[test]
    fn stale_wrong_owner_and_wrong_field_acks_cannot_unlock_then_matching_ack_sets_floor() {
        let mut engine = locked();
        let player = engine.active.as_ref().unwrap().reservation.player;
        report(&mut engine, 10, p(9.));
        for (server, index, field) in [(76, player, 5), (77, player + 1, 5), (77, player, 4)] {
            ack(&mut engine, server, 11, index, field);
            assert!(
                engine
                    .active
                    .as_ref()
                    .unwrap()
                    .tracking
                    .correction_pending()
            );
            assert_eq!(
                engine
                    .active
                    .as_ref()
                    .unwrap()
                    .tracking
                    .sample
                    .unwrap()
                    .position,
                p(10.)
            );
        }
        ack(&mut engine, 77, 11, player, 5);
        assert!(
            !engine
                .active
                .as_ref()
                .unwrap()
                .tracking
                .correction_pending()
        );
        assert_eq!(
            engine
                .active
                .as_ref()
                .unwrap()
                .tracking
                .accepted_position_sequence,
            10
        );
        report(&mut engine, 10, p(8.));
        assert_eq!(
            engine
                .active
                .as_ref()
                .unwrap()
                .tracking
                .sample
                .unwrap()
                .position,
            p(10.)
        );
        report(&mut engine, 11, p(10.25));
        assert_eq!(
            engine
                .active
                .as_ref()
                .unwrap()
                .tracking
                .sample
                .unwrap()
                .position,
            p(10.25)
        );
    }
    #[test]
    fn matching_ack_copies_highest_queued_report_only_when_at_or_after_client_sequence() {
        let mut engine = locked();
        let player = engine.active.as_ref().unwrap().reservation.player;
        report(&mut engine, 10, p(9.));
        report(&mut engine, 12, p(10.5));
        report(&mut engine, 11, p(8.));
        assert_eq!(
            engine
                .active
                .as_ref()
                .unwrap()
                .tracking
                .sample
                .unwrap()
                .position,
            p(10.)
        );
        ack(&mut engine, 77, 11, player, 5);
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(actor.tracking.sample.unwrap().position, p(10.5));
        assert_eq!(actor.tracking.accepted_position_sequence, 12);
        assert_eq!(actor.owner_fields.position, Some(p(10.5)));
        assert!(
            !engine
                .peers
                .values()
                .next()
                .unwrap()
                .pending_shared
                .is_empty()
        );
    }
    #[test]
    fn zero_client_sequence_drops_pending_data_without_permanent_freeze() {
        let mut engine = locked();
        let player = engine.active.as_ref().unwrap().reservation.player;
        report(&mut engine, 12, p(9.));
        ack(&mut engine, 77, 0, player, 5);
        assert_eq!(
            engine
                .active
                .as_ref()
                .unwrap()
                .tracking
                .sample
                .unwrap()
                .position,
            p(10.)
        );
        assert_eq!(
            engine
                .active
                .as_ref()
                .unwrap()
                .tracking
                .accepted_position_sequence,
            12
        );
        report(&mut engine, 13, p(10.5));
        assert_eq!(
            engine
                .active
                .as_ref()
                .unwrap()
                .tracking
                .sample
                .unwrap()
                .position,
            p(10.5)
        );
    }
    #[test]
    fn malformed_ack_count_cannot_mutate_a_pending_correction() {
        let mut engine = locked();
        let mut bytes = engine
            .wire_profile
            .wire_message_type(8)
            .to_be_bytes()
            .to_vec();
        bytes.push(8);
        crate::spawn_progression::dispatch(&mut engine, &bytes, 50).unwrap();
        let tracking = &engine.active.as_ref().unwrap().tracking;
        assert_eq!(
            tracking
                .pending_correction
                .as_ref()
                .unwrap()
                .server_sequence,
            Some(77)
        );
        assert_eq!(tracking.sample.unwrap().position, p(10.));
    }
    #[test]
    fn final_recipient_sequence_binds_only_the_matching_quantized_owner_packet() {
        let mut engine = crate::human_kill_reward_tests::fixture();
        let profile = engine.wire_profile;
        let active = engine.active.as_mut().unwrap();
        active.outbound_sequence = 70;
        let expected = p(10.05);
        active.tracking.server_position(
            expected,
            crate::player_tracking::PositionOrigin::ServerCorrection,
            1,
        );
        let fields = crate::player_tracking::ClientCharacterFields {
            position: Some(p(9.)),
            ..Default::default()
        };
        let wire = crate::replication::multiplayer::CharacterOwnerDelta {
            sequence: 2,
            timestamp_ms: 1,
            player: active.reservation.player,
            fields: &fields,
        }
        .encode_with_profile(profile)
        .unwrap();
        active.queue_shared(wire);
        let fields = crate::player_tracking::ClientCharacterFields {
            position: Some(expected),
            ..Default::default()
        };
        let wire = crate::replication::multiplayer::CharacterOwnerDelta {
            sequence: 2,
            timestamp_ms: 1,
            player: active.reservation.player,
            fields: &fields,
        }
        .encode_with_profile(profile)
        .unwrap();
        active.queue_shared(wire);
        engine.drain_shared(Vec::new(), 1).unwrap();
        assert_eq!(
            engine
                .active
                .as_ref()
                .unwrap()
                .tracking
                .pending_correction
                .as_ref()
                .unwrap()
                .server_sequence,
            Some(72)
        );
    }
    #[test]
    fn superseding_push_requires_its_new_server_sequence() {
        let mut engine = locked();
        let player = engine.active.as_ref().unwrap().reservation.player;
        let tracking = &mut engine.active.as_mut().unwrap().tracking;
        tracking.server_position(
            p(10.25),
            crate::player_tracking::PositionOrigin::ServerCorrection,
            2,
        );
        assert!(tracking.bind_correction(78, p(10.25)));
        ack(&mut engine, 77, 11, player, 5);
        assert!(
            engine
                .active
                .as_ref()
                .unwrap()
                .tracking
                .correction_pending()
        );
        ack(&mut engine, 78, 11, player, 5);
        assert!(
            !engine
                .active
                .as_ref()
                .unwrap()
                .tracking
                .correction_pending()
        );
    }
    #[test]
    fn record_failure_does_not_release_a_matched_correction() {
        let mut engine = locked();
        let player = engine.active.as_ref().unwrap().reservation.player;
        report(&mut engine, 12, p(10.5));
        engine.max_log = engine.written;
        let mut wire = engine
            .wire_profile
            .wire_message_type(8)
            .to_be_bytes()
            .to_vec();
        wire.push(1);
        wire.extend(77u32.to_be_bytes());
        wire.extend(11u32.to_be_bytes());
        wire.push(3);
        wire.extend(player.to_be_bytes());
        wire.push(5);
        assert!(matches!(
            crate::spawn_progression::dispatch(&mut engine, &wire, 50),
            Err("log_limit_reached")
        ));
        let tracking = &engine.active.as_ref().unwrap().tracking;
        assert!(tracking.correction_pending());
        assert_eq!(tracking.sample.unwrap().position, p(10.));
        assert_eq!(
            tracking
                .pending_correction
                .as_ref()
                .unwrap()
                .queued
                .unwrap()
                .client_sequence,
            Some(12)
        );
    }
    #[test]
    fn locked_landing_report_cannot_apply_fall_damage() {
        let mut engine = locked();
        let rules = crate::fall_damage::Rules {
            schema_version: 1,
            source_build: "retail".into(),
            application_id: 20231,
            min_height: 1.,
            damage_factor: 2.,
            minimum_damage_exclusive: 1.,
            jump_fall_state: 11,
            damage_type: 5,
            damage_source_type: 1,
        };
        let active = engine.active.as_mut().unwrap();
        active.fall_tracker.reset(4, 0, 20.).unwrap();
        active
            .fall_tracker
            .observe(&rules, 4, 1, Some(11), Some(20.))
            .unwrap();
        let fields = crate::player_tracking::ClientCharacterFields {
            position: Some(p(9.)),
            state: Some(crate::replication::CharacterState {
                state: 1,
                stance: 0,
                user_data: [0; 3],
            }),
            ..Default::default()
        };
        let wire = crate::replication::multiplayer::CharacterOwnerDelta {
            sequence: 10,
            timestamp_ms: 10,
            player: active.reservation.player,
            fields: &fields,
        }
        .encode_with_profile(engine.wire_profile)
        .unwrap();
        engine.fall_damage_rules = Some(rules);
        crate::spawn_progression::dispatch(&mut engine, &wire, 10).unwrap();
        assert!(matches!(
            engine.active.as_ref().unwrap().life,
            crate::CharacterLife::Alive {
                health_quarter_points: 120,
                ..
            }
        ));
        assert!(engine.active.as_ref().unwrap().grenade_damage.is_empty());
    }
}
