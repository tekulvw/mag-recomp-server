use super::*;
use crate::{
    hospital_rocket_runtime::tests as rocket_fixture,
    projectile::{ProjectileOwnerFields, ProjectileOwnerUpdate},
    projectile_replication::ProjectileOwnerDelta,
    replication::{CharacterPosition, ObjectUpdate, UpdateHeader},
    spawn_progression::dispatch,
};
use std::sync::Arc;

pub(super) const OBSERVER: &str = "synthetic-rocket-target";
const MARKSMAN: usize = 1;
const SMOKE_ITEM: u16 = 451;
const SMOKE_AMMO: u32 = 0x1fe2_3480;
const ORIGIN: CharacterPosition = CharacterPosition {
    x: 1.5,
    y: 2.5,
    z: 2.5,
};

pub(super) fn fixture() -> Engine<Vec<u8>> {
    let mut engine = rocket_fixture::tick_fixture();
    engine.frag_character_blast = true;
    engine.development_grenade_radius = true;
    let mut human = human_kill_reward_tests::fixture();
    let equipped = human.active.as_mut().unwrap();
    let mut kit = equipped.spawned_loadout.take().unwrap();
    let mut handle = equipped.current_inventory_handle.unwrap();
    handle.inventory_index = MARKSMAN as u8;
    kit.inventories[MARKSMAN].slots[smoke_grenades::GEAR_SLOT] = Some(SMOKE_ITEM);
    let actor = engine.active.as_mut().unwrap();
    actor.mounted_bunker = None;
    actor.mounted_seat = None;
    actor.grenade_remaining = 2;
    actor.current_inventory_handle = Some(handle);
    actor.spawned_loadout = Some(kit);
    actor.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    let faction = actor.reservation.character.faction;
    let smoke = std::array::from_fn(|index| smoke_grenades::Variant {
        faction: index as u8,
        item_id: if index == usize::from(faction) {
            SMOKE_ITEM
        } else {
            SMOKE_ITEM + 1 + index as u16
        },
        ammo_token: SMOKE_AMMO + index as u32,
        initial_count: 2,
        fuse_ms: 100,
        projectile_lifetime_ms: 10000,
        volume_duration_ms: 750,
    });
    engine.smoke_grenades = Some(Arc::new(smoke_grenades::Catalog::synthetic(smoke)));
    engine
}

pub(super) fn owner_terminal(engine: &Engine<Vec<u8>>, sequence: u32) -> Vec<u8> {
    ProjectileOwnerDelta {
        header: UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 19,
            timestamp_ms: sequence,
        },
        update: &ProjectileOwnerUpdate {
            slot: 3,
            object_index: engine.active.as_ref().unwrap().reservation.player,
            fields: ProjectileOwnerFields {
                state: Some(4),
                position: Some(ORIGIN),
                rotation_xyzw: None,
            },
        },
    }
    .encode_with_profile(engine.wire_profile)
    .unwrap()
}

fn shared_mask(packet: &[u8], engine: &Engine<Vec<u8>>) -> Option<u32> {
    ObjectUpdate::decode_with_profile(packet, engine.wire_profile)
        .ok()
        .map(|update| update.header.object_type_mask)
}

pub(super) fn drain_shared(engine: &mut Engine<Vec<u8>>, now_ms: u64) {
    engine
        .queue_projectile_publications(Duration::from_millis(now_ms))
        .unwrap();
    engine.drain_shared(Vec::new(), 1).unwrap();
}

