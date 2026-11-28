//! Ordinary native Structural interactions through authenticated Engine events.
use super::*;
use crate::{
    CharacterLife, DecodedEvent, Direction, Transport, acquisition_gameplay_engine,
    objective_catalog::{ObjectiveDemolitionRule, ObjectiveRepairRule},
    objective_range::TriggerBox,
    protocol::{ObjectiveInteraction, ObjectiveInteractionKind as Kind},
    replication::{CharacterPosition, ObjectiveState},
    spawn_progression,
};
use sha2::{Digest, Sha256};

const INDEX: u16 = 120;
const TARGET: u32 = 0x0c80_0078;

pub(crate) fn terrain_and_assets(
    catalog: &Catalog,
) -> (Arc<SequenceCollisionAssets>, Arc<TerrainMaterialGrid>) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "mag-acquisition-structural-{}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("index.json"),
        json!({
            "schema":"mag_domination_structural_collision_v1", "map":catalog.map,
            "level_archive_sha256":"a".repeat(64),"physics_sha256":"a".repeat(64),
            "geometry_sha256":"a".repeat(64),"resources":{},"parent_reference_poses":{}
        })
        .to_string(),
    )
    .unwrap();
    let assets = SequenceCollisionAssets::load(&root, catalog).unwrap();
    let pixels = vec![0u8; 4096 * 4096];
    std::fs::write(root.join("materials.bin"), &pixels).unwrap();
    std::fs::write(
        root.join("terrain.json"),
        json!({
            "schema":"mag_domination_terrain_material_grid_v1", "map":catalog.map,
            "level_archive_sha256":"a".repeat(64),"level_clv_sha256":"a".repeat(64),
            "retail_elf_sha256":"3".repeat(64),"columns":8,"cell_size":128.,"tile_width":512,
            "materials_sha256":format!("{:x}",Sha256::digest(&pixels)),"byte_count":pixels.len(),
            "source_tile_sha256":vec!["a".repeat(64);64]
        })
        .to_string(),
    )
    .unwrap();
    let terrain = TerrainMaterialGrid::load(
        &root.join("terrain.json"),
        &root.join("materials.bin"),
        catalog,
        &"3".repeat(64),
    )
    .unwrap();
    std::fs::remove_dir_all(root).unwrap();
    (Arc::new(assets), Arc::new(terrain))
}

pub(crate) fn fixture() -> crate::Engine<Vec<u8>> {
    fixture_with_damage(0.)
}

pub(super) fn fixture_with_damage(damage: f32) -> crate::Engine<Vec<u8>> {
    fixture_with_damage_and_area(damage, 0., 0.)
}

pub(super) fn fixture_with_damage_and_area(
    damage: f32,
    area_radius: f32,
    area_damage: f32,
) -> crate::Engine<Vec<u8>> {
    fixture_with_structural_team(damage, area_radius, area_damage, 1)
}

pub(super) fn fixture_with_structural_team(
    damage: f32,
    area_radius: f32,
    area_damage: f32,
    owner_team: u8,
) -> crate::Engine<Vec<u8>> {
    structural_fixture(damage, area_radius, area_damage, owner_team, None)
}

