use super::*;
use crate::{
    domination_helo_damage::HullDamage,
    domination_structural_world::hospital_rocket_helo_damage::tests::install_helo_fixture,
    events::EventInput,
    ground_turrets::tests::{equip_synthetic_ground_turret, opt_in, opt_in_primary},
    hospital_turrets::tests::{enable_synthetic_hospital_turrets, equip_synthetic_hospital_turret},
    human_kill_reward_tests::{combat_items_for_ground_test, native_hit},
    manual_ground_repair_domination_tests,
    replication::{BitReader, CharacterPosition, ObjectUpdate},
    retail_combat::hospital_primary_tests::combat_catalog,
    spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::ground_firearm_fixture,
    structures::domination_bunker_seat_tests,
};
use std::sync::Arc;

const APC: u32 = 0x0100_0000;
const MOTOR_POOL: u32 = 0x0100_0008;
const ESCORT: u32 = 0x0100_0004;
const HELO: u32 = 0x0280_0000;

fn domination(hospital: bool) -> Engine<Vec<u8>> {
    domination_with_drivers(hospital, false)
}

fn domination_with_drivers(hospital: bool, drivers: bool) -> Engine<Vec<u8>> {
    let (faction, team) = if hospital { (2, 1) } else { (0, 0) };
    let mut engine =
        manual_ground_repair_domination_tests::fixture_with_drivers(faction, team, drivers);
    engine.item_catalog = Arc::new(
        Arc::try_unwrap(combat_items_for_ground_test(50.))
            .unwrap()
            .with_retail_combat(combat_catalog()),
    );
    engine
        .domination_gameplay
        .as_mut()
        .unwrap()
        .state
        .ground_damage = Arc::new(
        serde_json::from_value(json!({
            "friendly_fire_multiplier":0.5,
            "profiles":[
                {"vehicle_type_token":110,"faction":faction,"max_health":150.,
                    "critical_loss_percent_per_second":0.,
                    "armor":{"rating":100.,"prevention":1.,"coverage":vec![0.97;4]}},
                {"vehicle_type_token":111,"faction":(faction+1)%3,"max_health":150.,
                    "critical_loss_percent_per_second":0.,
                    "armor":{"rating":0.,"prevention":1.,"coverage":vec![0.91;4]}}
            ],
            "areas":vec![json!({"armor_area":0,"multiplier":1.});16]
        }))
        .unwrap(),
    );
    if hospital {
        engine.bunker_world = domination_bunker_seat_tests::fixture().bunker_world;
        enable_synthetic_hospital_turrets(&mut engine);
        equip_synthetic_hospital_turret(&mut engine, 5);
    } else {
        opt_in(&mut engine, APC as u16);
        equip_synthetic_ground_turret(&mut engine, APC as u16);
    }
    engine
}

fn dispatch_hit(
    engine: &mut Engine<Vec<u8>>,
    wire: &[u8],
    transport: Transport,
    now_ms: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let actor = engine.active.as_ref().unwrap();
    let input = EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: actor.reservation.world,
        player_id: actor.reservation.player,
        transport,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(wire),
    };
    let decoded = DecodedEvent::with_profile(wire, input.player_id, engine.wire_profile, false);
    engine.handle_event(input, &decoded, Duration::from_millis(now_ms), 1)
}

fn target(engine: &Engine<Vec<u8>>, object: u32) -> &ground::Vehicle {
    engine
        .ground_runtime
        .as_ref()
        .unwrap()
        .get(object as u16)
        .unwrap()
}

fn last_record(engine: &Engine<Vec<u8>>) -> Value {
    serde_json::from_slice(
        engine
            .writer
            .split(|byte| *byte == b'\n')
            .rev()
            .nth(1)
            .unwrap(),
    )
    .unwrap()
}

