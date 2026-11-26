//! Source-shaped manual hospital repairs through the real structure handler.
use super::*;
use std::sync::Arc;

const HOSPITAL: u32 = 0x0d80_0000;

fn fixture() -> Engine<Vec<u8>> {
    let mut engine = bunker_team_tests::fixture(0, 0, true);
    engine.runtime_application_id = Some(20231);
    engine.wire_profile = engine.wire_profiles.profile(20231).unwrap();
    let retail = crate::resources::tests::retail_items();
    engine.item_catalog = Arc::new(
        item_catalog::ItemCatalog::from_value_for_application(
            json!({
                "schema_version": 2, "application_id": 20231,
                "items": {"FactionItemList0:99": {
                    "authored_ref": "FactionItemList0:99", "faction_list": "FactionItemList0",
                    "local_ordinal": 99, "wire_item_id": null, "name": "repairkit",
                    "item_type": "kActiveItem",
                    "definition": {"group": "active_items", "name": "repairkit"}
                }},
                "definitions": {"active_items": {"repairkit": {"parameters": {}}}},
                "resources": {}
            }),
            retail.character_blast_rules(),
            20231,
        )
        .unwrap(),
    );
    let actor = engine.active.as_mut().unwrap();
    actor.wire_profile = engine.wire_profile;
    actor.emplacement_creation = Some(Vec::new());
    actor.observed_primary.as_mut().unwrap().gear_slot = 3;
    actor.reservation.character.account_id = Some("synthetic-engineer".into());
    let world = engine.bunker_world.as_mut().unwrap();
    world.hospital_spawn_sha256 = Some("qualified synthetic profile".into());
    let hospital = world.bunkers.get_mut(&0).unwrap();
    hospital.hospital = Some(HospitalAccess {
        mount_radii: vec![],
        max_occupancy: 12,
        passenger_seats: 12,
        direct_damage: true,
    });
    hospital.exit = Some(vehicle_exit::BunkerExitPolicy {
        transform: vehicle_exit::BunkerTransform {
            position: replication::CharacterPosition {
                x: 104.,
                y: 3.,
                z: 100.,
            },
            rotation_xyzw: [0., 0., 0., 1.],
        },
        candidates: vec![vehicle_exit::ExitCandidate {
            ranking_offset: [1., 0., 0.],
            position_offset: [1., 0., 0.],
        }],
    });
    hospital.health.damage(40.).unwrap();
    hospital.damage.health_quarter_points = 640;
    hospital.profile.repair_seconds = 1600.; // 1 s cooldown on a 200 HP body gives 0.125 HP.
    engine
}

fn repair(engine: &mut Engine<Vec<u8>>, second: u64) -> Result<Vec<OutboundMessage>, &'static str> {
    let mut wire = 0x021d_u16.to_be_bytes().to_vec();
    wire.extend(HOSPITAL.to_be_bytes());
    let active = engine.active.as_ref().unwrap();
    let input = events::EventInput {
        reservation_id: active.reservation.id.clone(),
        world_id: active.reservation.world,
        player_id: active.reservation.player,
        transport: Transport::Tcp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(&wire),
    };
    let decoded = DecodedEvent::with_profile(
        &wire,
        input.player_id,
        engine.wire_profile,
        engine.ground_driver_enabled(),
    );
    engine.handle_event(input, &decoded, Duration::from_secs(second), second)
}

fn hospital(engine: &Engine<Vec<u8>>) -> &Bunker {
    &engine.bunker_world.as_ref().unwrap().bunkers[&0]
}

fn repair_stat(engine: &Engine<Vec<u8>>) -> i32 {
    engine
        .active
        .as_ref()
        .unwrap()
        .reward_statistics
        .get(&35)
        .copied()
        .unwrap_or_default()
}

fn enable_receipts(engine: &mut Engine<Vec<u8>>) {
    let actor = engine.active.as_mut().unwrap();
    actor
        .reservation
        .character
        .account_id
        .get_or_insert_with(|| "synthetic-engineer".into());
    actor.reservation.character.repair_rewards_enabled = Some(true);
    actor.reservation.character.frago_action_bonus_enabled = Some(true);
}

