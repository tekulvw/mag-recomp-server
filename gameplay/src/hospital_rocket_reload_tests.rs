//! Hospital secondary rocket salvos reload and fire again, in Domination
//! hospitals and Acquisition field hospitals, through real dispatch and
//! world ticks.
use super::tests::{equipped_fixture_for_mode, launch_wire, test_contact_bindings, tick_fixture};
use super::*;
use std::sync::Arc;

const SLOT: u8 = 3;
const TARGET: &str = "synthetic-rocket-target";

/// SYNTHETIC: the Domination tick fixture's shooter, wall, target and contact
/// bindings, installed in an Acquisition match whose field hospital 5 holds
/// the seated shooter. The Acquisition world supplies the blast collision.
fn acquisition_tick_fixture() -> Engine<Vec<u8>> {
    let mut reference = tick_fixture();
    let mut engine = equipped_fixture_for_mode(1);
    let mut acquisition = crate::acquisition_structural::connected_tests::fixture();
    let mut runtime = acquisition.acquisition_gameplay.take().unwrap();
    engine.objective_catalog = acquisition.objective_catalog.take();
    engine.objective_world = acquisition.objective_world.take();
    engine.acquisition_round = acquisition.acquisition_round.take();
    engine.item_catalog = reference.item_catalog.clone();
    engine.dynamic_body_resources = reference.dynamic_body_resources.take();
    // The same wall as the Domination fixture, on this profile's map.
    let static_world = collision::CollisionWorld::from_bytes(
        json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":engine.map_name(),"build":"synthetic",
                "coverage":"provisional","provenance":["synthetic"],"unknowns":[]},
            "instances":[{"id":"test-wall","vertices_xyz":[[0.,0.,3.],[2.5,0.,3.],[1.25,6.,3.]],
                "triangles":[[0,1,2]],"material_index":1,
                "collision":{"filter_info":9,"ray_child_filter_info":0,"convex_radius":0.,
                    "provenance":"synthetic","child_filter_policy":"uniform"}}]
        })
        .to_string()
        .as_bytes(),
        collision::CollisionLimits::default(),
    )
    .unwrap();
    let body_hash = engine
        .dynamic_body_resources
        .as_ref()
        .unwrap()
        .catalog
        .fingerprint()
        .to_owned();
    let bindings = test_contact_bindings(&engine, static_world.source_sha256(), &body_hash);
    engine.collision_world = Some(static_world);
    let state = &reference.domination_gameplay.as_ref().unwrap().state;
    let setup = Arc::get_mut(&mut runtime.setup).expect("the fixture owns its setup");
    setup.filter_rows = state.filter_rows;
    setup.projectile_contacts = Some(Arc::clone(&bindings));
    engine.acquisition_gameplay = Some(runtime);
    engine.ground_runtime = Some(Default::default());
    engine.projectile_contacts = Some(bindings);
    let spawns = engine.spawn_catalog.as_ref().unwrap();
    let rules = character_damage_rules::Rules::from_json(
        &json!({
            "schema_version":1,"application_id":20231,"map_name":spawns.map_name(),
            "game_type":1,"friendly_fire_multiplier":0.25,"provenance":{
                "spawn_catalog_sha256":spawns.source_sha256(),"source":"synthetic"},
        })
        .to_string(),
    )
    .unwrap();
    let mut target = reference.peers.remove(TARGET).unwrap();
    target.squad_catalog = Arc::new(spawns.for_squad(0).unwrap());
    engine = engine.with_character_damage_rules(rules).unwrap();
    engine.peers.insert(TARGET.into(), target);
    let bindings = engine.projectile_contacts.clone().unwrap();
    assert!(Arc::ptr_eq(
        &bindings,
        engine
            .acquisition_gameplay
            .as_ref()
            .unwrap()
            .setup
            .projectile_contacts
            .as_ref()
            .unwrap()
    ));
    engine.hospital_rockets = reference.hospital_rockets.take();
    engine.enable_world_ticks();
    engine
}

/// SYNTHETIC: every recipient has received what the server queued.
fn deliver(engine: &mut Engine<Vec<u8>>) {
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        actor.pending_shared.clear();
    }
}

fn records(engine: &Engine<Vec<u8>>) -> Vec<Value> {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect()
}