fn check_mounted_ground_primary(
    hospital: bool,
    transport: Transport,
    item: u16,
    applied: f64,
    quarters: u16,
) {
    let mut engine = domination(hospital);
    assert_eq!(target(&engine, MOTOR_POOL).body, ground::Body::Operational);
    let hit = native_hit(MOTOR_POOL, 181);
    dispatch_hit(&mut engine, &hit, transport, 3300).unwrap();
    let record = last_record(&engine);
    assert_eq!(record["gameplay_denial"], Value::Null, "{record}");
    assert_eq!(record["decoded"]["type"], "ground_bullet_hit");
    assert_eq!(record["decoded"]["weapon_item"], item);
    let amount = record["decoded"]["amount_hp"].as_f64().unwrap();
    assert!((amount - applied).abs() < 0.00001, "{record}");
    let damage = target(&engine, MOTOR_POOL).damage_wire();
    assert_eq!(damage.health_quarter_points, quarters);
    assert_eq!(damage.last_location_and_type, 0x13);
    assert_eq!(damage.last_attacker, 0x0380_0000);
    assert_eq!(damage.last_source_object, u32::MAX);
    assert_eq!(damage.last_user_data, 181);
    let queue = &engine.active.as_ref().unwrap().pending_shared;
    assert_eq!(queue.len(), 1);
    let mut bits = BitReader::with_profile(&queue[0], engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(MOTOR_POOL & 0xffff));
    for _ in 0..18 {
        assert_eq!(bits.read(1), Ok(0));
    }
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(8), Ok(0x13));
    assert_eq!(bits.read(16), Ok(u32::from(quarters)));
    assert_eq!(bits.read(32), Ok(0x0380_0000));
    assert_eq!(bits.read(32), Ok(u32::MAX));
    assert_eq!(bits.read(32), Ok(181));
    for _ in 19..30 {
        assert_eq!(bits.read(1), Ok(0));
    }
    assert_eq!(bits.read(1), Ok(0));

    let before = engine.ground_runtime.clone();
    dispatch_hit(&mut engine, &native_hit(APC, 182), transport, 3301).unwrap();
    let record = last_record(&engine);
    assert_eq!(record["gameplay_denial"], Value::Null, "{record}");
    assert_eq!(record["decoded"]["amount_hp"].as_f64(), Some(0.));
    assert_eq!(engine.ground_runtime, before);
    assert_eq!(engine.active.as_ref().unwrap().pending_shared.len(), 1);
}

fn helo_fixture(hospital: bool) -> Engine<Vec<u8>> {
    let mut engine = domination(hospital);
    install_helo_fixture(
        &mut engine,
        CharacterPosition {
            x: 2.,
            y: 2.,
            z: 1.,
        },
    );
    engine.active.as_mut().unwrap().helo_creation_sent = true;
    engine
}

fn helo_hull(engine: &Engine<Vec<u8>>) -> HullDamage {
    engine
        .domination_gameplay
        .as_ref()
        .unwrap()
        .state
        .helo
        .as_ref()
        .unwrap()
        .carriers[0]
        .damage
}

fn check_mounted_helo_primary(
    hospital: bool,
    transport: Transport,
    weapon: u16,
    amount: f64,
    health: u32,
) {
    let mut engine = helo_fixture(hospital);
    let observer = manual_ground_repair_domination_tests::add_peer(&mut engine, 1, 0, 0);
    let waiting = manual_ground_repair_domination_tests::add_peer(&mut engine, 2, 0, 0);
    engine.peers.get_mut(&observer).unwrap().helo_creation_sent = true;
    assert!(!engine.peers[&waiting].helo_creation_sent);
    assert_eq!(engine.active.as_ref().unwrap().team(), u8::from(hospital));
    assert_eq!(helo_hull(&engine).wire.health_quarter_points, 400);
    let wire = native_hit(HELO, 181);
    let xp = engine.active.as_ref().unwrap().match_experience;
    dispatch_hit(&mut engine, &wire, transport, 3300).unwrap();

    let record = last_record(&engine);
    assert_eq!(record["gameplay_denial"], Value::Null, "{record}");
    assert_eq!(record["decoded"]["type"], "domination_helo_bullet_hit");
    assert_eq!(record["decoded"]["weapon_item"], weapon);
    assert_eq!(record["decoded"]["amount_hp"], amount);
    assert_eq!(record["rewards"], json!([]));
    assert_eq!(engine.active.as_ref().unwrap().match_experience, xp);
    let damage = helo_hull(&engine).wire;
    assert_eq!(u32::from(damage.health_quarter_points), health);
    assert_eq!(damage.last_location_and_type, 0x13);
    assert_eq!(damage.last_attacker, 0x0380_0000);
    assert_eq!(damage.last_source_object, u32::MAX);
    assert_eq!(damage.last_user_data, 181);

    let actor = engine.active.as_ref().unwrap();
    let packet = &actor.pending_shared[0];
    let update = ObjectUpdate::decode_with_profile(packet, engine.wire_profile).unwrap();
    assert_eq!(update.header.object_type_mask, 1 << 2);
    let mut bits = BitReader::with_profile(packet, engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(HELO & 0xffff));
    for _ in 0..18 {
        assert_eq!(bits.read(1), Ok(0));
    }
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(8), Ok(0x13));
    assert_eq!(bits.read(16), Ok(health));
    assert_eq!(bits.read(32), Ok(0x0380_0000));
    assert_eq!(bits.read(32), Ok(u32::MAX));
    assert_eq!(bits.read(32), Ok(181));
    for _ in 19..27 {
        assert_eq!(bits.read(1), Ok(0));
    }
    assert_eq!(bits.read(1), Ok(0));
    assert_eq!(engine.peers[&observer].pending_shared.front(), Some(packet));
    assert!(engine.peers[&waiting].pending_shared.is_empty());
}

