use super::*;
use crate::active_ability_engine_tests::actor_mut;

const POSITION: replication::CharacterPosition = replication::CharacterPosition {
    x: 40.,
    y: 2.,
    z: 50.,
};
const LEADER: u16 = 2;

fn fixture() -> Engine<Vec<u8>> {
    let mut engine = strategic_engine_tests::fixture();
    engine.squad_orders = Some(squad_order_engine::Runtime::new(squad_orders::Rules {
        squad_order_cooldown_ms: 1000,
    }));
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

fn waypoint(clear: bool) -> Vec<u8> {
    let mut wire = 0x021au16.to_be_bytes().to_vec();
    for component in [POSITION.x, POSITION.y, POSITION.z] {
        wire.extend(component.to_be_bytes());
    }
    wire.push(u8::from(clear) << 7);
    wire
}

fn event(
    engine: &mut Engine<Vec<u8>>,
    player: u16,
    wire: &[u8],
    millis: u64,
    plugin_id: u8,
) -> Response {
    let active = actor(engine, player);
    engine.handle(
        Request::Event {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-squad-order-{player}-{millis}"),
            reservation_id: active.reservation.id.clone(),
            world_id: active.reservation.world,
            player_id: player,
            transport: Transport::Tcp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(plugin_id),
            payload_hex: protocol::encode_hex(wire),
        },
        Duration::from_millis(millis),
        1,
    )
}

fn last_decision(engine: &Engine<Vec<u8>>, kind: &str) -> Value {
    std::str::from_utf8(&engine.writer)
        .unwrap()
        .lines()
        .rev()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|record| record["decoded"]["type"] == kind)
        .unwrap()["decoded"]
        .clone()
}

fn set_waypoint(engine: &mut Engine<Vec<u8>>, player: u16, clear: bool, millis: u64) -> Value {
    let response = event(engine, player, &waypoint(clear), millis, 1);
    assert!(response.ok, "{:?}", response.error);
    assert!(response.rewards.is_empty());
    last_decision(engine, "squad_waypoint")
}

fn order_state(engine: &Engine<Vec<u8>>) -> &squad_orders::State {
    &engine.squad_orders.as_ref().unwrap().state
}

#[test]
fn only_held_squad_leader_can_set_or_clear_and_shared_gate_survives_removal() {
    let mut engine = fixture();
    let squad = actor(&engine, LEADER).squad_catalog.squad_index();
    for player in [0, 1] {
        assert_eq!(
            set_waypoint(&mut engine, player, false, 501)["reason"],
            "squad_order_requires_held_squad_leader"
        );
    }
    let response = event(&mut engine, LEADER, &waypoint(false), 501, 2);
    assert!(response.ok);
    assert!(order_state(&engine).waypoint(squad).is_none());
    assert_eq!(
        set_waypoint(&mut engine, LEADER, false, 500)["reason"],
        "squad_order_cooldown"
    );
    assert_eq!(
        set_waypoint(&mut engine, LEADER, false, 501)["accepted"],
        true
    );
    assert!(order_state(&engine).waypoint(squad).unwrap().active);
    assert_eq!(set_waypoint(&mut engine, 0, true, 502)["accepted"], false);
    assert_eq!(
        set_waypoint(&mut engine, LEADER, true, 502)["accepted"],
        true
    );
    assert_eq!(
        order_state(&engine).waypoint(squad).unwrap().position,
        POSITION
    );
    assert!(!order_state(&engine).waypoint(squad).unwrap().active);
    assert_eq!(
        set_waypoint(&mut engine, LEADER, false, 502)["reason"],
        "squad_order_cooldown"
    );
    assert_eq!(
        set_waypoint(&mut engine, LEADER, false, 1002)["accepted"],
        true
    );

    let max_log = engine.max_log;
    actor_mut(&mut engine, LEADER).nominations = 0;
    engine.max_log = engine.written;
    assert!(engine.reconcile_squad_leaders().is_err());
    assert_eq!(engine.squad_leader_index(squad, &[u16::MAX; 8]), LEADER);
    assert!(order_state(&engine).waypoint(squad).unwrap().active);
    engine.max_log = max_log;
    engine.failed = false;
    engine.reconcile_squad_leaders().unwrap();
    assert_ne!(engine.squad_leader_index(squad, &[u16::MAX; 8]), LEADER);
    assert!(!order_state(&engine).waypoint(squad).unwrap().active);
    assert!(!order_state(&engine).can_set(
        squad,
        1003,
        &engine.squad_orders.as_ref().unwrap().rules
    ));
    assert_eq!(actor(&engine, LEADER).match_experience, 0);
    assert!(engine.pending_rewards.is_empty());
    engine.reset_objective_world();
    assert!(order_state(&engine).waypoint(squad).is_none());
    assert!(order_state(&engine).can_set(squad, 501, &engine.squad_orders.as_ref().unwrap().rules));
}

