//! Connected Company orders over authenticated event and world transactions.
use super::*;
use crate::{
    active_abilities::{Ability, CommandRole, Side, StrategicProperty, StrategicRequest},
    active_ability_engine_tests::{activate, actor_mut, prepare_effect_tick, request},
    platoon_ability_engine_tests::{configured_platoon_engine, last_decision, platoon_leadership},
    strategic_abilities::StrategicAbility,
};

fn reserve_actor(engine: &mut Engine<Vec<u8>>, player: u16, faction: u8) {
    let mut loadouts = loadouts::SavedLoadouts {
        selected_index: 0,
        update_ids: [1; 5],
        inventories: [[None; 22]; 5],
    };
    loadouts.inventories[0][0] = Some(2);
    let result = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-oic-reserve-{player}"),
            reservation_id: format!("synthetic-reservation-{player:04}"),
            world_id: 1,
            player_id: player,
            account_id: u32::from(player) + 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({
                "name":format!("Synthetic OIC {player}"),"faction":faction,"head":0,"voice":0,
                "universe_id":26,"character_id":u32::from(player)+1,"loadouts":loadouts,
                "leadership":{"leadership_level":3,"nominations":7,"matches_since_last_command":0}
            }))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(result.ok, "{:?}", result.error);
    let actor = actor_mut(engine, player);
    let squad_index = if faction == 0 { 0 } else { 16 };
    actor.squad_catalog = actor.squad_catalog.for_squad(squad_index).unwrap().into();
    spawn_progression::ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.nominations = 7;
    assert_eq!(actor.team(), faction);
}

fn oic_leadership() -> (
    leadership_passives::LeadershipCatalog,
    leadership_runtime::RolePolicy,
) {
    let (catalog, _) = platoon_leadership();
    let policy = leadership_runtime::RolePolicy::from_json(
        &json!({
            "schema_version":4,"application_id":20231,"squad_leader":"native","development_bindings":[],
            "native_election":{"source_sha256":"c3".repeat(32),"squad_level_weight":1.,"squad_mslc_weight":1.,
                "default_record":{"leadership_level":3,"nominations":7,"matches_since_last_command":0}},
            "platoon_election":{"level_weight":2.,"mslc_weight":1.},
            "company_election":{"level_weight":2.,"mslc_weight":1.}
        })
        .to_string(),
    )
    .unwrap();
    (catalog, policy)
}

pub(crate) fn fixture() -> Engine<Vec<u8>> {
    let mut engine = configured_platoon_engine();
    let mut setup = engine
        .active_abilities
        .take()
        .unwrap()
        .setup
        .as_ref()
        .clone();
    let (catalog, policy) = oic_leadership();
    for mode in &mut setup.rules.modes {
        if mode.players == 256 {
            mode.strategic = [
                StrategicAbility::SignalsJamming,
                StrategicAbility::SensorSweep,
                StrategicAbility::TacticalStrike,
                match mode.side {
                    Side::Attacker => StrategicAbility::Blockade,
                    Side::Defender => StrategicAbility::RapidRedeploy,
                },
            ]
            .map(|ability| ability.name().into())
            .to_vec();
        }
    }
    for (ability, spawn_interval_modifier) in [
        (StrategicAbility::SignalsJamming, None),
        (StrategicAbility::SensorSweep, None),
        (StrategicAbility::TacticalStrike, None),
        (StrategicAbility::Blockade, Some(0.5)),
        (StrategicAbility::RapidRedeploy, Some(-0.5)),
    ] {
        setup.rules.strategic.push(StrategicProperty {
            ability: ability.name().into(),
            source_member: format!("invented-{}", ability.name()),
            source_sha256: "c3".repeat(32),
            cooldown_ms: 500,
            duration_ms: 40,
            spawn_interval_modifier,
        });
    }
    engine = engine
        .with_leadership_passives(catalog, policy)
        .unwrap()
        .with_active_abilities(setup)
        .unwrap();
    for player in 0..6 {
        reserve_actor(&mut engine, player, u8::from(player < 3));
    }
    engine.match_started = true;
    engine.reconcile_squad_leaders().unwrap();
    assert_eq!(engine.company_leader_index(0), 3);
    assert_eq!(engine.company_leader_index(1), 0);
    for (player, role) in [
        (1, CommandRole::PlatoonLeader),
        (2, CommandRole::SquadLeader),
        (4, CommandRole::PlatoonLeader),
        (5, CommandRole::SquadLeader),
    ] {
        assert_eq!(
            engine.command_leadership_role(actor(&engine, player)),
            Some(role)
        );
    }
    engine
}

