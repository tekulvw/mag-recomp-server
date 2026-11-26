use super::*;
use crate::{
    protocol,
    spawn_progression::{
        dispatch,
        tests::ground_vehicle_runtime::domination_ground_driver::{driver_engine, ready_on_foot},
    },
};

pub(crate) fn opt_in<W: Write>(engine: &mut Engine<W>, index: u16) {
    opt_in_primary(engine, index, 5);
}

pub(crate) fn opt_in_primary<W: Write>(engine: &mut Engine<W>, index: u16, primary: u16) {
    let kind = match engine.match_profile().game_type {
        1 => PolicyKind::Escort,
        _ => PolicyKind::Apc,
    };
    let grounds = engine.ground_vehicles.as_mut().unwrap();
    grounds.set_turret_policies(BTreeMap::from([(
        index,
        Policy {
            kind,
            primary,
            local_position_xyz: [1., 2., 3.],
        },
    )]));
    engine
        .ground_runtime
        .as_mut()
        .unwrap()
        .configure_turrets(grounds);
}

pub(crate) fn equip_synthetic_ground_turret<W: Write>(engine: &mut Engine<W>, index: u16) {
    let actor = engine.active.as_mut().unwrap();
    actor.ground_seat = Some(crate::ground_seats::PassengerSeat {
        vehicle_index: index,
        seat: 1,
    });
    let (turret, definition) = engine
        .ground_turret_seat_change(engine.active.as_ref().unwrap(), index, true)
        .unwrap();
    let (turret, _) = turret
        .report(crate::hospital_turret_wire::Fields {
            primary: Some(InventoryItem {
                inventory_update_id: definition.unwrap().version,
                gear_slot: 0,
                ammo_pool: 0,
                ammo_count: 20,
                reloading: false,
            }),
            ..Default::default()
        })
        .unwrap();
    engine
        .ground_runtime
        .as_mut()
        .unwrap()
        .install_turret(index, turret);
}

fn mount(
    engine: &mut Engine<Vec<u8>>,
    index: u16,
    seat: u8,
    now: u64,
) -> Result<Vec<crate::OutboundMessage>, &'static str> {
    let mut bytes = 0x0216u16.to_be_bytes().to_vec();
    bytes.extend((0x0100_0000 | u32::from(index)).to_be_bytes());
    bytes.extend(u32::from(seat).to_be_bytes());
    bytes.extend(
        (0x0380_0000 | u32::from(engine.active.as_ref().unwrap().reservation.player)).to_be_bytes(),
    );
    dispatch(engine, &bytes, now)
}

#[test]
fn ground_turret_opt_in_enables_seat_one_without_granting_driver() {
    let mut engine = driver_engine(8);
    opt_in(&mut engine, 8);
    ready_on_foot(&mut engine, 0, 10);
    let packets = mount(&mut engine, 8, 1, 20).unwrap();
    assert!(!packets.is_empty());
    assert!(
        packets
            .iter()
            .all(|packet| !protocol::decode_hex(&packet.payload_hex).is_empty())
    );
    let active = engine.active.as_ref().unwrap();
    assert!(engine.is_ground_gunner(active));
    assert_eq!(engine.ground_driver_owner(8), None);
    assert_eq!(
        active.tracking.sample.unwrap().position,
        CharacterPosition {
            x: 101.,
            y: 5.,
            z: 103.
        }
    );
    let snapshot = engine.ground_turret_snapshot(8).unwrap();
    assert_eq!(snapshot.turret_owner, Some(0));
    assert_eq!(snapshot.definition.unwrap().version, 3);
    let packet = engine.current_ground_batch(0, 25, 30).unwrap();
    assert_eq!(
        read_creation_turret(&packet, engine.wire_profile, 8),
        (0x105, 0, 3, 0, 0)
    );
}

fn primary(version: u8) -> InventoryItem {
    InventoryItem {
        inventory_update_id: version,
        gear_slot: 0,
        ammo_pool: 0,
        ammo_count: 20,
        reloading: false,
    }
}