#[derive(Default)]
struct WireOrders {
    leader: Option<u16>,
    tag: Option<u32>,
    has_position: bool,
    active: Option<bool>,
}

fn packet_orders(
    wire: &[u8],
    profile: wire_profile::WireProfile,
    squad: u16,
) -> Option<WireOrders> {
    let update = replication::ObjectUpdate::decode_with_profile(wire, profile).ok()?;
    if update.header.object_type_mask != 1 << 8 {
        return None;
    }
    let mut bits = replication::BitReader::with_profile(wire, profile);
    bits.position = profile.header_bits();
    if bits.read(1).ok()? != 1 || bits.read(16).ok()? != u32::from(squad) {
        return None;
    }
    let mut orders = WireOrders::default();
    for field in 0..16 {
        if bits.read(1).ok()? == 0 {
            continue;
        }
        match field {
            0 => {
                bits.read(1).ok()?;
            }
            1..=9 | 11..=12 => {
                bits.read(16).ok()?;
            }
            10 => orders.leader = Some(bits.read(16).ok()? as u16),
            13 => orders.tag = Some(bits.read(32).ok()?),
            14 => {
                bits.read(32).ok()?;
                bits.read(17).ok()?;
                orders.has_position = true;
            }
            15 => orders.active = Some(bits.read(1).ok()? == 1),
            _ => unreachable!(),
        }
    }
    Some(orders)
}

fn published_waypoint(engine: &Engine<Vec<u8>>, player: u16, squad: u16) -> Option<bool> {
    actor(engine, player)
        .pending_shared
        .iter()
        .filter_map(|wire| packet_orders(wire, engine.wire_profile, squad)?.active)
        .last()
}

#[test]
fn late_ready_waypoint_retries_after_full_fifo_and_failed_records() {
    let mut engine = fixture();
    engine.world_tick_enabled = true;
    let late = 4;
    let squad = actor(&engine, LEADER).squad_catalog.squad_index();
    actor_mut(&mut engine, late).ready_for_updates = false;
    let max_log = engine.max_log;
    engine.max_log = engine.written;
    let response = event(&mut engine, LEADER, &waypoint(false), 501, 1);
    assert!(!response.ok);
    assert!(order_state(&engine).waypoint(squad).is_none());
    engine.max_log = max_log;
    engine.failed = false;
    assert_eq!(
        set_waypoint(&mut engine, LEADER, false, 501)["accepted"],
        true
    );
    engine
        .queue_squad_orders(Duration::from_millis(502), 1)
        .unwrap();
    assert_eq!(published_waypoint(&engine, LEADER, squad), Some(true));
    assert_eq!(published_waypoint(&engine, late, squad), None);

    let late = actor_mut(&mut engine, late);
    late.ready_for_updates = true;
    late.pending_shared = std::iter::repeat_n(vec![0xaa], multiplayer::MAX_SHARED_QUEUE).collect();
    engine
        .queue_squad_orders(Duration::from_millis(503), 1)
        .unwrap();
    assert_eq!(
        actor(&engine, 4).pending_shared.len(),
        multiplayer::MAX_SHARED_QUEUE
    );
    assert!(
        actor(&engine, 4)
            .pending_shared
            .iter()
            .all(|packet| *packet == [0xaa])
    );
    actor_mut(&mut engine, 4).pending_shared.truncate(1);
    engine.max_log = engine.written;
    assert!(
        engine
            .queue_squad_orders(Duration::from_millis(504), 1)
            .is_err()
    );
    assert_eq!(actor(&engine, 4).pending_shared.len(), 1);

    engine.max_log = max_log;
    engine.failed = false;
    engine
        .queue_squad_orders(Duration::from_millis(505), 1)
        .unwrap();
    assert_eq!(actor(&engine, 4).pending_shared[0], [0xaa]);
    assert_eq!(published_waypoint(&engine, 4, squad), Some(true));
    let delivered = actor(&engine, 4).pending_shared.len();
    engine
        .queue_squad_orders(Duration::from_millis(506), 1)
        .unwrap();
    assert_eq!(actor(&engine, 4).pending_shared.len(), delivered);
}

