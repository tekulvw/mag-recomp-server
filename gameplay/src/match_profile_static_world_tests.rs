//! SYNTHETIC registry Sabotage launcher and frag qualification. The arena,
//! rules, profiles and items are invented; no game files or captures are used.
use super::*;
use crate::{
    human_kill_reward_tests::{
        combat_fixture_documents, finish_combat_fixture, synthetic_blast_rules,
    },
    infantry_rockets::tests::derivable_items,
    spawn_progression::tests::{objective_json, spawn_json},
};
use std::sync::atomic::{AtomicU64, Ordering};

/// The qualified profile: its own arena world and friendly-fire rules.
pub(crate) const ARMED: &str = "sabotage-armed";
/// The same arena world without friendly-fire rules.
pub(crate) const UNRULED: &str = "sabotage-unruled";
/// The same Sabotage map with no collision world at all.
pub(crate) const BARE: &str = "sabotage-bare";

/// A flat synthetic floor at y=3 with an optional wall across +Z.
fn arena(wall_z: Option<f64>) -> Value {
    let mut instances = vec![json!({"id":"floor",
        "vertices_xyz":[[0.,3.,0.],[200.,3.,0.],[200.,3.,200.],[0.,3.,200.]],
        "triangles":[[0,2,1],[0,3,2]]})];
    if let Some(z) = wall_z {
        instances.push(json!({"id":"wall",
            "vertices_xyz":[[80.,0.,z],[120.,0.,z],[120.,20.,z],[80.,20.,z]],
            "triangles":[[0,1,2],[0,2,3]]}));
    }
    json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
            "provenance":["synthetic"],"unknowns":["Not a game map"]},
        "instances":instances})
}

/// A registry of three Sabotage profiles on the synthetic arena: [`ARMED`]
/// with `friendly_fire` rules, [`UNRULED`] and [`BARE`].
pub(crate) fn arena_registry(friendly_fire: f32, wall_z: Option<f64>) -> Registry {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "mag-synthetic-static-world-profiles-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let write = |name: &str, value: Value| {
        let path = directory.join(name);
        std::fs::write(&path, value.to_string()).unwrap();
        path
    };
    let spawns = write("spawns.json", spawn_json());
    let objectives = write("objectives.json", objective_json());
    let collision = write("collision.json", arena(wall_z));
    let digest = spawn_catalog::SpawnCatalog::load(&spawns)
        .unwrap()
        .source_sha256()
        .unwrap()
        .to_owned();
    let damage = write(
        "damage.json",
        json!({"schema_version":1,"application_id":20231,"map_name":"synthetic_arena",
            "game_type":0,"friendly_fire_multiplier":friendly_fire,
            "provenance":{"spawn_catalog_sha256":digest,"source":"synthetic"}}),
    );
    let manifest = write(
        "profiles.json",
        json!({"format":"mag_match_profiles_v1","profiles":[
            {"profile_id":ARMED,"spawn_catalog":spawns,"objective_catalog":objectives,
                "collision_world":collision,"character_damage_rules":damage},
            {"profile_id":UNRULED,"spawn_catalog":spawns,"objective_catalog":objectives,
                "collision_world":collision},
            {"profile_id":BARE,"spawn_catalog":spawns,"objective_catalog":objectives}]}),
    );
    let registry = Registry::load(&manifest).unwrap();
    // Profiles are preloaded; selection never reads the files again.
    std::fs::remove_dir_all(&directory).unwrap();
    registry
}

/// An idle registry worker on `items` that selected `profile` from
/// [`arena_registry`], before any human is admitted.
pub(crate) fn selected_arena_engine(
    items: Arc<item_catalog::ItemCatalog>,
    friendly_fire: f32,
    wall_z: Option<f64>,
    profile: &str,
) -> Engine<Vec<u8>> {
    let mut engine = crate::spawn_progression::engine();
    engine.item_catalog = items;
    let mut engine = engine
        .with_match_profiles(arena_registry(friendly_fire, wall_z))
        .unwrap();
    engine
        .select_match_profile("synthetic-formation-1".into(), profile.into(), 1)
        .unwrap();
    engine
}

/// Synthetic retail items with ordinary frag grenade damage authority.
pub(crate) fn frag_items() -> Arc<item_catalog::ItemCatalog> {
    let (raw, metadata) = combat_fixture_documents(true, 0., 0., None, false, false);
    finish_combat_fixture(
        item_catalog::ItemCatalog::from_value_for_application(raw, synthetic_blast_rules(), 20231)
            .unwrap(),
        &metadata,
    )
}

