//! SYNTHETIC structural field-hospital spawn scope fixtures. No captured map data.
use super::*;

const HOME: u16 = 0;
const HOSPITAL: u16 = 2;

fn spawns(hospital_kind: &str, hospital_available: bool) -> SpawnCatalog {
    let available: Vec<u16> = if hospital_available {
        vec![HOME, 1, HOSPITAL]
    } else {
        vec![HOME, 1]
    };
    SpawnCatalog::from_json(
        &serde_json::json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC field hospital",
            "active":{"map_name":"synthetic_acquisition","squad_index":16,"default_spawn":HOME,
                "available":available},
            "match_profile":{"game_type":1,"has_bunkers":false,"has_objectives":false},
            "maps":[{"name":"synthetic_acquisition","owner_faction":0,"spawn_count":3,
                "runtime_indices_verified":true,"runtime_indices_basis":"SYNTHETIC pool identity",
                "locations":[
                    {"index":0,"token":100,"kind":"insertion","position":{"x":10.,"y":0.,"z":10.}},
                    {"index":1,"token":101,"kind":"alternate","position":{"x":20.,"y":0.,"z":10.}},
                    {"index":2,"token":102,"kind":hospital_kind,"position":{"x":40.,"y":0.,"z":40.}}],
                "squads":[{"index":16,"team":1,"platoon":0,"squad":0,"faction":0,
                    "available":available,"default_spawn":HOME}]}]})
        .to_string(),
    )
    .unwrap()
}

/// One structural objective whose forward spawn (a type-8 field hospital of
/// user class 0) belongs to squad 16, active while the structure stands.
fn bindings() -> Bindings {
    serde_json::from_value(serde_json::json!({
        "schema":"SYNTHETIC","map":"synthetic_acquisition",
        "metadata_sha256":"0".repeat(64),"initial_objective_sha256":"1".repeat(64),
        "level_archive_sha256":"2".repeat(64),"level_clv_sha256":"3".repeat(64),
        "rows":{"7":{"token":700,"class":"structural","start_deactivated":true,"hooks":{},
            "spawn":{"token":102,"index":HOSPITAL,"team":1,"user_class":0,"spawn_type":8,
                "fsp_type":0,"vehicle":false}}},
        "squads":{"16":{"points":[[HOME,100],[1,101],[HOSPITAL,102]],"category_zero":[HOME,100]}}}))
    .unwrap()
}

#[test]
fn an_unsupported_field_hospital_falls_back_locally_and_stays_the_native_default() {
    let mut catalog = spawns("unsupported", false);
    bindings().configure_spawns(&mut catalog).unwrap();
    assert_eq!(catalog.locally_selectable(16, HOSPITAL), HOME);
    catalog.set_runtime_squad_default(16, HOSPITAL).unwrap();
    assert_eq!(catalog.native_unselectable_default(16), Some(HOSPITAL));
    catalog
        .set_runtime_point_active(HOSPITAL, 102, false)
        .unwrap();
    assert_eq!(catalog.native_unselectable_default(16), None);
}

#[test]
fn a_selectable_field_hospital_needs_no_fallback_and_becomes_the_squad_default() {
    let mut catalog = spawns("alternate", true);
    bindings().configure_spawns(&mut catalog).unwrap();
    assert_eq!(catalog.locally_selectable(16, HOSPITAL), HOSPITAL);
    // The structure standing at match start routes the squad there.
    let plan = bindings()
        .plan_fsp(
            700,
            bindings().rows[&7].spawn.unwrap(),
            ObjectiveState::Deactivated,
        )
        .unwrap();
    let (catalog, ()) = fsp::stage_spawn_effects(&catalog, &plan)
        .unwrap()
        .complete(|_| Ok(()))
        .unwrap();
    assert_eq!(catalog.default_for_squad(16), Some(HOSPITAL));
    assert_eq!(catalog.native_unselectable_default(16), None);
    assert!(catalog.bot_arrival_point(16, HOSPITAL).is_some());
}

// SYNTHETIC: Acquisition field hospitals bound to indexed emplacements.
mod bound_emplacements {
    use super::*;
    use crate::{
        CharacterLife, protocol, replication,
        spawn_progression::dispatch,
        structures::domination_bunker_seat_tests::{fixture_for_mode, occupant_fixture},
    };

    fn acquisition() -> Engine<Vec<u8>> {
        fixture_for_mode(1)
    }

