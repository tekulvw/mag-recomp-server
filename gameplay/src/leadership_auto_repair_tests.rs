//! Connected automatic repair uses real source-shaped vehicle and leadership inputs.
use super::*;
use crate::leadership_passives::LeadershipCatalog;
use crate::leadership_runtime::RolePolicy;

fn passive_rules() -> (LeadershipCatalog, RolePolicy) {
    let catalog=LeadershipCatalog::from_json(&json!({
        "schema_version":1,"application_id":20231,
        "source":{"member":"scripts/character_leadership_abilities.zpr","member_sha256":"a".repeat(64),
        "archive_sha256":"b".repeat(64),"extractor":"synthetic","projector":"synthetic"},
        "max_separation_range":30.,"refresh_hold_ms":3000,"removal_grace_ms":1000,
        "levels":[{"level":0,"name":"none","modifiers":[]},
        {"level":1,"name":"squad","modifiers":[{"kind":"kCharModReloadTime","value":0.1}]},
        {"level":2,"name":"platoon","modifiers":[{"kind":"kCharModAutoRepair","value":1.}]},
        {"level":3,"name":"officer","modifiers":[{"kind":"kCharModAutoHeal","value":1.}]}]
    }).to_string()).unwrap();
    let policy = RolePolicy::from_json(
        &json!({"schema_version":1,"application_id":20231,
            "first_member_squad_leader":false,
            "development_bindings":[{"account_id":1,"character_id":1,"native_role":2}]
        })
        .to_string(),
    )
    .unwrap();
    (catalog, policy)
}

fn fixture() -> Engine<Vec<u8>> {
    let (catalog, policy) = passive_rules();
    let mut e = crate::spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::automatic_repair_fixture(catalog, policy);
    let ground = e.ground_runtime.as_mut().unwrap();
    let key = ground.get(4).unwrap().key;
    ground.apply_damage(key, 160, 0x0380_0007).unwrap(); // 150 max HP -> 110 HP.
    e
}
fn hp(e: &Engine<Vec<u8>>) -> u16 {
    e.ground_runtime
        .as_ref()
        .unwrap()
        .get(4)
        .unwrap()
        .health_quarter_points
}
fn tick(e: &mut Engine<Vec<u8>>, ms: u64) -> Response {
    let response = e.tick_world(Duration::from_millis(ms), 3, format!("auto-repair-{ms}"));
    assert!(response.ok, "{:?}", response.error);
    response
}
#[test]
fn world_scheduler_repairs_after_strict_interval_and_deduplicates_transport_ticks() {
    let mut e = fixture();
    assert_eq!(
        e.plan_leadership(Duration::from_millis(3200))
            .unwrap()
            .modifiers(e.active.as_ref().unwrap())
            .value("kCharModAutoRepair"),
        1.
    );
    tick(&mut e, 3200);
    tick(&mut e, 4700);
    assert_eq!(hp(&e), 440);
    tick(&mut e, 4750);
    assert_eq!(hp(&e), 449);
    tick(&mut e, 4750);
    tick(&mut e, 4799);
    assert_eq!(hp(&e), 449);
}

#[test]
fn inherited_repair_uses_current_ground_root_instead_of_cached_on_foot_position() {
    let mut e = fixture();
    e.active.as_mut().unwrap().ground_seat = None;
    e.active
        .as_mut()
        .unwrap()
        .owner_fields
        .state
        .as_mut()
        .unwrap()
        .state = 1;
    let squad = std::sync::Arc::clone(&e.active.as_ref().unwrap().squad_catalog);
    crate::spawn_progression::tests::reserve(&mut e, 1);
    let actor = e.active.as_mut().unwrap();
    actor.squad_catalog = squad;
    crate::spawn_progression::tests::ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.ground_seat = Some(crate::ground_seats::PassengerSeat {
        vehicle_index: 4,
        seat: 0,
    });
    actor.ground_vehicle_batch = 1;
    actor.owner_fields.state = Some(crate::replication::CharacterState {
        state: 14,
        stance: 0,
        user_data: [1, 4, 0],
    });
    // Both cached character samples are [100,3,100]. The actual occupant is
    // at the vehicle root [1000,3,1000], beyond the leader's authored range.
    tick(&mut e, 3200);
    tick(&mut e, 4750);
    assert_eq!(hp(&e), 440);
}

#[test]
fn new_life_cannot_inherit_the_previous_lifes_accumulated_repair_time() {
    let mut e = fixture();
    tick(&mut e, 3200);
    tick(&mut e, 4600);
    e.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 2,
        health_quarter_points: 400,
    };
    tick(&mut e, 4700);
    tick(&mut e, 4750);
    assert_eq!(hp(&e), 440);
    tick(&mut e, 6250);
    assert_eq!(hp(&e), 449);
}

