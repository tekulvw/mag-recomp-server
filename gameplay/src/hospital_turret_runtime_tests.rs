use super::*;
use crate::{spawn_progression::dispatch, structures::domination_bunker_seat_tests::fixture, *};

pub(crate) fn enable_synthetic_hospital_turrets<W: Write>(engine: &mut Engine<W>) {
    engine.bunker_world.as_mut().unwrap().hospital_turrets =
        BTreeMap::from([(5, Runtime::new(4, Some(5))), (6, Runtime::new(4, Some(5)))]);
}

pub(crate) fn equip_synthetic_hospital_turret<W: Write>(engine: &mut Engine<W>, index: u16) {
    let actor = engine.active.as_mut().unwrap();
    actor.mounted_bunker = Some(index);
    actor.mounted_seat = Some(0);
    let runtime = &mut engine.bunker_world.as_mut().unwrap().hospital_turrets;
    let (fresh, definition) = runtime[&index].mount(actor).unwrap();
    let (equipped, _) = fresh
        .report(Fields {
            primary: Some(primary(definition.version)),
            ..Fields::default()
        })
        .unwrap();
    runtime.insert(index, equipped);
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

fn mount(
    engine: &mut Engine<Vec<u8>>,
    index: u16,
    now: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let mut wire = 0x0216u16.to_be_bytes().to_vec();
    wire.extend((0x0d00_0000 | u32::from(index)).to_be_bytes());
    wire.extend(0u32.to_be_bytes());
    wire.extend(
        (0x0380_0000 | u32::from(engine.active.as_ref().unwrap().reservation.player)).to_be_bytes(),
    );
    dispatch(engine, &wire, now)
}

fn report(
    engine: &mut Engine<Vec<u8>>,
    index: u16,
    sequence: u32,
    fields: Fields,
    transport: Transport,
) -> Result<Vec<OutboundMessage>, &'static str> {
    report_at(engine, index, sequence, fields, transport, 1020)
}

fn report_at(
    engine: &mut Engine<Vec<u8>>,
    index: u16,
    sequence: u32,
    fields: Fields,
    transport: Transport,
    now_ms: u32,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let bytes = wire::encode_updates(
        sequence,
        now_ms,
        &[ServerUpdate {
            index,
            definition: None,
            fields,
        }],
        engine.wire_profile,
    )
    .unwrap()
    .remove(0);
    let actor = engine.active.as_ref().unwrap();
    let event = events::EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: actor.reservation.world,
        player_id: actor.reservation.player,
        transport,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(&bytes),
    };
    let decoded = DecodedEvent::with_hospital_ownership(
        &bytes,
        event.player_id,
        engine.wire_profile,
        false,
        true,
    );
    engine.handle_event(event, &decoded, Duration::from_millis(now_ms.into()), 1)
}

fn turret<W: Write>(engine: &Engine<W>, index: u16) -> &Runtime {
    &engine.bunker_world.as_ref().unwrap().hospital_turrets[&index]
}

#[test]
fn indexed_hospital_gunner_mount_requires_opt_in_and_replays_committed_grant() {
    let mut engine = fixture();
    assert!(mount(&mut engine, 5, 1000).unwrap().is_empty());
    enable_synthetic_hospital_turrets(&mut engine);
    assert_eq!(turret(&engine, 5).definition(false).version, 2);
    let mounted = mount(&mut engine, 5, 1000).unwrap();
    assert!(!mounted.is_empty());
    let replay = mount(&mut engine, 5, 1001).unwrap();
    assert_eq!(mounted[0].payload_hex, replay[0].payload_hex);
    assert_eq!(turret(&engine, 5).version, 3);
    assert!(turret(&engine, 5).grant.as_ref().unwrap().primary.is_none());
}