fn report(
    engine: &mut Engine<Vec<u8>>,
    sequence: u32,
    row: TurretUpdate,
    position: Option<CharacterPosition>,
    now: u64,
) -> Result<Vec<crate::OutboundMessage>, &'static str> {
    use crate::{
        DecodedEvent, Direction, Transport,
        events::EventInput,
        replication::{BitWriter, UpdateHeader},
    };
    let mut bits = BitWriter::with_profile(engine.wire_profile);
    UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 1 << 1,
        timestamp_ms: now as u32,
    }
    .encode(&mut bits)?;
    bits.write(1, 1)?;
    bits.write(row.index.into(), 16)?;
    for field in 0..30 {
        if row.write_field(field, &mut bits)? {
            continue;
        }
        bits.write(u32::from(field == 25 && position.is_some()), 1)?;
        if field == 25
            && let Some(position) = position
        {
            position.encode(&mut bits)?;
        }
    }
    bits.write(0, 1)?;
    let bytes = bits.finish();
    let actor = engine.active.as_ref().unwrap();
    let event = EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: actor.reservation.world,
        player_id: actor.reservation.player,
        transport: Transport::Udp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(&bytes),
    };
    let decoded = DecodedEvent::with_vehicle_ownership(
        &bytes,
        event.player_id,
        engine.wire_profile,
        true,
        false,
        true,
    );
    engine.handle_event(event, &decoded, Duration::from_millis(now), 1)
}

fn focus(engine: &mut Engine<Vec<u8>>, player: u16) {
    let previous = engine.active.take().unwrap();
    engine
        .peers
        .insert(previous.reservation.id.clone(), previous);
    engine.active = engine
        .peers
        .remove(&format!("synthetic-reservation-{player:04}"));
}

#[test]
fn ground_turret_and_driver_can_control_the_same_vehicle_independently() {
    use crate::hospital_turret_wire::{Fields, Yaw};
    let mut engine = driver_engine(8);
    opt_in(&mut engine, 8);
    ready_on_foot(&mut engine, 0, 10);
    mount(&mut engine, 8, 0, 20).unwrap();
    ready_on_foot(&mut engine, 1, 25);
    mount(&mut engine, 8, 1, 30).unwrap();

    let row = TurretUpdate {
        index: 8,
        driver_owner: None,
        turret_owner: None,
        definition: None,
        fields: Fields {
            primary: Some(primary(3)),
            yaw: Some(Yaw::new(128).unwrap()),
            ..Fields::default()
        },
    };
    let movement = Some(CharacterPosition {
        x: 101.,
        y: 3.,
        z: 100.,
    });
    // An authorized aim report may share the family row with forbidden movement.
    report(&mut engine, 20, row, movement, 130).unwrap();
    assert_eq!(engine.current_ground_vehicle(8).unwrap().position.x, 100.);
    assert_eq!(
        engine.ground_turret_snapshot(8).unwrap().fields.yaw,
        row.fields.yaw
    );
    assert_eq!(engine.ground_driver_owner(8), Some(0));
    assert_eq!(
        engine.ground_turret_snapshot(8).unwrap().turret_owner,
        Some(1)
    );

    focus(&mut engine, 0);
    let mut driver_row = row;
    driver_row.fields.yaw = Some(Yaw::new(256).unwrap());
    report(&mut engine, 20, driver_row, movement, 150).unwrap();
    assert_eq!(engine.current_ground_vehicle(8).unwrap().position.x, 101.);
    assert_eq!(
        engine.ground_turret_snapshot(8).unwrap().fields.yaw,
        row.fields.yaw
    );

    focus(&mut engine, 1);
    let seat = engine.active.as_ref().unwrap().ground_seat.unwrap();
    assert_eq!(
        engine.ground_seat_position(seat).unwrap(),
        CharacterPosition {
            x: 102.,
            y: 5.,
            z: 103.
        }
    );
    let snapshot = engine.ground_turret_snapshot(8).unwrap();
    assert_eq!(snapshot.definition.unwrap().items, [Some(5), None, None]);
    let wire = engine.current_ground_batch(0, 25, 160).unwrap();
    assert!(wire.len() <= 1397);
    assert_eq!(
        read_creation_turret(&wire, engine.wire_profile, 8),
        (0, 1, 3, 128, 3)
    );
}