#[test]
fn vacant_seat_resets_the_clock_before_the_next_association() {
    let mut e = fixture();
    tick(&mut e, 3200);
    tick(&mut e, 4600);
    let seat = e.active.as_mut().unwrap().ground_seat.take();
    tick(&mut e, 4650);
    e.active.as_mut().unwrap().ground_seat = seat;
    tick(&mut e, 4700);
    tick(&mut e, 4750);
    assert_eq!(hp(&e), 440);
    tick(&mut e, 6250);
    assert_eq!(hp(&e), 449);
}

#[test]
fn delayed_updates_emit_one_pulse_and_carry_the_elapsed_remainder() {
    let mut e = fixture();
    tick(&mut e, 3200);
    tick(&mut e, 8200);
    assert_eq!(hp(&e), 449);
    tick(&mut e, 8250);
    assert_eq!(hp(&e), 458);
    tick(&mut e, 8300);
    assert_eq!(hp(&e), 467);
    tick(&mut e, 8350);
    assert_eq!(hp(&e), 467);
}

#[test]
fn failed_record_preserves_timer_ground_and_publication_then_retry_heals_once() {
    let mut e = fixture();
    e.tick_leadership_auto_repair(Duration::from_millis(3200), 3)
        .unwrap();
    let before = e.automatic_repair.clone();
    let ground = e.ground_runtime.clone();
    let pending = e.active.as_ref().unwrap().pending_shared.clone();
    let max = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        e.tick_leadership_auto_repair(Duration::from_millis(4750), 3),
        Err("log_limit_reached")
    );
    assert_eq!(e.automatic_repair, before);
    assert_eq!(e.ground_runtime, ground);
    assert_eq!(e.active.as_ref().unwrap().pending_shared, pending);
    e.failed = false;
    e.max_log = max;
    e.tick_leadership_auto_repair(Duration::from_millis(4750), 3)
        .unwrap();
    assert_eq!(hp(&e), 449);
    let damage = e
        .ground_runtime
        .as_ref()
        .unwrap()
        .get(4)
        .unwrap()
        .damage_wire();
    assert_eq!(damage.last_location_and_type, 0);
    assert_eq!(damage.last_attacker, 0x0380_0007);
    assert_eq!(damage.last_source_object, u32::MAX);
}

#[test]
fn provider_loss_keeps_native_hold_then_expiry_stops_auto_repair() {
    let mut e = fixture();
    tick(&mut e, 3200);
    e.active
        .as_mut()
        .unwrap()
        .owner_fields
        .state
        .as_mut()
        .unwrap()
        .state = 0;
    tick(&mut e, 4750);
    assert_eq!(hp(&e), 449);
    tick(&mut e, 6250);
    assert_eq!(hp(&e), 458);
    tick(&mut e, 7200);
    tick(&mut e, 8250);
    assert_eq!(hp(&e), 458);
}

#[test]
fn complete_world_reset_discards_passive_clocks() {
    let mut e = fixture();
    tick(&mut e, 3200);
    tick(&mut e, 4600);
    assert_ne!(
        e.automatic_repair,
        crate::leadership_auto_repair::State::default()
    );
    e.reset_objective_world();
    assert_eq!(
        e.automatic_repair,
        crate::leadership_auto_repair::State::default()
    );
}

fn second_occupant(e: &mut Engine<Vec<u8>>, seat: u8) {
    let squad = std::sync::Arc::clone(&e.active.as_ref().unwrap().squad_catalog);
    crate::spawn_progression::tests::reserve(e, 1);
    let actor = e.active.as_mut().unwrap();
    crate::spawn_progression::tests::ready(actor);
    actor.squad_catalog = squad;
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.ground_seat = Some(crate::ground_seats::PassengerSeat {
        vehicle_index: 4,
        seat,
    });
    actor.ground_vehicle_batch = 1;
    actor.owner_fields.state = Some(replication::CharacterState {
        state: 14,
        stance: 0,
        user_data: [1, 4, seat],
    });
}
#[test]
fn distinct_occupants_contribute_once_each_and_duplicate_seat_claims_contribute_nothing() {
    let mut e = fixture();
    second_occupant(&mut e, 2);
    tick(&mut e, 3200);
    tick(&mut e, 4750);
    assert_eq!(hp(&e), 458);
    let mut invalid = fixture();
    second_occupant(&mut invalid, 0);
    tick(&mut invalid, 3200);
    tick(&mut invalid, 4750);
    assert_eq!(hp(&invalid), 440);
}