#[test]
fn indexed_hospital_owner_report_requires_exact_route_index_version_and_readiness() {
    let mut engine = fixture();
    enable_synthetic_hospital_turrets(&mut engine);
    mount(&mut engine, 5, 1000).unwrap();
    let first = wire::Yaw::new(128).unwrap();
    let aim = Fields {
        yaw: Some(first),
        ..Fields::default()
    };
    report(&mut engine, 5, 10, aim, Transport::Udp).unwrap();
    assert!(turret(&engine, 5).yaw().is_none());
    let full = Fields {
        primary: Some(primary(3)),
        ..aim
    };
    report(&mut engine, 6, 11, full, Transport::Udp).unwrap();
    report(&mut engine, 5, 12, full, Transport::Tcp).unwrap();
    report(
        &mut engine,
        5,
        13,
        Fields {
            primary: Some(primary(2)),
            ..aim
        },
        Transport::Udp,
    )
    .unwrap();
    assert!(turret(&engine, 5).yaw().is_none());
    assert!(turret(&engine, 6).yaw().is_none());
    report(&mut engine, 5, 20, full, Transport::Udp).unwrap();
    assert_eq!(turret(&engine, 5).yaw(), Some(first.radians()));
    report(
        &mut engine,
        5,
        19,
        Fields {
            yaw: Some(wire::Yaw::new(256).unwrap()),
            ..Fields::default()
        },
        Transport::Udp,
    )
    .unwrap();
    assert_eq!(turret(&engine, 5).yaw(), Some(first.radians()));
    let switched = InventoryItem {
        gear_slot: 1,
        ..primary(3)
    };
    report(
        &mut engine,
        5,
        21,
        Fields {
            primary: Some(switched),
            ..Fields::default()
        },
        Transport::Udp,
    )
    .unwrap();
    let active = engine.active.as_ref().unwrap();
    let (turret, grant) = engine.hospital_authority(active, 5).unwrap();
    assert!(!turret.can_fire(grant.primary.unwrap()));
}

#[test]
fn indexed_hospital_grants_never_reuse_versions_across_rebuild() {
    let mut engine = fixture();
    enable_synthetic_hospital_turrets(&mut engine);
    equip_synthetic_hospital_turret(&mut engine, 5);
    let world = engine.bunker_world.as_mut().unwrap();
    world.apply_domination_cycle(1, false, u32::MAX).unwrap();
    world.apply_domination_cycle(1, true, u32::MAX).unwrap();
    assert!(world.hospital_turrets[&5].grant.is_none());
    let actor = engine.active.as_ref().unwrap();
    for version in 4..=u8::MAX {
        let (next, definition) = world.hospital_turrets[&5].mount(actor).unwrap();
        assert_eq!(definition.version, version);
        assert!(next.grant.as_ref().unwrap().primary.is_none());
        world.hospital_turrets.insert(5, next);
    }
    assert_eq!(
        world.hospital_turrets[&5].mount(actor).unwrap_err(),
        "hospital_turret_grants_exhausted"
    );
    assert_eq!(world.hospital_turrets[&5].version, u8::MAX);
}