fn tag(engine: &mut Engine<Vec<u8>>, target_id: u32, millis: u64) -> Value {
    let mut wire = 0x0206u16.to_be_bytes().to_vec();
    wire.extend(target_id.to_be_bytes());
    let response = event(engine, 0, &wire, millis, 1);
    assert!(response.ok, "{:?}", response.error);
    assert!(response.rewards.is_empty());
    last_decision(engine, "tagged_object")
}

#[test]
fn acquisition_tags_require_exact_live_identity_and_share_waypoint_gate() {
    use crate::spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::capture_fixture;
    let (catalog, policy) = crate::platoon_ability_engine_tests::platoon_leadership();
    let mut engine = capture_fixture()
        .with_leadership_passives(catalog, policy)
        .unwrap()
        .with_squad_orders(squad_orders::Rules {
            squad_order_cooldown_ms: 1000,
        })
        .unwrap();
    crate::spawn_progression::reserve(&mut engine, 0);
    let leader = actor_mut(&mut engine, 0);
    crate::spawn_progression::ready(leader);
    leader.nominations = 1;
    let squad = leader.squad_catalog.squad_index();
    engine.match_started = true;
    engine.match_epoch = Some(Duration::ZERO);
    engine.reconcile_squad_leaders().unwrap();
    engine
        .tick_acquisition_objectives(Duration::from_millis(1), 1)
        .unwrap();

    const TARGET: u32 = 0x0c80_0064;
    for forged in [0x0c00_0064, 0x0d80_0064, 0x0c80_ffff] {
        assert_eq!(
            tag(&mut engine, forged, 501)["reason"],
            "squad_order_target_unavailable"
        );
    }
    let assignment = |engine: &Engine<Vec<u8>>| {
        engine
            .acquisition_gameplay
            .as_ref()
            .unwrap()
            .state
            .squad(squad)
            .unwrap()
            .assignment
    };
    let original = assignment(&engine);
    assert_eq!(tag(&mut engine, TARGET, 501)["accepted"], true);
    assert_eq!(assignment(&engine).tagged_object, TARGET);
    assert_eq!(assignment(&engine).primary_index, original.primary_index);
    assert_eq!(
        set_waypoint(&mut engine, 0, false, 502)["reason"],
        "squad_order_cooldown"
    );
    assert_eq!(tag(&mut engine, u32::MAX, 502)["accepted"], true);
    assert_eq!(assignment(&engine), original);
    assert_eq!(
        tag(&mut engine, TARGET, 1001)["reason"],
        "squad_order_cooldown"
    );
    assert_eq!(tag(&mut engine, TARGET, 1002)["accepted"], true);
    let max_log = engine.max_log;
    actor_mut(&mut engine, 0).nominations = 0;
    engine.max_log = engine.written;
    assert!(engine.reconcile_squad_leaders().is_err());
    assert_eq!(assignment(&engine).tagged_object, TARGET);
    engine.max_log = max_log;
    engine.failed = false;
    engine.reconcile_squad_leaders().unwrap();
    assert_eq!(assignment(&engine), original);
    assert!(engine.pending_rewards.is_empty());
}