/// `published` adds controller sequence rows to the effects catalog and lets
/// the first tick start the match, so the initial captures enter Deactivated.
pub(super) fn structural_fixture(
    damage: f32,
    area_radius: f32,
    area_damage: f32,
    owner_team: u8,
    published: Option<Value>,
) -> crate::Engine<Vec<u8>> {
    let mut engine = crate::spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::capture_fixture();
    let catalog = engine.objective_catalog.as_mut().unwrap();
    let object = catalog
        .objects
        .iter_mut()
        .find(|object| object.object_index == INDEX)
        .unwrap();
    let boxes = [10., 20.].map(|x| TriggerBox {
        centre: [x, 3., 10.],
        axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        half_extents: [2., 2., 2.],
    });
    object.native_class = Some(NativeClass::Structural);
    object.demolition = Some(ObjectiveDemolitionRule {
        owner_team,
        arm_ms: 10,
        detonate_ms: 20,
        defuse_ms: 8,
        can_defuse: true,
        activation_score: 0,
        deactivation_score: 0,
        position: [10., 3., 10.],
        trigger_count: 2,
        maximum_distance_m: 15.,
        distance_policy: "authored_trigger_box".into(),
        trigger_volumes: boxes.to_vec(),
    });
    object.repair = Some(ObjectiveRepairRule {
        owner_team,
        can_repair: true,
        repair_ms: 1000,
        minimum_repair_ms: 1000,
    });
    engine.objective_world = Some(ObjectiveWorld::new(catalog));

    let mut indices = BTreeMap::from_iter(
        [
            "Inactive",
            "Deactivated",
            "Activating",
            "Activated",
            "Deactivating",
            "Completed",
            "Repairing",
            "Repaired",
            "Locked",
            "Retaking",
            "Retaken",
            "CancelRetaken",
        ]
        .map(|name| (name, -2)),
    );
    indices.insert("Deactivated", 0);
    indices.insert("Repaired", 0);
    indices.insert("Completed", 1);
    let mut effects = json!({
        "schema":"mag_domination_structural_effect_inputs_v1","map":"synthetic_arena",
        "metadata_sha256":"0".repeat(64),"level_archive_sha256":"a".repeat(64),"level_clv_sha256":"a".repeat(64),
        "global_data_sha256":"a".repeat(64),"physics_sha256":"a".repeat(64),
        "collision_resources":{},"unresolved_collision_tokens":[],
        "definitions":{"0x000000aa":{"definition_sha256":"a".repeat(64),"state_sequence_indices":indices,
            "sequence_tokens":["00000001","00000002"],"blast_properties":{"SubType":1,
                "FullDamageRadius":6.,"HalfDamageRadius":2.,"DamageValue":damage,"MaterialPenetration":0.,
                "CharacterDamageOverTimeRadius":area_radius,"CharacterDamageOverTimeValue":area_damage}}},
        "placements":[{"objective_token":"0x000000fe","definition_token":"0x000000aa",
            "pose":{"translation":[10.,3.,10.],"rotation":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]]}}],
        "sequences":{"00000001":{"source_sha256":"a".repeat(64),"events":[]},
            "00000002":{"source_sha256":"a".repeat(64),"events":[]}}
    });
    if let Some(rows) = &published {
        effects["published_sequences"] = rows.clone();
    }
    let effects = Catalog::from_json(&effects.to_string()).unwrap();
    let (assets, terrain) = terrain_and_assets(&effects);
    let playback =
        Structural::from_acquisition_catalog(effects, engine.objective_catalog.as_ref().unwrap())
            .unwrap();
    spawn_progression::reserve(&mut engine, 0);
    let actor = engine.active.as_mut().unwrap();
    spawn_progression::ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.tracking.server_position(
        CharacterPosition {
            x: 20.,
            y: 3.,
            z: 10.,
        },
        crate::player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    let repair_tool =
        crate::structural_repair_authority::tests::equip_synthetic_repair_kit(&mut engine);
    let setup = Setup {
        playback,
        assets,
        terrain,
        repair_tool: Some(repair_tool),
        bindings: Arc::new(Bindings {
            schema: "mag_acquisition_structural_bindings_v2".into(),
            map: "synthetic_arena".into(),
            metadata_sha256: "0".repeat(64),
            initial_objective_sha256: "1".repeat(64),
            level_archive_sha256: "a".repeat(64),
            level_clv_sha256: "a".repeat(64),
            rows: BTreeMap::from([(
                INDEX,
                Row {
                    token: 0xfe,
                    class: NativeClass::Structural,
                    spawn: None,
                    start_deactivated: true,
                    hooks: BTreeMap::new(),
                    trigger_positions: vec![[50., 3., 50.], [24., 3., 10.]],
                },
            )]),
            squads: BTreeMap::new(),
        }),
    };
    acquisition_gameplay_engine::test_support::install_structural_fixture(
        engine.acquisition_gameplay.as_mut().unwrap(),
        setup,
    );
    if published.is_some() {
        acquisition_gameplay_engine::test_support::unstart(
            engine.acquisition_gameplay.as_mut().unwrap(),
        );
    }
    engine.match_started = true;
    engine.match_epoch = Some(Duration::ZERO);
    engine
        .tick_acquisition_objectives(Duration::ZERO, 1)
        .unwrap();
    engine
}