#[test]
fn hospital_helo_primary_applies_vehicle_damage_and_follows_helo_creation() {
    check_mounted_helo_primary(true, Transport::Tcp, 4, 3., 388);
}

#[test]
fn apc_helo_primary_applies_friendly_vehicle_damage_after_creation() {
    for transport in [Transport::Tcp, Transport::Udp] {
        // Synthetic data uses 50 HP, a 0.25 friendly factor, armor rating 1,
        // and 0.5 prevention at the chosen body location.
        check_mounted_helo_primary(false, transport, 5, 5.75, 377);
    }
}

#[test]
fn hospital_helo_primary_accepts_initialized_udp_hit() {
    let mut engine = helo_fixture(true);
    dispatch_hit(&mut engine, &native_hit(HELO, 181), Transport::Udp, 3300).unwrap();
    let record = last_record(&engine);
    assert_eq!(record["gameplay_denial"], Value::Null, "{record}");
    assert_eq!(helo_hull(&engine).wire.health_quarter_points, 388);
}

#[test]
fn hospital_helo_primary_retains_round_grant_and_target_guards() {
    let mut engine = helo_fixture(true);
    let hit = native_hit(HELO, 183);
    let mut secondary = hit.clone();
    secondary[2] = 1;
    let mut wrong_mode = hit.clone();
    wrong_mode[3] = 1;
    for invalid in [secondary, wrong_mode] {
        dispatch_hit(&mut engine, &invalid, Transport::Tcp, 3300).unwrap();
        assert_eq!(
            last_record(&engine)["gameplay_denial"],
            "unsupported_hospital_turret_fire_mode"
        );
    }
    dispatch_hit(
        &mut engine,
        &native_hit(HELO | 7, 184),
        Transport::Tcp,
        3301,
    )
    .unwrap();
    assert_eq!(
        last_record(&engine)["gameplay_denial"],
        "unknown_domination_helo_bullet_target"
    );
    engine.match_started = false;
    dispatch_hit(&mut engine, &hit, Transport::Tcp, 3302).unwrap();
    assert_eq!(
        last_record(&engine)["gameplay_denial"],
        "hospital_primary_requires_active_round"
    );
    engine.match_started = true;
    engine
        .domination_gameplay
        .as_mut()
        .unwrap()
        .state
        .helo
        .as_mut()
        .unwrap()
        .carriers[0]
        .published_state = domination_helo::FlightState::Dormant;
    dispatch_hit(&mut engine, &hit, Transport::Tcp, 3303).unwrap();
    assert_eq!(helo_hull(&engine).wire.health_quarter_points, 400);
    assert!(engine.active.as_ref().unwrap().pending_shared.is_empty());

    let mut stale = helo_fixture(true);
    stale.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 2,
        health_quarter_points: 400,
    };
    dispatch_hit(&mut stale, &hit, Transport::Tcp, 3304).unwrap();
    assert_eq!(
        last_record(&stale)["gameplay_denial"],
        "hospital_turret_grant_required"
    );
    assert_eq!(helo_hull(&stale).wire.health_quarter_points, 400);
    assert!(stale.active.as_ref().unwrap().pending_shared.is_empty());

    let mut passenger = helo_fixture(true);
    passenger.active.as_mut().unwrap().mounted_seat = Some(1);
    dispatch_hit(&mut passenger, &hit, Transport::Udp, 3305).unwrap();
    assert_eq!(
        last_record(&passenger)["gameplay_denial"],
        "vehicle_hit_requires_living_onfoot_actor"
    );
    assert_eq!(helo_hull(&passenger).wire.health_quarter_points, 400);
}

#[test]
fn hospital_helo_primary_refused_record_keeps_health_and_publication() {
    let mut engine = helo_fixture(true);
    let observer = manual_ground_repair_domination_tests::add_peer(&mut engine, 1, 0, 0);
    engine.peers.get_mut(&observer).unwrap().helo_creation_sent = true;
    engine.max_log = engine.written;
    let before = engine.writer.clone();
    assert!(matches!(
        dispatch_hit(&mut engine, &native_hit(HELO, 185), Transport::Tcp, 3300),
        Err("log_limit_reached")
    ));
    assert_eq!(engine.writer, before);
    assert_eq!(helo_hull(&engine).wire.health_quarter_points, 400);
    assert!(engine.active.as_ref().unwrap().pending_shared.is_empty());
    assert!(engine.peers[&observer].pending_shared.is_empty());
}

#[test]
fn apc_ground_primary_uses_ground_armor_and_records_attacker() {
    check_mounted_ground_primary(false, Transport::Tcp, 5, 4.5, 582);
}