#[test]
fn traffic_driven_event_returns_order_and_roster_replacement_contains_cleanup() {
    let mut engine = fixture();
    engine.world_tick_enabled = false;
    let squad = actor(&engine, LEADER).squad_catalog.squad_index();
    let response = event(&mut engine, LEADER, &waypoint(false), 501, 1);
    assert!(response.ok, "{:?}", response.error);
    assert!(
        response
            .messages
            .iter()
            .filter(|packet| packet.player_id == LEADER)
            .filter_map(|packet| packet_orders(
                &protocol::decode_hex(&packet.payload_hex),
                engine.wire_profile,
                squad
            ))
            .any(|order| order.active == Some(true) && order.has_position)
    );
    let response = event(&mut engine, LEADER, &waypoint(true), 502, 1);
    assert!(response.ok);
    assert!(
        response
            .messages
            .iter()
            .filter(|packet| packet.player_id == LEADER)
            .filter_map(|packet| packet_orders(
                &protocol::decode_hex(&packet.payload_hex),
                engine.wire_profile,
                squad
            ))
            .any(|order| order.active == Some(false) && !order.has_position)
    );
    assert_eq!(
        set_waypoint(&mut engine, LEADER, false, 1002)["accepted"],
        true
    );
    engine.queue_roster(Duration::from_millis(1003)).unwrap();
    let replacement = 6;
    crate::spawn_progression::reserve(&mut engine, replacement);
    let replacement_actor = actor_mut(&mut engine, replacement);
    replacement_actor.squad_catalog = replacement_actor
        .squad_catalog
        .for_squad(squad)
        .unwrap()
        .into();
    crate::spawn_progression::ready(replacement_actor);
    replacement_actor.nominations = 1;
    actor_mut(&mut engine, LEADER).pending_shared.clear();
    actor_mut(&mut engine, LEADER).nominations = 0;
    engine.queue_roster(Duration::from_millis(1004)).unwrap();
    let replacements: Vec<_> = actor(&engine, LEADER)
        .pending_shared
        .iter()
        .filter_map(|wire| packet_orders(wire, engine.wire_profile, squad))
        .filter(|order| order.leader.is_some())
        .collect();
    assert_eq!(replacements.len(), 1);
    assert_eq!(replacements[0].leader, Some(replacement));
    assert_eq!(replacements[0].tag, Some(u32::MAX));
    assert_eq!(replacements[0].active, Some(false));
    assert!(!replacements[0].has_position);
}

const SABOTAGE_LEADER: u16 = 1;
const SABOTAGE_MEMBER: u16 = 2;
const SABOTAGE_SQUAD: u16 = 16;

fn sabotage_leadership() -> (
    leadership_passives::LeadershipCatalog,
    leadership_runtime::RolePolicy,
) {
    let (passives, _) = crate::platoon_ability_engine_tests::platoon_leadership();
    let policy = leadership_runtime::RolePolicy::from_json(
        &json!({
            "schema_version":2,"application_id":20231,"squad_leader":"native","development_bindings":[],
            "native_election":{"source_sha256":"c3".repeat(32),"squad_level_weight":1.0,
                "squad_mslc_weight":1.0,"default_record":{
                    "leadership_level":1,"nominations":1,"matches_since_last_command":0}}
        })
        .to_string(),
    )
    .unwrap();
    (passives, policy)
}

/// Independently authored Sabotage worker: game type 0 objective catalog, no
/// Acquisition/Domination provider, native squad election and squad orders.
fn sabotage_fixture(orders: bool) -> Engine<Vec<u8>> {
    let (passives, policy) = sabotage_leadership();
    let mut base = crate::spawn_progression::tests::engine()
        .with_leadership_passives(passives, policy)
        .unwrap();
    if orders {
        base = base
            .with_squad_orders(squad_orders::Rules {
                squad_order_cooldown_ms: 1000,
            })
            .unwrap();
    }
    let mut engine = crate::human_kill_reward_tests::configured_fixture(base);
    crate::spawn_progression::reserve(&mut engine, SABOTAGE_MEMBER);
    let catalog = std::sync::Arc::new(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(SABOTAGE_SQUAD)
            .unwrap(),
    );
    let member = actor_mut(&mut engine, SABOTAGE_MEMBER);
    member.squad_catalog = catalog;
    crate::spawn_progression::ready(member);
    member.nominations = 0;
    // Keep the original killer as the event-dispatching active actor.
    if let Some(active) = engine.active.take() {
        engine.peers.insert(active.reservation.id.clone(), active);
    }
    engine.active = engine
        .peers
        .remove(&format!("synthetic-reservation-{SABOTAGE_LEADER:04}"));
    assert!(engine.active.is_some());
    actor_mut(&mut engine, 0).nominations = 0;
    actor_mut(&mut engine, SABOTAGE_LEADER).nominations = 1;
    engine.match_started = true;
    engine.reconcile_squad_leaders().unwrap();
    assert_eq!(engine.match_profile().game_type, 0);
    assert!(engine.sabotage_squad_assignments());
    assert_eq!(
        engine.squad_leader_index(SABOTAGE_SQUAD, &[u16::MAX; 8]),
        SABOTAGE_LEADER
    );
    engine
}

