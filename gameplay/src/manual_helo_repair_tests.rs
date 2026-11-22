//! Authenticated operational Helo repair and ordered publication.
use super::*;
use crate::{
    manual_ground_repair_domination_tests::{add_peer, fixture},
    manual_ground_repair_tests::{repair, set_modifier},
    replication::CharacterPosition,
};

const GRENADE: u32 = 0x1080_0002;
const HELO: u32 = 0x0280_0000;

fn helo_repair_fixture(team: u8) -> Engine<Vec<u8>> {
    let mut engine = fixture(0, team);
    crate::domination_structural_world::hospital_rocket_helo_damage::tests::install_helo_fixture(
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

fn repair_hull(engine: &Engine<Vec<u8>>) -> &crate::domination_helo_damage::HullDamage {
    &engine
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

fn helo_wire_update(engine: &Engine<Vec<u8>>) -> Vec<u8> {
    crate::domination_helo_damage::damage_delta(
        0,
        Some(repair_hull(engine).state),
        Some(repair_hull(engine).wire),
        Duration::ZERO,
        engine.wire_profile,
    )
    .unwrap()
}

fn damage_helo(engine: &mut Engine<Vec<u8>>, hp: f32, attacker: u32) {
    let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
    let carrier = &mut state.helo.as_mut().unwrap().carriers[0];
    let profile = state
        .helo_damage
        .as_ref()
        .unwrap()
        .profile(carrier.source.faction)
        .unwrap();
    let thresholds = engine
        .bunker_world
        .as_ref()
        .unwrap()
        .damage_catalog()
        .rules();
    assert!(
        carrier
            .damage
            .apply(
                carrier.published_state,
                profile,
                thresholds,
                crate::domination_helo_damage::DamageDose {
                    hp: 100. - hp,
                    attacker,
                    location_and_type: 0xa0,
                    source_object: GRENADE,
                    user_data: 0,
                },
            )
            .unwrap()
    );
}

#[test]
fn operational_helo_repair_uses_native_event_and_records_health_and_credit_once() {
    let mut engine = helo_repair_fixture(0);
    damage_helo(&mut engine, 80., u32::MAX);
    let mut expected_wire = repair_hull(&engine).wire;
    expected_wire.health_quarter_points = 388;
    let messages = repair(&mut engine, HELO, 2000, Transport::Tcp).unwrap();

    let hull = repair_hull(&engine);
    assert_eq!(hull.wire.health_quarter_points, 388);
    assert_eq!(hull.wire.last_source_object, GRENADE);
    assert_eq!(hull.wire.last_location_and_type, 0xa0);
    assert_eq!(hull.wire.last_attacker, u32::MAX);
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.reward_statistics.get(&35), Some(&1));
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].payload_hex, "02040000012301");
    assert_eq!(actor.pending_shared.len(), 1);
    assert_eq!(
        actor.pending_shared[0],
        crate::domination_helo_damage::damage_delta(
            0,
            None,
            Some(expected_wire),
            Duration::from_millis(2000),
            engine.wire_profile,
        )
        .unwrap()
    );
    let rewards = engine.pending_reward_batch();
    assert_eq!(rewards.len(), 1);
    assert_eq!(rewards[0].frago, Some(true));

    assert!(
        repair(&mut engine, HELO, 3249, Transport::Tcp)
            .unwrap()
            .is_empty()
    );
    assert_eq!(repair_hull(&engine).wire.health_quarter_points, 388);
    assert_eq!(engine.pending_reward_batch().len(), 1);
}

