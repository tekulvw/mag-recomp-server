//! SYNTHETIC Acquisition LALO drop-zone fixtures. Invented poses and tokens only.
//!
//! A drop zone is the forward spawn (active when complete) of a passive objective
//! that the defenders' anti-aircraft structure completes when destroyed.
use super::*;
use crate::{replication::SpawnAvailability, wire_profile::WireProfile};

const HOME: u16 = 0;
const DROP: u16 = 2;
const SQUAD: u16 = 0;

fn spawns(drop_kind: &str) -> SpawnCatalog {
    let mut drop = serde_json::json!({"index":DROP,"token":102,"kind":drop_kind,"active":false,
        "position":{"x":40.,"y":60.,"z":40.}});
    if drop_kind == "parachute" {
        drop["parachute"] = serde_json::json!({"forward":[0.,0.,1.],"spacing_m":10.,"yaw":0});
    }
    SpawnCatalog::from_json(
        &serde_json::json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC drop zone",
            "active":{"map_name":"synthetic_acquisition","squad_index":SQUAD,"default_spawn":HOME,
                "available":[HOME,1]},
            "match_profile":{"game_type":1,"has_bunkers":false,"has_objectives":false},
            "maps":[{"name":"synthetic_acquisition","owner_faction":1,"spawn_count":3,
                "runtime_indices_verified":true,"runtime_indices_basis":"SYNTHETIC pool identity",
                "locations":[
                    {"index":0,"token":100,"kind":"insertion","position":{"x":10.,"y":0.,"z":10.}},
                    {"index":1,"token":101,"kind":"alternate","position":{"x":20.,"y":0.,"z":10.}},
                    drop],
                "squads":[{"index":SQUAD,"team":0,"platoon":0,"squad":0,
                    "available":[HOME,1],"default_spawn":HOME}]}]})
        .to_string(),
    )
    .unwrap()
}

/// The passive objective (index 8) whose forward spawn is the drop zone.
fn bindings() -> Bindings {
    serde_json::from_value(serde_json::json!({
        "schema":"SYNTHETIC","map":"synthetic_acquisition",
        "metadata_sha256":"0".repeat(64),"initial_objective_sha256":"1".repeat(64),
        "level_archive_sha256":"2".repeat(64),"level_clv_sha256":"3".repeat(64),
        "rows":{"8":{"token":800,"class":"secure_hold","start_deactivated":false,"hooks":{},
            "spawn":{"token":102,"index":DROP,"team":0,"user_class":0,"spawn_type":5,
                "fsp_type":1,"vehicle":false}}},
        "squads":{"0":{"points":[[HOME,100],[1,101],[DROP,102]],"category_zero":[HOME,100]}}}))
    .unwrap()
}

/// Stage the passive objective's entry as the anti-aircraft hook requests it.
fn enter(catalog: &SpawnCatalog, state: ObjectiveState) -> SpawnCatalog {
    let bindings = bindings();
    let plan = bindings
        .plan_fsp(800, bindings.rows[&8].spawn.unwrap(), state)
        .unwrap();
    let (next, ()) = fsp::stage_spawn_effects(catalog, &plan)
        .unwrap()
        .complete(|_| Ok(()))
        .unwrap();
    next
}

fn published(catalog: &SpawnCatalog) -> Vec<u8> {
    SpawnAvailability::encode_update_with_profile(1, 0, catalog, WireProfile::default()).unwrap()
}

#[test]
fn a_parachute_drop_zone_opens_when_the_anti_aircraft_falls_and_closes_on_repair() {
    let mut catalog = spawns("parachute");
    bindings().configure_spawns(&mut catalog).unwrap();
    // Mission start: the standing anti-aircraft keeps the zone closed.
    let closed = enter(&catalog, ObjectiveState::Deactivated);
    assert!(!closed.point_active(DROP));
    assert_eq!(closed.default_for_squad(SQUAD), Some(HOME));
    assert!(closed.for_squad(SQUAD).unwrap().alpha(DROP).is_none());

    // Destroyed: the zone is published, selectable and the squad's spawn.
    let open = enter(&closed, ObjectiveState::Completed);
    assert!(open.point_active(DROP));
    assert_ne!(published(&open), published(&closed));
    assert_eq!(open.default_for_squad(SQUAD), Some(DROP));
    let squad = open.for_squad(SQUAD).unwrap();
    assert_eq!(squad.default_spawn(), DROP);
    let point = squad.alpha(DROP).expect("selectable drop zone");
    assert!(matches!(
        point.kind,
        crate::spawn_catalog::SpawnKind::Parachute
    ));
    assert!(open.bot_arrival_point(SQUAD, DROP).is_some());
    // Later jumpers of one wave start further along the authored forward.
    assert_eq!(point.parachute_position(3).unwrap().z, 70.);
    assert_eq!(point.parachute_position(3).unwrap().y, 60.);

    // Repaired: closed again and the squad returns to its category-zero point.
    let repaired = enter(&open, ObjectiveState::Repaired);
    assert!(!repaired.point_active(DROP));
    assert_eq!(repaired.default_for_squad(SQUAD), Some(HOME));
    assert_eq!(published(&repaired), published(&closed));
    assert!(repaired.for_squad(SQUAD).unwrap().alpha(DROP).is_none());
}