fn read_creation_turret(
    packet: &[u8],
    profile: crate::wire_profile::WireProfile,
    target: u32,
) -> (u32, u32, u32, u32, u32) {
    use crate::replication::BitReader;
    let mut bits = BitReader::with_profile(packet, profile);
    bits.position = profile.header_bits();
    while bits.read(1).unwrap() != 0 {
        let index = bits.read(16).unwrap();
        let (mut driver, mut gunner, mut version, mut yaw, mut equipped) = (0, 0, 0, 0, 0);
        for field in 0..30 {
            if bits.read(1).unwrap() == 0 {
                continue;
            }
            match field {
                0 | 27 | 29 => {
                    bits.read(1).unwrap();
                }
                1 => driver = bits.read(16).unwrap(),
                3 => {
                    bits.read(32).unwrap();
                }
                4 | 5 | 19 | 21 => {
                    bits.read(8).unwrap();
                }
                6 => gunner = bits.read(16).unwrap(),
                7 | 20 => {
                    bits.read(16).unwrap();
                }
                8 => yaw = bits.read(11).unwrap(),
                9 => {
                    bits.read(10).unwrap();
                }
                10 => {
                    version = bits.read(8).unwrap();
                    for _ in 0..3 {
                        if bits.read(1).unwrap() != 0 {
                            bits.read(12).unwrap();
                        }
                    }
                }
                11 | 12 => {
                    equipped = bits.read(8).unwrap();
                    bits.read(16).unwrap();
                }
                18 => {
                    for width in [8, 16, 32, 32, 32] {
                        bits.read(width).unwrap();
                    }
                }
                25 => {
                    for width in [16, 17, 16] {
                        bits.read(width).unwrap();
                    }
                }
                26 => {
                    for _ in 0..4 {
                        bits.read(11).unwrap();
                    }
                }
                28 => {
                    bits.read(10).unwrap();
                }
                _ => panic!("unexpected family1 field {field}"),
            };
        }
        if index == target {
            return (driver, gunner, version, yaw, equipped);
        }
    }
    panic!("missing vehicle index {target}");
}