#[test]
fn indexed_hospital_turrets_isolate_owners_and_late_join_snapshots() {
    let mut engine = fixture();
    enable_synthetic_hospital_turrets(&mut engine);
    mount(&mut engine, 5, 1000).unwrap();
    let yaw5 = wire::Yaw::new(128).unwrap();
    let yaw6 = wire::Yaw::new(256).unwrap();
    let updates = |yaw| Fields {
        yaw: Some(yaw),
        primary: Some(primary(3)),
        ..Fields::default()
    };
    report(&mut engine, 5, 10, updates(yaw5), Transport::Udp).unwrap();
    let first = engine.active.take().unwrap();
    let first_id = first.reservation.id.clone();
    engine.peers.insert(first_id.clone(), first);

    let mut second = fixture().active.take().unwrap();
    second.reservation.id = "synthetic-second-hospital-owner".into();
    second.reservation.player = 1;
    engine.active = Some(second);
    mount(&mut engine, 6, 1000).unwrap();
    report(&mut engine, 6, 10, updates(yaw6), Transport::Udp).unwrap();
    report(&mut engine, 5, 11, updates(yaw6), Transport::Udp).unwrap();
    assert_eq!(turret(&engine, 5).yaw(), Some(yaw5.radians()));
    assert_eq!(turret(&engine, 6).yaw(), Some(yaw6.radians()));

    let snapshots = engine
        .hospital_turret_late_join(Duration::from_millis(1030))
        .unwrap();
    let expected = [5, 6].map(|index| ServerUpdate {
        index,
        fields: updates(if index == 5 { yaw5 } else { yaw6 }),
        definition: Some(InventoryDefinition {
            version: 3,
            items: [Some(4), Some(5), None],
        }),
    });
    assert_eq!(snapshots.len(), 3);
    assert_eq!(
        snapshots[0],
        wire::encode_updates(1, 1030, &expected, engine.wire_profile).unwrap()[0]
    );
    assert!(!engine.peers[&first_id].pending_shared.is_empty());

    engine.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 1 };
    engine.revoke_inactive_hospital_turrets();
    assert!(turret(&engine, 6).grant.is_none());
    assert_eq!(turret(&engine, 6).yaw(), Some(yaw6.radians()));
    assert_eq!(
        engine
            .hospital_turret_late_join(Duration::from_millis(1040))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn indexed_hospital_budget_and_recorder_failure_leave_aim_and_grants_uncommitted() {
    let mut engine = fixture();
    enable_synthetic_hospital_turrets(&mut engine);
    mount(&mut engine, 5, 1000).unwrap();
    let mut peer = fixture().active.take().unwrap();
    peer.reservation.id = "synthetic-hospital-backlogged".into();
    peer.reservation.player = 1;
    let id = peer.reservation.id.clone();
    peer.pending_shared
        .resize(crate::multiplayer::MAX_SHARED_QUEUE, vec![0]);
    engine.peers.insert(id.clone(), peer);
    let fields = Fields {
        yaw: Some(wire::Yaw::new(256).unwrap()),
        primary: Some(primary(3)),
        ..Fields::default()
    };
    report(&mut engine, 5, 10, fields, Transport::Udp).unwrap();
    assert!(turret(&engine, 5).grant.as_ref().unwrap().primary.is_none());
    assert!(turret(&engine, 5).yaw().is_none());

    engine.peers.get_mut(&id).unwrap().pending_shared.clear();
    engine.max_log = engine.written;
    assert!(report(&mut engine, 5, 11, fields, Transport::Udp).is_err());
    assert!(turret(&engine, 5).grant.as_ref().unwrap().primary.is_none());
    assert!(turret(&engine, 5).yaw().is_none());
    assert!(engine.peers[&id].pending_shared.is_empty());

    let mut engine = fixture();
    enable_synthetic_hospital_turrets(&mut engine);
    engine
        .bunker_world
        .as_mut()
        .unwrap()
        .hospital_turrets
        .get_mut(&5)
        .unwrap()
        .version = u8::MAX;
    assert!(mount(&mut engine, 5, 1000).unwrap().is_empty());
    assert!(engine.active.as_ref().unwrap().mounted_seat.is_none());
    assert!(turret(&engine, 5).grant.is_none());
}

#[test]
fn indexed_hospital_aim_reserves_space_for_same_update_grenade_damage() {
    use crate::spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::ground_grenade_fixture_for_map;

    let mut hospital = fixture();
    let mut engine = ground_grenade_fixture_for_map(false, 100.0, hospital.map_name());
    let mut gunner = hospital.active.take().unwrap();
    // Keep the grenade already launched and reported by the living on-foot
    // actor. Its terminal becomes due on the first report after mounting.
    gunner.projectiles = engine.active.take().unwrap().projectiles;
    engine.active = Some(gunner);
    engine.peers.clear();
    engine.bunker_world = hospital.bunker_world;
    engine.spawn_catalog = hospital.spawn_catalog;
    enable_synthetic_hospital_turrets(&mut engine);
    assert!(!mount(&mut engine, 5, 3350).unwrap().is_empty());

    let mut peer = fixture().active.take().unwrap();
    peer.reservation.id = "synthetic-hospital-grenade-subscriber".into();
    peer.reservation.player = 1;
    peer.pending_shared
        .resize(crate::multiplayer::MAX_SHARED_QUEUE - 1, vec![0]);
    let id = peer.reservation.id.clone();
    engine.peers.insert(id.clone(), peer);

    report_at(
        &mut engine,
        5,
        10,
        Fields {
            yaw: Some(wire::Yaw::new(256).unwrap()),
            primary: Some(primary(3)),
            ..Fields::default()
        },
        Transport::Udp,
        3400,
    )
    .unwrap();
    assert_eq!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .health_quarter_points,
        300
    );
    assert_eq!(
        engine.peers[&id].pending_shared.len(),
        crate::multiplayer::MAX_SHARED_QUEUE
    );
    assert!(!engine.peers[&id].replication_backlogged);
    assert!(turret(&engine, 5).yaw().is_none());
    assert!(turret(&engine, 5).grant.as_ref().unwrap().primary.is_none());
}