#[test]
fn an_unsupported_drop_zone_is_never_published_even_when_opened() {
    // The catalogs before this projection: the runtime opened the zone, but
    // its availability bit stayed clear, so the client kept the icon greyed.
    let mut catalog = spawns("unsupported");
    bindings().configure_spawns(&mut catalog).unwrap();
    let closed = enter(&catalog, ObjectiveState::Deactivated);
    let open = enter(&closed, ObjectiveState::Completed);
    assert!(open.point_active(DROP));
    assert_eq!(published(&open), published(&closed));
    assert_eq!(open.locally_selectable(SQUAD, DROP), HOME);
    assert!(open.for_squad(SQUAD).unwrap().alpha(DROP).is_none());
}

/// SYNTHETIC two-lane map: each attacking platoon has its own anti-aircraft
/// Structural (24 for platoon 0, 27 for platoon 1) whose hook names four
/// passive objectives of that platoon. Here one squad per platoon owns a zone.
mod lanes {
    use super::*;

    const LANE_SQUADS: [u16; 2] = [0, 4];
    const LANE_HOMES: [u16; 2] = [0, 3];
    const LANE_DROPS: [u16; 2] = [2, 5];
    const LANE_AA: [u16; 2] = [24, 27];

    fn spawns() -> SpawnCatalog {
        let mut locations = Vec::new();
        let mut squads = Vec::new();
        for lane in 0..2 {
            let home = LANE_HOMES[lane];
            let x = 100. * lane as f64;
            locations.push(
                serde_json::json!({"index":home,"token":100 + home,"kind":"insertion",
                "position":{"x":x + 10.,"y":0.,"z":10.}}),
            );
            locations.push(serde_json::json!({"index":home + 1,"token":101 + home,
                "kind":"alternate","position":{"x":x + 20.,"y":0.,"z":10.}}));
            locations.push(
                serde_json::json!({"index":LANE_DROPS[lane],"token":100 + LANE_DROPS[lane],
                "kind":"parachute","active":false,"position":{"x":x + 40.,"y":60.,"z":40.},
                "parachute":{"forward":[0.,0.,1.],"spacing_m":10.,"yaw":0}}),
            );
            squads.push(serde_json::json!({"index":LANE_SQUADS[lane],"team":0,
                "platoon":lane,"squad":0,"available":[home, home + 1],"default_spawn":home}));
        }
        SpawnCatalog::from_json(
            &serde_json::json!({
                "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC lanes",
                "active":{"map_name":"synthetic_acquisition","squad_index":0,"default_spawn":0,
                    "available":[0,1]},
                "match_profile":{"game_type":1,"has_bunkers":false,"has_objectives":false},
                "maps":[{"name":"synthetic_acquisition","owner_faction":1,"spawn_count":6,
                    "runtime_indices_verified":true,"runtime_indices_basis":"SYNTHETIC pool identity",
                    "locations":locations,"squads":squads}]})
            .to_string(),
        )
        .unwrap()
    }

