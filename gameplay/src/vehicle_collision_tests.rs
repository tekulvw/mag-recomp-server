//! Synthetic ground-vehicle impacts on characters: a Domination APC driver
//! reports touching a character and the server applies the damage.
use super::*;
use crate::vehicle_collision::synthetic_rules;

const APC: u32 = 0x0100_0000;

fn report(engine: &mut Engine<Vec<u8>>, kind: u8, vehicle: u32, target: u32, now: u64) {
    let mut wire = 0x020du16.to_be_bytes().to_vec();
    wire.push(kind);
    wire.extend(vehicle.to_be_bytes());
    wire.extend(target.to_be_bytes());
    dispatch(engine, &wire, now).unwrap();
}

fn last_event(engine: &Engine<Vec<u8>>) -> Value {
    std::str::from_utf8(&engine.writer)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .rfind(|entry| entry["op"] == "event")
        .unwrap()
}

/// Armor the synthetic combat catalog fully resolves (rating 0, multiplier 1).
fn arm(actor: &mut Active, items: &item_catalog::ItemCatalog) {
    actor
        .reservation
        .character
        .loadouts
        .as_mut()
        .unwrap()
        .inventories[0][12] = Some(3);
    actor.current_inventory_handle = Some(replication::InventoryHandle {
        update_id: 1,
        inventory_index: 0,
    });
    actor.spawned_loadout = Some(
        actor
            .reservation
            .character
            .replication_loadout(items, 0, actor.wire_profile)
            .unwrap(),
    );
}

fn reservation(player: u16) -> String {
    format!("synthetic-reservation-{player:04}")
}

fn actor(engine: &mut Engine<Vec<u8>>, player: u16) -> &mut Active {
    let id = reservation(player);
    if engine.active.as_ref().unwrap().reservation.id == id {
        engine.active.as_mut().unwrap()
    } else {
        engine.peers.get_mut(&id).unwrap()
    }
}

/// Player 0 drives APC 0 (team 0) at 10 m/s along +x. Player 1 is on foot
/// with `faction` 1 (the APC's side) or 2 (the other side).
fn scene(victim_faction: u8, victim_hp: u16) -> Engine<Vec<u8>> {
    let mut engine = driver_engine(0);
    engine.item_catalog = crate::human_kill_reward_tests::combat_items_for_ground_test(0.0);
    engine.vehicle_collision_rules = Some(synthetic_rules());
    engine.match_started = true;
    let profile = engine.wire_profile;
    ready_on_foot(&mut engine, 0, 10);
    let driver = engine.active.as_mut().unwrap();
    driver.tracking.server_position(
        CharacterPosition {
            x: 100.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        760,
    );
    ground_seat_event(&mut engine, true, APC, 0, 0, 770);
    assert_eq!(engine.ground_driver_owner(0), Some(0));
    reserve_faction(&mut engine, 1, victim_faction);
    let items = engine.item_catalog.clone();
    let victim = engine.active.as_mut().unwrap();
    ready(victim);
    victim.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: victim_hp,
    };
    arm(victim, &items);
    focus(&mut engine, 0);
    let items = engine.item_catalog.clone();
    let driver = engine.active.as_mut().unwrap();
    arm(driver, &items);
    driver.reservation.character.account_id = Some("synthetic-driver-account".into());
    dispatch(&mut engine, &ground_driver_relay(profile, 0, 20, 3232), 840).unwrap();
    dispatch(&mut engine, &ground_driver_relay(profile, 0, 21, 3264), 940).unwrap();
    assert_eq!(engine.current_ground_vehicle(0).unwrap().position.x, 102.);
    engine
}

fn victim(engine: &Engine<Vec<u8>>) -> &Active {
    &engine.peers[&reservation(1)]
}