#[test]
fn ground_turret_revokes_on_release_and_keeps_grant_counter_after_vehicle_destruction() {
    use crate::acquisition_ground_runtime::{Capabilities, Destruction, QualifiedRespawn};
    let mut engine = driver_engine(8);
    opt_in(&mut engine, 8);
    ready_on_foot(&mut engine, 0, 10);
    mount(&mut engine, 8, 0, 12).unwrap();
    ready_on_foot(&mut engine, 1, 14);
    equip_synthetic_ground_turret(&mut engine, 8);
    let reply = engine.handle(
        crate::Request::Release {
            v: crate::BRIDGE_VERSION,
            request_id: "synthetic-apc-gunner-release".into(),
            reservation_id: "synthetic-reservation-0001".into(),
        },
        Duration::from_millis(20),
        1,
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(
        engine.ground_turret_snapshot(8).unwrap().turret_owner,
        Some(u16::MAX)
    );
    assert!(!engine.ground_turret(8).unwrap().has_grant());

    assert_eq!(engine.ground_driver_owner(8), Some(0));
    ready_on_foot(&mut engine, 2, 25);
    mount(&mut engine, 8, 1, 30).unwrap();
    assert_eq!(
        engine
            .ground_turret_snapshot(8)
            .unwrap()
            .definition
            .unwrap()
            .version,
        4
    );
    let ground = engine.ground_runtime.as_mut().unwrap();
    let selected = ground.get(8).unwrap();
    let key = selected.key;
    let spawn = QualifiedRespawn {
        spawn_point_index: selected.source_spawn_point_index,
        spawn_point_token: selected.source_spawn_point_token,
        pose: selected.pose,
        active: true,
    };
    ground
        .configure(
            key,
            Capabilities {
                destruction: Some(Destruction {
                    damage: 0.,
                    full_radius: 0.,
                    half_radius: 0.,
                }),
                ..Default::default()
            },
        )
        .unwrap();
    ground.apply_lethal(key, u32::MAX, true).unwrap();
    ground.update(key, 0., 100).unwrap();
    ground.respawn_existing(key, spawn, false).unwrap();
    assert!(!engine.ground_turret(8).unwrap().has_grant());
    assert_eq!(
        engine
            .ground_turret_seat_change(engine.active.as_ref().unwrap(), 8, true)
            .unwrap()
            .1
            .unwrap()
            .version,
        5
    );
}

#[test]
fn ground_turret_record_failure_preserves_actor_and_grant() {
    let mut engine = driver_engine(8);
    opt_in(&mut engine, 8);
    ready_on_foot(&mut engine, 0, 10);
    engine.max_log = engine.written;
    assert!(mount(&mut engine, 8, 1, 20).is_err());
    assert!(engine.active.as_ref().unwrap().ground_seat.is_none());
    assert!(!engine.ground_turret(8).unwrap().has_grant());
    assert_eq!(
        engine
            .ground_turret_snapshot(8)
            .unwrap()
            .definition
            .unwrap()
            .version,
        2
    );
}

#[test]
fn ground_turret_current_pose_exit_and_remount_require_fresh_equipment() {
    use crate::{
        acquisition_ground_runtime::Pose,
        hospital_turret_wire::{Fields, Yaw},
    };
    let mut engine = driver_engine(8);
    opt_in(&mut engine, 8);
    ready_on_foot(&mut engine, 0, 10);
    mount(&mut engine, 8, 1, 20).unwrap();
    let mut row = TurretUpdate {
        index: 8,
        driver_owner: None,
        turret_owner: None,
        definition: None,
        fields: Fields {
            primary: Some(primary(3)),
            yaw: Some(Yaw::new(128).unwrap()),
            ..Default::default()
        },
    };
    report(&mut engine, 10, row, None, 30).unwrap();
    assert!(
        engine
            .ground_turret_snapshot(8)
            .unwrap()
            .fields
            .primary
            .is_some()
    );
    let ground = engine.ground_runtime.as_mut().unwrap();
    let key = ground.get(8).unwrap().key;
    ground
        .force_pose(
            key,
            Pose {
                position: CharacterPosition {
                    x: 104.,
                    y: 3.,
                    z: 100.,
                },
                rotation: [0., 0., 0., 1.],
            },
        )
        .unwrap();
    let dismount = |engine: &mut Engine<Vec<u8>>, now| {
        let mut bytes = 0x0217u16.to_be_bytes().to_vec();
        for word in [0x0100_0008u32, 1, 0x0380_0000, 90] {
            bytes.extend(word.to_be_bytes());
        }
        dispatch(engine, &bytes, now).unwrap()
    };
    assert!(!dismount(&mut engine, 40).is_empty());
    let active = engine.active.as_ref().unwrap();
    assert_eq!(active.tracking.sample.unwrap().position.x, 105.);
    assert!(active.ground_seat.is_none());
    assert_eq!(
        engine.ground_turret_snapshot(8).unwrap().turret_owner,
        Some(u16::MAX)
    );

    assert!(!mount(&mut engine, 8, 1, 50).unwrap().is_empty());
    assert_eq!(
        engine
            .ground_turret_snapshot(8)
            .unwrap()
            .definition
            .unwrap()
            .version,
        4
    );
    row.fields.yaw = Some(Yaw::new(256).unwrap());
    report(&mut engine, 11, row, None, 60).unwrap();
    assert!(
        engine
            .ground_turret_snapshot(8)
            .unwrap()
            .fields
            .primary
            .is_none()
    );
    assert_eq!(
        engine.ground_turret_snapshot(8).unwrap().fields.yaw,
        Some(Yaw::new(128).unwrap())
    );
    row.fields.primary = Some(primary(4));
    report(&mut engine, 12, row, None, 70).unwrap();
    assert_eq!(
        engine.ground_turret_snapshot(8).unwrap().fields.yaw,
        row.fields.yaw
    );
}

#[test]
fn ground_primary_only_accepts_current_native_empty_secondary_without_arming_it() {
    use crate::{
        hospital_turret_wire::{Fields, Yaw},
        spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::delivery::delivery_fixture,
    };

    for kind in [PolicyKind::Apc, PolicyKind::Escort] {
        let (mut engine, index, now) = match kind {
            PolicyKind::Apc => (driver_engine(8), 8, 20),
            PolicyKind::Escort => (delivery_fixture(), 4, 3250),
        };
        opt_in(&mut engine, index);
        ready_on_foot(&mut engine, 1, now - 5);
        let position = engine.current_ground_vehicle(index).unwrap().position;
        engine.active.as_mut().unwrap().tracking.server_position(
            position,
            crate::player_tracking::PositionOrigin::ServerExit,
            now - 5,
        );
        mount(&mut engine, index, 1, now).unwrap();
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(engine.ground_gunner_kind(actor), Some(kind));
        let version = engine
            .ground_turret_snapshot(index)
            .unwrap()
            .definition
            .unwrap()
            .version;
        let empty = InventoryItem {
            ammo_count: 0,
            gear_slot: 1,
            ..primary(version)
        };
        let mut row = TurretUpdate {
            index,
            driver_owner: None,
            turret_owner: None,
            definition: None,
            fields: Fields {
                secondary: Some(empty),
                ..Default::default()
            },
        };
        report(&mut engine, 10, row, None, now + 1).unwrap();
        assert_eq!(
            engine.ground_turret_snapshot(index).unwrap().fields,
            Fields::default()
        );

        row.fields.primary = Some(primary(version));
        row.fields.yaw = Some(Yaw::new(72).unwrap());
        for (offset, item) in [
            InventoryItem {
                inventory_update_id: version - 1,
                ..empty
            },
            InventoryItem {
                gear_slot: 0,
                ..empty
            },
            InventoryItem {
                gear_slot: 2,
                ..empty
            },
            InventoryItem {
                ammo_pool: 1,
                ..empty
            },
            InventoryItem {
                ammo_count: 1,
                ..empty
            },
            InventoryItem {
                reloading: true,
                ..empty
            },
        ]
        .into_iter()
        .enumerate()
        {
            row.fields.secondary = Some(item);
            report(
                &mut engine,
                11 + offset as u32,
                row,
                None,
                now + 2 + offset as u64,
            )
            .unwrap();
            assert_eq!(
                engine.ground_turret_snapshot(index).unwrap().fields,
                Fields::default()
            );
        }
        row.fields.primary = Some(empty);
        row.fields.secondary = None;
        report(&mut engine, 18, row, None, now + 9).unwrap();
        assert_eq!(
            engine.ground_turret_snapshot(index).unwrap().fields,
            Fields::default()
        );

        row.fields.primary = Some(primary(version));
        row.fields.secondary = Some(empty);
        report(&mut engine, 19, row, None, now + 10).unwrap();
        let snapshot = engine.ground_turret_snapshot(index).unwrap();
        assert_eq!(snapshot.definition.unwrap().items, [Some(5), None, None]);
        assert_eq!(snapshot.fields, row.fields);
        assert_eq!(
            engine
                .ground_turret(index)
                .unwrap()
                .primary_for(engine.active.as_ref().unwrap()),
            Some((5, primary(version), version))
        );
    }
}

/// Driver (field 1) and gunner (field 6) owners carried by one seat transition.
/// `None` means the field is absent from the update. Also checks that the
/// same update moves the character into `seat`: occupancy is carried only by
/// the occupant's state14 vehicle/seat user data.
fn transition_owners(
    frames: &[crate::OutboundMessage],
    profile: crate::wire_profile::WireProfile,
    index: u16,
    seat: u8,
) -> (Option<u32>, Option<u32>) {
    use crate::replication::{BitReader, ObjectUpdate};
    let wire = frames
        .iter()
        .map(|frame| protocol::decode_hex(&frame.payload_hex))
        .find(|wire| {
            ObjectUpdate::decode_with_profile(wire, profile)
                .is_ok_and(|update| update.header.object_type_mask & (1 << 3) != 0)
        })
        .expect("seat transition");
    let update = ObjectUpdate::decode_with_profile(&wire, profile).unwrap();
    let mut bits = BitReader::with_profile(&wire, profile);
    bits.position = profile.header_bits();
    let owners = if update.header.object_type_mask & (1 << 1) != 0 {
        ground_row_owners(&mut bits, index)
    } else {
        (None, None)
    };
    assert_eq!(bits.read(1), Ok(1));
    bits.read(16).unwrap();
    for field in 0..22 {
        assert_eq!(bits.read(1), Ok(u32::from(field == 4)), "field {field}");
        if field == 4 {
            assert_eq!(bits.read(8), Ok(14));
            let user_data = [bits.read(8), bits.read(8), bits.read(8)].map(Result::unwrap);
            assert_eq!(user_data, [1, u32::from(index), u32::from(seat)]);
        }
    }
    assert_eq!(bits.read(1), Ok(0));
    owners
}

fn ground_row_owners(
    bits: &mut crate::replication::BitReader<'_>,
    index: u16,
) -> (Option<u32>, Option<u32>) {
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(index.into()));
    let (mut driver, mut gunner) = (None, None);
    for field in 0..30 {
        if bits.read(1).unwrap() == 0 {
            continue;
        }
        match field {
            1 => driver = Some(bits.read(16).unwrap()),
            6 => gunner = Some(bits.read(16).unwrap()),
            10 => {
                bits.read(8).unwrap();
                for _ in 0..3 {
                    if bits.read(1).unwrap() != 0 {
                        bits.read(12).unwrap();
                    }
                }
            }
            _ => panic!("unexpected seat transition field {field}"),
        }
    }
    assert_eq!(bits.read(1), Ok(0), "one Ground row per seat transition");
    (driver, gunner)
}