    fn bindings() -> Bindings {
        let mut rows = serde_json::Map::new();
        let mut squads = serde_json::Map::new();
        for lane in 0..2 {
            let first = 8 + 4 * lane as u16;
            let targets: Vec<u16> = (first..first + 4).collect();
            let hook =
                serde_json::json!({"function":"antiaircraft","platoon":lane,"spawns":targets});
            rows.insert(
                LANE_AA[lane].to_string(),
                serde_json::json!({
                "token":2400 + LANE_AA[lane] as u32,"class":"structural","start_deactivated":true,
                "hooks":{"1":hook.clone(),"6":hook.clone(),"7":hook},"spawn":null}),
            );
            let drop = LANE_DROPS[lane];
            for target in targets {
                // Only the platoon's first hold has a zone in this fixture.
                let spawn = (target == first).then(|| {
                    serde_json::json!({"token":100 + drop,
                    "index":drop,"team":0,"user_class":0,"spawn_type":5,"fsp_type":1,
                    "vehicle":false})
                });
                rows.insert(
                    target.to_string(),
                    serde_json::json!({"token":800 + target as u32,
                    "class":"secure_hold","start_deactivated":false,"hooks":{},"spawn":spawn}),
                );
            }
            let home = LANE_HOMES[lane];
            squads.insert(
                LANE_SQUADS[lane].to_string(),
                serde_json::json!({
                "points":[[home,100 + home],[home + 1,101 + home],[drop,100 + drop]],
                "category_zero":[home,100 + home]}),
            );
        }
        serde_json::from_value(serde_json::json!({
            "schema":"SYNTHETIC","map":"synthetic_acquisition",
            "metadata_sha256":"0".repeat(64),"initial_objective_sha256":"1".repeat(64),
            "level_archive_sha256":"2".repeat(64),"level_clv_sha256":"3".repeat(64),
            "rows":rows,"squads":squads}))
        .unwrap()
    }

    /// One anti-aircraft entry as the runtime stages it: its own hook's
    /// targets take Completed (destroyed) or Deactivated (standing,
    /// repaired), and each target's forward spawn follows.
    fn anti_aircraft(catalog: &SpawnCatalog, lane: usize, state: ObjectiveState) -> SpawnCatalog {
        let bindings = bindings();
        let hook = &bindings.rows[&LANE_AA[lane]].hooks[&(state as u8)];
        let target_state = if state == ObjectiveState::Completed {
            ObjectiveState::Completed
        } else {
            ObjectiveState::Deactivated
        };
        let mut next = catalog.clone();
        for target in hook.spawn_targets() {
            let row = &bindings.rows[target];
            let Some(spawn) = row.spawn else { continue };
            let plan = bindings.plan_fsp(row.token, spawn, target_state).unwrap();
            (next, ()) = fsp::stage_spawn_effects(&next, &plan)
                .unwrap()
                .complete(|_| Ok(()))
                .unwrap();
        }
        next
    }

    fn open(catalog: &SpawnCatalog, lane: usize) -> bool {
        let squad = catalog.for_squad(LANE_SQUADS[lane]).unwrap();
        let drop = LANE_DROPS[lane];
        let selectable = squad.alpha(drop).is_some();
        assert_eq!(catalog.point_active(drop), selectable);
        assert_eq!(
            catalog.default_for_squad(LANE_SQUADS[lane]),
            Some(if selectable { drop } else { LANE_HOMES[lane] })
        );
        selectable
    }

    #[test]
    fn each_lanes_drop_zone_follows_only_its_own_anti_aircraft() {
        let mut catalog = spawns();
        bindings().configure_spawns(&mut catalog).unwrap();
        // Mission start: both anti-aircraft structures stand.
        let start = anti_aircraft(
            &anti_aircraft(&catalog, 0, ObjectiveState::Deactivated),
            1,
            ObjectiveState::Deactivated,
        );
        assert_eq!([open(&start, 0), open(&start, 1)], [false, false]);

        // Lane 1's anti-aircraft falls: only lane 1's zone opens and publishes.
        let lane_one = anti_aircraft(&start, 1, ObjectiveState::Completed);
        assert_eq!([open(&lane_one, 0), open(&lane_one, 1)], [false, true]);
        assert_ne!(published(&lane_one), published(&start));

        // Lane 0's falls too; then repairing lane 1's closes only lane 1's.
        let both = anti_aircraft(&lane_one, 0, ObjectiveState::Completed);
        assert_eq!([open(&both, 0), open(&both, 1)], [true, true]);
        let repaired = anti_aircraft(&both, 1, ObjectiveState::Repaired);
        assert_eq!([open(&repaired, 0), open(&repaired, 1)], [true, false]);

        // Lane 0 alone from the start matches, whatever lane 1 does.
        let lane_zero = anti_aircraft(&start, 0, ObjectiveState::Completed);
        assert_eq!(published(&lane_zero), published(&repaired));
    }
}