fn row(engine: &crate::Engine<Vec<u8>>) -> crate::replication::ObjectiveControlDelta {
    engine
        .objective_world
        .as_ref()
        .unwrap()
        .snapshot(1, 0)
        .objectives
        .into_iter()
        .find(|delta| delta.object_index == INDEX)
        .unwrap()
}
pub(super) fn event(
    engine: &mut crate::Engine<Vec<u8>>,
    wire: &[u8],
    now: u64,
    transport: Transport,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let actor = engine.active.as_ref().unwrap();
    let input = crate::events::EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: actor.reservation.world,
        player_id: actor.reservation.player,
        transport,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: crate::protocol::encode_hex(wire),
    };
    let decoded = DecodedEvent::with_profile(
        wire,
        actor.reservation.player,
        engine.wire_profile,
        engine.ground_driver_enabled(),
    );
    engine.handle_event(input, &decoded, Duration::from_millis(now), 1)
}
pub(super) fn interact(
    engine: &mut crate::Engine<Vec<u8>>,
    kind: Kind,
    trigger: u8,
    now: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let wire = ObjectiveInteraction {
        target_object_id: TARGET,
        kind,
        actor_index: engine.active.as_ref().unwrap().reservation.player,
        trigger_index: trigger,
    }
    .encode();
    event(engine, &wire, now, Transport::Udp)
}
fn ack(messages: &[OutboundMessage]) -> Kind {
    messages
        .iter()
        .map(|m| crate::protocol::decode_hex(&m.payload_hex))
        .find_map(|wire| ObjectiveInteraction::decode(&wire).ok())
        .unwrap()
        .kind
}
fn tick(engine: &mut crate::Engine<Vec<u8>>, now: u64) {
    engine
        .tick_acquisition_objectives(Duration::from_millis(now), 1)
        .unwrap();
}

fn defender(engine: &mut crate::Engine<Vec<u8>>) {
    defender_at(engine, 1, Duration::ZERO);
}

pub(super) fn defender_at(engine: &mut crate::Engine<Vec<u8>>, slot: u16, now: Duration) {
    // Admission needs a real synthetic primary weapon. The repair fixture
    // deliberately uses the same item slot for an equipped active tool.
    let tools = Arc::clone(&engine.item_catalog);
    engine.item_catalog = crate::spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::capture_fixture().item_catalog;
    let mut saved = crate::loadouts::SavedLoadouts {
        layout_revision: 0,
        selected_index: 0,
        update_ids: [1; 5],
        inventories: [[None; 22]; 5],
    };
    saved.inventories[0][0] = Some(2);
    let faction = engine.match_profile().defending_faction;
    let response = engine.handle(
        crate::Request::Reserve {
            v: crate::BRIDGE_VERSION,
            request_id: format!("synthetic-structural-defender-{slot}"),
            reservation_id: format!("synthetic-reservation-{slot:04}"),
            world_id: 1,
            player_id: slot,
            account_id: u32::from(slot) + 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(
                json!({"name":"Synthetic Defender","faction":faction,
            "head":0,"voice":0,"universe_id":26,"character_id":u32::from(slot) + 1,"loadouts":saved}),
            )
            .unwrap(),
        },
        now,
        1,
    );
    engine.item_catalog = tools;
    assert!(response.ok, "{:?}", response.error);
    let actor = engine.active.as_mut().unwrap();
    spawn_progression::ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.tracking.server_position(
        CharacterPosition {
            x: 20.,
            y: 3.,
            z: 10.,
        },
        crate::player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    assert_eq!(actor.team(), 1);
    crate::structural_repair_authority::tests::equip_synthetic_repair_kit(engine);
}

fn repair(
    engine: &mut crate::Engine<Vec<u8>>,
    now: u64,
    transport: Transport,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let mut wire = 0x021du16.to_be_bytes().to_vec();
    wire.extend(TARGET.to_be_bytes());
    event(engine, &wire, now, transport)
}

pub(crate) fn bind_repair_counter_squad(engine: &mut crate::Engine<Vec<u8>>, object: u16) -> u16 {
    let owner = engine.active.as_ref().unwrap();
    let squad = owner.squad_catalog.clone();
    let game_info = crate::protocol::GameInfo {
        client_index: 0,
        level_name: engine.map_name().into(),
        start_time_ms: 0,
        attacking_faction: engine.match_profile().attacking_faction,
        defending_faction: engine.match_profile().defending_faction,
        maximum_points: [100, 100],
        current_points: [100, 100],
        game_type: engine.match_profile().game_type,
    };
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        actor.squad_catalog = squad.clone();
        actor.claimed = Some(crate::Claimed {
            wire: Vec::new(),
            np_identity: [actor.reservation.player as u8; 36],
            game_info: crate::protocol::GameInfo {
                client_index: actor.reservation.player,
                ..game_info.clone()
            },
        });
    }
    engine
        .objective_catalog
        .as_mut()
        .unwrap()
        .objects
        .iter_mut()
        .find(|row| row.object_index == object && row.native_class == Some(NativeClass::Structural))
        .unwrap()
        .reward = Some(crate::objective_catalog::ObjectiveRewardRule {
        category: 4,
        subtype: 3,
        demolition: true,
    });
    squad.squad_index()
}

pub(crate) fn select_tool_actor(engine: &mut crate::Engine<Vec<u8>>, actor: &str) {
    let next = engine.peers.remove(actor).unwrap();
    let previous = engine.active.replace(next).unwrap();
    engine
        .peers
        .insert(previous.reservation.id.clone(), previous);
}