#[test]
fn float_repair_updates_stat_without_crossing_a_wire_quarter_or_requiring_xp_capability() {
    let mut engine = fixture();
    let messages = repair(&mut engine, 1).unwrap();
    assert_eq!(hospital(&engine).health.hp(), 160.125);
    assert_eq!(hospital(&engine).damage.health_quarter_points, 640);
    assert_eq!(repair_stat(&engine), 1);
    assert!(engine.pending_reward_batch().is_empty());
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].player_id, 0);
    assert_eq!(messages[0].payload_hex, "02040000012301");
    assert_eq!(engine.active.as_ref().unwrap().pending_shared.len(), 1);
}

#[test]
fn repair_requires_positive_applied_float_health_for_credit() {
    let mut engine = fixture();
    enable_receipts(&mut engine);
    // The requested positive amount is below half a float ULP at 160 HP.
    engine
        .bunker_world
        .as_mut()
        .unwrap()
        .bunkers
        .get_mut(&0)
        .unwrap()
        .profile
        .repair_seconds = 40_000_000.;
    assert!(repair(&mut engine, 1).unwrap().is_empty());
    assert_eq!(hospital(&engine).health.hp(), 160.);
    assert_eq!(repair_stat(&engine), 0);
    assert!(engine.pending_reward_batch().is_empty());
}

#[test]
fn source_team_controls_credit_independently_of_faction_and_successful_repair() {
    for (team, faction, expected_units) in [(0, 2, 0), (1, 0, 1)] {
        let mut engine = fixture();
        enable_receipts(&mut engine);
        let mut attacker = bunker_team_tests::fixture(team, team, true)
            .active
            .take()
            .unwrap();
        attacker.reservation.character.faction = faction;
        attacker.reservation.player = 1;
        attacker.reservation.id = "previous-attacker".into();
        engine
            .peers
            .insert(attacker.reservation.id.clone(), attacker);
        let damage = &mut engine
            .bunker_world
            .as_mut()
            .unwrap()
            .bunkers
            .get_mut(&0)
            .unwrap()
            .damage;
        damage.last_attacker = 0x0380_0001;
        damage.last_location_and_type = 0x13;
        damage.last_user_data = 17;
        repair(&mut engine, 1).unwrap();
        assert_eq!(hospital(&engine).health.hp(), 160.125);
        let damage = hospital(&engine).damage;
        assert_eq!(damage.last_attacker, 0x0380_0001);
        assert_eq!(damage.last_location_and_type, 0x13);
        assert_eq!(damage.last_user_data, 17);
        assert_eq!(repair_stat(&engine), expected_units);
        assert_eq!(engine.pending_reward_batch().len(), expected_units as usize);
    }
}