#[test]
fn hospital_ground_primary_uses_ground_armor_and_friendly_fire() {
    check_mounted_ground_primary(true, Transport::Udp, 4, 0.54, 597);
}

#[test]
fn mounted_ground_primary_preserves_grant_round_and_record_guards() {
    for hospital in [false, true] {
        let mut engine = domination(hospital);
        let hit = native_hit(MOTOR_POOL, 181);
        let mut wrong_mode = hit.clone();
        wrong_mode[3] = 1;
        let mut secondary = hit.clone();
        secondary[2] = 1;
        for invalid in [&wrong_mode, &secondary] {
            dispatch_hit(&mut engine, invalid, Transport::Tcp, 3300).unwrap();
            let record = last_record(&engine);
            assert_eq!(
                record["gameplay_denial"],
                if hospital {
                    "unsupported_hospital_turret_fire_mode"
                } else {
                    "unsupported_ground_turret_fire_mode"
                },
                "{record}"
            );
        }
        engine.match_started = false;
        dispatch_hit(&mut engine, &hit, Transport::Tcp, 3301).unwrap();
        assert_eq!(
            last_record(&engine)["gameplay_denial"],
            if hospital {
                "hospital_primary_requires_active_round"
            } else {
                "ground_apc_primary_requires_active_round"
            }
        );
        engine.match_started = true;
        assert_eq!(target(&engine, MOTOR_POOL).health_quarter_points, 600);
        let before = engine.ground_runtime.clone();
        let pending = engine.active.as_ref().unwrap().pending_shared.clone();
        let recorded = engine.written;
        engine.max_log = recorded;
        assert!(matches!(
            dispatch_hit(&mut engine, &hit, Transport::Tcp, 3302),
            Err("log_limit_reached")
        ));
        assert_eq!(engine.ground_runtime, before);
        assert_eq!(engine.active.as_ref().unwrap().pending_shared, pending);
        assert_eq!(engine.written, recorded);

        let mut stale = domination(hospital);
        stale.active.as_mut().unwrap().life = CharacterLife::Alive {
            generation: 2,
            health_quarter_points: 400,
        };
        dispatch_hit(&mut stale, &hit, Transport::Udp, 3303).unwrap();
        let record = last_record(&stale);
        assert_eq!(
            record["gameplay_denial"],
            if hospital {
                "hospital_turret_grant_required"
            } else {
                "ground_turret_requires_living_exclusive_gunner"
            },
            "{record}"
        );
        assert_eq!(target(&stale, MOTOR_POOL).health_quarter_points, 600);

        let mut passenger = domination(hospital);
        let actor = passenger.active.as_mut().unwrap();
        if hospital {
            actor.mounted_seat = Some(1);
        } else {
            actor.ground_seat.as_mut().unwrap().seat = 2;
        }
        dispatch_hit(&mut passenger, &hit, Transport::Udp, 3304).unwrap();
        assert_eq!(
            last_record(&passenger)["gameplay_denial"],
            "ground_hit_requires_living_onfoot_actor"
        );
        assert_eq!(target(&passenger, MOTOR_POOL).health_quarter_points, 600);
    }
}

#[test]
fn acquisition_ground_hits_accept_ordinary_tcp_and_a_qualified_hospital_primary() {
    let hit = native_hit(ESCORT, 185);
    let mut engine = ground_firearm_fixture(100.125, false);
    dispatch_hit(&mut engine, &hit, Transport::Tcp, 3300).unwrap();
    let record = last_record(&engine);
    assert_eq!(record["gameplay_denial"], Value::Null, "{record}");
    assert_eq!(record["decoded"]["weapon_item"], 2);
    assert_eq!(target(&engine, ESCORT).health_quarter_points, 199);

    // A bound Acquisition field-hospital gunner reaches the same Ground
    // targets as an on-foot firearm, with its projected primary.
    let mut mounted = ground_firearm_fixture(100.125, false);
    mounted.item_catalog = Arc::new(
        Arc::try_unwrap(combat_items_for_ground_test(50.))
            .unwrap()
            .with_retail_combat(combat_catalog()),
    );
    mounted.bunker_world = domination_bunker_seat_tests::fixture_for_mode(1).bunker_world;
    enable_synthetic_hospital_turrets(&mut mounted);
    equip_synthetic_hospital_turret(&mut mounted, 5);
    assert!(
        mounted
            .hospital_primary_for_actor(
                mounted.active.as_ref().unwrap(),
                &bullet_hit::NetMessageHit::decode(&hit).unwrap(),
            )
            .is_ok()
    );
    dispatch_hit(&mut mounted, &hit, Transport::Udp, 3300).unwrap();
    let record = last_record(&mounted);
    assert_eq!(record["gameplay_denial"], Value::Null, "{record}");
    assert_eq!(record["decoded"]["weapon_item"], 4);
    // The invented hospital round deals 12 vehicle HP: 48 quarter points.
    assert_eq!(record["decoded"]["amount_hp"].as_f64(), Some(12.));
    assert_eq!(target(&mounted, ESCORT).health_quarter_points, 552);

    // Without a turret grant the mounted actor stays refused.
    let mut unbound = ground_firearm_fixture(100.125, false);
    unbound.bunker_world = domination_bunker_seat_tests::fixture_for_mode(1).bunker_world;
    let actor = unbound.active.as_mut().unwrap();
    actor.mounted_bunker = Some(5);
    actor.mounted_seat = Some(0);
    dispatch_hit(&mut unbound, &hit, Transport::Udp, 3300).unwrap();
    assert_ne!(last_record(&unbound)["gameplay_denial"], Value::Null);
    assert_eq!(target(&unbound, ESCORT).health_quarter_points, 600);
}