#[test]
fn indexed_hospital_aim_waits_for_emplacement_creation() {
    let mut engine = fixture();
    enable_synthetic_hospital_turrets(&mut engine);
    let mounted = mount(&mut engine, 5, 1000).unwrap();
    let mut loading = fixture().active.take().unwrap();
    loading.reservation.id = "synthetic-hospital-loading".into();
    loading.reservation.player = 1;
    loading.emplacement_creation = None;
    // The ordinary history relay can queue this real Character3 + family13
    // grant frame before the recipient has created its emplacements.
    loading.queue_shared(protocol::decode_hex(&mounted[0].payload_hex));
    let id = loading.reservation.id.clone();
    engine.peers.insert(id.clone(), loading);
    let fields = Fields {
        yaw: Some(wire::Yaw::new(256).unwrap()),
        primary: Some(primary(3)),
        ..Fields::default()
    };
    report(&mut engine, 5, 10, fields, Transport::Udp).unwrap();
    assert_eq!(engine.peers[&id].pending_shared.len(), 2);
    let loading = engine.peers.get_mut(&id).unwrap();
    let queued = loading.pending_shared.back().unwrap().clone();
    let capacity = crate::multiplayer::MAX_SHARED_QUEUE;
    loading.pending_shared.resize(capacity, queued);
    assert!(engine.drain_shared(Vec::new(), 1).unwrap().is_empty());
    assert_eq!(turret(&engine, 5).yaw(), fields.yaw.map(wire::Yaw::radians));
    let gunner = engine.active.take().unwrap();
    engine.peers.insert(gunner.reservation.id.clone(), gunner);
    engine.active = Some(engine.peers.remove(&id).unwrap());
    let creation = first_emplacement_reply(&mut engine).unwrap();
    let loading = engine.active.as_ref().unwrap();
    assert!(loading.emplacement_creation.is_some());
    assert_eq!(loading.ordinary_shared_len(), capacity);
    assert_eq!(loading.hospital_bootstrap, capacity..capacity + 2);
    let mut deliveries = engine.drain_shared(creation, 1).unwrap();
    loop {
        let batch = engine.drain_shared(Vec::new(), 1).unwrap();
        if batch.is_empty() {
            break;
        }
        deliveries.extend(batch);
    }
    let masks = deliveries
        .iter()
        .map(|message| {
            let wire = protocol::decode_hex(&message.payload_hex);
            replication::ObjectUpdate::decode_with_profile(&wire, engine.wire_profile)
                .unwrap()
                .header
                .object_type_mask
        })
        .collect::<Vec<_>>();
    assert_eq!(masks[0], (1 << 3) | (1 << 7) | (1 << 8) | (1 << 13));
    assert_eq!(masks[1], (1 << 3) | (1 << 13));
    assert_eq!(masks[2..], vec![1 << 13; capacity + 1]);
    let expected = wire::encode_updates(
        (capacity + 6) as u32,
        1021,
        &[
            ServerUpdate {
                index: 5,
                fields,
                definition: Some(turret(&engine, 5).definition(true)),
            },
            ServerUpdate {
                index: 6,
                fields: Fields::default(),
                definition: Some(turret(&engine, 6).definition(false)),
            },
        ],
        engine.wire_profile,
    )
    .unwrap();
    assert_eq!(
        deliveries[capacity + 1].payload_hex,
        protocol::encode_hex(&expected[0])
    );
    let loading = engine.active.as_ref().unwrap();
    assert_eq!(loading.hospital_bootstrap, 0..0);
    assert_eq!(loading.ordinary_shared_len(), 0);
}

fn first_emplacement_reply(
    engine: &mut Engine<Vec<u8>>,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let bytes = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence: 10,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: 1021,
        },
        wire: &[],
    }
    .encode_empty_with_profile(engine.wire_profile)
    .unwrap();
    dispatch(engine, &bytes, 1021)
}