fn damage_field(
    wire: &[u8],
    profile: wire_profile::WireProfile,
    index: u16,
) -> Option<(u32, u32, u32, u32, u32)> {
    let mut bits = replication::BitReader::with_profile(wire, profile);
    bits.position = profile.header_bits();
    while bits.read(1).unwrap() != 0 {
        let current = bits.read(16).unwrap();
        let mut damage = None;
        for field in 0..30 {
            if bits.read(1).unwrap() == 0 {
                continue;
            }
            match field {
                0 => {
                    bits.read(1).unwrap();
                }
                1 | 6 | 7 | 20 => {
                    bits.read(16).unwrap();
                }
                3 => {
                    bits.read(32).unwrap();
                }
                4 | 5 | 19 => {
                    bits.read(8).unwrap();
                }
                18 => {
                    damage = Some((
                        bits.read(8).unwrap(),
                        bits.read(16).unwrap(),
                        bits.read(32).unwrap(),
                        bits.read(32).unwrap(),
                        bits.read(32).unwrap(),
                    ));
                }
                25 => {
                    bits.read(16).unwrap();
                    bits.read(17).unwrap();
                    bits.read(16).unwrap();
                }
                26 => {
                    for _ in 0..4 {
                        bits.read(11).unwrap();
                    }
                }
                _ => panic!("unexpected synthetic Ground field{field}"),
            }
        }
        if current == u32::from(index) {
            return damage;
        }
    }
    None
}
#[test]
fn current_and_late_join_publication_preserve_full_damage_attribution() {
    let mut e = fixture();
    second_occupant(&mut e, 2);
    // Only the first occupant repairs; the second remains a ready observer.
    e.active.as_mut().unwrap().ground_seat = None;
    let ground = e.ground_runtime.as_mut().unwrap();
    let key = ground.get(4).unwrap().key;
    ground
        .apply_grenade_damage(key, 10., 0x0380_0009, 0x1080_0002)
        .unwrap();
    e.tick_leadership_auto_repair(Duration::from_millis(3200), 3)
        .unwrap();
    for actor in e.active.iter_mut().chain(e.peers.values_mut()) {
        actor.pending_shared.clear();
    }
    e.tick_leadership_auto_repair(Duration::from_millis(4750), 3)
        .unwrap();
    let expected = Some((0xa0, 409 | (1 << 12), 0x0380_0009, 0x1080_0002, 0));
    for actor in e.active.iter().chain(e.peers.values()) {
        let packet = actor
            .pending_shared
            .iter()
            .find(|wire| {
                replication::ObjectUpdate::decode_with_profile(wire, e.wire_profile)
                    .is_ok_and(|update| update.header.object_type_mask == 1 << 1)
            })
            .unwrap();
        assert_eq!(damage_field(packet, e.wire_profile, 4), expected);
    }
    let snapshot = e.current_ground_batch(0, 2, 4800).unwrap();
    assert_eq!(damage_field(&snapshot, e.wire_profile, 4), expected);
}

#[test]
fn unsupported_association_and_critical_vehicle_cannot_heal() {
    let mut e = fixture();
    e.active
        .as_mut()
        .unwrap()
        .ground_seat
        .as_mut()
        .unwrap()
        .seat = 15;
    tick(&mut e, 3200);
    tick(&mut e, 4750);
    assert_eq!(hp(&e), 440);
    let mut critical = fixture();
    let ground = critical.ground_runtime.as_mut().unwrap();
    let key = ground.get(4).unwrap().key;
    ground.apply_damage(key, 400, 0).unwrap();
    // Test the passive transaction directly: ordinary Ground critical decay
    // requires its independent damage-policy fixture.
    critical
        .tick_leadership_auto_repair(Duration::from_millis(3200), 3)
        .unwrap();
    critical
        .tick_leadership_auto_repair(Duration::from_millis(4750), 3)
        .unwrap();
    assert_eq!(hp(&critical), 40);
}