    fn owner_tick(engine: &mut Engine<Vec<u8>>, timestamp_ms: u32) {
        let header = replication::UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms,
        };
        let packet = replication::ObjectUpdate { header, wire: &[] }
            .encode_empty_with_profile(engine.wire_profile)
            .unwrap();
        dispatch(engine, &packet, u64::from(timestamp_ms)).unwrap();
    }

    fn await_hospital_respawn(engine: &mut Engine<Vec<u8>>, point: u16) {
        let actor = engine.active.as_mut().unwrap();
        actor.squad_spawn = point;
        actor.selected_spawn = Some(protocol::CharacterSpawnSelection {
            spawn_point_index: point,
        });
        actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
        actor.pending_spawn_deadline = Some(Duration::from_millis(100));
        actor.mounted_seat = None;
        actor.mounted_bunker = None;
        actor.round_scores = Some([111, 222]);
        for tick in 100..108 {
            owner_tick(engine, tick);
            if engine.active.as_ref().unwrap().life.is_alive() {
                break;
            }
        }
    }

    #[test]
    fn an_acquisition_field_hospital_life_starts_seated_in_its_own_emplacement() {
        let mut engine = acquisition();
        assert!(engine.indexed_hospital_seats());
        let seat_position = replication::CharacterPosition {
            x: 11.,
            y: 3.,
            z: 17.,
        };
        engine
            .bunker_world
            .as_mut()
            .unwrap()
            .set_synthetic_hospital_transform(
                6,
                crate::vehicle_exit::BunkerTransform {
                    position: seat_position,
                    rotation_xyzw: [0., 0., 0., 1.],
                },
            );
        await_hospital_respawn(&mut engine, 2);
        let actor = engine.active.as_ref().unwrap();
        assert!(actor.life.is_alive(), "{:?}", actor.life);
        assert_eq!(actor.mounted_bunker, Some(6));
        assert_eq!(actor.mounted_seat, Some(12));
        assert_eq!(actor.owner_fields.state.unwrap().user_data, [13, 6, 12]);
        assert_eq!(actor.tracking.sample.unwrap().position, seat_position);
    }

    #[test]
    fn a_destroyed_acquisition_hospital_starts_the_life_on_foot_at_its_point() {
        let mut engine = acquisition();
        let hospitals = engine.bunker_world.as_mut().unwrap();
        hospitals
            .apply_hospital_cycle(2, false, 0x0c80_0007)
            .unwrap();
        await_hospital_respawn(&mut engine, 2);
        let actor = engine.active.as_ref().unwrap();
        assert!(actor.life.is_alive(), "{:?}", actor.life);
        assert_eq!((actor.mounted_bunker, actor.mounted_seat), (None, None));
        let record = String::from_utf8_lossy(&engine.writer);
        assert!(record.contains("bunker_spawn_not_operational_or_wrong_side"));
    }

    #[test]
    fn acquisition_hospital_mounts_use_the_indexed_emplacement() {
        let mut engine = acquisition();
        let mut wire = 0x0216u16.to_be_bytes().to_vec();
        wire.extend(0x0d00_0005u32.to_be_bytes());
        wire.extend(12u32.to_be_bytes());
        wire.extend(0x0380_0000u32.to_be_bytes());
        assert!(!dispatch(&mut engine, &wire, 1000).unwrap().is_empty());
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(
            (actor.mounted_bunker, actor.mounted_seat),
            (Some(5), Some(12))
        );
        assert_eq!(engine.alpha_owner(), None);
    }

    #[test]
    fn acquisition_hospitals_take_no_direct_vehicle_damage() {
        let engine = acquisition();
        let hospitals = engine.bunker_world.as_ref().unwrap();
        assert!(hospitals.hospital(5).is_some_and(|h| h.operational));
        assert!(!hospitals.hospital_damageable(5));
        assert_eq!(hospitals.damageable_domination_hospitals().count(), 0);
        let domination = fixture_for_mode(2);
        assert!(
            domination
                .bunker_world
                .as_ref()
                .unwrap()
                .hospital_damageable(5)
        );
    }

    fn hospital_bindings() -> Bindings {
        serde_json::from_value(serde_json::json!({
            "schema":"SYNTHETIC","map":"hospital_test",
            "metadata_sha256":"0".repeat(64),"initial_objective_sha256":"1".repeat(64),
            "level_archive_sha256":"2".repeat(64),"level_clv_sha256":"3".repeat(64),
            "rows":{
                "7":{"token":700,"class":"structural","start_deactivated":true,"hooks":{},
                    "spawn":{"token":101,"index":1,"team":1,"user_class":0,"spawn_type":8,
                        "fsp_type":0,"vehicle":false}},
                "8":{"token":800,"class":"structural","start_deactivated":true,"hooks":{},
                    "spawn":{"token":102,"index":2,"team":1,"user_class":0,"spawn_type":8,
                        "fsp_type":0,"vehicle":false}}},
            "squads":{"16":{"points":[[0,100],[1,101],[2,102]],"category_zero":[0,100]}}}))
        .unwrap()
    }

    #[test]
    fn every_bound_hospital_must_be_a_structural_forward_spawn() {
        let engine = acquisition();
        let hospitals = engine.bunker_world.as_ref().unwrap();
        hospital_bindings().validate_hospitals(hospitals).unwrap();
        let mut bindings = hospital_bindings();
        bindings.rows.remove(&8);
        assert_eq!(
            bindings.validate_hospitals(hospitals),
            Err("acquisition_hospital_forward_spawn_unbound")
        );
        let mut bindings = hospital_bindings();
        bindings
            .rows
            .get_mut(&8)
            .unwrap()
            .spawn
            .as_mut()
            .unwrap()
            .spawn_type = 4;
        assert!(bindings.validate_hospitals(hospitals).is_err());
    }

    fn request(token: u32, index: u16, hook: VehicleHook) -> fsp::VehicleRequest {
        fsp::VehicleRequest {
            point: SpawnIdentity {
                token,
                native_id: 0x2380_0000 | u32::from(index),
            },
            objective_token: 700,
            hook,
        }
    }

    /// Stage one hospital hook inside the shared character transaction.
    fn stage(
        engine: &mut Engine<Vec<u8>>,
        native: &BTreeMap<u32, NativeState>,
        hospitals: &mut Option<crate::structures::BunkerWorld>,
        requests: &[fsp::VehicleRequest],
    ) -> (
        crate::acquisition_character_effects::Candidate,
        Vec<Vec<u8>>,
        Vec<Value>,
    ) {
        let mut packets = Vec::new();
        let mut rows = Vec::new();
        let mut ground = ground::Runtime::default();
        let mut spawns = fixture_for_mode(1).spawn_catalog.take().unwrap();
        let candidate = engine
            .prepare_acquisition_character_callback(
                native,
                Duration::from_secs(1),
                |engine, native| {
                    let mut audit = Vec::new();
                    let mut messages = Vec::new();
                    let mut frame = Frame {
                        engine,
                        assignments: &[],
                        ground: &mut ground,
                        spawns: &mut spawns,
                        native,
                        ground_changes: &mut BTreeMap::new(),
                        audit: &mut audit,
                        messages: &mut messages,
                        sequences: &mut BTreeMap::new(),
                        filter_rows: &[0; 32],
                        physics: Settings::default(),
                        ground_policy: None,
                        hospitals,
                        hospital_packets: &mut packets,
                        now: Duration::from_secs(1),
                    };
                    for &request in requests {
                        frame.stage_hospital(request, 7)?;
                    }
                    rows = audit.clone();
                    Ok((audit, messages))
                },
            )
            .unwrap();
        (candidate, packets, rows)
    }

    #[test]
    fn a_retired_field_hospital_destroys_its_emplacement_and_kills_its_occupants() {
        let (mut engine, mut native) = occupant_fixture();
        let mut hospitals = None;
        let (candidate, packets, rows) = stage(
            &mut engine,
            &native,
            &mut hospitals,
            &[request(101, 1, VehicleHook::OnFspDeactivate)],
        );
        // Staged only: the live emplacement and its occupant are unchanged.
        assert!(
            engine
                .bunker_world
                .as_ref()
                .unwrap()
                .hospital(5)
                .unwrap()
                .operational
        );
        assert_eq!(engine.active.as_ref().unwrap().mounted_bunker, Some(5));
        let staged = hospitals.as_ref().unwrap();
        assert!(!staged.hospital(5).unwrap().operational);
        assert!(staged.hospital(6).unwrap().operational);
        assert_eq!(packets.len(), 1);
        assert!(
            rows.iter()
                .any(|row| row["op"] == "acquisition_hospital_cycle"
                    && row["bunker_index"] == 5
                    && row["occupants_killed"] == true)
        );
        assert!(
            candidate
                .audit()
                .iter()
                .any(|row| row["op"] == "hospital_occupant_damage" && row["bunker_index"] == 5)
        );
        engine
            .record_acquisition_character_effects(candidate, &mut native, 1)
            .unwrap();
        let active = engine.active.as_ref().unwrap();
        assert!(!active.life.is_alive());
        assert_eq!((active.mounted_bunker, active.mounted_seat), (None, None));
        let other = engine.peers.values().next().unwrap();
        assert!(other.life.is_alive());
        assert_eq!(other.mounted_bunker, Some(6));

        // The FSP activation that follows a repair restores the body.
        let (_, packets, _) = stage(
            &mut engine,
            &native,
            &mut hospitals,
            &[request(101, 1, VehicleHook::OnFspActivate)],
        );
        assert!(hospitals.as_ref().unwrap().hospital(5).unwrap().operational);
        assert_eq!(packets.len(), 1);
    }

    #[test]
    fn forward_spawns_without_a_bound_emplacement_have_no_vehicle_work() {
        let (mut engine, native) = occupant_fixture();
        let mut hospitals = None;
        let (candidate, packets, rows) = stage(
            &mut engine,
            &native,
            &mut hospitals,
            &[
                request(100, 0, VehicleHook::OnFspDeactivate),
                // A standing hospital's activation at mission start changes nothing.
                request(101, 1, VehicleHook::OnFspActivate),
            ],
        );
        assert!(packets.is_empty() && rows.is_empty() && candidate.audit().is_empty());
        assert!(hospitals.is_none_or(|world| world.hospital(5).unwrap().operational));
    }
}
