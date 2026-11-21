use super::*;
use crate::{
    protocol,
    spawn_progression::{
        dispatch,
        tests::ground_vehicle_runtime::domination_ground_driver::{driver_engine, ready_on_foot},
    },
};

pub(crate) fn opt_in<W: Write>(engine: &mut Engine<W>, index: u16) {
    let kind = match engine.match_profile().game_type {
        1 => PolicyKind::Escort,
        _ => PolicyKind::Apc,
    };
    let grounds = engine.ground_vehicles.as_mut().unwrap();
    grounds.set_turret_policies(BTreeMap::from([(
        index,
        Policy {
            kind,
            primary: 5,
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