fn occupied(engine: &Engine<Vec<u8>>) -> bool {
    engine
        .active
        .as_ref()
        .unwrap()
        .hospital_rocket_flights
        .occupies(SLOT)
}

/// Fire one rocket at `at_ms`; the result is the launch or its denial.
fn fire(engine: &mut Engine<Vec<u8>>, at_ms: u64) -> Result<(), String> {
    assert!(!occupied(engine), "slot {SLOT} must be free before {at_ms}");
    let before = records(engine).len();
    let request = launch_wire(engine, SLOT.into(), 1);
    spawn_progression::dispatch(engine, &request, at_ms).unwrap();
    if occupied(engine) {
        return Ok(());
    }
    Err(records(engine)[before..]
        .iter()
        .find_map(|record| record["gameplay_denial"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "no_denial_recorded".into()))
}

/// Let the rocket reach the wall and its cleanup retire the slot.
fn fly_out(engine: &mut Engine<Vec<u8>>, at_ms: u64) {
    for (offset, label) in [(50, "contact"), (600, "cleanup")] {
        let response = engine.tick_world(
            Duration::from_millis(at_ms + offset),
            1,
            format!("rocket-{label}-{at_ms}"),
        );
        assert!(response.ok, "{label} at {at_ms}: {:?}", response.error);
    }
    assert!(
        !occupied(engine),
        "retired={:?}",
        records(engine)
            .iter()
            .rfind(|r| r["op"] == "hospital_rocket_retired")
    );
    deliver(engine);
}

fn target_health(engine: &Engine<Vec<u8>>) -> u16 {
    engine.peers[TARGET].last_damage.health_quarter_points
}

/// Fire whole magazines: each salvo is accepted at the refire cadence, the
/// next request waits for the reload and is then accepted again.
fn assert_salvos_reload(engine: &mut Engine<Vec<u8>>) {
    let weapon = engine.hospital_rockets.as_ref().unwrap().variants()[0].weapon;
    assert!(weapon.infinite_ammo && weapon.magazine_size > 1);
    let refire = (weapon.refire_seconds * 1000.) as u64;
    let reload = (weapon.reload_seconds * 1000.) as u64;
    assert!(reload > refire + 600);
    let mut at = 1000;
    for salvo in 0..3 {
        let mut last = at;
        for shot in 0..weapon.magazine_size {
            fire(engine, at).unwrap_or_else(|code| panic!("salvo {salvo} shot {shot}: {code}"));
            fly_out(engine, at);
            if salvo == 0 && shot == 0 {
                assert!(
                    target_health(engine) < 400,
                    "the first rocket deals damage: {:?}",
                    records(engine)
                        .iter()
                        .find(|r| r["op"] == "hospital_rocket_terminal")
                        .map(|r| &r["damage_denial"])
                );
            }
            last = at;
            at += refire;
        }
        assert_eq!(
            fire(engine, at).unwrap_err(),
            "hospital_rocket_reloading",
            "salvo {salvo}"
        );
        at = last + reload;
    }
    let launches = records(engine)
        .iter()
        .filter(|record| record["decoded"]["type"] == "hospital_rocket_launch")
        .filter(|record| record["decoded"]["disposition"] == "created")
        .count();
    assert_eq!(launches, 3 * usize::from(weapon.magazine_size));
}

#[test]
fn domination_hospital_rocket_salvo_reloads_and_fires_again() {
    let mut engine = tick_fixture();
    assert_salvos_reload(&mut engine);
}

#[test]
fn acquisition_field_hospital_rocket_salvo_reloads_and_fires_again() {
    let mut engine = acquisition_tick_fixture();
    assert_eq!(engine.match_profile().game_type, 1);
    assert!(engine.domination_gameplay.is_none());
    assert_salvos_reload(&mut engine);
}

/// Without rocket rules an Acquisition bunker shot stays refused, as on the
/// worker that reported the stuck launcher.
#[test]
fn acquisition_field_hospital_rocket_needs_its_rules() {
    let mut engine = acquisition_tick_fixture();
    engine.hospital_rockets = None;
    assert_eq!(
        fire(&mut engine, 1000).unwrap_err(),
        "projectile_requires_living_on_foot_character"
    );
}