fn actor(engine: &Engine<Vec<u8>>, player: u16) -> &Active {
    engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|actor| actor.reservation.player == player)
        .unwrap()
}

fn cast(
    engine: &mut Engine<Vec<u8>>,
    player: u16,
    ability: StrategicAbility,
    millis: u64,
) -> Value {
    let request = StrategicRequest {
        ability_code: ability.code(),
    };
    let response = engine.handle(
        Request::Event {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-strategic-{player}-{millis}"),
            reservation_id: format!("synthetic-reservation-{player:04}"),
            world_id: 1,
            player_id: player,
            transport: Transport::Tcp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex: protocol::encode_hex(&request.to_bytes()),
        },
        Duration::from_millis(millis),
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    std::str::from_utf8(&engine.writer)
        .unwrap()
        .lines()
        .rev()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|event| event["decoded"]["type"] == "strategic_ability")
        .unwrap()
}

fn advance(engine: &mut Engine<Vec<u8>>, millis: u64) {
    let mut native = Default::default();
    let tick = prepare_effect_tick(engine, &native, millis).unwrap();
    engine
        .record_acquisition_character_effects(tick, &mut native, 1)
        .unwrap();
}

#[test]
fn strategic_orders_require_held_company_role_and_side_without_spending_on_denial() {
    let mut engine = fixture();
    let wrong_role = cast(&mut engine, 1, StrategicAbility::SensorSweep, 20);
    assert_eq!(wrong_role["decoded"]["accepted"], false);
    let wrong_side = cast(&mut engine, 0, StrategicAbility::Blockade, 20);
    assert_eq!(wrong_side["decoded"]["accepted"], false);
    let acquisition = spawn_catalog::SpawnCatalog::from_json(
        &json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"synthetic wrong mode",
            "active":{"map_name":engine.map_name(),"squad_index":16,"default_spawn":0,"available":[0]},
            "match_profile":{"game_type":1,"auto_assign_teams":false,"has_objectives":false,"has_bunkers":false},
            "maps":[{"name":engine.map_name(),"owner_faction":1,"spawn_count":1,
                "locations":[{"index":0,"token":71,"kind":"on_foot","position":{"x":100.,"y":3.,"z":100.}}],
                "squads":[{"index":16,"team":1,"platoon":0,"squad":0,"faction":1,"available":[0],"default_spawn":0}]}]
        }).to_string(),
    ).unwrap();
    let domination = engine.spawn_catalog.replace(acquisition);
    assert_eq!(
        cast(&mut engine, 0, StrategicAbility::RapidRedeploy, 20)["decoded"]["accepted"],
        false
    );
    engine.spawn_catalog = domination;
    engine.match_started = false;
    let not_playing = cast(&mut engine, 0, StrategicAbility::RapidRedeploy, 20);
    assert_eq!(not_playing["decoded"]["accepted"], false);
    engine.match_started = true;
    let accepted = cast(&mut engine, 0, StrategicAbility::RapidRedeploy, 20);
    assert_eq!(accepted["decoded"]["accepted"], true, "{accepted}");
    assert_eq!(engine.strategic_spawn_interval(0, 1000), Ok(1000));
    assert_eq!(engine.strategic_spawn_interval(1, 1000), Ok(500));
    assert_eq!(
        cast(&mut engine, 3, StrategicAbility::Blockade, 20)["decoded"]["accepted"],
        true
    );
    assert_eq!(engine.strategic_spawn_interval(1, 1000), Ok(1000));
}

#[test]
fn signals_jamming_locks_enemy_command_and_company_orders_through_expiry_boundary() {
    let mut engine = fixture();
    let started = cast(&mut engine, 0, StrategicAbility::SignalsJamming, 20);
    assert_eq!(started["decoded"]["accepted"], true, "{started}");
    for player in [4, 5] {
        assert!(activate(&mut engine, request(0, Ability::Sensor), player, 21).ok);
        let event = last_decision(&engine);
        assert_eq!(event["decoded"]["accepted"], false, "{event}");
        assert!(
            event["gameplay_denial"].as_str().unwrap().contains("jam"),
            "{event}"
        );
    }
    assert!(activate(&mut engine, request(16, Ability::Sensor), 2, 21).ok);
    assert_eq!(last_decision(&engine)["decoded"]["accepted"], true);
    assert_eq!(
        cast(&mut engine, 0, StrategicAbility::RapidRedeploy, 30)["decoded"]["accepted"],
        true
    );

    advance(&mut engine, 60);
    let locked = cast(&mut engine, 3, StrategicAbility::Blockade, 60);
    assert_eq!(locked["decoded"]["accepted"], false);
    assert!(
        locked["gameplay_denial"].as_str().unwrap().contains("jam"),
        "{locked}"
    );
    advance(&mut engine, 61);
    let unlocked = cast(&mut engine, 3, StrategicAbility::Blockade, 61);
    assert_eq!(unlocked["decoded"]["accepted"], true, "{unlocked}");
}

