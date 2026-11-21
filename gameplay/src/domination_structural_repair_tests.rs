//! Authenticated Domination Structural repair callbacks with synthetic assets.
use super::*;
use crate::{
    CharacterLife, DecodedEvent, Direction, Engine, OutboundMessage, Transport,
    acquisition_structural::connected_tests::terrain_and_assets,
    domination_gameplay_effects::{OwnedAssignment, Structural},
    domination_gameplay_provider::{State, World},
    domination_structural_world::SequenceCollisionState,
    objective_catalog::ObjectiveRepairRule,
    progression::RewardKind,
    spawn_progression::{engine, ready, reserve},
    structural_repair_authority::tests::{
        equip_synthetic_repair_kit, synthetic_repair_bunker_world,
    },
};

fn repair_fixture() -> Engine<Vec<u8>> {
    let (mut runtime, _, spawns, round) = fixture();
    runtime.started = true;
    let mut objectives = (*runtime.setup.catalog).clone();
    objectives.objects[0].initial_state = ObjectiveState::Completed as u8;
    objectives.objects[0].repair = Some(ObjectiveRepairRule {
        owner_team: 1,
        can_repair: true,
        repair_ms: 1000,
        minimum_repair_ms: 1000,
    });
    let states = [
        "Inactive",
        "Deactivated",
        "Activating",
        "Activated",
        "Deactivating",
        "Repairing",
        "Repaired",
        "Completed",
        "Locked",
        "Retaking",
        "Retaken",
        "CancelRetaken",
    ];
    let asset_hash = "a".repeat(64);
    let pose = json!({"translation":[1.,2.,1.],"rotation":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]]});
    let placements = ["0x0000000a", "0x0000000b"]
        .map(|token| json!({"objective_token":token,"definition_token":"0x000000aa","pose":pose}));
    let structural = Structural::from_json(&json!({
        "schema":"mag_domination_structural_effect_inputs_v1","map":"synthetic",
        "metadata_sha256":objectives.source().authored_catalog_sha256,"level_archive_sha256":asset_hash,
        "level_clv_sha256":asset_hash,"global_data_sha256":asset_hash,"physics_sha256":asset_hash,
        "collision_resources":{},"unresolved_collision_tokens":[],
        "definitions":{"0x000000aa":{"definition_sha256":asset_hash,
            "state_sequence_indices":BTreeMap::from_iter(states.map(|name| (name,-2))),
            "sequence_tokens":[],"blast_properties":{"SubType":1,"FullDamageRadius":0.,
                "HalfDamageRadius":0.,"DamageValue":0.,"MaterialPenetration":0.,
                "CharacterDamageOverTimeRadius":0.,"CharacterDamageOverTimeValue":0.}}},
        "placements":placements,"sequences":{}
    }).to_string(), &objectives, "synthetic", &asset_hash, &asset_hash).unwrap();
    let (assets, terrain_materials) = terrain_and_assets(structural.catalog());
    let mut engine = engine();
    reserve(&mut engine, 0);
    equip_synthetic_repair_kit(&mut engine);
    engine.bunker_world = Some(synthetic_repair_bunker_world());
    let actor = engine.active.as_mut().unwrap();
    ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.squad_catalog = Arc::new(spawns.for_squad(16).unwrap());
    actor.reservation.character.account_id = Some("synthetic-repairer".into());
    actor.reservation.character.repair_rewards_enabled = Some(true);
    actor.reservation.character.frago_action_bonus_enabled = Some(true);
    engine.match_started = true;
    objectives.frago = Some(serde_json::from_value(json!({
        "schema_version":1,"map_name":"synthetic","game_type":2,
        "source":{"authored_catalog_sha256":objectives.source().authored_catalog_sha256,
            "progression_catalog_sha256":"b".repeat(64),"source_image_sha256":objectives.source().source_image_sha256},
        "vehicle_range":7.,"objectives":[{"object_index":0,"position":[1.,2.,1.],"radius":5.},
            {"object_index":1,"position":[2.,2.,1.],"radius":5.}]
    })).unwrap());
    engine.objective_world = Some(ObjectiveWorld::new(&objectives));
    engine.objective_catalog = Some(objectives.clone());
    engine.spawn_catalog = Some(spawns);
    engine.domination_round = Some(round);
    let mut assignments = [None; 32];
    assignments[16] = Some(OwnedAssignment {
        assignment: crate::acquisition_objective_runtime::Assignment {
            primary_index: 1,
            tagged_object: u32::MAX,
        },
        manual_tag: u32::MAX,
    });
    engine.domination_gameplay = Some(World {
        runtime,
        state: State {
            objectives: Arc::new(objectives),
            structural,
            assets,
            hospital_hulks: None,
            hospital_turret_yaws: Default::default(),
            helo: None,
            helo_collision: None,
            helo_damage: None,
            root_poses: Default::default(),
            trigger_positions: Default::default(),
            filter_rows: [0; 32],
            terrain_materials,
            ground_respawn_physics: None,
            ground_damage: Arc::new(
                serde_json::from_value(json!({
                    "friendly_fire_multiplier":0.25,"profiles":[],
                    "areas":vec![json!({"armor_area":0,"multiplier":1.});16]
                }))
                .unwrap(),
            ),
            ground_capabilities: Default::default(),
            ground_material_penetration: 0.,
            vehicle_waves: None,
            last_ground_update: None,
            known_empty_vehicle_spawns: Default::default(),
            disabled_helo_spawns: Default::default(),
            platoons: Default::default(),
            assignments,
            ability_enabled: [u16::MAX; 8],
            collisions: SequenceCollisionState::default(),
            initial_collisions: Default::default(),
            area: Default::default(),
        },
    });
    engine
}