#[test]
fn apc_helo_primary_rejects_invalid_gunner_and_hull_state() {
    let hit = native_hit(HELO, 186);
    let mut engine = helo_fixture(false);
    for byte in [2, 3] {
        let mut wrong = hit.clone();
        wrong[byte] = 1;
        dispatch_hit(&mut engine, &wrong, Transport::Tcp, 3300).unwrap();
        assert_eq!(
            last_record(&engine)["gameplay_denial"],
            "unsupported_ground_turret_fire_mode"
        );
    }
    dispatch_hit(
        &mut engine,
        &native_hit(HELO | 7, 186),
        Transport::Tcp,
        3301,
    )
    .unwrap();
    assert_eq!(
        last_record(&engine)["gameplay_denial"],
        "unknown_domination_helo_bullet_target"
    );
    engine.match_started = false;
    dispatch_hit(&mut engine, &hit, Transport::Tcp, 3302).unwrap();
    assert_eq!(
        last_record(&engine)["gameplay_denial"],
        "ground_apc_primary_requires_active_round"
    );
    engine.match_started = true;
    let (uninitialized, _) = engine
        .ground_turret_seat_change(engine.active.as_ref().unwrap(), APC as u16, true)
        .unwrap();
    engine
        .ground_runtime
        .as_mut()
        .unwrap()
        .install_turret(APC as u16, uninitialized);
    dispatch_hit(&mut engine, &hit, Transport::Udp, 3303).unwrap();
    assert_eq!(
        last_record(&engine)["gameplay_denial"],
        "ground_turret_primary_not_initialized"
    );
    equip_synthetic_ground_turret(&mut engine, APC as u16);
    engine.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 2,
        health_quarter_points: 400,
    };
    dispatch_hit(&mut engine, &hit, Transport::Udp, 3304).unwrap();
    assert_eq!(
        last_record(&engine)["gameplay_denial"],
        "ground_turret_requires_living_exclusive_gunner"
    );
    equip_synthetic_ground_turret(&mut engine, APC as u16);
    engine
        .active
        .as_mut()
        .unwrap()
        .ground_seat
        .as_mut()
        .unwrap()
        .seat = 0;
    dispatch_hit(&mut engine, &hit, Transport::Udp, 3305).unwrap();
    assert_eq!(
        last_record(&engine)["gameplay_denial"],
        "vehicle_hit_requires_living_onfoot_actor"
    );

    equip_synthetic_ground_turret(&mut engine, APC as u16);
    engine
        .domination_gameplay
        .as_mut()
        .unwrap()
        .state
        .helo
        .as_mut()
        .unwrap()
        .carriers[0]
        .published_state = domination_helo::FlightState::Dormant;
    dispatch_hit(&mut engine, &hit, Transport::Tcp, 3306).unwrap();
    assert_eq!(helo_hull(&engine).wire.health_quarter_points, 400);
    assert!(engine.active.as_ref().unwrap().pending_shared.is_empty());
}

#[test]
fn apc_helo_primary_refused_record_keeps_hull_and_recipients_unchanged() {
    let mut engine = helo_fixture(false);
    let observer = manual_ground_repair_domination_tests::add_peer(&mut engine, 1, 0, 0);
    engine.peers.get_mut(&observer).unwrap().helo_creation_sent = true;
    let before = engine.writer.clone();
    engine.max_log = engine.written;
    assert!(matches!(
        dispatch_hit(&mut engine, &native_hit(HELO, 187), Transport::Tcp, 3300),
        Err("log_limit_reached")
    ));
    assert_eq!(engine.writer, before);
    assert_eq!(helo_hull(&engine).wire.health_quarter_points, 400);
    assert!(engine.active.as_ref().unwrap().pending_shared.is_empty());
    assert!(engine.peers[&observer].pending_shared.is_empty());
}