#[test]
fn tactical_refresh_resets_friendly_individual_clock_and_keeps_active_bit_and_shared_gate() {
    let mut engine = fixture();
    assert!(activate(&mut engine, request(16, Ability::Sensor), 2, 20).ok);
    assert_eq!(last_decision(&engine)["decoded"]["accepted"], true);
    let tactical = cast(&mut engine, 0, StrategicAbility::TacticalStrike, 21);
    assert_eq!(tactical["decoded"]["accepted"], true, "{tactical}");
    engine
        .queue_active_ability_state(Duration::from_millis(22), 1)
        .unwrap();
    let squad = snapshots(&engine, 2, 8)
        .into_iter()
        .find(|value| value.index == 16)
        .unwrap();
    assert_eq!(squad.active_mask, Some(1 << 2));
    assert_eq!(squad.activation_times[2], Some(0));

    assert!(activate(&mut engine, request(16, Ability::Sensor), 2, 29).ok);
    assert_eq!(
        last_decision(&engine)["gameplay_denial"],
        "command_cooldown"
    );
    assert!(activate(&mut engine, request(16, Ability::Sensor), 2, 31).ok);
    assert_eq!(last_decision(&engine)["decoded"]["accepted"], true);
}

#[test]
fn sensor_sweep_marks_an_enemy_squad_member_without_accepted_movement() {
    let mut engine = fixture();
    actor_mut(&mut engine, 3).tracking = Default::default();
    assert_eq!(
        cast(&mut engine, 0, StrategicAbility::SensorSweep, 20)["decoded"]["accepted"],
        true
    );
    advance(&mut engine, 21);
    engine
        .queue_active_ability_state(Duration::from_millis(21), 1)
        .unwrap();
    assert!(actor(&engine, 3).tracking.sample.is_none());
    assert_sweep_detection(&engine);
}

#[test]
fn sensor_sweep_republishes_detection_for_a_new_character_generation() {
    let mut engine = fixture();
    let activated = cast(&mut engine, 0, StrategicAbility::SensorSweep, 20);
    assert_eq!(activated["decoded"]["accepted"], true);
    advance(&mut engine, 21);
    engine
        .queue_active_ability_state(Duration::from_millis(21), 1)
        .unwrap();
    assert_sweep_detection(&engine);
    actor_mut(&mut engine, 0).pending_shared.clear();

    // Sweep advances the other fifteen squad slots before returning to squad 0.
    for millis in 22..=36 {
        advance(&mut engine, millis);
    }
    actor_mut(&mut engine, 3).life = CharacterLife::Alive {
        generation: 2,
        health_quarter_points: 400,
    };
    advance(&mut engine, 37);
    engine
        .queue_active_ability_state(Duration::from_millis(37), 1)
        .unwrap();
    assert_sweep_detection(&engine);
}