/// Send one owner driving report and return whether the vehicle moved to `x`.
fn drives(engine: &mut Engine<Vec<u8>>, index: u16, sequence: u32, x: f32, now: u64) -> bool {
    use crate::spawn_progression::tests::ground_vehicle_runtime::ground_driver_relay;
    let report = ground_driver_relay(engine.wire_profile, index, sequence, (x * 32.) as u32);
    dispatch(engine, &report, now).unwrap();
    engine.current_ground_vehicle(index).unwrap().position.x == x
}

/// Name, engine, vehicle index, the seated human's player slot and a passenger
/// seat, for a vehicle with driver seat 0 and gunner seat 1.
type SeatChangeVehicle = (&'static str, Engine<Vec<u8>>, u16, u16, u8);

fn seat_change_vehicles() -> Vec<SeatChangeVehicle> {
    use crate::spawn_progression::tests::ground_vehicle_runtime::escort_driver_engine;
    let mut apc = driver_engine(8);
    opt_in(&mut apc, 8);
    ready_on_foot(&mut apc, 0, 10);
    let mut escort = escort_driver_engine();
    opt_in(&mut escort, 4);
    vec![
        ("domination_apc", apc, 8, 0, 2),
        ("acquisition_escort", escort, 4, 3, 2),
    ]
}

#[test]
fn in_place_seat_changes_grant_driving_exactly_like_a_fresh_driver_mount() {
    for (name, mut engine, index, player, passenger) in seat_change_vehicles() {
        let profile = engine.wire_profile;
        let owner = u32::from(player);
        let server = u32::from(crate::ground_vehicles::SERVER_OWNER);
        let unowned = u32::from(u16::MAX);
        let gunner_owner =
            |engine: &Engine<Vec<u8>>| engine.ground_turret_snapshot(index).unwrap().turret_owner;

        // Fresh driver mount: atomic owner grant, and driving is accepted.
        let frames = mount(&mut engine, index, 0, 20).unwrap();
        assert_eq!(
            transition_owners(&frames, profile, index, 0),
            (Some(owner), None),
            "{name}"
        );
        assert_eq!(engine.ground_driver_owner(index), Some(player), "{name}");
        assert!(drives(&mut engine, index, 20, 101., 120), "{name}");

        // Driver -> gunner in place releases driving and grants the turret.
        let frames = mount(&mut engine, index, 1, 130).unwrap();
        assert_eq!(
            transition_owners(&frames, profile, index, 1),
            (Some(server), Some(owner)),
            "{name}"
        );
        assert_eq!(engine.ground_driver_owner(index), None, "{name}");
        assert_eq!(gunner_owner(&engine), Some(player), "{name}");
        assert!(!drives(&mut engine, index, 21, 102., 230), "{name}");

        // Gunner -> driver in place releases the turret and grants driving.
        let frames = mount(&mut engine, index, 0, 240).unwrap();
        assert_eq!(
            transition_owners(&frames, profile, index, 0),
            (Some(owner), Some(unowned)),
            "{name}"
        );
        assert_eq!(engine.ground_driver_owner(index), Some(player), "{name}");
        assert_eq!(gunner_owner(&engine), Some(u16::MAX), "{name}");
        assert!(!engine.ground_turret(index).unwrap().has_grant(), "{name}");
        assert!(drives(&mut engine, index, 22, 102., 340), "{name}");

        // The live Escort cycle: driver -> gunner -> passenger -> driver.
        mount(&mut engine, index, 1, 350).unwrap();
        let frames = mount(&mut engine, index, passenger, 360).unwrap();
        assert_eq!(
            transition_owners(&frames, profile, index, passenger),
            (None, Some(unowned)),
            "{name}"
        );
        assert_eq!(engine.ground_driver_owner(index), None, "{name}");
        assert_eq!(gunner_owner(&engine), Some(u16::MAX), "{name}");
        assert!(
            !engine.is_ground_gunner(engine.active.as_ref().unwrap()),
            "{name}"
        );
        assert!(!drives(&mut engine, index, 23, 103., 460), "{name}");
        let frames = mount(&mut engine, index, 0, 470).unwrap();
        assert_eq!(
            transition_owners(&frames, profile, index, 0),
            (Some(owner), None),
            "{name}"
        );
        assert!(drives(&mut engine, index, 24, 103., 570), "{name}");

        // Driver -> passenger -> driver.
        let frames = mount(&mut engine, index, passenger, 580).unwrap();
        assert_eq!(
            transition_owners(&frames, profile, index, passenger),
            (Some(server), None),
            "{name}"
        );
        assert!(!drives(&mut engine, index, 25, 104., 680), "{name}");
        mount(&mut engine, index, 0, 690).unwrap();
        assert!(drives(&mut engine, index, 26, 104., 790), "{name}");
        assert_eq!(
            engine.active.as_ref().unwrap().ground_seat.unwrap().seat,
            0,
            "{name}"
        );
        // A late joiner's creation row carries the same current owners.
        let creation = engine.current_ground_batch(0, 25, 800).unwrap();
        let (driver, gunner, ..) = read_creation_turret(&creation, profile, index.into());
        assert_eq!((driver, gunner), (owner, unowned), "{name}");
    }
}

#[test]
fn in_place_seat_change_into_a_held_driver_seat_is_refused() {
    for (name, mut engine, index, driver, _) in seat_change_vehicles() {
        let gunner = driver + 1;
        mount(&mut engine, index, 0, 20).unwrap();
        ready_on_foot(&mut engine, gunner, 25);
        assert!(
            !mount(&mut engine, index, 1, 30).unwrap().is_empty(),
            "{name}"
        );

        // The gunner cannot take the held driver seat; nothing changes.
        assert!(
            mount(&mut engine, index, 0, 40).unwrap().is_empty(),
            "{name}"
        );
        let active = engine.active.as_ref().unwrap();
        assert_eq!(active.ground_seat.unwrap().seat, 1, "{name}");
        assert!(engine.is_ground_gunner(active), "{name}");
        assert!(engine.ground_turret(index).unwrap().has_grant(), "{name}");
        assert_eq!(engine.ground_driver_owner(index), Some(driver), "{name}");
        assert!(!drives(&mut engine, index, 20, 101., 140), "{name}");
        focus(&mut engine, driver);
        assert!(drives(&mut engine, index, 20, 101., 150), "{name}");

        // Once the driver moves to a passenger seat, the gunner can drive.
        mount(&mut engine, index, 2, 160).unwrap();
        assert_eq!(engine.ground_driver_owner(index), None, "{name}");
        focus(&mut engine, gunner);
        assert!(
            !mount(&mut engine, index, 0, 170).unwrap().is_empty(),
            "{name}"
        );
        assert_eq!(engine.ground_driver_owner(index), Some(gunner), "{name}");
        assert_eq!(
            engine.ground_turret_snapshot(index).unwrap().turret_owner,
            Some(u16::MAX),
            "{name}"
        );
        assert!(drives(&mut engine, index, 21, 102., 270), "{name}");
    }
}