pub(crate) fn assert_tool_completion_counters(engine: &crate::Engine<Vec<u8>>, count: i32) {
    for (player, tags, repairs) in [(0, count, 0), (1, 0, count)] {
        let actor = engine
            .active
            .iter()
            .chain(engine.peers.values())
            .find(|actor| actor.reservation.player == player)
            .unwrap();
        assert_eq!(actor.reward_statistics.get(&41).copied().unwrap_or(0), tags);
        assert_eq!(
            actor.reward_statistics.get(&43).copied().unwrap_or(0),
            repairs
        );
        assert_eq!(actor.match_experience, 0);
    }
}

#[test]
fn ordinary_activation_commits_immediate_entry_and_preserves_duplicate_deadline() {
    let mut engine = fixture();
    assert_eq!(
        ack(&interact(&mut engine, Kind::Activate, 1, 100).unwrap()),
        Kind::Ack
    );
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Activating);
    assert_eq!(row(&engine).expiration_time_ms, 110);
    interact(&mut engine, Kind::Activate, 1, 105).unwrap();
    assert_eq!(row(&engine).expiration_time_ms, 110);
    tick(&mut engine, 110);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Activated);
    tick(&mut engine, 130);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Completed);
    assert_eq!(row(&engine).health_percent, 0);
}

#[test]
fn rejected_contact_and_failed_record_leave_the_live_objective_unchanged() {
    let mut engine = fixture();
    assert_eq!(
        ack(&interact(&mut engine, Kind::Activate, 0, 100).unwrap()),
        Kind::Nak
    );
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Deactivated);
    let max_log = engine.max_log;
    engine.max_log = engine.written;
    assert!(interact(&mut engine, Kind::Activate, 1, 100).is_err());
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Deactivated);
    engine.max_log = max_log;
    engine.failed = false;
    assert_eq!(
        ack(&interact(&mut engine, Kind::Activate, 1, 100).unwrap()),
        Kind::Ack
    );
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Activating);
    let actor = engine.active.as_mut().unwrap();
    actor.life = CharacterLife::Alive {
        generation: 2,
        health_quarter_points: 400,
    };
    tick(&mut engine, 110);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Deactivated);
}

#[test]
fn defender_defuses_the_selected_trigger_and_release_keeps_the_original_fuse() {
    let mut engine = fixture();
    interact(&mut engine, Kind::Activate, 1, 100).unwrap();
    tick(&mut engine, 110);
    defender(&mut engine);
    engine.active.as_mut().unwrap().tracking.server_position(
        CharacterPosition {
            x: 10.,
            y: 3.,
            z: 10.,
        },
        crate::player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    assert_eq!(
        ack(&interact(&mut engine, Kind::Deactivate, 0, 112).unwrap()),
        Kind::Nak
    );
    engine.active.as_mut().unwrap().tracking.server_position(
        CharacterPosition {
            x: 20.,
            y: 3.,
            z: 10.,
        },
        crate::player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    assert_eq!(
        ack(&interact(&mut engine, Kind::Deactivate, 1, 113).unwrap()),
        Kind::Ack
    );
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Deactivating);
    assert_eq!(
        ack(&interact(&mut engine, Kind::Release, u8::MAX, 114).unwrap()),
        Kind::Ack
    );
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Activated);
    assert_eq!(row(&engine).expiration_time_ms, 130);
    interact(&mut engine, Kind::Deactivate, 1, 115).unwrap();
    tick(&mut engine, 123);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Deactivated);
    tick(&mut engine, 131);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Deactivated);
}

#[test]
fn tcp_repair_survives_empty_frames_then_completes_and_allows_rearming() {
    let mut engine = fixture();
    interact(&mut engine, Kind::Activate, 1, 100).unwrap();
    tick(&mut engine, 110);
    tick(&mut engine, 130);
    defender(&mut engine);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Completed);
    repair(&mut engine, 131, Transport::Udp).unwrap();
    tick(&mut engine, 140);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Completed);
    repair(&mut engine, 141, Transport::Tcp).unwrap();
    tick(&mut engine, 150);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Repairing);
    // Entering Repairing consumes the start packet. Empty frames freeze the
    // batch counter; they still advance the world clock before a later packet.
    tick(&mut engine, 1000);
    tick(&mut engine, 3100);
    assert_eq!(row(&engine).health_percent, 0);
    repair(&mut engine, 3101, Transport::Tcp).unwrap();
    tick(&mut engine, 3150);
    tick(&mut engine, 4099);
    assert_eq!(row(&engine).health_percent, 0);
    tick(&mut engine, 4100);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Repaired);
    assert_eq!(row(&engine).health_percent, 100);
    let current = engine.active.take().unwrap();
    engine.peers.insert(current.reservation.id.clone(), current);
    engine.active = engine.peers.remove("synthetic-reservation-0000");
    assert_eq!(
        ack(&interact(&mut engine, Kind::Activate, 1, 4200).unwrap()),
        Kind::Ack
    );
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Activating);
}