#[test]
fn indexed_hospital_bootstrap_preserves_the_ordinary_queue_limit() {
    let mut actor = fixture().active.take().unwrap();
    let ordinary_limit = crate::multiplayer::MAX_SHARED_QUEUE;
    let total_limit = ordinary_limit + MAX_LATE_JOIN_PACKETS;
    actor.pending_shared.resize(ordinary_limit - 1, vec![1]);
    actor.queue_hospital_bootstrap(vec![vec![2]; MAX_LATE_JOIN_PACKETS]);
    actor.queue_shared(vec![3]);
    assert_eq!(actor.ordinary_shared_len(), ordinary_limit);
    assert_eq!(actor.pending_shared.len(), total_limit);
    assert_eq!(actor.pending_shared.back(), Some(&vec![3]));
    actor.queue_shared(vec![4]);
    assert!(actor.replication_backlogged);
    assert_eq!(actor.pending_shared.len(), total_limit);
}

#[test]
fn indexed_hospital_failed_creation_record_keeps_bootstrap_out_of_the_fifo() {
    let mut engine = fixture();
    enable_synthetic_hospital_turrets(&mut engine);
    let loading = engine.active.as_mut().unwrap();
    loading.emplacement_creation = None;
    loading.queue_shared(vec![1]);
    engine.max_log = engine.written;
    assert!(matches!(
        first_emplacement_reply(&mut engine),
        Err("log_limit_reached")
    ));
    let loading = engine.active.as_ref().unwrap();
    assert!(loading.emplacement_creation.is_none());
    assert_eq!(loading.hospital_bootstrap, 0..0);
    assert_eq!(loading.pending_shared, [vec![1]]);
}

#[test]
fn indexed_hospital_release_revokes_readiness_and_preserves_grant_version() {
    let mut engine = fixture();
    enable_synthetic_hospital_turrets(&mut engine);
    equip_synthetic_hospital_turret(&mut engine, 5);
    let reservation_id = engine.active.as_ref().unwrap().reservation.id.clone();
    let reply = engine.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "synthetic-hospital-release".into(),
            reservation_id,
        },
        Duration::from_millis(1030),
        1,
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert!(turret(&engine, 5).grant.is_none());
    assert_eq!(turret(&engine, 5).version, 3);
}

#[test]
fn indexed_hospital_failed_mount_record_does_not_consume_version() {
    let mut engine = fixture();
    enable_synthetic_hospital_turrets(&mut engine);
    engine.max_log = engine.written;
    assert!(mount(&mut engine, 5, 1000).is_err());
    assert!(engine.active.as_ref().unwrap().mounted_seat.is_none());
    assert_eq!(turret(&engine, 5).version, 2);
    assert!(turret(&engine, 5).grant.is_none());
}

#[test]
fn indexed_hospital_rule_source_matches_attested_archive_and_exact_vehicle_type() {
    let source = Source {
        structure_sha256: "1".repeat(64),
        item_catalog_sha256: "2".repeat(64),
        retail_elf_sha256: "3".repeat(64),
        retail_archive_sha256: "4".repeat(64),
        vehicle_type_token: 17,
        seat_pose_sha256: "5".repeat(64),
        wire_proof_sha256: "6".repeat(64),
    };
    let mut provenance = json!({
        "catalog_sha256": source.item_catalog_sha256,
        "source_archive_sha256": source.retail_archive_sha256,
    });
    let matches = |token, provenance: &serde_json::Value| {
        source.matches(
            Some(&source.structure_sha256),
            &source.retail_elf_sha256,
            token,
            provenance,
        )
    };
    assert!(matches(17, &provenance));
    assert!(!matches(18, &provenance));
    assert!(!matches(0, &provenance));
    provenance["source_archive_sha256"] = json!("7".repeat(64));
    assert!(!matches(17, &provenance));
    provenance
        .as_object_mut()
        .unwrap()
        .remove("source_archive_sha256");
    assert!(!matches(17, &provenance));
}

#[test]
fn indexed_hospital_rejects_equipment_for_unpopulated_slot() {
    let mut engine = fixture();
    enable_synthetic_hospital_turrets(&mut engine);
    equip_synthetic_hospital_turret(&mut engine, 5);
    let turret = turret(&engine, 5);
    let undefined = InventoryItem {
        gear_slot: 2,
        ..primary(3)
    };
    for fields in [
        Fields {
            primary: Some(undefined),
            ..Fields::default()
        },
        Fields {
            secondary: Some(undefined),
            ..Fields::default()
        },
    ] {
        assert!(turret.report(fields).is_err());
    }
}
