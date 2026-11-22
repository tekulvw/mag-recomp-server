use super::*;
use crate::{
    hospital_rocket_runtime::tests::launch_wire,
    smoke_runtime_tests::{OBSERVER, drain_shared, fixture, owner_terminal},
    spawn_progression::dispatch,
};

#[test]
fn predicted_smoke_effect_is_not_retriggered_on_the_thrower() {
    let mut engine = fixture();
    let launch = launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &launch, 1000).unwrap();
    let report = owner_terminal(&engine, 1100);
    dispatch(&mut engine, &report, 1100).unwrap();
    engine
        .queue_projectile_publications(Duration::from_millis(1100))
        .unwrap();
    let terminal_count = |actor: &Active| {
        actor
            .pending_shared
            .iter()
            .filter(|packet| {
                replication::ObjectUpdate::decode_with_profile(packet, engine.wire_profile)
                    .ok()
                    .and_then(|update| {
                        projectile::decode_projectile_updates_with_profile(
                            &update,
                            engine.wire_profile,
                        )
                        .ok()
                    })
                    .is_some_and(|reports| {
                        reports.iter().any(|report| report.fields.state == Some(2))
                    })
            })
            .count()
    };
    assert_eq!(terminal_count(engine.active.as_ref().unwrap()), 0);
    assert_eq!(terminal_count(&engine.peers[OBSERVER]), 1);
    for actor in [engine.active.as_ref().unwrap(), &engine.peers[OBSERVER]] {
        assert!(actor.pending_shared.iter().any(|packet| {
            replication::ObjectUpdate::decode_with_profile(packet, engine.wire_profile)
                .is_ok_and(|update| update.header.object_type_mask == 1 << 5)
        }));
    }
}

#[test]
fn releasing_the_thrower_removes_its_volume_from_the_remaining_world() {
    let mut engine = fixture();
    let launch = launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &launch, 1000).unwrap();
    let report = owner_terminal(&engine, 1100);
    dispatch(&mut engine, &report, 1100).unwrap();
    drain_shared(&mut engine, 1100);
    let source = engine.active.as_ref().unwrap().reservation.id.clone();
    let reply = engine.handle(
        Request::Release {
            v: 2,
            request_id: "synthetic-smoke-release".into(),
            reservation_id: source.clone(),
        },
        Duration::from_millis(1200),
        1,
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert!(
        engine
            .active
            .iter()
            .chain(engine.peers.values())
            .all(|actor| actor.reservation.id != source)
    );
    let delivered = reply
        .messages
        .iter()
        .map(|message| protocol::decode_hex(&message.payload_hex));
    let queued = engine
        .active
        .iter()
        .chain(engine.peers.values())
        .flat_map(|actor| actor.pending_shared.iter().cloned());
    assert!(delivered.chain(queued).any(|packet| {
        smoke_volume_replication::is_removal(&packet, engine.wire_profile) == Some(0)
    }));
    let removal_still_queued = engine
        .active
        .iter()
        .chain(engine.peers.values())
        .any(|actor| {
            actor.pending_shared.iter().any(|packet| {
                smoke_volume_replication::is_removal(packet, engine.wire_profile) == Some(0)
            })
        });
    assert_eq!(
        engine.free_smoke_volume_index(&smoke_runtime::State::default()),
        Some(u16::from(removal_still_queued))
    );
}

#[test]
fn smoke_pool_removals_fence_every_index_and_a_deferred_terminal_retries_once() {
    let mut engine = fixture();
    let launch = launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &launch, 1000).unwrap();
    for index in 0..smoke_volume_replication::CAPACITY {
        let removal = smoke_volume_replication::VolumeRemoval {
            object_index: index,
        }
        .encode_with_profile(1, 1, engine.wire_profile)
        .unwrap();
        engine
            .peers
            .get_mut(OBSERVER)
            .unwrap()
            .queue_shared(removal);
    }
    let report = owner_terminal(&engine, 1100);
    dispatch(&mut engine, &report, 1100).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.smoke_volumes.volumes().is_empty());
    assert!(!actor.projectiles.get(3).unwrap().completed);
    assert_eq!(actor.grenade_remaining, 1);
    drain_shared(&mut engine, 1100);
    drain_shared(&mut engine, 1100);
    let report = owner_terminal(&engine, 1101);
    dispatch(&mut engine, &report, 1101).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.smoke_volumes.volumes().len(), 1);
    assert_eq!(actor.smoke_volumes.volumes()[0].object_index, 0);
    assert_eq!(actor.grenade_remaining, 1);
}