#[test]
fn smoke_runtime_native_launch_terminal_and_expiry_do_not_damage_and_fence_reuse() {
    let mut engine = fixture();
    let request = rocket_fixture::launch_wire(&engine, 3, 4);
    assert!(!dispatch(&mut engine, &request, 1000).unwrap().is_empty());
    assert_eq!(engine.active.as_ref().unwrap().grenade_remaining, 1);
    let mut next_request = request.clone();
    next_request[2..6].copy_from_slice(&1.5_f32.to_bits().to_be_bytes());
    drain_shared(&mut engine, 1000);

    let early = owner_terminal(&engine, 1099);
    dispatch(&mut engine, &early, 1099).unwrap();
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
    let report = owner_terminal(&engine, 1100);
    dispatch(&mut engine, &report, 1100).unwrap();
    engine
        .queue_projectile_publications(Duration::from_millis(1100))
        .unwrap();

    let owner = engine.active.as_ref().unwrap();
    let volumes = owner.smoke_volumes.volumes();
    assert_eq!(volumes.len(), 1);
    assert_eq!(volumes[0].object_index, 0);
    assert_eq!(volumes[0].expires, Duration::from_millis(1850));
    assert_eq!(volumes[0].creation.position.x, ORIGIN.x);
    assert_eq!(volumes[0].creation.position.y, ORIGIN.y);
    assert_eq!(volumes[0].creation.position.z, ORIGIN.z);
    assert!(engine.projectile_cleanup_pending(owner, 3));
    assert!(owner.grenade_damage.is_empty());
    assert_eq!(owner.life.damage_health().unwrap().1, 400);
    assert_eq!(engine.peers[OBSERVER].life.damage_health().unwrap().1, 400);
    assert!(engine.peers[OBSERVER].pending_shared.iter().any(|packet| {
        shared_mask(packet, &engine) == Some(1 << 5)
            && smoke_volume_replication::is_removal(packet, engine.wire_profile).is_none()
    }));
    assert!(engine.peers[OBSERVER].pending_shared.iter().any(|packet| {
        ObjectUpdate::decode_with_profile(packet, engine.wire_profile)
            .ok()
            .and_then(|update| {
                projectile::decode_projectile_updates_with_profile(&update, engine.wire_profile)
                    .ok()
            })
            .is_some_and(|reports| {
                reports
                    .iter()
                    .any(|report| report.slot == 3 && report.fields.state == Some(2))
            })
    }));
    assert_eq!(
        owner
            .smoke_volumes
            .snapshot_packets(1200, Duration::from_millis(1200), engine.wire_profile)
            .unwrap()
            .len(),
        3
    );
    drain_shared(&mut engine, 1100);

    dispatch(&mut engine, &report, 1101).unwrap();
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
    dispatch(&mut engine, &next_request, 1849).unwrap();
    assert_eq!(engine.active.as_ref().unwrap().grenade_remaining, 1);
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .projectiles
            .get(3)
            .unwrap()
            .completed
    );

    dispatch(&mut engine, &next_request, 1850).unwrap();
    engine
        .queue_projectile_publications(Duration::from_millis(1850))
        .unwrap();
    let owner = engine.active.as_ref().unwrap();
    assert!(owner.smoke_volumes.volumes().is_empty());
    assert!(engine.projectile_cleanup_pending(owner, 3));
    assert!(engine.peers[OBSERVER].pending_shared.iter().any(|packet| {
        smoke_volume_replication::is_removal(packet, engine.wire_profile) == Some(0)
    }));
    assert_eq!(owner.grenade_remaining, 1);
    // The owner can request a replacement after every recipient drains the
    // old parent removal into reliable history.
    drain_shared(&mut engine, 1850);
    dispatch(&mut engine, &next_request, 1851).unwrap();
    assert_eq!(engine.active.as_ref().unwrap().grenade_remaining, 0);
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
}

#[test]
fn smoke_runtime_terminal_record_refusal_keeps_owner_flight_and_shared_fifo() {
    let mut engine = fixture();
    let request = rocket_fixture::launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &request, 1000).unwrap();
    let before = engine.peers[OBSERVER].pending_shared.clone();
    let report = owner_terminal(&engine, 1100);
    engine.max_log = engine.written;
    assert_eq!(
        dispatch(&mut engine, &report, 1100).unwrap_err(),
        "log_limit_reached"
    );
    let owner = engine.active.as_ref().unwrap();
    assert!(owner.smoke_volumes.volumes().is_empty());
    assert!(!engine.projectile_cleanup_pending(owner, 3));
    assert!(!owner.projectiles.get(3).unwrap().completed);
    assert_eq!(owner.grenade_remaining, 1);
    assert_eq!(engine.peers[OBSERVER].pending_shared, before);
}