fn tag_as(engine: &mut Engine<Vec<u8>>, player: u16, target_id: u32, millis: u64) -> Value {
    let mut wire = 0x0206u16.to_be_bytes().to_vec();
    wire.extend(target_id.to_be_bytes());
    let response = event(engine, player, &wire, millis, 1);
    assert!(response.ok, "{:?}", response.error);
    assert!(response.rewards.is_empty());
    last_decision(engine, "tagged_object")
}

fn published_tag(engine: &Engine<Vec<u8>>, player: u16, squad: u16) -> Option<u32> {
    actor(engine, player)
        .pending_shared
        .iter()
        .filter_map(|wire| packet_orders(wire, engine.wire_profile, squad)?.tag)
        .last()
}

#[test]
fn sabotage_held_leader_sets_changes_and_clears_a_replicated_objective_tag() {
    const TARGET: u32 = 0x0c80_000b;
    const CHANGED: u32 = 0x0c80_000a;
    let mut engine = sabotage_fixture(true);
    engine.world_tick_enabled = true;
    let tag_state = |engine: &Engine<Vec<u8>>| order_state(engine).manual_tag(SABOTAGE_SQUAD);

    // Neither a member of the leader's squad nor another squad's member may tag.
    for player in [SABOTAGE_MEMBER, 0] {
        assert_eq!(
            tag_as(&mut engine, player, TARGET, 501)["reason"],
            "squad_order_requires_held_squad_leader"
        );
    }
    for forged in [0x0c80_0063, 0x0d80_000b, 0x0c80_ffff] {
        assert_eq!(
            tag_as(&mut engine, SABOTAGE_LEADER, forged, 501)["reason"],
            "squad_order_target_unavailable"
        );
    }
    assert_eq!(tag_state(&engine), u32::MAX);

    assert_eq!(
        tag_as(&mut engine, SABOTAGE_LEADER, TARGET, 501)["accepted"],
        true
    );
    assert_eq!(tag_state(&engine), TARGET);
    engine
        .queue_squad_orders(Duration::from_millis(501), 1)
        .unwrap();
    assert_eq!(
        engine.squad_roster_orders(SABOTAGE_SQUAD),
        Some((TARGET, false))
    );
    let assignment = engine
        .frago_assignment(actor(&engine, SABOTAGE_MEMBER))
        .unwrap();
    assert_eq!(assignment.tagged_object, TARGET);
    assert_eq!(assignment.primary_index, 10);
    for player in [SABOTAGE_LEADER, SABOTAGE_MEMBER] {
        assert_eq!(
            published_tag(&engine, player, SABOTAGE_SQUAD),
            Some(TARGET),
            "squad member {player} receives the FRAGO"
        );
    }

    // Changing shares the native set gate; clearing does not.
    assert_eq!(
        tag_as(&mut engine, SABOTAGE_LEADER, CHANGED, 502)["reason"],
        "squad_order_cooldown"
    );
    assert_eq!(
        tag_as(&mut engine, SABOTAGE_LEADER, u32::MAX, 502)["accepted"],
        true
    );
    assert_eq!(tag_state(&engine), u32::MAX);
    engine
        .queue_squad_orders(Duration::from_millis(502), 1)
        .unwrap();
    assert_eq!(
        published_tag(&engine, SABOTAGE_MEMBER, SABOTAGE_SQUAD),
        Some(u32::MAX)
    );
    assert_eq!(
        tag_as(&mut engine, SABOTAGE_LEADER, CHANGED, 1501)["accepted"],
        true
    );
    assert_eq!(tag_state(&engine), CHANGED);

    // Losing the held role clears the order with the reconciliation.
    actor_mut(&mut engine, SABOTAGE_LEADER).nominations = 0;
    engine.reconcile_squad_leaders().unwrap();
    assert_eq!(
        engine.squad_leader_index(SABOTAGE_SQUAD, &[u16::MAX; 8]),
        u16::MAX
    );
    assert_eq!(tag_state(&engine), u32::MAX);
    actor_mut(&mut engine, SABOTAGE_LEADER).nominations = 1;
    engine.reconcile_squad_leaders().unwrap();
    assert_eq!(
        tag_as(&mut engine, SABOTAGE_LEADER, TARGET, 2600)["accepted"],
        true
    );
    engine.reset_objective_world();
    assert_eq!(tag_state(&engine), u32::MAX);
    assert!(engine.pending_rewards.is_empty());
}