fn repair(engine: &mut Engine<Vec<u8>>, now_ms: u64) -> Result<Vec<OutboundMessage>, &'static str> {
    let wire = [0x02, 0x1d, 0x0c, 0x80, 0x00, 0x00];
    let actor = engine.active.as_ref().unwrap();
    let event = crate::events::EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: actor.reservation.world,
        player_id: actor.reservation.player,
        transport: Transport::Tcp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(&wire),
    };
    let decoded = DecodedEvent::with_profile(&wire, event.player_id, engine.wire_profile, false);
    engine.handle_event(event, &decoded, test_time(now_ms), 1)
}

#[test]
fn accepted_structural_queue_credits_repaired_object_context_before_hp_changes() {
    let mut engine = repair_fixture();
    let messages = repair(&mut engine, 200).unwrap();
    let world = engine.objective_world.as_ref().unwrap();
    assert_eq!(world.state(0), Some(ObjectiveState::Completed));
    assert_eq!(world.snapshot(1, 0).objectives[0].health_percent, 0);
    let rewards = engine.pending_reward_batch();
    assert_eq!(rewards.len(), 1);
    assert_eq!(rewards[0].kind, RewardKind::RepairUnits);
    assert_eq!(rewards[0].units, 1);
    assert_eq!(rewards[0].frago, Some(true));
    assert!(
        messages
            .iter()
            .any(|message| message.payload_hex == "02040000012301")
    );
}

#[test]
fn enemy_actor_and_covered_queue_leave_repair_credit_unchanged() {
    let mut engine = repair_fixture();
    let spawns = engine.spawn_catalog.as_ref().unwrap();
    let owner = engine.active.as_ref().unwrap().squad_catalog.clone();
    engine.active.as_mut().unwrap().squad_catalog = Arc::new(spawns.for_squad(0).unwrap());
    assert_eq!(
        repair(&mut engine, 200).unwrap_err(),
        "structural_repair_requires_live_owner"
    );
    assert_eq!(engine.active.as_ref().unwrap().repair_credit.sequence, 0);
    assert!(engine.pending_reward_batch().is_empty());
    engine.active.as_mut().unwrap().squad_catalog = owner;
    repair(&mut engine, 200).unwrap();
    let accepted = engine.pending_reward_batch();
    assert_eq!(
        repair(&mut engine, 201).unwrap_err(),
        "structural_repair_health_already_covered"
    );
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.repair_credit.sequence, 1);
    assert_eq!(actor.reward_statistics.get(&35), Some(&1));
    assert_eq!(engine.pending_reward_batch(), accepted);
}

#[test]
fn failed_record_rolls_back_the_structural_queue_and_repair_credit() {
    let mut engine = repair_fixture();
    let limit = engine.max_log;
    engine.max_log = engine.written;
    assert_eq!(repair(&mut engine, 200).unwrap_err(), "log_limit_reached");
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.repair_credit, Default::default());
    assert!(!actor.reward_statistics.contains_key(&35));
    assert!(engine.pending_reward_batch().is_empty());
    engine.max_log = limit;
    engine.failed = false;
    repair(&mut engine, 200).unwrap(); // A leaked full contribution would reject the retry.
    assert_eq!(engine.pending_reward_batch().len(), 1);
    assert_eq!(engine.active.as_ref().unwrap().repair_credit.sequence, 1);
}