#[test]
fn smoke_runtime_join_between_terminal_and_tick_gets_one_current_activation() {
    let mut engine = fixture();
    let profile = engine.wire_profile;
    let source = engine.active.as_ref().unwrap().reservation.player;
    let match_profile = engine.match_profile();
    let level_name = engine.map_name().to_owned();
    let claim = |player| Claimed {
        wire: Vec::new(),
        np_identity: [player as u8; 36],
        game_info: protocol::GameInfo {
            client_index: player,
            level_name: level_name.clone(),
            start_time_ms: 0,
            attacking_faction: match_profile.attacking_faction,
            defending_faction: match_profile.defending_faction,
            maximum_points: match_profile.maximum_points,
            current_points: match_profile.maximum_points,
            game_type: match_profile.game_type,
        },
    };
    engine.active.as_mut().unwrap().claimed = Some(claim(source));
    let joiner = engine.peers.get_mut(OBSERVER).unwrap();
    joiner.initial_objects = None;
    joiner.ready_for_updates = false;
    joiner.entered_game = false;
    joiner.outbound_sequence = 0;
    joiner.replication_history.clear();
    joiner.claimed = Some(claim(joiner.reservation.player));
    let player = joiner.reservation.player;
    let world = joiner.reservation.world;

    let request = rocket_fixture::launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &request, 1000).unwrap();
    let terminal = owner_terminal(&engine, 1100);
    dispatch(&mut engine, &terminal, 1100).unwrap();
    // Keep the terminal publications staged as they are when the world tick
    // has yet to run. A newcomer needs the current snapshot in this window.
    let owner = engine.active.as_ref().unwrap();
    assert!(
        owner
            .projectile_publications
            .iter()
            .any(|publication| { shared_mask(&publication.wire, &engine) == Some(1 << 5) })
    );

    let mut ready = profile.wire_message_type(0x0005).to_be_bytes().to_vec();
    ready.push(0x80);
    let reply = engine.handle(
        Request::Event {
            v: 2,
            request_id: "synthetic-smoke-ready".into(),
            reservation_id: OBSERVER.into(),
            world_id: world,
            player_id: player,
            transport: Transport::Tcp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex: protocol::encode_hex(&ready),
        },
        Duration::from_millis(1101),
        1,
    );
    assert!(reply.ok, "{:?}", reply.error);

    engine
        .queue_projectile_publications(Duration::from_millis(1102))
        .unwrap();
    let newcomer = engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|actor| actor.reservation.id == OBSERVER)
        .unwrap();
    let sent = reply
        .messages
        .iter()
        .filter(|message| message.reservation_id == OBSERVER)
        .map(|message| protocol::decode_hex(&message.payload_hex))
        .collect::<Vec<_>>();
    let smoke_packets = sent
        .iter()
        .chain(&newcomer.pending_shared)
        .filter_map(|packet| ObjectUpdate::decode_with_profile(packet, profile).ok())
        .filter(|update| matches!(update.header.object_type_mask, 0x20 | 0x8_0000))
        .collect::<Vec<_>>();
    assert_eq!(
        smoke_packets
            .iter()
            .map(|update| update.header.object_type_mask)
            .collect::<Vec<_>>(),
        [1 << 19, 1 << 19, 1 << 5]
    );
    let terminal =
        projectile::decode_projectile_updates_with_profile(&smoke_packets[1], profile).unwrap();
    assert_eq!(terminal[0].object_index, source);
    assert_eq!(terminal[0].fields.state, Some(2));
}

#[test]
fn smoke_runtime_full_late_join_bootstrap_preserves_ordinary_queue_budget() {
    let mut engine = fixture();
    let profile = engine.wire_profile;
    let now = Duration::from_millis(1100);
    let actor = engine.active.as_mut().unwrap();
    let ordinary = ObjectUpdate {
        header: UpdateHeader {
            sequence: 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: 1100,
        },
        wire: &[],
    }
    .encode_empty_with_profile(profile)
    .unwrap();
    for _ in 0..63 {
        actor.queue_shared(ordinary.clone());
    }
    // Eight existing owners have all four native slots occupied. Each live
    // smoke contributes a creation, state 2, and one separate volume row.
    let mut bootstrap = Vec::new();
    for player in 0..8 {
        let mut smoke = smoke_runtime::State::default();
        for slot in 0..4 {
            let object_index = 4 * player + u16::from(slot);
            smoke.push(smoke_runtime::Volume {
                object_index,
                generation: 1 + u32::from(slot),
                creation: projectile::ProjectileCreation {
                    slot,
                    object_index: player,
                    owner_index: player,
                    ammo_token: SMOKE_AMMO,
                    position: ORIGIN,
                    rotation_xyzw: [0., 0., 0., 1.],
                    shooter_id: 0x0380_0000 | u32::from(player),
                },
                expires: now + Duration::from_secs(1),
                poison: None,
            });
        }
        bootstrap.extend(smoke.snapshot_packets(1100, now, profile).unwrap());
    }
    assert_eq!(bootstrap.len(), 96);
    actor.queue_smoke_bootstrap(bootstrap);
    assert!(!actor.replication_backlogged);
    assert_eq!(actor.ordinary_shared_len(), 63);
    assert_eq!(actor.pending_shared.len(), 159);

    for _ in 0..(crate::multiplayer::MAX_SHARED_QUEUE - 63) {
        actor.queue_shared(ordinary.clone());
    }
    assert!(!actor.replication_backlogged);
    assert_eq!(
        actor.ordinary_shared_len(),
        crate::multiplayer::MAX_SHARED_QUEUE
    );
    assert_eq!(
        actor.pending_shared.len(),
        crate::multiplayer::MAX_SHARED_QUEUE + 96
    );
    actor.queue_shared(ordinary);
    assert!(actor.replication_backlogged);
    assert_eq!(
        actor.pending_shared.len(),
        crate::multiplayer::MAX_SHARED_QUEUE + 96
    );

    // Pop the prior prefix and part of the reserved segment. Every ordinary
    // suffix packet must remain, and protected frames consume no capacity.
    for _ in 0..(63 + 35) {
        actor.pop_shared();
    }
    assert_eq!(
        actor.ordinary_shared_len(),
        crate::multiplayer::MAX_SHARED_QUEUE - 63
    );
    assert_eq!(
        actor.pending_shared.len(),
        crate::multiplayer::MAX_SHARED_QUEUE - 2
    );
    for _ in 0..61 {
        actor.pop_shared();
    }
    assert_eq!(
        actor.ordinary_shared_len(),
        crate::multiplayer::MAX_SHARED_QUEUE - 63
    );
    assert_eq!(
        actor.pending_shared.len(),
        crate::multiplayer::MAX_SHARED_QUEUE - 63
    );
}

