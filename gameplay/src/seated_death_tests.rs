//! SYNTHETIC seated-death checks; see docs/seated-death-dismount.md.
use crate::replication::{BitReader, CharacterPosition, CharacterState, ObjectUpdate};
use crate::*;

/// The fields of one terminal Character update, as a client would read them.
#[derive(Debug, Default)]
pub(crate) struct Terminal {
    pub character: u16,
    pub state: Option<CharacterState>,
    pub position: Option<CharacterPosition>,
    pub yaw: Option<u16>,
    pub health: Option<u16>,
    pub ground_owner: Option<(u16, u16)>,
    pub bunker: Option<(u16, u16, u16)>,
}

/// Decode a Character damage update that may carry a seat exit, a Ground
/// owner record and a family13 ownership record. None for anything else.
pub(crate) fn decode(wire: &[u8], profile: wire_profile::WireProfile) -> Option<Terminal> {
    let update = ObjectUpdate::decode_with_profile(wire, profile).ok()?;
    let mask = update.header.object_type_mask;
    if mask & (1 << 3) == 0 || mask & !((1 << 1) | (1 << 3) | (1 << 13)) != 0 {
        return None;
    }
    let mut bits = BitReader::with_profile(wire, profile);
    bits.position = profile.header_bits();
    let mut out = Terminal::default();
    if mask & (1 << 1) != 0 {
        if bits.read(1).ok()? != 1 {
            return None;
        }
        let index = bits.read(16).ok()? as u16;
        if bits.read(2).ok()? != 1 {
            return None;
        }
        out.ground_owner = Some((index, bits.read(16).ok()? as u16));
        if bits.read(29).ok()? != 0 {
            return None;
        }
    }
    if bits.read(1).ok()? != 1 {
        return None;
    }
    out.character = bits.read(16).ok()? as u16;
    for field in 0..22 {
        if bits.read(1).ok()? == 0 {
            continue;
        }
        match field {
            4 => {
                let packed = bits.read(8).ok()? as u8;
                out.state = Some(CharacterState {
                    state: packed & 63,
                    stance: packed >> 6,
                    user_data: [
                        bits.read(8).ok()? as u8,
                        bits.read(8).ok()? as u8,
                        bits.read(8).ok()? as u8,
                    ],
                });
            }
            5 => {
                let x = bits.read(16).ok()? as f32 / 32.0;
                let negative = bits.read(1).ok()? != 0;
                let y = bits.read(16).ok()? as f32 / 32.0;
                let z = bits.read(16).ok()? as f32 / 32.0;
                out.position = Some(CharacterPosition {
                    x,
                    y: if negative { -y } else { y },
                    z,
                });
            }
            7 => out.yaw = Some(bits.read(10).ok()? as u16),
            8 => {
                bits.read(8).ok()?;
                out.health = Some((bits.read(16).ok()? & 0xfff) as u16);
                bits.read(32).ok()?;
                bits.read(32).ok()?;
                bits.read(32).ok()?;
            }
            _ => return None,
        }
    }
    if bits.read(1).ok()? != 0 {
        return None;
    }
    if mask & (1 << 13) != 0 {
        if bits.read(1).ok()? != 1 {
            return None;
        }
        let index = bits.read(16).ok()? as u16;
        let (mut owner, mut turret) = (None, None);
        for field in 0..25 {
            if bits.read(1).ok()? == 0 {
                continue;
            }
            match field {
                1 => owner = Some(bits.read(16).ok()? as u16),
                6 => turret = Some(bits.read(16).ok()? as u16),
                _ => return None,
            }
        }
        if bits.read(1).ok()? != 0 {
            return None;
        }
        out.bunker = Some((index, owner?, turret?));
    }
    Some(out)
}

/// The one terminal update for `player` among the frames or queued packets.
pub(crate) fn terminal_for<'a>(
    wires: impl IntoIterator<Item = &'a [u8]>,
    player: u16,
    profile: wire_profile::WireProfile,
) -> Terminal {
    let found: Vec<_> = wires
        .into_iter()
        .filter_map(|wire| decode(wire, profile))
        .filter(|t| t.character == player && t.state.is_some_and(|s| s.state == 7))
        .collect();
    assert_eq!(found.len(), 1, "one terminal update: {found:?}");
    found.into_iter().next().unwrap()
}