#[test]
fn repaired_hospital_position_qualifies_frago_with_repairer_out_of_range() {
    let mut engine = fixture();
    enable_receipts(&mut engine);
    let mut mode =
        spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::capture_fixture();
    engine.acquisition_gameplay = mode.acquisition_gameplay.take();
    frago_tests::configure_geometry(&mut engine);
    engine
        .acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(0)
        .unwrap()
        .assignment = acquisition_objective_runtime::Assignment {
        primary_index: u16::MAX,
        tagged_object: HOSPITAL,
    };
    let actor = engine.active.as_mut().unwrap();
    actor.tracking.server_position(
        replication::CharacterPosition {
            x: 200.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    repair(&mut engine, 1).unwrap();
    let rewards = engine.pending_reward_batch();
    assert_eq!(rewards.len(), 1);
    assert_eq!(rewards[0].kind, progression::RewardKind::RepairUnits);
    assert_eq!(rewards[0].units, 1);
    assert_eq!(rewards[0].frago, Some(true));
}

#[test]
fn failed_record_preserves_health_carry_and_publications_then_retry_credits_once() {
    let mut engine = fixture();
    enable_receipts(&mut engine);
    let skills = skill_catalog::SkillCatalog::from_value(json!({
        "schema_version": 2, "application_id": 20231,
        "table": {"Skills": {"MagCharacterSkill1": {"MagCharacterSkill": {
            "Default": {"Modifier": "kCharModRepairAmount", "ModifierValue": 0.5},
            "Green": {"Available": true}
        }}}}
    }))
    .unwrap();
    engine.active.as_mut().unwrap().modifiers = skills.resolve(0, &[2, 0, 0, 0, 0]).unwrap();
    let max_log = engine.max_log;
    engine.max_log = engine.written;
    assert_eq!(repair(&mut engine, 1).err(), Some("log_limit_reached"));
    assert_eq!(hospital(&engine).health.hp(), 160.);
    assert_eq!(repair_stat(&engine), 0);
    assert!(engine.next_repair.is_empty());
    assert!(engine.pending_reward_batch().is_empty());
    assert!(engine.active.as_ref().unwrap().pending_shared.is_empty());

    engine.failed = false;
    engine.max_log = max_log;
    assert_eq!(repair(&mut engine, 1).unwrap().len(), 1);
    assert_eq!(hospital(&engine).health.hp(), 160.1875);
    assert_eq!(repair_stat(&engine), 1);
    assert_eq!(engine.pending_reward_batch()[0].units, 1);
    assert_eq!(engine.active.as_ref().unwrap().pending_shared.len(), 1);
    // Two distinct deliveries can share the server's processing timestamp.
    assert_eq!(repair(&mut engine, 1).unwrap().len(), 1);
    assert_eq!(hospital(&engine).health.hp(), 160.375);
    assert_eq!(engine.active.as_ref().unwrap().pending_shared.len(), 2);
    let rewards = engine.pending_reward_batch();
    assert_eq!(rewards.len(), 2);
    assert_eq!(rewards[1].units, 2);
    assert_eq!(repair_stat(&engine), 3);
    assert_ne!(rewards[0].reward_id, rewards[1].reward_id);
}

#[test]
fn leadership_automatic_repair_changes_ground_hp_without_awarding_repair_credit() {
    let catalog = leadership_passives::LeadershipCatalog::from_json(
        &json!({"schema_version": 1, "application_id": 20231,
            "source": {"member": "scripts/character_leadership_abilities.zpr",
                "member_sha256": "a".repeat(64), "archive_sha256": "b".repeat(64),
                "extractor": "synthetic", "projector": "synthetic"},
            "max_separation_range": 30., "refresh_hold_ms": 3000, "removal_grace_ms": 1000,
            "levels": [
                {"level": 0, "name": "none", "modifiers": []},
                {"level": 1, "name": "squad", "modifiers": [{"kind": "kCharModReloadTime", "value": 0.1}]},
                {"level": 2, "name": "platoon", "modifiers": [{"kind": "kCharModAutoRepair", "value": 1.}]},
                {"level": 3, "name": "officer", "modifiers": [{"kind": "kCharModAutoHeal", "value": 1.}]}
            ]
        }).to_string(),
    )
    .unwrap();
    let policy = leadership_runtime::RolePolicy::from_json(
        &json!({"schema_version": 1, "application_id": 20231,
            "first_member_squad_leader": false,
            "development_bindings": [{"account_id": 1, "character_id": 1, "native_role": 2}]
        })
        .to_string(),
    )
    .unwrap();
    let mut engine =
        spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::automatic_repair_fixture(catalog, policy);
    enable_receipts(&mut engine);
    let ground = engine.ground_runtime.as_mut().unwrap();
    let key = ground.get(4).unwrap().key;
    ground.apply_damage(key, 160, 0x0380_0007).unwrap();
    for ms in [3200, 4750] {
        engine
            .tick_leadership_auto_repair(Duration::from_millis(ms), 3)
            .unwrap();
    }
    let ground = engine.ground_runtime.as_ref().unwrap();
    assert_eq!(ground.get(4).unwrap().health_quarter_points, 449);
    assert_eq!(repair_stat(&engine), 0);
    assert!(engine.pending_reward_batch().is_empty());
}