#[test]
fn duplicate_activation_cannot_ack_a_different_trigger_for_the_same_hold() {
    let mut engine = fixture();
    assert_eq!(
        ack(&interact(&mut engine, Kind::Activate, 1, 100).unwrap()),
        Kind::Ack
    );
    engine.active.as_mut().unwrap().tracking.server_position(
        CharacterPosition {
            x: 10.,
            y: 3.,
            z: 10.,
        },
        crate::player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    assert_eq!(
        ack(&interact(&mut engine, Kind::Activate, 0, 101).unwrap()),
        Kind::Nak
    );
    assert_eq!(row(&engine).expiration_time_ms, 110);
    engine.active.as_mut().unwrap().tracking.server_position(
        CharacterPosition {
            x: 20.,
            y: 3.,
            z: 10.,
        },
        crate::player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    tick(&mut engine, 110);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Activated);
}

#[test]
fn late_join_gets_completed_state_and_reset_restores_native_structure() {
    let mut engine = fixture();
    interact(&mut engine, Kind::Activate, 1, 100).unwrap();
    tick(&mut engine, 110);
    tick(&mut engine, 130);
    defender(&mut engine);
    let mut claim = engine
        .wire_profile
        .wire_message_type(3)
        .to_be_bytes()
        .to_vec();
    claim.extend([0; 48]);
    event(&mut engine, &claim, 140, Transport::Udp).unwrap();
    let actor = engine.active.as_mut().unwrap();
    spawn_progression::ready(actor);
    actor.objective_delta = None;
    for now in 141..160 {
        let header = crate::replication::UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: now as u32,
        };
        let wire = crate::replication::ObjectUpdate { header, wire: &[] }
            .encode_empty_with_profile(engine.wire_profile)
            .unwrap();
        event(&mut engine, &wire, now, Transport::Udp).unwrap();
        if engine.active.as_ref().unwrap().objective_delta.is_some() {
            break;
        }
    }
    let wire = engine
        .active
        .as_ref()
        .unwrap()
        .objective_delta
        .as_ref()
        .unwrap();
    let mut bits = crate::replication::BitReader::with_profile(wire, engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    let mut current = None;
    while bits.read(1).unwrap() != 0 {
        let index = bits.read(16).unwrap();
        let mut state = None;
        let mut health = None;
        let mut expiration = None;
        for field in 0..11 {
            if bits.read(1).unwrap() == 0 {
                continue;
            }
            match field {
                2 => {
                    state = Some(bits.read(8).unwrap());
                    bits.read(32).unwrap();
                    bits.read(16).unwrap();
                }
                3 => expiration = Some(bits.read(32).unwrap()),
                4 | 9 | 10 => {
                    bits.read(32).unwrap();
                }
                6 => health = Some(bits.read(8).unwrap()),
                _ => panic!("unexpected control field {field}"),
            }
        }
        if index == u32::from(INDEX) {
            current = Some((state, health, expiration));
        }
    }
    // Native SetStateCompleted retains the planted fuse's expiration. A late
    // join must receive that committed value, even though the timer has fired.
    assert_eq!(
        current,
        Some((Some(ObjectiveState::Completed as u32), Some(0), Some(130)))
    );
    engine.reset_objective_world();
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Deactivated);
    assert_eq!(row(&engine).health_percent, 100);
    assert_eq!(row(&engine).expiration_time_ms, 0);
    assert!(
        engine
            .acquisition_gameplay
            .as_ref()
            .unwrap()
            .current_structural_sequences()
            .is_empty()
    );
}