#[test]
fn an_unqualified_ground_source_does_not_gain_the_escort_consumer() {
    let mut e = fixture();
    let mut creation = e
        .ground_vehicles
        .as_ref()
        .unwrap()
        .vehicle(4)
        .unwrap()
        .clone();
    creation.initial_state = 2;
    let mut runtime =
        acquisition_ground_runtime::Runtime::from_creation(&creation, Some(3)).unwrap();
    let key = runtime.get(4).unwrap().key;
    runtime.apply_damage(key, 160, 0).unwrap();
    e.ground_runtime = Some(runtime);
    e.tick_leadership_auto_repair(Duration::from_millis(3200), 3)
        .unwrap();
    e.tick_leadership_auto_repair(Duration::from_millis(4750), 3)
        .unwrap();
    assert_eq!(hp(&e), 440);
}
#[test]
fn full_health_consumes_the_due_attempt_before_subsequent_damage() {
    let mut e = fixture();
    let ground = e.ground_runtime.as_mut().unwrap();
    let key = ground.get(4).unwrap().key;
    ground.heal_percent(key, 100.).unwrap();
    e.tick_leadership_auto_repair(Duration::from_millis(3200), 3)
        .unwrap();
    e.tick_leadership_auto_repair(Duration::from_millis(4750), 3)
        .unwrap();
    assert_eq!(hp(&e), 600);
    e.ground_runtime
        .as_mut()
        .unwrap()
        .apply_damage(key, 160, 0)
        .unwrap();
    e.tick_leadership_auto_repair(Duration::from_millis(4800), 3)
        .unwrap();
    assert_eq!(hp(&e), 440);
    e.tick_leadership_auto_repair(Duration::from_millis(6250), 3)
        .unwrap();
    assert_eq!(hp(&e), 449);
}

#[test]
fn same_life_reentry_counts_the_current_update_delta_after_reset() {
    let mut e = fixture();
    e.tick_leadership_auto_repair(Duration::from_millis(3200), 3)
        .unwrap();
    let seat = e.active.as_mut().unwrap().ground_seat.take();
    e.tick_leadership_auto_repair(Duration::from_millis(3250), 3)
        .unwrap();
    e.active.as_mut().unwrap().ground_seat = seat;
    e.tick_leadership_auto_repair(Duration::from_millis(3300), 3)
        .unwrap();
    e.tick_leadership_auto_repair(Duration::from_millis(4750), 3)
        .unwrap();
    assert_eq!(hp(&e), 440);
    e.tick_leadership_auto_repair(Duration::from_millis(4800), 3)
        .unwrap();
    assert_eq!(hp(&e), 449);
}

#[test]
fn quiet_timer_observations_do_not_consume_the_evidence_log_budget() {
    let mut e = fixture();
    e.leadership = None;
    e.tick_leadership_auto_repair(Duration::from_millis(3200), 3)
        .unwrap();
    let written = e.written;
    for ms in (3250..=4250).step_by(50) {
        e.tick_leadership_auto_repair(Duration::from_millis(ms), 3)
            .unwrap();
    }
    assert_eq!(e.written, written);
    assert_eq!(hp(&e), 440);
}

fn apc(engine: &Engine<Vec<u8>>) -> &crate::acquisition_ground_runtime::Vehicle {
    engine.ground_runtime.as_ref().unwrap().get(0).unwrap()
}

fn mount_for_repair(engine: &mut Engine<Vec<u8>>, index: u16, seat: u8, now: u64) {
    let position = engine.current_ground_vehicle(index).unwrap().position;
    let actor = engine.active.as_mut().unwrap();
    actor
        .tracking
        .server_position(position, player_tracking::PositionOrigin::ServerSpawn, now);
    let character = 0x0380_0000 | u32::from(actor.reservation.player);
    let mut wire = 0x0216u16.to_be_bytes().to_vec();
    wire.extend((0x0100_0000 | u32::from(index)).to_be_bytes());
    wire.extend(u32::from(seat).to_be_bytes());
    wire.extend(character.to_be_bytes());
    crate::spawn_progression::dispatch(engine, &wire, now).unwrap();
    assert_eq!(
        engine.active.as_ref().unwrap().ground_seat,
        Some(ground_seats::PassengerSeat {
            vehicle_index: index,
            seat
        })
    );
}