#[test]
fn exploded_smoke_survives_owner_death_until_its_independent_expiry() {
    let mut engine = fixture();
    let launch = launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &launch, 1000).unwrap();
    let report = owner_terminal(&engine, 1100);
    dispatch(&mut engine, &report, 1100).unwrap();
    drain_shared(&mut engine, 1100);
    let actor = engine.active.as_mut().unwrap();
    actor
        .apply_peer_damage(
            4,
            replication::CharacterDamage {
                health_quarter_points: 0,
                ..actor.last_damage
            },
            Duration::from_millis(1200),
        )
        .unwrap();
    assert!(!actor.life.is_alive());
    assert_eq!(actor.smoke_volumes.volumes().len(), 1);
    assert_eq!(
        actor
            .smoke_volumes
            .snapshot_packets(1200, Duration::from_millis(1200), engine.wire_profile)
            .unwrap()
            .len(),
        3
    );
    engine
        .queue_projectile_publications(Duration::from_millis(1849))
        .unwrap();
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .smoke_volumes
            .volumes()
            .len(),
        1
    );
    engine
        .queue_projectile_publications(Duration::from_millis(1850))
        .unwrap();
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .smoke_volumes
            .volumes()
            .is_empty()
    );
    assert!(engine.peers[OBSERVER].pending_shared.iter().any(|packet| {
        smoke_volume_replication::is_removal(packet, engine.wire_profile) == Some(0)
    }));
}

#[test]
fn smoke_without_a_reported_terminal_position_expires_without_a_volume() {
    let mut engine = fixture();
    let launch = launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &launch, 1000).unwrap();
    let report = |sequence| {
        projectile_replication::ProjectileOwnerDelta {
            header: replication::UpdateHeader {
                sequence,
                nak_sequence: 0,
                client_index: 0,
                object_type_mask: 1 << 19,
                timestamp_ms: sequence,
            },
            update: &projectile::ProjectileOwnerUpdate {
                slot: 3,
                object_index: engine.active.as_ref().unwrap().reservation.player,
                fields: projectile::ProjectileOwnerFields {
                    state: Some(4),
                    ..Default::default()
                },
            },
        }
        .encode_with_profile(engine.wire_profile)
        .unwrap()
    };
    let first = report(1100);
    let expired = report(40000);
    dispatch(&mut engine, &first, 1100).unwrap();
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .smoke_volumes
            .volumes()
            .is_empty()
    );
    assert!(
        !engine
            .active
            .as_ref()
            .unwrap()
            .projectiles
            .get(3)
            .unwrap()
            .completed
    );
    dispatch(&mut engine, &expired, 40000).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.smoke_volumes.volumes().is_empty());
    assert!(actor.projectiles.get(3).unwrap().completed);
    assert!(actor.projectile_cleanup[3].is_some());
}

#[test]
fn mixed_admitted_frag_and_smoke_terminals_reach_only_their_own_consumers() {
    let mut engine = fixture();
    engine.development_grenade_radius = false;
    engine.item_catalog = human_kill_reward_tests::grenade_items_with_cover();
    let actor = engine.active.as_mut().unwrap();
    let kit = actor.spawned_loadout.as_mut().unwrap();
    kit.inventories[1].slots[12] = kit.inventories[0].slots[12];
    let launch = launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &launch, 1000).unwrap();
    let actor = engine.active.as_mut().unwrap();
    let position = replication::CharacterPosition {
        x: 1.5,
        y: 0.5,
        z: 2.5,
    };
    let disposition = actor
        .projectiles
        .admit_item_with_profile(
            2,
            4,
            1000,
            actor.latest_owner_sequence,
            &[1; 57],
            4,
            &engine.item_catalog,
            engine.wire_profile,
        )
        .unwrap();
    let projectile::LaunchDisposition::Created { generation } = disposition else {
        panic!("new frag")
    };
    actor
        .projectile_replicas
        .remember_creation(
            generation,
            projectile::ProjectileCreation {
                slot: 2,
                object_index: actor.reservation.player,
                owner_index: actor.reservation.player,
                ammo_token: 91,
                position,
                rotation_xyzw: [0., 0., 0., 1.],
                shooter_id: 0x03800000 | u32::from(actor.reservation.player),
            },
        )
        .unwrap();
    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    replication::UpdateHeader {
        sequence: 1100,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: (1 << 18) | (1 << 19),
        timestamp_ms: 1100,
    }
    .encode(&mut bits)
    .unwrap();
    for slot in [2, 3] {
        bits.write(1, 1).unwrap();
        bits.write(u32::from(actor.reservation.player), 16).unwrap();
        projectile::ProjectileOwnerFields {
            state: Some(if slot == 2 { 2 } else { 4 }),
            position: Some(position),
            rotation_xyzw: None,
        }
        .encode_fields(&mut bits)
        .unwrap();
        bits.write(0, 1).unwrap();
    }
    dispatch(&mut engine, &bits.finish(), 1100).unwrap();
    let rows: Vec<Value> = engine
        .writer
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect();
    let event = rows
        .iter()
        .rev()
        .find_map(|row| {
            let audit = &row["decoded"];
            (audit["type"] == "object_update").then_some(audit)
        })
        .unwrap();
    assert_eq!(event["projectile_terminals"].as_array().unwrap().len(), 1);
    assert_eq!(event["projectile_terminals"][0]["item_id"], 4);
    assert_eq!(event["smoke_terminals"].as_array().unwrap().len(), 1);
    assert_eq!(event["smoke_terminals"][0]["created"], true);
    let checks = event["native_grenade_character_checks"].as_array().unwrap();
    assert!(!checks.is_empty());
    assert!(
        checks.iter().all(|check| check.get("error").is_none()),
        "{event}"
    );
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .smoke_volumes
            .volumes()
            .len(),
        1
    );
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .projectiles
            .get(2)
            .unwrap()
            .completed
    );
}