fn repair_credit_fixture(modifier: f32) -> crate::Engine<Vec<u8>> {
    let mut engine = fixture();
    interact(&mut engine, Kind::Activate, 1, 100).unwrap();
    tick(&mut engine, 110);
    tick(&mut engine, 130);
    defender(&mut engine);
    crate::frago_tests::configure_geometry(&mut engine);
    let actor = engine.active.as_mut().unwrap();
    actor.reservation.character.account_id = Some("synthetic-repairer".into());
    actor.reservation.character.progression =
        Some(crate::progression::ProgressionSnapshot::default());
    actor.reservation.character.repair_rewards_enabled = Some(true);
    actor.reservation.character.frago_action_bonus_enabled = Some(true);
    actor.modifiers = crate::skill_catalog::SkillCatalog::from_value(json!({
        "schema_version":2,"application_id":20231,"table":{"Skills":{
            "MagCharacterSkill1":{"MagCharacterSkill":{"Default":{"Modifier":"kCharModRepairAmount","ModifierValue":modifier},"Green":{"Available":true},"Black":{"Available":true},"Red":{"Available":true}}}
        }}
    })).unwrap().resolve(actor.reservation.character.faction,&[2,0,0,0,0]).unwrap();
    let squad = actor.squad_catalog.squad_index();
    engine
        .acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(squad)
        .unwrap()
        .assignment = crate::acquisition_objective_runtime::Assignment {
        primary_index: u16::MAX,
        tagged_object: TARGET,
    };
    engine
}

#[test]
fn accepted_repair_work_credits_fractional_units_before_any_structural_hp_batch() {
    let mut e = repair_credit_fixture(-0.75);
    let before = row(&e).health_percent;
    for i in 1..=4 {
        repair(&mut e, 130 + i, Transport::Tcp).unwrap();
        assert_eq!(row(&e).health_percent, before);
        let actor = e.active.as_ref().unwrap();
        assert_eq!(
            actor.repair_credit.carry,
            if i == 4 { 0. } else { i as f32 * 0.25 }
        );
        assert_eq!(actor.repair_credit.sequence, i);
        assert_eq!(
            actor.reward_statistics.get(&35).copied().unwrap_or(0),
            if i == 4 { 1 } else { 0 }
        );
        assert_eq!(e.pending_reward_batch().len(), usize::from(i == 4));
    }
    let reward = e.pending_reward_batch().pop().unwrap();
    assert_eq!(reward.kind, crate::progression::RewardKind::RepairUnits);
    assert_eq!(reward.units, 1);
    assert_eq!(reward.frago, Some(true));
}

#[test]
fn failed_repair_record_or_wrong_route_does_not_consume_work_and_retry_is_once() {
    let mut e = repair_credit_fixture(-0.5);
    repair(&mut e, 131, Transport::Udp).unwrap();
    assert_eq!(
        e.active.as_ref().unwrap().repair_credit,
        crate::repair_rewards::State::default()
    );
    let max = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        repair(&mut e, 132, Transport::Tcp).err(),
        Some("log_limit_reached")
    );
    assert_eq!(
        e.active.as_ref().unwrap().repair_credit,
        crate::repair_rewards::State::default()
    );
    assert!(e.pending_reward_batch().is_empty());
    e.max_log = max;
    e.failed = false;
    repair(&mut e, 132, Transport::Tcp).unwrap();
    assert_eq!(e.active.as_ref().unwrap().repair_credit.carry, 0.5);
    assert!(e.pending_reward_batch().is_empty());
    repair(&mut e, 133, Transport::Tcp).unwrap();
    assert_eq!(e.pending_reward_batch().len(), 1);
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics.get(&35),
        Some(&1)
    );
}

#[test]
fn covered_repair_records_latch_only_and_recorder_failure_keeps_the_original_actor() {
    let mut e = repair_credit_fixture(0.);
    repair(&mut e, 131, Transport::Tcp).unwrap();
    tick(&mut e, 131);
    assert_eq!(row(&e).state_actor.state, ObjectiveState::Repairing);
    e.objective_world
        .as_mut()
        .unwrap()
        .stage_repair(
            TARGET,
            crate::domination_repair_runtime::Authority {
                actor: crate::domination_objective_runtime::Actor {
                    player: 77,
                    generation: 4,
                    team: 1,
                },
                live: true,
                tool_active: true,
                repair_rate: 1.,
            },
        )
        .unwrap();
    let credit_before = e.active.as_ref().unwrap().repair_credit;
    let rewards_before = e.pending_reward_batch();
    let max_log = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        repair(&mut e, 132, Transport::Tcp).err(),
        Some("log_limit_reached")
    );
    let mut candidate = e.objective_world.clone().unwrap();
    let finish = Duration::from_millis(1131);
    candidate.begin_native_tick(finish);
    candidate.tick_native_object(INDEX, finish, &BTreeMap::new());
    let failed_entry = candidate.take_native_entries().pop().unwrap();
    assert_eq!(failed_entry.reward_actor, Some(77));

    e.max_log = max_log;
    e.failed = false;
    let recorder_before = e.writer.len();
    repair(&mut e, 132, Transport::Tcp).unwrap();
    assert!(e.writer.len() > recorder_before);
    assert_eq!(e.active.as_ref().unwrap().repair_credit, credit_before);
    assert_eq!(e.pending_reward_batch(), rewards_before);
    let expected = e.active.as_ref().unwrap().reservation.player;
    tick(&mut e, 1131);
    assert_eq!(row(&e).state_actor.state, ObjectiveState::Repaired);
    assert_eq!(row(&e).state_actor.actor_index, expected);
}