fn gas_fixture() -> Engine<Vec<u8>> {
    let mut engine = fixture();
    let smoke = engine.smoke_grenades.take().unwrap();
    engine
        .active
        .as_mut()
        .unwrap()
        .spawned_loadout
        .as_mut()
        .unwrap()
        .inventories[MARKSMAN]
        .slots[12] = Some(3);
    engine.gas_grenades = Some(Arc::new(gas_grenades::Catalog::synthetic(
        std::array::from_fn(|i| {
            let s = smoke.variants()[i];
            gas_grenades::Variant {
                faction: s.faction,
                item_id: s.item_id,
                ammo_token: s.ammo_token,
                initial_count: s.initial_count,
                fuse_ms: s.fuse_ms,
                projectile_lifetime_ms: s.projectile_lifetime_ms,
                volume_duration_ms: 3000,
                warmup_ms: 1000,
                interval_ms: 500,
                inner_radius_m: 3.0,
                inner_scale: 0.5,
                outer_radius_m: 5.0,
                outer_scale: 0.0,
                character_hp_per_pulse: 20.0,
            }
        }),
    )));
    engine
}

#[test]
fn gas_grenade_native_terminal_builds_one_volume_and_a_due_poison_pulse() {
    let mut engine = gas_fixture();
    let request = rocket_fixture::launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &request, 1000).unwrap();
    assert_eq!(engine.active.as_ref().unwrap().grenade_remaining, 2);

    engine.active.as_mut().unwrap().reservation.character.skills = Some([0, 1 << 18, 0, 0, 0]);
    dispatch(&mut engine, &request, 1001).unwrap();
    assert_eq!(engine.active.as_ref().unwrap().grenade_remaining, 1);
    dispatch(&mut engine, &request, 1002).unwrap();
    assert_eq!(engine.active.as_ref().unwrap().grenade_remaining, 1);
    drain_shared(&mut engine, 1002);

    let early = owner_terminal(&engine, 1100);
    dispatch(&mut engine, &early, 1100).unwrap();
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .smoke_volumes
            .volumes()
            .is_empty()
    );
    let terminal = owner_terminal(&engine, 1101);
    dispatch(&mut engine, &terminal, 1101).unwrap();
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
    assert!(engine.active.as_ref().unwrap().grenade_damage.is_empty());

    for ms in [1601, 2100] {
        engine
            .tick_gas_grenades(Duration::from_millis(ms), 1)
            .unwrap();
        assert_eq!(engine.peers[OBSERVER].life.damage_health().unwrap().1, 400);
    }
    engine
        .tick_gas_grenades(Duration::from_millis(2101), 1)
        .unwrap();
    let victim = &engine.peers[OBSERVER];
    assert!(victim.life.damage_health().unwrap().1 < 400);
    assert_eq!(victim.last_damage.last_location_and_type, 0x41);
    assert_eq!(victim.last_damage.last_source_object, 0x1380_0000);
    assert_eq!(victim.last_damage.last_attacker, 0x0380_0000);

    drain_shared(&mut engine, 4101);
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
    drain_shared(&mut engine, 4102);
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .smoke_volumes
            .volumes()
            .is_empty()
    );
}