#[test]
fn sabotage_without_squad_order_rules_denies_with_a_configuration_reason() {
    let mut engine = sabotage_fixture(false);
    assert_eq!(
        tag_as(&mut engine, SABOTAGE_LEADER, 0x0c80_000b, 501)["reason"],
        "squad_orders_unconfigured"
    );
}

fn sabotage_geometry(engine: &mut Engine<Vec<u8>>, near: &[u16]) {
    let map_name = engine.map_name().to_owned();
    let catalog = engine.objective_catalog.as_mut().unwrap();
    let objectives: Vec<_> = catalog
        .objects
        .iter()
        .map(|o| {
            let position = if near.contains(&o.object_index) {
                [130., 3., 100.]
            } else {
                [1000., 3., 1000.]
            };
            json!({"object_index":o.object_index,"position":position,"radius":5.})
        })
        .collect();
    catalog.frago = Some(
        serde_json::from_value(json!({"schema_version":1,"map_name":map_name,"game_type":0,
            "source":{"authored_catalog_sha256":catalog.source().authored_catalog_sha256,
                "progression_catalog_sha256":"b".repeat(64),
                "source_image_sha256":catalog.source().source_image_sha256},
            "vehicle_range":7.,"objectives":objectives}))
        .unwrap(),
    );
}