#[test]
fn recorded_tool_repair_completion_credits_last_repairer_and_tagged_leader() {
    let mut e = repair_credit_fixture(0.);
    let squad = bind_repair_counter_squad(&mut e, INDEX);
    select_tool_actor(&mut e, "synthetic-reservation-0000");
    crate::structural_repair_authority::tests::equip_synthetic_repair_kit(&mut e);
    repair(&mut e, 131, Transport::Tcp).unwrap();
    tick(&mut e, 131);
    assert_eq!(row(&e).state_actor.state, ObjectiveState::Repairing);
    repair(&mut e, 132, Transport::Tcp).unwrap();
    select_tool_actor(&mut e, "synthetic-reservation-0001");
    assert!(repair(&mut e, 133, Transport::Tcp).unwrap().is_empty());
    let tagged = e
        .acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .set_manual_tag(squad, TARGET)
        .unwrap();
    assert_eq!(tagged.tagged_object, TARGET);
    assert_tool_completion_counters(&e, 0);

    let finish = Duration::from_millis(1131);
    let max = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        e.tick_acquisition_objectives(finish, 1).unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(row(&e).state_actor.state, ObjectiveState::Repairing);
    assert_tool_completion_counters(&e, 0);

    e.max_log = max;
    e.failed = false;
    let rewards = e.pending_reward_batch();
    e.tick_acquisition_objectives(finish, 1).unwrap();
    assert_eq!(row(&e).state_actor.state, ObjectiveState::Repaired);
    assert_eq!(row(&e).state_actor.actor_index, 1);
    assert_tool_completion_counters(&e, 1);
    tick(&mut e, 1132);
    assert_tool_completion_counters(&e, 1);
    assert_eq!(e.pending_reward_batch(), rewards);
}

#[test]
fn unsupported_repair_xp_admission_keeps_gameplay_stat_without_a_new_receipt_kind() {
    let mut e = repair_credit_fixture(0.);
    e.active
        .as_mut()
        .unwrap()
        .reservation
        .character
        .repair_rewards_enabled = None;
    repair(&mut e, 131, Transport::Tcp).unwrap();
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics.get(&35),
        Some(&1)
    );
    assert!(e.pending_reward_batch().is_empty());
}

#[test]
fn repair_ack_does_not_increment_the_gameplay_owned_stat35_again() {
    let mut e = repair_credit_fixture(0.);
    repair(&mut e, 131, Transport::Tcp).unwrap();
    let reward = e.pending_reward_batch().pop().unwrap();
    let ack = json!({"op":"progression_committed","v":crate::BRIDGE_VERSION,"request_id":"repair-ack",
        "reservation_id":reward.reservation_id,"world_id":reward.world_id,"player_id":reward.player_id,
        "reward_id":reward.reward_id,"progression":{"experience":4,"level":1,"skill_points":0},
        "match_experience":4,"match_frago_points":2});
    for _ in 0..2 {
        let response = e.handle(
            serde_json::from_value(ack.clone()).unwrap(),
            Duration::from_millis(140),
            1,
        );
        assert!(response.ok, "{:?}", response.error);
    }
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics.get(&35),
        Some(&1)
    );
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics.get(&3),
        Some(&2)
    );
    assert!(e.pending_reward_batch().is_empty());
}

#[test]
fn same_match_repair_carry_follows_character_identity_without_slot_inheritance() {
    for replacement in [false, true] {
        let mut e = repair_credit_fixture(0.25);
        repair(&mut e, 131, Transport::Tcp).unwrap();
        let mut character = e.active.as_ref().unwrap().reservation.character.clone();
        let state = e.active.as_ref().unwrap().repair_credit;
        let reply = e.handle(
            crate::Request::Release {
                v: crate::BRIDGE_VERSION,
                request_id: "release-repair".into(),
                reservation_id: "synthetic-reservation-0001".into(),
            },
            Duration::from_millis(140),
            1,
        );
        assert!(reply.ok, "{:?}", reply.error);
        if replacement {
            character.account_id = Some("replacement-account".into());
        }
        let tools = Arc::clone(&e.item_catalog);
        e.item_catalog=crate::spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::capture_fixture().item_catalog;
        let reply = e.handle(
            crate::Request::Reserve {
                v: crate::BRIDGE_VERSION,
                request_id: "rejoin-repair".into(),
                reservation_id: "synthetic-rejoined-repairer".into(),
                world_id: 1,
                player_id: 1,
                account_id: 2,
                session_key: "aaaaaaaaaaaaaaaa".into(),
                access_key: "bbbbbbbbbbbbbbbb".into(),
                character,
            },
            Duration::from_millis(150),
            1,
        );
        e.item_catalog = tools;
        assert!(reply.ok, "{:?}", reply.error);
        let actor = e.active.as_ref().unwrap();
        assert_eq!(
            actor.repair_credit,
            if replacement {
                crate::repair_rewards::State::default()
            } else {
                state
            }
        );
        assert_eq!(
            actor.reward_statistics.get(&35),
            if replacement { None } else { Some(&1) }
        );
    }
}