#[test]
fn a_moving_apc_hurts_an_enemy_by_closing_speed_and_credits_its_driver() {
    let mut engine = scene(2, 400);
    report(&mut engine, 1, APC, 0x0380_0001, 950);
    let event = last_event(&engine);
    assert_eq!(event["gameplay_denial"], Value::Null, "{event}");
    let decoded = &event["decoded"];
    assert_eq!(decoded["type"], "vehicle_report");
    assert_eq!(decoded["closing_speed_mps"], 10.0);
    // Synthetic rules: 20 HP per m/s above 4 m/s.
    assert_eq!(decoded["raw_damage_hp"], 120.0);
    assert_eq!(decoded["applied_damage"]["amount_hp"], 120.0);
    let damage = victim(&engine).last_damage;
    // ExternalKinetic (5) without KineticResponse, at location 0.
    assert_eq!(damage.last_location_and_type, 0x50);
    assert_eq!(damage.last_attacker, 0x0380_0000);
    assert_eq!(damage.last_source_object, APC);
    assert_eq!(damage.health_quarter_points, 0);
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::AwaitingRespawn { generation: 1 }
    ));
    // The driver gets the kill, its reward and the death notice names a vehicle.
    let driver = engine.active.as_ref().unwrap();
    assert_eq!(driver.reward_statistics.get(&10), Some(&1));
    assert!(
        engine
            .pending_reward_batch()
            .iter()
            .any(|reward| reward.kind == progression::RewardKind::EnemyKilled)
    );
    let notice = &decoded["death_notice"]["payload_hex"];
    let wire = protocol::decode_hex(notice.as_str().unwrap());
    // ID16, attacker16, defender16, source type8, source index32.
    assert_eq!(&wire[..7], &[0x01, 0xff, 0x00, 0x00, 0x00, 0x01, 0x03]);
    assert_eq!(&wire[7..11], &APC.to_be_bytes());
    for recipient in [engine.active.as_ref().unwrap(), victim(&engine)] {
        assert!(
            recipient
                .pending_notifications
                .iter()
                .any(|publication| matches!(
                    publication,
                    match_flow::Publication::Application(bytes) if *bytes == wire
                ))
        );
    }
}

#[test]
fn a_slow_collision_does_nothing_and_a_report_needs_the_driver() {
    let mut engine = scene(2, 400);
    let profile = engine.wire_profile;
    // Creep one fixed-point cell in 100 ms: far below the minimum speed.
    dispatch(
        &mut engine,
        &ground_driver_relay(profile, 0, 22, 3265),
        1040,
    )
    .unwrap();
    report(&mut engine, 1, APC, 0x0380_0001, 1050);
    let event = last_event(&engine);
    assert_eq!(
        event["decoded"]["outcome"], "below_minimum_speed",
        "{event}"
    );
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Alive {
            health_quarter_points: 400,
            ..
        }
    ));
    // An on-foot player cannot report somebody else's vehicle.
    focus(&mut engine, 1);
    report(&mut engine, 2, APC, 0x0380_0000, 1060);
    assert_eq!(
        last_event(&engine)["gameplay_denial"],
        "vehicle_report_requires_vehicle_driver"
    );
}

#[test]
fn motion_goes_stale_and_the_victim_running_into_the_vehicle_adds_speed() {
    let mut engine = scene(2, 400);
    // The latest vehicle sample is too old: it counts as parked.
    report(&mut engine, 1, APC, 0x0380_0001, 940 + 2501);
    assert_eq!(
        last_event(&engine)["decoded"]["outcome"],
        "below_minimum_speed"
    );
    let mut engine = scene(2, 400);
    // The victim runs at 2 m/s toward the vehicle (along -x).
    let victim = actor(&mut engine, 1);
    for (x, at) in [(110.0, 900), (109.8, 1000)] {
        victim.tracking.accept_client(
            CharacterPosition { x, y: 3., z: 100. },
            victim.tracking.accepted_position_sequence + 1,
            at,
        );
    }
    report(&mut engine, 1, APC, 0x0380_0001, 1000);
    let decoded = last_event(&engine)["decoded"].clone();
    let speed = decoded["closing_speed_mps"].as_f64().unwrap();
    assert!((speed - 12.0).abs() < 1e-3, "{decoded}");
}