#[test]
fn operational_helo_repair_preserves_float_work_and_suppresses_friendly_attacker_credit() {
    let mut engine = helo_repair_fixture(0);
    damage_helo(&mut engine, 80., u32::MAX);
    set_modifier(&mut engine, -0.99264705);
    assert!(
        repair(&mut engine, HELO, 2000, Transport::Tcp)
            .unwrap()
            .is_empty()
    );
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(repair_hull(&engine).wire.health_quarter_points, 320);
    assert!(actor.repair_credit.carry > 0.);
    assert_eq!(actor.repair_credit.sequence, 1);
    assert_eq!(actor.pending_shared.len(), 1);
    assert!(engine.pending_reward_batch().is_empty());

    let mut engine = helo_repair_fixture(0);
    let waiting_peer = add_peer(&mut engine, 2, 0, 1);
    damage_helo(&mut engine, 80., 0x0380_0002);
    assert!(
        repair(&mut engine, HELO, 2000, Transport::Tcp)
            .unwrap()
            .is_empty()
    );
    assert_eq!(repair_hull(&engine).wire.health_quarter_points, 388);
    assert_eq!(repair_hull(&engine).wire.last_attacker, 0x0380_0002);
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.repair_credit, repair_rewards::State::default());
    assert_eq!(actor.pending_shared.len(), 1);
    assert!(engine.peers[&waiting_peer].pending_shared.is_empty());
    assert!(engine.pending_reward_batch().is_empty());
}

#[test]
fn operational_helo_repair_rejected_record_preserves_damage_cooldown_and_queues() {
    let mut engine = helo_repair_fixture(0);
    damage_helo(&mut engine, 80., u32::MAX);
    let before = helo_wire_update(&engine);
    let before_hp = repair_hull(&engine).health_hp();
    engine.max_log = engine.written;
    assert_eq!(
        repair(&mut engine, HELO, 2000, Transport::Tcp).err(),
        Some("log_limit_reached")
    );
    assert_eq!(helo_wire_update(&engine), before);
    assert_eq!(repair_hull(&engine).health_hp(), before_hp);
    assert!(engine.next_repair.is_empty());
    assert!(engine.pending_reward_batch().is_empty());
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.repair_credit, repair_rewards::State::default());
    assert!(actor.pending_shared.is_empty());
}

#[test]
fn operational_helo_repair_denies_ineligible_actor_target_and_transport() {
    use crate::{domination_helo::FlightState, replication::VehicleState};
    for (case, team, hp, target, transport) in [
        ("full", 0, 100., HELO, Transport::Tcp),
        ("enemy", 1, 80., HELO, Transport::Tcp),
        ("unknown", 0, 80., HELO | 4, Transport::Tcp),
        ("udp", 0, 80., HELO, Transport::Udp),
        ("missing_tool", 0, 80., HELO, Transport::Tcp),
        ("actor_creation", 0, 80., HELO, Transport::Tcp),
        ("mounted", 0, 80., HELO, Transport::Tcp),
        ("critical", 0, 80., HELO, Transport::Tcp),
        ("dormant", 0, 80., HELO, Transport::Tcp),
        ("missing_pose", 0, 80., HELO, Transport::Tcp),
        ("malformed_handle", 0, 80., HELO | 0x10000, Transport::Tcp),
    ] {
        let mut engine = helo_repair_fixture(team);
        if hp < 100. {
            damage_helo(&mut engine, hp, u32::MAX);
        }
        let actor = engine.active.as_mut().unwrap();
        let carrier = &mut engine
            .domination_gameplay
            .as_mut()
            .unwrap()
            .state
            .helo
            .as_mut()
            .unwrap()
            .carriers[0];
        match case {
            "missing_tool" => actor.observed_primary = None,
            "actor_creation" => actor.helo_creation_sent = false,
            "mounted" => {
                actor.helo_seat = Some(crate::domination_helo::PassengerSeat {
                    object_index: 0,
                    seat: 2,
                })
            }
            "critical" => carrier.damage.state = VehicleState::Critical,
            "dormant" => carrier.published_state = FlightState::Dormant,
            "missing_pose" => carrier.current_pose = None,
            _ => {}
        }
        let before = helo_wire_update(&engine);
        assert!(
            repair(&mut engine, target, 2000, transport)
                .unwrap()
                .is_empty(),
            "{case}"
        );
        assert_eq!(helo_wire_update(&engine), before, "{case}");
        assert!(engine.next_repair.is_empty(), "{case}");
        assert!(engine.pending_reward_batch().is_empty(), "{case}");
        assert!(
            engine.active.as_ref().unwrap().pending_shared.is_empty(),
            "{case}"
        );
    }
}