#[test]
fn ordinary_helo_primary_uses_both_authenticated_hit_transports() {
    for transport in [Transport::Tcp, Transport::Udp] {
        let mut engine = helo_fixture(false);
        let defend = Arc::new(
            engine
                .spawn_catalog
                .as_ref()
                .unwrap()
                .for_squad(16)
                .unwrap(),
        );
        let actor = engine.active.as_mut().unwrap();
        actor.ground_seat = None;
        actor.squad_catalog = defend;
        let inventory = actor.current_inventory_handle.unwrap();
        actor.spawned_loadout.as_mut().unwrap().inventories
            [usize::from(inventory.inventory_index)]
        .slots[0] = Some(2);
        actor.observed_primary = Some(replication::InventoryItem {
            inventory_update_id: inventory.update_id,
            gear_slot: 0,
            ammo_pool: 0,
            ammo_count: 9,
            reloading: false,
        });
        actor.primary_equipped = true;
        let hit = native_hit(HELO, 188);
        dispatch_hit(&mut engine, &hit, transport, 3300).unwrap();
        let record = last_record(&engine);
        assert_eq!(record["gameplay_denial"], Value::Null, "{record}");
        assert_eq!(record["decoded"]["weapon_item"], 2);
        assert_eq!(record["decoded"]["amount_hp"], 24.5);
        assert_eq!(helo_hull(&engine).wire.health_quarter_points, 302);
    }
}

#[test]
fn acquisition_escort_primary_does_not_enter_domination_helo_damage() {
    let mut engine = ground_firearm_fixture(100.125, false);
    opt_in(&mut engine, ESCORT as u16);
    equip_synthetic_ground_turret(&mut engine, ESCORT as u16);
    let hit = native_hit(HELO, 189);
    dispatch_hit(&mut engine, &hit, Transport::Tcp, 3300).unwrap();
    assert_eq!(
        last_record(&engine)["gameplay_denial"],
        "ground_escort_primary_requires_supported_target"
    );
}

const NOTICE_VICTIM: &str = "synthetic-mounted-notice-target";

fn mounted_notice_fixture(item: u16) -> Engine<Vec<u8>> {
    mounted_notice_fixture_with_drivers(item, false)
}

fn mounted_notice_fixture_with_drivers(item: u16, drivers: bool) -> Engine<Vec<u8>> {
    let mut engine = match item {
        4 => domination(true),
        5 => domination_with_drivers(false, drivers),
        6 => {
            let mut engine = ground_firearm_fixture(50., true);
            engine.item_catalog = Arc::new(
                Arc::try_unwrap(combat_items_for_ground_test(50.))
                    .unwrap()
                    .with_retail_combat(combat_catalog()),
            );
            let faction = engine
                .current_ground_vehicle(ESCORT as u16)
                .unwrap()
                .faction;
            let batch = engine.ground_vehicles.as_ref().unwrap().batch_count();
            let actor = engine.active.as_mut().unwrap();
            actor.reservation.character.faction = faction;
            actor.ground_vehicle_batch = batch;
            opt_in_primary(&mut engine, ESCORT as u16, item);
            equip_synthetic_ground_turret(&mut engine, ESCORT as u16);
            engine
        }
        _ => panic!("unsupported synthetic notice role"),
    };
    let mut donor = human_kill_reward_tests::fixture();
    engine.incapacitation_rules = donor.incapacitation_rules.take();
    let mut victim = donor.peers.remove("synthetic-reservation-0000").unwrap();
    let actor = engine.active.as_mut().unwrap();
    actor.reservation.character.account_id = Some("synthetic-mounted-notice-account".into());
    actor.reservation.character.character_id = Some(10);
    victim.reservation.player = 2;
    victim.reservation.id = NOTICE_VICTIM.into();
    victim.reservation.world = actor.reservation.world;
    victim.squad_catalog = Arc::new(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(u16::from(1 - actor.team()) * 16)
            .unwrap(),
    );
    victim.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 20,
    };
    engine.peers.insert(NOTICE_VICTIM.into(), victim);
    engine
}