#[test]
fn full_reward_outbox_cannot_commit_repair_queue_or_work_before_retry() {
    let mut e = repair_credit_fixture(0.);
    let recipient =
        crate::progression::RewardRecipient::from_active(e.active.as_ref().unwrap()).unwrap();
    e.commit_rewards(
        (0..crate::progression::MAX_PENDING_REWARDS)
            .map(|i| {
                recipient.event(
                    format!("synthetic-pending-{i}"),
                    crate::progression::RewardKind::EnemyFinished,
                    1,
                )
            })
            .collect(),
    );
    assert_eq!(
        repair(&mut e, 131, Transport::Tcp).err(),
        Some("reward_outbox_full")
    );
    assert_eq!(
        e.active.as_ref().unwrap().repair_credit,
        crate::repair_rewards::State::default()
    );
    assert!(
        !e.active
            .as_ref()
            .unwrap()
            .reward_statistics
            .contains_key(&35)
    );
    e.pending_rewards.clear();
    e.reward_order.clear();
    repair(&mut e, 131, Transport::Tcp).unwrap();
    assert_eq!(e.pending_reward_batch().len(), 1);
    assert_eq!(e.active.as_ref().unwrap().repair_credit.sequence, 1);
}

fn last_record(engine: &crate::Engine<Vec<u8>>) -> serde_json::Value {
    serde_json::from_slice(
        engine
            .writer
            .split(|&b| b == b'\n')
            .rfind(|r| !r.is_empty())
            .unwrap(),
    )
    .unwrap()
}

fn destroyed_with_defender() -> crate::Engine<Vec<u8>> {
    let mut engine = fixture();
    interact(&mut engine, Kind::Activate, 1, 100).unwrap();
    tick(&mut engine, 110);
    tick(&mut engine, 130);
    defender(&mut engine);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Completed);
    engine
}

// Medius ends the session on any event error. The repair tool keeps sending
// about twice a second, so a packet can still be in flight when its sender
// dies or the structure finishes; both stay in the match.
#[test]
fn tool_packet_in_flight_when_the_repairer_dies_keeps_the_session() {
    let mut engine = destroyed_with_defender();
    repair(&mut engine, 141, Transport::Tcp).unwrap();
    engine.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 1 };
    let credit = engine.active.as_ref().unwrap().repair_credit;
    assert!(repair(&mut engine, 142, Transport::Tcp).unwrap().is_empty());
    let record = last_record(&engine);
    assert_eq!(record["op"], "acquisition_structural_repair");
    assert_eq!(
        record["gameplay_denial"],
        "structural_repair_requires_live_actor"
    );
    assert_eq!(engine.active.as_ref().unwrap().repair_credit, credit);
    // Work accepted before the death still survives the actor going away.
    tick(&mut engine, 150);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Repairing);
}

#[test]
fn tool_packet_after_the_structure_is_repaired_keeps_the_session() {
    let mut engine = destroyed_with_defender();
    repair(&mut engine, 141, Transport::Tcp).unwrap();
    tick(&mut engine, 150);
    repair(&mut engine, 151, Transport::Tcp).unwrap();
    tick(&mut engine, 1150);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Repaired);
    let before = row(&engine);
    let credit = engine.active.as_ref().unwrap().repair_credit;
    let rewards = engine.pending_reward_batch();
    assert!(
        repair(&mut engine, 1151, Transport::Tcp)
            .unwrap()
            .is_empty()
    );
    let record = last_record(&engine);
    assert_eq!(record["op"], "acquisition_structural_repair");
    assert_eq!(
        record["gameplay_denial"],
        "structural_objective_not_repairable"
    );
    assert_eq!(row(&engine), before);
    assert_eq!(engine.active.as_ref().unwrap().repair_credit, credit);
    assert_eq!(engine.pending_reward_batch(), rewards);
    tick(&mut engine, 1200);
    assert_eq!(row(&engine).state_actor.state, ObjectiveState::Repaired);
}