#[test]
fn gas_grenade_pulse_recorder_refusal_keeps_health_and_pending_volume() {
    let mut engine = gas_fixture();
    engine.active.as_mut().unwrap().reservation.character.skills = Some([0, 1 << 18, 0, 0, 0]);
    let request = rocket_fixture::launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &request, 1000).unwrap();
    let report = owner_terminal(&engine, 1100);
    dispatch(&mut engine, &report, 1100).unwrap();
    engine
        .tick_gas_grenades(Duration::from_millis(1600), 1)
        .unwrap();
    let victim_before = engine.peers[OBSERVER].pending_shared.clone();
    engine.max_log = engine.written;
    assert!(
        engine
            .tick_gas_grenades(Duration::from_millis(2100), 1)
            .is_err()
    );
    let victim = &engine.peers[OBSERVER];
    assert_eq!(victim.life.damage_health().unwrap().1, 400);
    assert_eq!(victim.pending_shared, victim_before);
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
}

#[test]
fn gas_grenade_stale_launch_does_not_create_a_volume_in_a_later_round() {
    let mut engine = gas_fixture();
    engine.active.as_mut().unwrap().reservation.character.skills = Some([0, 1 << 18, 0, 0, 0]);
    let request = rocket_fixture::launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &request, 1000).unwrap();
    engine.round_number += 1;
    let report = owner_terminal(&engine, 1100);
    dispatch(&mut engine, &report, 1100).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.smoke_volumes.volumes().is_empty());
    assert!(actor.grenade_damage.is_empty());
    assert!(actor.projectiles.get(3).unwrap().completed);
}

#[test]
fn gas_grenade_and_leadership_pulse_share_one_current_life_cooldown() {
    use crate::active_ability_gas_world::{GasPulse, GasSource};

    let mut engine = gas_fixture();
    engine.active.as_mut().unwrap().reservation.character.skills = Some([0, 1 << 18, 0, 0, 0]);
    let request = rocket_fixture::launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &request, 1000).unwrap();
    let report = owner_terminal(&engine, 1100);
    dispatch(&mut engine, &report, 1100).unwrap();
    let now = Duration::from_millis(2100);
    engine.tick_gas_grenades(now, 1).unwrap();
    let owner = engine.active.as_ref().unwrap();
    let gas = owner.smoke_volumes.volumes()[0].poison.as_ref().unwrap();
    let falloff = gas.falloff;
    let source = GasSource {
        allocation: None,
        ..gas.source.clone()
    };
    let victim_health = engine.peers[OBSERVER].life.damage_health().unwrap().1;
    assert!(victim_health < 400);
    let mut rows = [0; 32];
    rows[6] = 1 << 21;
    let native = engine.owned_acquisition_characters();
    let next = engine
        .prepare_acquisition_character_callback(&native, now, |engine, native| {
            engine.stage_gas_pulses(
                [GasPulse {
                    projectile_id: None,
                    origin: [ORIGIN.x, ORIGIN.y, ORIGIN.z],
                    falloff,
                    raw_hp: 200.0,
                    source,
                    operation: "gas_bombardment_pulse",
                }],
                native,
                now,
                domination_structural_world::BlastCollision {
                    filter_rows: &rows,
                    limits: Default::default(),
                    sequences: &Default::default(),
                    material_costs: &Default::default(),
                    terrain_materials: None,
                },
            )
        })
        .unwrap();
    assert_eq!(next.audit()[0]["op"], "gas_bombardment_pulse");
    assert!(
        next.audit()
            .iter()
            .all(|entry| entry["on_character_hit"] != true)
    );
}

#[test]
fn gas_grenade_unresolved_firer_keeps_the_historical_projectile_source() {
    let mut engine = gas_fixture();
    engine.active.as_mut().unwrap().reservation.character.skills = Some([0, 1 << 18, 0, 0, 0]);
    let request = rocket_fixture::launch_wire(&engine, 3, 4);
    dispatch(&mut engine, &request, 1000).unwrap();
    let report = owner_terminal(&engine, 1100);
    dispatch(&mut engine, &report, 1100).unwrap();
    // Exercise mismatched allocation in the shared receiver without attributing
    // this fixture's retained cloud to the replacement occupant.
    engine.active.as_mut().unwrap().allocation_sequence += 1;
    engine
        .tick_gas_grenades(Duration::from_millis(2100), 1)
        .unwrap();
    let victim = &engine.peers[OBSERVER];
    assert!(victim.life.damage_health().unwrap().1 < 400);
    assert_eq!(victim.last_damage.last_attacker, u32::MAX);
    assert_eq!(victim.last_damage.last_source_object, 0x1380_0000);
}