#[test]
fn catastrophic_collisions_reach_teammates_only_with_friendly_fire_on() {
    for (multiplier, expected_raw, expected_amount) in [(0.5, 500.0, 250.0), (0.0, 250.0, 0.0)] {
        let mut engine = scene(1, 400);
        engine.character_damage_rules = Some(
            crate::character_damage_rules::Rules::from_json(
                &json!({"schema_version":1,"application_id":20231,"map_name":"synthetic",
                    "game_type":0,"friendly_fire_multiplier":multiplier,
                    "provenance":{"spawn_catalog_sha256":"a".repeat(64),"source":"synthetic"}})
                .to_string(),
            )
            .unwrap(),
        );
        assert_eq!(
            engine.active.as_ref().unwrap().team(),
            victim(&engine).team()
        );
        report(&mut engine, 2, APC, 0x0380_0001, 950);
        let event = last_event(&engine);
        let decoded = &event["decoded"];
        assert_eq!(decoded["raw_damage_hp"], expected_raw, "{event}");
        if expected_amount > 0.0 {
            assert_eq!(decoded["applied_damage"]["amount_hp"], expected_amount);
            assert_eq!(victim(&engine).last_damage.health_quarter_points, 0);
            // A friendly kill, not an enemy kill.
            let driver = engine.active.as_ref().unwrap();
            assert_eq!(driver.reward_statistics.get(&14), Some(&1));
            assert_eq!(driver.reward_statistics.get(&10), None);
        } else {
            assert_eq!(decoded["outcome"], "no_damage");
            assert!(victim(&engine).life.is_alive());
        }
    }
}

#[test]
fn a_lethal_impact_can_down_because_it_carries_no_kinetic_response() {
    let mut engine = scene(2, 400)
        .with_incapacitation_rules(incapacitation::Rules {
            schema_version: 1,
            incapacitated_damage_threshold_hp: 150.0,
            incapacitated_duration_seconds: 7.0,
            excluded_character_states: vec![],
            provenance: None,
        })
        .unwrap();
    report(&mut engine, 1, APC, 0x0380_0001, 950);
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Incapacitated { generation: 1, .. }
    ));
    // Above the threshold the same rule finishes the victim outright.
    let mut engine = scene(2, 400)
        .with_incapacitation_rules(incapacitation::Rules {
            schema_version: 1,
            incapacitated_damage_threshold_hp: 100.0,
            incapacitated_duration_seconds: 7.0,
            excluded_character_states: vec![],
            provenance: None,
        })
        .unwrap();
    report(&mut engine, 1, APC, 0x0380_0001, 950);
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::AwaitingRespawn { generation: 1 }
    ));
}

#[test]
fn a_character_that_just_left_the_vehicle_is_not_run_over_by_it() {
    let mut engine = scene(1, 400);
    actor(&mut engine, 1).ground_exit = Some((0, Duration::from_millis(800)));
    // Synthetic grace: 1.5 s after getting out.
    report(&mut engine, 2, APC, 0x0380_0001, 2299);
    assert_eq!(last_event(&engine)["decoded"]["outcome"], "recent_occupant");
    assert!(victim(&engine).life.is_alive());
    // Leaving a different vehicle grants no grace.
    actor(&mut engine, 1).ground_exit = Some((1, Duration::from_millis(800)));
    report(&mut engine, 2, APC, 0x0380_0001, 2299);
    assert!(!victim(&engine).life.is_alive());
}

#[test]
fn dismounting_records_which_vehicle_was_left_and_when() {
    let mut engine = scene(2, 400);
    assert_eq!(engine.active.as_ref().unwrap().ground_exit, None);
    ground_seat_event(&mut engine, false, APC, 0, 0, 980);
    let driver = engine.active.as_ref().unwrap();
    assert!(driver.ground_seat.is_none());
    assert_eq!(driver.ground_exit, Some((0, Duration::from_millis(980))));
}

#[test]
fn unsupported_reports_stay_visible() {
    let mut engine = scene(2, 400);
    report(&mut engine, 0, APC, 0x0380_0000, 950);
    assert_eq!(
        last_event(&engine)["gameplay_denial"],
        "vehicle_stuck_report_unsupported"
    );
    // Retail helicopters ignore character collisions.
    report(&mut engine, 1, 0x0200_0000, 0x0380_0001, 950);
    let event = last_event(&engine);
    assert_eq!(event["gameplay_denial"], Value::Null);
    assert_eq!(
        event["decoded"]["outcome"],
        "helicopter_collisions_have_no_effect"
    );
    report(&mut engine, 1, APC, 0x0380_0042, 950);
    assert_eq!(
        last_event(&engine)["gameplay_denial"],
        "unsupported_vehicle_collision_target"
    );
    engine.vehicle_collision_rules = None;
    report(&mut engine, 1, APC, 0x0380_0001, 950);
    assert_eq!(
        last_event(&engine)["gameplay_denial"],
        "vehicle_collision_rules_missing"
    );
    assert!(victim(&engine).life.is_alive());
}