fn death_notice_bytes(actor: &Active) -> Vec<&[u8]> {
    actor
        .pending_notifications
        .iter()
        .filter_map(|publication| match publication {
            match_flow::Publication::Application(packet) if packet.starts_with(&[1, 255]) => {
                Some(packet.as_slice())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn mounted_primary_death_notifies_both_squads_once_using_the_held_weapon() {
    for item in [4, 5, 6] {
        let mut engine = mounted_notice_fixture(item);
        let actor = engine.active.as_ref().unwrap();
        let attacker = actor.reservation.player.to_be_bytes();
        let hit = native_hit(0x0380_0002, 190);
        let notice = [
            1,
            255,
            attacker[0],
            attacker[1],
            0,
            2,
            2,
            0,
            0,
            0,
            item as u8,
            0x80,
        ];
        dispatch_hit(&mut engine, &hit, Transport::Tcp, 3300).unwrap();
        let record = last_record(&engine);
        assert_eq!(
            record["gameplay_denial"],
            Value::Null,
            "item {item}: {record}"
        );
        assert_eq!(record["decoded"]["weapon_item"], item, "{record}");
        assert!(record["bot_firearm_kd"].is_null());
        assert!(record["firearm_assists"].is_null());
        assert!(matches!(
            engine.peers[NOTICE_VICTIM].life,
            CharacterLife::Incapacitated { generation: 4, .. }
        ));
        assert_eq!(
            death_notice_bytes(engine.active.as_ref().unwrap()),
            vec![notice.as_slice()],
            "item {item}"
        );
        assert_eq!(
            death_notice_bytes(&engine.peers[NOTICE_VICTIM]),
            vec![notice.as_slice()]
        );
        assert_eq!(
            engine.active.as_ref().unwrap().reward_statistics.get(&10),
            Some(&1)
        );
        assert_eq!(
            engine.peers[NOTICE_VICTIM].reward_statistics.get(&12),
            Some(&1)
        );
        let kills = |engine: &Engine<Vec<u8>>| {
            engine
                .pending_reward_batch()
                .into_iter()
                .filter(|reward| reward.kind == progression::RewardKind::EnemyKilled)
                .count()
        };
        assert_eq!(kills(&engine), 1);
        dispatch_hit(&mut engine, &hit, Transport::Udp, 3301).unwrap();
        assert_eq!(kills(&engine), 1);
        assert_eq!(death_notice_bytes(&engine.peers[NOTICE_VICTIM]).len(), 1);
        assert_eq!(
            engine.peers[NOTICE_VICTIM].reward_statistics.get(&12),
            Some(&1)
        );
    }
}

#[test]
fn mounted_primary_death_refused_record_keeps_the_target_alive_and_notices_empty() {
    let mut engine = mounted_notice_fixture(5);
    let rewards = engine.pending_reward_batch();
    let writer = engine.writer.clone();
    engine.max_log = engine.written;
    assert!(matches!(
        dispatch_hit(
            &mut engine,
            &native_hit(0x0380_0002, 191),
            Transport::Tcp,
            3300
        ),
        Err("log_limit_reached")
    ));
    assert_eq!(engine.writer, writer);
    assert!(matches!(
        engine.peers[NOTICE_VICTIM].life,
        CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 20
        }
    ));
    assert_eq!(engine.pending_reward_batch(), rewards);
    assert!(death_notice_bytes(engine.active.as_ref().unwrap()).is_empty());
    assert!(death_notice_bytes(&engine.peers[NOTICE_VICTIM]).is_empty());
}

#[test]
fn mounted_primary_death_requires_the_current_gunner_grant() {
    let mut engine = mounted_notice_fixture(5);
    engine.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 2,
        health_quarter_points: 400,
    };
    dispatch_hit(
        &mut engine,
        &native_hit(0x0380_0002, 192),
        Transport::Tcp,
        3300,
    )
    .unwrap();
    assert_eq!(
        last_record(&engine)["gameplay_denial"],
        "ground_turret_requires_living_exclusive_gunner"
    );
    assert!(matches!(
        engine.peers[NOTICE_VICTIM].life,
        CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 20
        }
    ));
    assert!(death_notice_bytes(engine.active.as_ref().unwrap()).is_empty());
    assert!(death_notice_bytes(&engine.peers[NOTICE_VICTIM]).is_empty());
}

const ASSIST_DRIVER: &str = "synthetic-ground-assist-driver";

fn mounted_assist_fixture(item: u16) -> Engine<Vec<u8>> {
    let mut engine = mounted_notice_fixture_with_drivers(item, true)
        .with_firearm_assist_rules(firearm_assists::Rules {
            schema_version: 1,
            application_id: 20231,
            assist_timeout_ms: 5_000,
        })
        .unwrap();
    let gunner = engine.active.as_mut().unwrap();
    gunner.owner_fields.state = Some(gunner.ground_seat.unwrap().character_state());
    let mut driver = human_kill_reward_tests::fixture().active.take().unwrap();
    let gunner = engine.active.as_ref().unwrap();
    let seat = ground_seats::PassengerSeat {
        vehicle_index: gunner.ground_seat.unwrap().vehicle_index,
        seat: 0,
    };
    driver.reservation.id = ASSIST_DRIVER.into();
    driver.reservation.player = 3;
    driver.reservation.world = gunner.reservation.world;
    driver.reservation.character.faction = gunner.reservation.character.faction;
    driver.reservation.character.assist_rewards_enabled = Some(true);
    driver.squad_catalog = gunner.squad_catalog.clone();
    driver.ground_vehicle_batch = gunner.ground_vehicle_batch;
    driver.ground_seat = Some(seat);
    driver.owner_fields.state = Some(seat.character_state());
    assert!(engine.ground_driver_authority(&driver, seat.vehicle_index));
    engine.peers.insert(ASSIST_DRIVER.into(), driver);
    engine
}

#[test]
fn ground_primary_first_collapse_awards_current_driver_once() {
    for item in [5, 6] {
        let mut engine = mounted_assist_fixture(item);
        let hit = native_hit(0x0380_0002, 190);
        dispatch_hit(&mut engine, &hit, Transport::Tcp, 3300).unwrap();
        assert_eq!(
            engine.peers[ASSIST_DRIVER].reward_statistics.get(&11),
            Some(&1),
            "item {item}"
        );
        assert_eq!(
            engine.active.as_ref().unwrap().reward_statistics.get(&10),
            Some(&1)
        );
        assert_eq!(
            engine.peers[NOTICE_VICTIM].reward_statistics.get(&12),
            Some(&1)
        );
        assert_eq!(
            engine
                .pending_reward_batch()
                .iter()
                .filter(
                    |reward| reward.kind == progression::RewardKind::EnemyKillAssist
                        && reward.player_id == 3
                )
                .count(),
            1
        );
        dispatch_hit(&mut engine, &hit, Transport::Udp, 3301).unwrap();
        assert_eq!(
            engine.peers[ASSIST_DRIVER].reward_statistics.get(&11),
            Some(&1)
        );
    }
}

#[test]
fn ground_primary_assists_preserve_prior_contributors_and_record_atomicity() {
    let mut engine = mounted_assist_fixture(5);
    let mut prior = human_kill_reward_tests::fixture().active.take().unwrap();
    let driver = &engine.peers[ASSIST_DRIVER];
    prior.reservation.id = "synthetic-earlier-ground-contributor".into();
    prior.reservation.player = 4;
    prior.reservation.world = driver.reservation.world;
    prior.reservation.character.faction = driver.reservation.character.faction;
    prior.squad_catalog = driver.squad_catalog.clone();
    let prior_id = prior.reservation.id.clone();
    engine.peers.insert(prior_id.clone(), prior);
    let seat = engine
        .peers
        .get_mut(ASSIST_DRIVER)
        .unwrap()
        .ground_seat
        .take();
    for id in [&prior_id, &ASSIST_DRIVER.to_owned()] {
        let source = &engine.peers[id];
        let victim = &engine.peers[NOTICE_VICTIM];
        let ordinary = firearm_assists::Plan::prepare(
            &engine,
            source.reservation.world,
            Duration::from_millis(1_000),
            firearm_assists::Fighter::human(source),
            firearm_assists::Fighter::human(victim),
            false,
        )
        .unwrap();
        ordinary.install(&mut engine).unwrap();
    }
    engine.peers.get_mut(ASSIST_DRIVER).unwrap().ground_seat = seat;
    let hit = native_hit(0x0380_0002, 190);
    dispatch_hit(&mut engine, &hit, Transport::Tcp, 3300).unwrap();
    assert_eq!(
        engine.peers[ASSIST_DRIVER].reward_statistics.get(&11),
        Some(&1)
    );
    assert_eq!(engine.peers[&prior_id].reward_statistics.get(&11), Some(&1));

    let mut refused = mounted_assist_fixture(5);
    refused.max_log = refused.written;
    assert!(dispatch_hit(&mut refused, &hit, Transport::Tcp, 3300).is_err());
    assert!(refused.peers[NOTICE_VICTIM].life.is_alive());
    assert!(
        !refused.peers[ASSIST_DRIVER]
            .reward_statistics
            .contains_key(&11)
    );
    assert!(refused.pending_reward_batch().is_empty());
}

#[test]
fn ground_primary_denies_an_unassociated_or_friendly_driver_assist() {
    for friendly in [false, true] {
        let mut engine = mounted_assist_fixture(5);
        if friendly {
            engine.peers.get_mut(NOTICE_VICTIM).unwrap().squad_catalog =
                engine.active.as_ref().unwrap().squad_catalog.clone();
        } else {
            engine.peers.get_mut(ASSIST_DRIVER).unwrap().ground_seat = None;
        }
        dispatch_hit(
            &mut engine,
            &native_hit(0x0380_0002, 190),
            Transport::Tcp,
            3300,
        )
        .unwrap();
        assert!(
            !engine.peers[ASSIST_DRIVER]
                .reward_statistics
                .contains_key(&11)
        );
    }
}