fn assert_sweep_detection(engine: &Engine<Vec<u8>>) {
    let wire = actor(engine, 0)
        .pending_shared
        .iter()
        .find(|wire| {
            replication::ObjectUpdate::decode_with_profile(wire, engine.wire_profile)
                .is_ok_and(|update| update.header.object_type_mask == 1 << 3)
        })
        .unwrap();
    let mut bits = replication::BitReader::with_profile(wire, engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    assert_eq!(bits.read(1).unwrap(), 1);
    assert_eq!(bits.read(16).unwrap(), 3);
    assert_eq!(bits.read(10).unwrap(), 0);
    assert_eq!(bits.read(2).unwrap(), 3);
}

#[derive(Debug)]
struct Snapshot {
    index: u32,
    active_mask: Option<u32>,
    activation_times: [Option<u32>; 9],
}

fn snapshots(engine: &Engine<Vec<u8>>, player: u16, object_type: u32) -> Vec<Snapshot> {
    let (field_count, first_data, data_count, active_mask, leader, shadow) = match object_type {
        8 => (26, 16, 6, 22, 10, 24),
        11 => (15, 2, 9, 12, 11, 14),
        _ => panic!("unexpected ability object type"),
    };
    let profile = engine.wire_profile;
    let mut result = Vec::new();
    for wire in &actor(engine, player).pending_shared {
        if !replication::ObjectUpdate::decode_with_profile(wire, profile)
            .is_ok_and(|update| update.header.object_type_mask == 1 << object_type)
        {
            continue;
        }
        let mut bits = replication::BitReader::with_profile(wire, profile);
        bits.position = profile.header_bits();
        while bits.read(1).unwrap() == 1 {
            let mut snapshot = Snapshot {
                index: bits.read(16).unwrap(),
                active_mask: None,
                activation_times: [None; 9],
            };
            for field in 0..field_count {
                if bits.read(1).unwrap() == 0 {
                    continue;
                }
                if (first_data..first_data + data_count).contains(&field) {
                    for width in [32, 32, 32, 16] {
                        bits.read(width).unwrap();
                    }
                    snapshot.activation_times[field - first_data] = Some(bits.read(32).unwrap());
                    bits.read(1).unwrap();
                    bits.read(32).unwrap();
                } else if field == active_mask {
                    snapshot.active_mask = Some(bits.read(16).unwrap());
                } else {
                    let width = match field {
                        0 => 1,
                        field if field == shadow => 8,
                        field if field == 1 || field == leader || field == active_mask + 1 => 16,
                        field => panic!("unexpected ability field {field}"),
                    };
                    bits.read(width).unwrap();
                }
            }
            result.push(snapshot);
        }
    }
    result
}

#[test]
fn company_late_join_fifo_preserves_active_order_after_publication_record_failure() {
    let mut engine = fixture();
    assert_eq!(
        cast(&mut engine, 0, StrategicAbility::SignalsJamming, 20)["decoded"]["accepted"],
        true
    );
    engine
        .queue_active_ability_state(Duration::from_millis(21), 1)
        .unwrap();
    reserve_actor(&mut engine, 6, 0);
    let existing_packet = actor(&engine, 0)
        .pending_shared
        .iter()
        .find(|wire| {
            replication::ObjectUpdate::decode_with_profile(wire, engine.wire_profile)
                .is_ok_and(|update| update.header.object_type_mask == 1 << 10)
        })
        .unwrap()
        .clone();
    let late = actor_mut(&mut engine, 6);
    late.pending_shared.clear();
    late.pending_shared.push_back(existing_packet.clone());
    let max_log = engine.max_log;
    engine.max_log = engine.written;
    assert!(
        engine
            .queue_active_ability_state(Duration::from_millis(22), 1)
            .is_err()
    );
    assert_eq!(actor(&engine, 6).pending_shared.len(), 1);
    assert_eq!(actor(&engine, 6).pending_shared[0], existing_packet);
    engine.max_log = max_log;
    engine.failed = false;
    engine
        .queue_active_ability_state(Duration::from_millis(23), 1)
        .unwrap();

    assert_eq!(actor(&engine, 6).pending_shared[0], existing_packet);
    let snapshots = snapshots(&engine, 6, 11);
    assert_eq!(snapshots.len(), 2);
    let friendly_to_caster = snapshots.iter().find(|value| value.index == 1).unwrap();
    assert_eq!(
        friendly_to_caster.active_mask,
        Some(u32::from(StrategicAbility::SignalsJamming.mask()))
    );
    assert_eq!(friendly_to_caster.activation_times[5], Some(20));
}

#[test]
fn oic_policy_requires_strategic_data_in_both_builder_orders() {
    let (catalog, policy) = oic_leadership();
    assert_eq!(
        configured_platoon_engine()
            .with_leadership_passives(catalog.clone(), policy.clone())
            .err(),
        Some("oic_leadership_requires_strategic_effects")
    );
    let mut engine = configured_platoon_engine();
    let setup = engine
        .active_abilities
        .take()
        .unwrap()
        .setup
        .as_ref()
        .clone();
    assert_eq!(
        engine
            .with_leadership_passives(catalog, policy)
            .unwrap()
            .with_active_abilities(setup)
            .err(),
        Some("oic_leadership_requires_strategic_effects")
    );
}