fn report(registry: &Registry, profile: &str) -> Value {
    registry.character_weapon_report(profile)
}

#[test]
fn a_sabotage_profile_with_its_own_world_and_rules_derives_launcher_rows() {
    let mut registry = arena_registry(0.5, None);
    registry.bind_static_world_weapons(&derivable_items(true));
    assert!(registry.has_static_world_rockets());
    let expected = crate::infantry_rockets::tests::synthetic_catalog();
    let armed = &registry.static_world[ARMED];
    assert_eq!(armed.rockets().unwrap().variants(), expected.variants());
    let ids: Vec<_> = expected.variants().iter().map(|v| v.item_id).collect();
    assert_eq!(
        report(&registry, ARMED)["infantry_rockets"],
        json!({"source":"item_catalog","scene":"static_collision_world","items":ids})
    );
    // These items carry no grenade damage authority; frags stay refused.
    assert!(!armed.frags());
    assert_eq!(report(&registry, ARMED)["frag_blast"]["enabled"], false);
}

#[test]
fn a_sabotage_profile_with_grenade_authority_qualifies_frag_blast() {
    let mut registry = arena_registry(0.5, None);
    let items = frag_items();
    let frags = crate::static_world_frags::qualified_items(&items, true).unwrap();
    registry.bind_static_world_weapons(&items);
    let armed = &registry.static_world[ARMED];
    assert!(armed.frags());
    assert_eq!(
        report(&registry, ARMED)["frag_blast"],
        json!({"enabled":true,"scene":"static_collision_world",
            "cover":"any_static_crossing","items":frags})
    );
    // The combat fixture authors no dumb-fire launchers.
    assert!(armed.rockets().is_none());
    assert_eq!(
        report(&registry, ARMED)["infantry_rockets"]["enabled"],
        false
    );
}

#[test]
fn profiles_without_rules_world_or_bound_items_are_refused() {
    let mut registry = arena_registry(0.5, None);
    registry.bind_static_world_weapons(&derivable_items(true));
    assert_eq!(
        report(&registry, UNRULED),
        json!({"infantry_rockets":{"enabled":false,"reason":"character_damage_rules_required"},
            "frag_blast":{"enabled":false,
                "reason":"static_world_frag_blast_requires_character_damage_rules"}})
    );
    assert_eq!(
        report(&registry, BARE),
        json!({"infantry_rockets":null,"frag_blast":null})
    );
    assert!(!registry.static_world.contains_key(BARE));

    // Combat metadata naming a different item catalog derives nothing.
    let mut unbound = arena_registry(0.5, None);
    unbound.bind_static_world_weapons(&derivable_items(false));
    assert!(!unbound.has_static_world_rockets());
    assert_eq!(
        report(&unbound, ARMED)["infantry_rockets"],
        json!({"enabled":false,"reason":"derived_infantry_rocket_source_unbound"})
    );

    let (rockets, frags) = registry.character_weapon_reports();
    assert_eq!(rockets["per_profile"][BARE], Value::Null);
    assert_eq!(
        rockets["per_profile"][ARMED]["scene"],
        "static_collision_world"
    );
    assert_eq!(frags["per_profile"][UNRULED]["enabled"], false);
}

#[test]
fn selecting_a_profile_installs_or_clears_its_static_world_weapons() {
    let mut engine = selected_arena_engine(Arc::new(derivable_items(true)), 0.5, None, ARMED);
    assert!(engine.infantry_rockets.is_some());
    assert!(engine.static_rocket_world().is_some());
    assert!(engine.character_damage_rules.is_some());
    let selected: Vec<Value> = engine
        .writer
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter(|row| row["op"] == "match_profile_selected")
        .collect();
    assert_eq!(
        selected[0]["character_weapons"]["infantry_rockets"]["scene"],
        "static_collision_world"
    );

    // The next formation's profile lacks rules: nothing carries over.
    engine
        .select_match_profile("synthetic-formation-2".into(), UNRULED.into(), 2)
        .unwrap();
    assert!(engine.infantry_rockets.is_none());
    assert!(engine.static_frag_world().is_none());

    let mut frags = selected_arena_engine(frag_items(), 0.5, None, ARMED);
    assert!(frags.static_frag_world().is_some());
    assert!(frags.frag_character_blast_enabled());
    frags
        .select_match_profile("synthetic-formation-2".into(), BARE.into(), 2)
        .unwrap();
    assert!(frags.static_frag_world().is_none());
    assert!(!frags.static_world_frag_blast);
}