fn apc_repair_fixture(domination_seat: Option<u8>) -> Engine<Vec<u8>> {
    let (catalog, policy) = passive_rules();
    let mut engine = if let Some(seat) = domination_seat {
        let mut engine =
            crate::manual_ground_repair_domination_tests::fixture_with_drivers(0, 0, true);
        let mut actor = engine.active.take().unwrap();
        actor.reservation.account = 1;
        actor.reservation.character.character_id = Some(1);
        engine = engine
            .with_development_control()
            .with_leadership_passives(catalog, policy)
            .unwrap();
        engine.active = Some(actor);
        crate::ground_turrets::tests::opt_in(&mut engine, 0);
        mount_for_repair(&mut engine, 0, seat, 3240);
        if seat == 1 {
            let actor = engine.active.as_ref().unwrap();
            assert!(apc(&engine).turret.as_ref().unwrap().grant_matches(actor));
            assert!(
                apc(&engine)
                    .turret
                    .as_ref()
                    .unwrap()
                    .primary_for(actor)
                    .is_none()
            );
        }
        engine
    } else {
        crate::spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::automatic_repair_fixture_for_modeled_vehicle(catalog, policy, true)
    };
    let key = apc(&engine).key;
    engine
        .ground_runtime
        .as_mut()
        .unwrap()
        .apply_grenade_damage(key, 50., 0x0380_0007, 0x1080_0002)
        .unwrap();
    assert_eq!(apc(&engine).health_quarter_points, 400);
    assert_eq!(
        engine
            .plan_leadership(Duration::from_millis(3250))
            .unwrap()
            .modifiers(engine.active.as_ref().unwrap())
            .value("kCharModAutoRepair"),
        1.
    );
    engine
}

#[test]
fn occupied_apc_passengers_and_crew_repair_without_weapon_readiness() {
    for seat in [None, Some(0), Some(1), Some(2)] {
        let mut engine = apc_repair_fixture(seat);
        let before = apc(&engine).clone();
        engine
            .tick_leadership_auto_repair(Duration::from_millis(3250), 3)
            .unwrap();
        engine
            .tick_leadership_auto_repair(Duration::from_millis(4750), 3)
            .unwrap();
        assert_eq!(apc(&engine), &before);
        engine
            .tick_leadership_auto_repair(Duration::from_millis(4800), 3)
            .unwrap();
        let vehicle = apc(&engine);
        assert_eq!(vehicle.health_hp(), 102.25, "APC seat {seat:?}");
        assert_eq!(vehicle.health_quarter_points, 409);
        assert_eq!(vehicle.key, before.key);
        assert_eq!(vehicle.pose, before.pose);
        assert_eq!(
            vehicle.damage_wire().last_attacker,
            before.damage_wire().last_attacker
        );
        assert_eq!(
            vehicle.damage_wire().last_source_object,
            before.damage_wire().last_source_object
        );
        assert_eq!(
            vehicle.damage_wire().last_location_and_type,
            before.damage_wire().last_location_and_type
        );
        assert_eq!(
            vehicle.damage_wire().last_user_data,
            before.damage_wire().last_user_data
        );
    }
}

#[test]
fn apc_auto_repair_refuses_stale_gunner_grant_and_rolls_back_refused_record() {
    let mut engine = apc_repair_fixture(Some(1));
    engine
        .tick_leadership_auto_repair(Duration::from_millis(3250), 3)
        .unwrap();
    let hp_before = apc(&engine).clone();
    let timers_before = engine.automatic_repair.clone();
    let pending_before = engine.active.as_ref().unwrap().pending_shared.clone();
    let log_limit = engine.max_log;
    engine.max_log = engine.written;
    assert_eq!(
        engine.tick_leadership_auto_repair(Duration::from_millis(4800), 3),
        Err("log_limit_reached")
    );
    assert_eq!(apc(&engine), &hp_before);
    assert_eq!(engine.automatic_repair, timers_before);
    assert_eq!(
        engine.active.as_ref().unwrap().pending_shared,
        pending_before
    );
    engine.failed = false;
    engine.max_log = log_limit;
    engine
        .tick_leadership_auto_repair(Duration::from_millis(4800), 3)
        .unwrap();
    assert_eq!(apc(&engine).health_quarter_points, 409);
    engine.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 2,
        health_quarter_points: 400,
    };
    for ms in [4850, 6400] {
        engine
            .tick_leadership_auto_repair(Duration::from_millis(ms), 3)
            .unwrap();
    }
    assert_eq!(apc(&engine).health_quarter_points, 409);
}

#[test]
fn qualified_escort_gunner_auto_repairs_before_primary_initialization() {
    let mut engine = fixture();
    crate::ground_turrets::tests::opt_in(&mut engine, 4);
    mount_for_repair(&mut engine, 4, 1, 3240);
    let vehicle = engine.ground_runtime.as_ref().unwrap().get(4).unwrap();
    let turret = vehicle.turret.as_ref().unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert!(turret.grant_matches(actor));
    assert!(turret.primary_for(actor).is_none());
    engine
        .tick_leadership_auto_repair(Duration::from_millis(3250), 3)
        .unwrap();
    engine
        .tick_leadership_auto_repair(Duration::from_millis(4800), 3)
        .unwrap();
    assert_eq!(hp(&engine), 449);
}
