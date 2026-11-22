use super::*;
use crate::{
    domination_helo_damage::HullDamage,
    domination_structural_world::hospital_rocket_helo_damage::tests::install_helo_fixture,
    events::EventInput,
    ground_turrets::tests::{equip_synthetic_ground_turret, opt_in},
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
    let (faction, team) = if hospital { (2, 1) } else { (0, 0) };
    let mut engine = manual_ground_repair_domination_tests::fixture(faction, team);
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

#[test]
fn hospital_helo_primary_applies_vehicle_damage_and_follows_helo_creation() {
    let mut engine = helo_fixture(true);
    let observer = manual_ground_repair_domination_tests::add_peer(&mut engine, 1, 0, 0);
    let waiting = manual_ground_repair_domination_tests::add_peer(&mut engine, 2, 0, 0);
    engine.peers.get_mut(&observer).unwrap().helo_creation_sent = true;
    assert!(!engine.peers[&waiting].helo_creation_sent);
    assert_eq!(engine.active.as_ref().unwrap().team(), 1);
    assert_eq!(helo_hull(&engine).wire.health_quarter_points, 400);
    let wire = native_hit(HELO, 181);
    dispatch_hit(&mut engine, &wire, Transport::Tcp, 3300).unwrap();

    let record = last_record(&engine);
    assert_eq!(record["gameplay_denial"], Value::Null, "{record}");
    assert_eq!(record["decoded"]["type"], "domination_helo_bullet_hit");
    assert_eq!(record["decoded"]["weapon_item"], 4);
    assert_eq!(record["decoded"]["amount_hp"], 3.0);
    let damage = helo_hull(&engine).wire;
    assert_eq!(damage.health_quarter_points, 388);
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
    assert_eq!(bits.read(16), Ok(388));
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

    let mut apc = helo_fixture(false);
    dispatch_hit(&mut apc, &hit, Transport::Udp, 3306).unwrap();
    assert_eq!(
        last_record(&apc)["gameplay_denial"],
        "ground_apc_primary_requires_character_target"
    );
    assert_eq!(helo_hull(&apc).wire.health_quarter_points, 400);
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
fn ground_primary_keeps_acquisition_onfoot_only_and_accepts_ordinary_tcp_hits() {
    let hit = native_hit(ESCORT, 185);
    let mut engine = ground_firearm_fixture(100.125, false);
    dispatch_hit(&mut engine, &hit, Transport::Tcp, 3300).unwrap();
    let record = last_record(&engine);
    assert_eq!(record["gameplay_denial"], Value::Null, "{record}");
    assert_eq!(record["decoded"]["weapon_item"], 2);
    assert_eq!(target(&engine, ESCORT).health_quarter_points, 199);

    let mut mounted = ground_firearm_fixture(100.125, false);
    mounted.bunker_world = domination_bunker_seat_tests::fixture().bunker_world;
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
    assert_eq!(
        last_record(&mounted)["gameplay_denial"],
        "ground_hit_requires_living_onfoot_actor"
    );
    assert_eq!(target(&mounted, ESCORT).health_quarter_points, 600);
}