pub(crate) fn frame_wires(frames: &[OutboundMessage]) -> Vec<Vec<u8>> {
    frames
        .iter()
        .map(|frame| protocol::decode_hex(&frame.payload_hex))
        .collect()
}

/// A terminal update that left a seat: state 7, the exit pose and HP0.
pub(crate) fn assert_seated_exit(terminal: &Terminal) {
    assert_eq!(terminal.state.unwrap().state, 7, "{terminal:?}");
    assert!(terminal.position.is_some(), "exit position: {terminal:?}");
    assert!(terminal.yaw.is_some(), "exit facing: {terminal:?}");
    assert_eq!(terminal.health, Some(0), "{terminal:?}");
}

mod hospital {
    use super::*;
    use crate::spawn_progression::dispatch;
    use crate::structures::domination_bunker_seat_tests::{fixture_for_mode, occupant_fixture};

    fn request(
        engine: &mut Engine<Vec<u8>>,
        kind: u16,
        seat: u32,
        now: u64,
    ) -> Vec<OutboundMessage> {
        let mut wire = kind.to_be_bytes().to_vec();
        if kind != 0x0215 {
            wire.extend(0x0d00_0005u32.to_be_bytes());
            wire.extend(seat.to_be_bytes());
        }
        wire.extend(0x0380_0000u32.to_be_bytes());
        if kind == 0x0217 {
            wire.extend(90u32.to_be_bytes());
        }
        dispatch(engine, &wire, now).unwrap()
    }

    fn seated(game_type: u8) -> Engine<Vec<u8>> {
        let mut engine = fixture_for_mode(game_type);
        engine.item_catalog = human_kill_reward_tests::bot_firearm_items();
        request(&mut engine, 0x0216, 12, 1000);
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(
            (actor.mounted_bunker, actor.mounted_seat),
            (Some(5), Some(12))
        );
        engine
    }

    /// The fixture's one authored exit stands 2 m along +x of the body.
    const EXIT: CharacterPosition = CharacterPosition {
        x: 2.,
        y: 0.,
        z: 0.,
    };

    #[test]
    fn a_seated_hospital_kill_self_leaves_the_seat_in_one_terminal_update() {
        for game_type in [1, 2] {
            let mut engine = seated(game_type);
            let frames = request(&mut engine, 0x0215, 0, 1100);
            let terminal = terminal_for(
                frame_wires(&frames).iter().map(Vec::as_slice),
                0,
                engine.wire_profile,
            );
            assert_seated_exit(&terminal);
            assert_eq!(terminal.position, Some(EXIT));
            // The seat release and the emplacement's new ownership share the packet.
            assert_eq!(terminal.bunker, Some((5, 0x105, u16::MAX)));
            let actor = engine.active.as_ref().unwrap();
            assert!(!actor.life.is_alive());
            assert_eq!((actor.mounted_bunker, actor.mounted_seat), (None, None));
            assert_eq!(actor.tracking.sample.unwrap().position, EXIT);
            assert_eq!(
                actor.terminal_seat,
                Some(seated_death::SeatIdentity::Bunker {
                    index: Some(5),
                    seat: 12
                })
            );
        }
    }

    #[test]
    fn a_dead_occupant_asking_to_dismount_gets_the_terminal_exit_again() {
        let mut engine = seated(1);
        request(&mut engine, 0x0215, 0, 1100);
        let mut sequences = Vec::new();
        for now in [1200, 1300] {
            let frames = request(&mut engine, 0x0217, 12, now);
            let wires = frame_wires(&frames);
            let terminal = terminal_for(wires.iter().map(Vec::as_slice), 0, engine.wire_profile);
            assert_seated_exit(&terminal);
            assert_eq!(terminal.position, Some(EXIT));
            assert_eq!(terminal.bunker, Some((5, 0x105, u16::MAX)));
            sequences.push(
                ObjectUpdate::decode_with_profile(&wires[0], engine.wire_profile)
                    .unwrap()
                    .header
                    .sequence,
            );
        }
        assert!(
            sequences[1] > sequences[0],
            "freshly sequenced: {sequences:?}"
        );
        let record = String::from_utf8_lossy(&engine.writer);
        assert!(!record.contains("bunker_seat_not_owned"));
        assert!(record.contains("\"terminal_exit_replayed\":true"));
        // Another seat was never this life's seat.
        request(&mut engine, 0x0217, 11, 1400);
        assert!(String::from_utf8_lossy(&engine.writer).contains("bunker_seat_not_owned"));
    }