/// One synthetic registry bot on the other side from the driver, at full health.
fn place_enemy_bot(engine: &mut Engine<Vec<u8>>) -> u16 {
    // Borrow one synthetic registry bot from the bot fixture world.
    let mut bots = crate::bot_registry::identity_tests::engine("Synthetic Impact Bot");
    let response = bots.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: "synthetic-reserve-bot-host".into(),
            reservation_id: "synthetic-bot-host".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic Host","faction":2,"head":0,
                    "voice":0,"universe_id":26,"character_id":19}))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    ready(bots.active.as_mut().unwrap());
    bots.reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let (&index, bot) = bots.bot_registry.bots.iter().next().unwrap();
    let mut bot = bot.clone();
    let kit = actor(engine, 1).equipped_loadout().unwrap();
    bot.target.loadout = kit;
    // The other side from the driver.
    bot.target.squad = replication::SquadAddress::from_index(16).unwrap();
    bot.health = 400;
    bot.damage.health_quarter_points = 400;
    engine.bot_registry.bots.insert(index, bot);
    engine.bot_registry.config = bots.bot_registry.config.clone();
    index
}

#[test]
fn a_registered_bot_is_a_victim_too() {
    let mut engine = scene(2, 400);
    let index = place_enemy_bot(&mut engine);
    report(&mut engine, 1, APC, 0x0380_0000 | u32::from(index), 950);
    let event = last_event(&engine);
    assert_eq!(event["gameplay_denial"], Value::Null, "{event}");
    assert_eq!(event["decoded"]["applied_damage"]["amount_hp"], 120.0);
    let bot = &engine.bot_registry.bots[&index];
    assert_eq!(bot.health, 0);
    assert_eq!(bot.damage.last_attacker, 0x0380_0000);
    assert_eq!(bot.damage.last_source_object, APC);
    assert_eq!(bot.damage.last_location_and_type, 0x50);
    let driver = engine.active.as_ref().unwrap();
    assert_eq!(driver.reward_statistics.get(&10), Some(&1));
    assert!(!event["decoded"]["death_notice"].is_null());
}

#[test]
fn a_lethal_impact_downs_a_bot_and_a_second_one_finishes_it() {
    let mut engine = scene(2, 400)
        .with_incapacitation_rules(incapacitation::Rules {
            schema_version: 1,
            incapacitated_damage_threshold_hp: 150.0,
            incapacitated_duration_seconds: 30.0,
            excluded_character_states: vec![],
            provenance: None,
        })
        .unwrap();
    let index = place_enemy_bot(&mut engine);
    report(&mut engine, 1, APC, 0x0380_0000 | u32::from(index), 950);
    let bot = &engine.bot_registry.bots[&index];
    assert_eq!(bot.health, 0);
    assert!(bot.downed.is_some(), "120 HP is below the 150 HP threshold");
    assert_eq!(bot.damage.conditions & 2, 2);
    let driver = engine.active.as_ref().unwrap();
    assert_eq!(
        driver.reward_statistics.get(&10),
        Some(&1),
        "the kill is the down"
    );
    // Finishing damage builds up separately: 120 HP, then 240 HP.
    report(&mut engine, 1, APC, 0x0380_0000 | u32::from(index), 960);
    assert!(engine.bot_registry.bots[&index].downed.is_some());
    report(&mut engine, 1, APC, 0x0380_0000 | u32::from(index), 970);
    let event = last_event(&engine);
    assert_eq!(event["gameplay_denial"], Value::Null, "{event}");
    assert!(
        event["decoded"]["death_notice"].is_null(),
        "no second notice"
    );
    let bot = &engine.bot_registry.bots[&index];
    assert!(bot.downed.is_none());
    assert_eq!(bot.damage.conditions & 2, 0);
    let driver = engine.active.as_ref().unwrap();
    assert_eq!(
        driver.reward_statistics.get(&10),
        Some(&1),
        "no second kill"
    );
    assert_eq!(driver.reward_statistics.get(&24), Some(&1), "a finish");
}