fn sabotage_kill(engine: &mut Engine<Vec<u8>>, now_ms: u64) -> progression::RewardEvent {
    let leader = actor_mut(engine, SABOTAGE_LEADER);
    leader.reservation.character.frago_bonus_enabled = Some(true);
    leader.tracking.server_position(
        replication::CharacterPosition {
            x: 130.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    let before: std::collections::BTreeSet<_> = engine.pending_rewards.keys().cloned().collect();
    crate::human_kill_reward_tests::hit(engine, 20, now_ms).unwrap();
    engine
        .pending_rewards
        .values()
        .find(|r| r.kind == progression::RewardKind::EnemyKilled && !before.contains(&r.reward_id))
        .unwrap()
        .clone()
}

#[test]
fn sabotage_frago_kill_bonus_follows_the_manual_tag_or_current_primary() {
    // Only the tagged objective is in range: no bonus until the leader tags it.
    let mut engine = sabotage_fixture(true);
    sabotage_geometry(&mut engine, &[11]);
    assert_eq!(sabotage_kill(&mut engine, 1000).frago, None);
    let mut engine = sabotage_fixture(true);
    sabotage_geometry(&mut engine, &[11]);
    assert_eq!(
        tag_as(&mut engine, SABOTAGE_LEADER, 0x0c80_000b, 501)["accepted"],
        true
    );
    assert_eq!(sabotage_kill(&mut engine, 1000).frago, Some(true));

    // The current primary objective qualifies on its own.
    let mut engine = sabotage_fixture(true);
    sabotage_geometry(&mut engine, &[10]);
    assert_eq!(sabotage_kill(&mut engine, 1000).frago, Some(true));
}

fn sabotage_activation_rewards(frago_admitted: bool) -> Vec<progression::RewardEvent> {
    let mut engine = sabotage_fixture(true);
    sabotage_geometry(&mut engine, &[]);
    let catalog = engine.objective_catalog.as_mut().unwrap();
    catalog
        .objects
        .iter_mut()
        .find(|o| o.object_index == 10)
        .unwrap()
        .reward = Some(objective_catalog::ObjectiveRewardRule {
        category: 1,
        subtype: 12,
        demolition: false,
    });
    engine.objective_world = Some(objective_control::ObjectiveWorld::new(catalog));
    let attacker = actor_mut(&mut engine, 0);
    attacker.reservation.character.frago_objective_bonus_enabled = Some(frago_admitted);
    if let Some(active) = engine.active.take() {
        engine.peers.insert(active.reservation.id.clone(), active);
    }
    engine.active = engine.peers.remove("synthetic-reservation-0000");
    assert_eq!(
        engine
            .frago_assignment(actor(&engine, 0))
            .unwrap()
            .primary_index,
        10
    );
    let wire = protocol::ObjectiveInteraction {
        target_object_id: 0x0c80_000a,
        kind: protocol::ObjectiveInteractionKind::Activate,
        actor_index: 0,
        trigger_index: 0,
    }
    .encode();
    crate::spawn_progression::tests::dispatch(&mut engine, &wire, 100).unwrap();
    engine
        .tick_objectives(Duration::from_millis(110), 1)
        .unwrap();
    engine.pending_reward_batch()
}

#[test]
fn sabotage_primary_objective_rewards_carry_the_frago_fact_only_when_admitted() {
    let rewards = sabotage_activation_rewards(true);
    assert_eq!(
        rewards[0].kind,
        progression::RewardKind::PrimaryObjectiveActivated
    );
    assert!(rewards.iter().all(|reward| reward.frago == Some(true)));
    let rewards = sabotage_activation_rewards(false);
    assert!(!rewards.is_empty());
    assert!(rewards.iter().all(|reward| reward.frago.is_none()));
}

#[test]
fn automatic_bot_tags_skip_leader_authority_and_the_set_gate_until_a_leader_tags() {
    const AUTOMATIC: u32 = 0x0c80_000a;
    const LEADER_TAG: u32 = 0x0c80_000b;
    let mut engine = sabotage_fixture(true);
    assert!(engine.squad_has_held_leader(SABOTAGE_SQUAD));
    assert!(!engine.squad_has_held_leader(0));
    // An unknown target is reported per squad and changes nothing.
    let (candidate, results) = engine
        .plan_automatic_squad_tags(&[(SABOTAGE_SQUAD, 0x0c80_0063)])
        .unwrap();
    assert!(candidate.is_none());
    assert_eq!(
        results[&SABOTAGE_SQUAD],
        Err("squad_order_target_unavailable")
    );
    let (candidate, results) = engine
        .plan_automatic_squad_tags(&[(SABOTAGE_SQUAD, AUTOMATIC)])
        .unwrap();
    assert_eq!(results[&SABOTAGE_SQUAD], Ok(()));
    engine.install_squad_order_candidate(candidate.unwrap());
    assert_eq!(
        engine.squad_order_tags(SABOTAGE_SQUAD),
        Some((AUTOMATIC, AUTOMATIC))
    );
    engine
        .queue_squad_orders(Duration::from_millis(500), 1)
        .unwrap();
    assert_eq!(
        published_tag(&engine, SABOTAGE_MEMBER, SABOTAGE_SQUAD),
        Some(AUTOMATIC)
    );
    // The inherited tag stays the server's until the held leader tags.
    let mut book = crate::bot_squad_orders::Book::default();
    book.set(SABOTAGE_SQUAD, AUTOMATIC);
    assert!(!book.human_order(SABOTAGE_SQUAD, true, AUTOMATIC));
    // No set gate was stamped: the leader may tag at once.
    assert_eq!(
        tag_as(&mut engine, SABOTAGE_LEADER, LEADER_TAG, 501)["accepted"],
        true
    );
    let (_, manual) = engine.squad_order_tags(SABOTAGE_SQUAD).unwrap();
    assert_eq!(manual, LEADER_TAG);
    assert!(book.human_order(SABOTAGE_SQUAD, true, manual));
}