    fn owner_tick(engine: &mut Engine<Vec<u8>>, timestamp_ms: u32) -> Vec<OutboundMessage> {
        let header = replication::UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms,
        };
        let packet = replication::ObjectUpdate { header, wire: &[] }
            .encode_empty_with_profile(engine.wire_profile)
            .unwrap();
        dispatch(engine, &packet, u64::from(timestamp_ms)).unwrap()
    }

    #[test]
    fn the_next_wave_after_a_seated_death_seats_the_player_with_the_full_loadout() {
        let mut engine = seated(1);
        request(&mut engine, 0x0215, 0, 1100);
        // Sticky rejoin: the stored selection respawns at the next wave.
        let actor = engine.active.as_mut().unwrap();
        actor.squad_spawn = 1;
        actor.selected_spawn = Some(protocol::CharacterSpawnSelection {
            spawn_point_index: 1,
        });
        actor.pending_spawn_deadline = Some(Duration::from_millis(1150));
        actor.round_scores = Some([111, 222]);
        for tick in 1200..1210 {
            owner_tick(&mut engine, tick);
            if engine.active.as_ref().unwrap().life.is_alive() {
                break;
            }
        }
        let actor = engine.active.as_ref().unwrap();
        assert!(actor.life.is_alive(), "{:?}", actor.life);
        assert_eq!(
            (actor.mounted_bunker, actor.mounted_seat),
            (Some(5), Some(12))
        );
        assert_eq!(actor.owner_fields.state.unwrap().user_data, [13, 5, 12]);
        assert_eq!(actor.terminal_seat, None);
        assert_eq!(actor.last_damage.health_quarter_points, 400);
        // The whole selected kit is issued again, full magazine included.
        let handle = actor.current_inventory_handle.unwrap();
        let full = actor
            .reservation
            .character
            .replication_loadout(
                &engine.item_catalog,
                handle.inventory_index,
                actor.wire_profile,
            )
            .unwrap();
        assert_eq!(
            format!("{:?}", actor.spawned_loadout),
            format!("{:?}", Some(full))
        );
    }

    #[test]
    fn occupants_killed_with_their_emplacement_leave_its_seats() {
        let (mut engine, mut native) = occupant_fixture();
        let candidate = engine
            .prepare_hospital_occupants(5, u32::MAX, &native, Duration::from_secs(1))
            .unwrap();
        engine
            .record_acquisition_character_effects(candidate, &mut native, 1)
            .unwrap();
        let active = engine.active.as_ref().unwrap();
        let player = active.reservation.player;
        let terminal = terminal_for(
            active.pending_shared.iter().map(Vec::as_slice),
            player,
            engine.wire_profile,
        );
        assert_seated_exit(&terminal);
        assert_eq!(terminal.bunker, Some((5, 0x105, u16::MAX)));
        assert_eq!(active.mounted_seat, None);
        // The other hospital's occupant is untouched.
        let other = engine.peers.values().next().unwrap();
        assert_eq!(other.mounted_bunker, Some(6));
    }
}

#[test]
fn only_a_downed_seated_human_update_carries_an_exit() {
    let mut engine = structures::domination_bunker_seat_tests::fixture();
    let damage = replication::CharacterDamage {
        last_location_and_type: 0,
        health_quarter_points: 0,
        conditions: 0,
        last_attacker: 0,
        last_source_object: 0,
        last_user_data: 0,
    };
    // On foot: the death update is unchanged.
    assert!(
        engine
            .character_damage_update(5, 1, 0, damage, true)
            .seated_exit
            .is_none()
    );
    let actor = engine.active.as_mut().unwrap();
    actor.mounted_bunker = Some(5);
    actor.mounted_seat = Some(3);
    assert!(
        engine
            .character_damage_update(5, 1, 0, damage, true)
            .seated_exit
            .is_some()
    );
    // Non-lethal damage does not move a seated character.
    assert!(
        engine
            .character_damage_update(5, 1, 0, damage, false)
            .seated_exit
            .is_none()
    );
    // No reserved human holds this Character index.
    assert!(
        engine
            .character_damage_update(5, 1, 200, damage, true)
            .seated_exit
            .is_none()
    );
}
