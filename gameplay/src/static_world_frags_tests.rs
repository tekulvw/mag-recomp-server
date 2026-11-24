//! SYNTHETIC Sabotage frag checks. The arena, frag row and loadouts are
//! invented; no game files or captures are used.
use super::*;
use crate::{
    Active, CharacterLife, OutboundMessage,
    human_kill_reward_tests::{
        combat_fixture_documents, enable_incoming_base, finish_combat_fixture, fixture,
        synthetic_blast_rules, victim,
    },
    item_catalog, projectile, projectile_replication, replication,
    spawn_progression::dispatch,
};
use serde_json::{Value, json};
use std::sync::Arc;

const FRAG: u16 = 4;
const SLOT: u16 = 1;
/// The synthetic frag's authored Character dose and radii.
const DOSE_HP: f32 = 20.;
const HALF_M: f32 = 4.;
const OUTER_M: f32 = 8.;
/// Reported resting position of the frag, just above the floor at y=3.
const BLAST: [f32; 3] = [100., 3.05, 100.];
const THROWER: [f32; 3] = [100., 3., 85.];

/// A flat synthetic floor at y=3 with an optional wall across +Z.
fn arena(map_name: &str, wall_z: Option<f32>) -> CollisionWorld {
    let mut instances = vec![json!({"id":"floor",
        "vertices_xyz":[[0.,3.,0.],[200.,3.,0.],[200.,3.,200.],[0.,3.,200.]],
        "triangles":[[0,2,1],[0,3,2]]})];
    if let Some(z) = wall_z {
        instances.push(json!({"id":"wall",
            "vertices_xyz":[[80.,0.,z],[120.,0.,z],[120.,20.,z],[80.,20.,z]],
            "triangles":[[0,1,2],[0,2,3]]}));
    }
    CollisionWorld::from_bytes(
        &serde_json::to_vec(&json!({"format":"mag_collision_world","schema_version":1,
            "coordinate_space":"world","metadata":{"map_name":map_name,"build":"synthetic",
            "coverage":"provisional","provenance":["synthetic"],"unknowns":["Not a game map"]},
            "instances":instances}))
        .unwrap(),
        Default::default(),
    )
    .unwrap()
}

/// Synthetic retail items whose frag row has no material penetration, as a
/// Sabotage metadata projection without material bindings.
fn frag_items() -> Arc<item_catalog::ItemCatalog> {
    let (raw, metadata) = combat_fixture_documents(true, 0., 0., None, false, false);
    finish_combat_fixture(
        item_catalog::ItemCatalog::from_value_for_application(raw, synthetic_blast_rules(), 20231)
            .unwrap(),
        &metadata,
    )
}

fn arm(actor: &mut Active) {
    let kit = actor.spawned_loadout.as_mut().unwrap();
    kit.inventories[0].slots[4] = Some(FRAG);
    actor.grenade_remaining = 1;
}

fn place(actor: &mut Active, feet: [f32; 3]) {
    actor.tracking.server_position(
        replication::CharacterPosition {
            x: feet[0],
            y: feet[1],
            z: feet[2],
        },
        crate::player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    actor.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    actor.last_damage.health_quarter_points = 400;
}

/// Two humans on opposing teams in Sabotage; the thrower carries the frag.
fn unconfigured(wall_z: Option<f32>, friendly_fire: f32) -> Engine<Vec<u8>> {
    let mut engine = fixture();
    engine.item_catalog = frag_items();
    let mut engine = enable_incoming_base(
        engine.with_collision_world(arena("synthetic_arena", wall_z)),
        friendly_fire,
    );
    engine.match_started = true;
    let thrower = engine.active.as_mut().unwrap();
    arm(thrower);
    place(thrower, THROWER);
    place_victim(&mut engine, [100., 3., 103.]);
    engine
}

fn sabotage(wall_z: Option<f32>, friendly_fire: f32) -> Engine<Vec<u8>> {
    unconfigured(wall_z, friendly_fire)
        .with_static_world_frag_blast()
        .unwrap()
}

fn place_victim(engine: &mut Engine<Vec<u8>>, feet: [f32; 3]) {
    place(
        engine.peers.get_mut("synthetic-reservation-0000").unwrap(),
        feet,
    );
}

fn same_team(engine: &mut Engine<Vec<u8>>) {
    let squads = Arc::clone(&victim(engine).squad_catalog);
    engine.active.as_mut().unwrap().squad_catalog = squads;
}

/// Authenticated `0210` frag throw into `SLOT`.
fn throw(engine: &mut Engine<Vec<u8>>, now_ms: u64) -> Vec<OutboundMessage> {
    let player = engine.active.as_ref().unwrap().reservation.player;
    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    bits.write(0x0210, 16).unwrap();
    for value in THROWER
        .into_iter()
        .chain([0., 0., 0., 1.])
        .chain([0., 4., 10.])
    {
        bits.write(f32::to_bits(value), 32).unwrap();
    }
    for (value, width) in [
        (u32::from(FRAG), 8),
        (511, 10),
        (u32::MAX, 32),
        (0x0380_0000 | u32::from(player), 32),
        (u32::from(player), 16),
        (u32::from(SLOT), 16),
        (0, 6),
    ] {
        bits.write(value, width).unwrap();
    }
    dispatch(engine, &bits.finish(), now_ms).unwrap()
}

/// The owner reports the frag resting at `at` (state 2).
fn report_detonation(engine: &mut Engine<Vec<u8>>, at: [f32; 3], now_ms: u64) {
    let actor = engine.active.as_ref().unwrap();
    let wire = projectile_replication::ProjectileOwnerDelta {
        header: replication::UpdateHeader {
            sequence: actor.latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << (16 + SLOT),
            timestamp_ms: now_ms as u32,
        },
        update: &projectile::ProjectileOwnerUpdate {
            slot: SLOT as u8,
            object_index: actor.reservation.player,
            fields: projectile::ProjectileOwnerFields {
                state: Some(2),
                position: Some(replication::CharacterPosition {
                    x: at[0],
                    y: at[1],
                    z: at[2],
                }),
                rotation_xyzw: None,
            },
        },
    }
    .encode_with_profile(engine.wire_profile)
    .unwrap();
    dispatch(engine, &wire, now_ms).unwrap();
}

fn idle(engine: &mut Engine<Vec<u8>>, now_ms: u64) {
    let wire = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: now_ms as u32,
        },
        wire: &[],
    }
    .encode_empty_with_profile(engine.wire_profile)
    .unwrap();
    dispatch(engine, &wire, now_ms).unwrap();
}

/// Throw, report the detonation after the fuse, then drain the queued doses.
fn detonate(engine: &mut Engine<Vec<u8>>) {
    detonate_at(engine, BLAST);
}

fn detonate_at(engine: &mut Engine<Vec<u8>>, at: [f32; 3]) {
    assert!(!throw(engine, 1000).is_empty(), "{:?}", denials(engine));
    report_detonation(engine, at, 1200);
    for step in 1..=4 {
        idle(engine, 1200 + step * 50);
    }
}

fn events(engine: &Engine<Vec<u8>>) -> Vec<Value> {
    engine
        .writer
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter(|row| row["op"] == "event")
        .collect()
}

fn denials(engine: &Engine<Vec<u8>>) -> Vec<String> {
    events(engine)
        .into_iter()
        .filter_map(|row| row["gameplay_denial"].as_str().map(str::to_owned))
        .collect()
}

fn health(actor: &Active) -> u16 {
    actor.life.damage_health().map_or(0, |(_, health)| health)
}

/// The authored dose in quarter points at `distance` from the blast.
fn quarter_points(distance: f32, friendly_fire: f32) -> u16 {
    let scale = armor::native_radial_scale(distance, HALF_M, OUTER_M).unwrap();
    (DOSE_HP * scale * friendly_fire * 4.).round() as u16
}

/// Distance from `BLAST` to a standing Character's anchor at `feet`.
fn anchor_distance(feet: [f32; 3]) -> f32 {
    let anchor = [
        feet[0],
        feet[1] + CHARACTER_BLAST_ANCHOR_HEIGHT_M[0],
        feet[2],
    ];
    (0..3)
        .map(|axis| (anchor[axis] - BLAST[axis]).powi(2))
        .sum::<f32>()
        .sqrt()
}

#[test]
fn a_sabotage_frag_damages_an_enemy_with_the_authored_falloff() {
    let mut engine = sabotage(None, 1.);
    detonate(&mut engine);
    assert_eq!(denials(&engine), Vec::<String>::new());
    let expected = quarter_points(anchor_distance([100., 3., 103.]), 1.);
    assert!(expected > 0);
    let dealt = 400 - health(victim(&engine));
    assert!(dealt.abs_diff(expected) <= 1, "{dealt} vs {expected}");
    let damage = victim(&engine).last_damage;
    assert_eq!(damage.last_attacker, 0x0380_0001);
    assert_eq!(damage.last_source_object, 0x1180_0001);
    // The thrower stands outside the outer radius.
    assert_eq!(health(engine.active.as_ref().unwrap()), 400);
    let rows = events(&engine);
    assert!(rows.iter().all(|row| {
        row["decoded"]["grenade_visibility_policy"]
            .as_str()
            .is_none_or(|policy| policy == "static_world_stance_anchor_ray")
    }));
    let checks: Vec<_> = rows
        .iter()
        .filter_map(|row| row["decoded"]["native_grenade_character_checks"].as_array())
        .flatten()
        .collect();
    assert_eq!(checks.len(), 1, "{checks:?}");
    assert_eq!(checks[0]["character_id"], 0x0380_0000);
}

#[test]
fn static_geometry_covers_and_the_outer_radius_bounds_the_blast() {
    let mut covered = sabotage(Some(101.5), 1.);
    detonate(&mut covered);
    assert_eq!(health(victim(&covered)), 400);

    let mut far = sabotage(None, 1.);
    place_victim(&mut far, [100., 3., 100. + OUTER_M]);
    detonate(&mut far);
    assert_eq!(health(victim(&far)), 400);
}

#[test]
fn a_thrower_inside_the_radius_is_hit_by_its_own_frag() {
    let mut engine = sabotage(None, 0.);
    let feet = [100., 3., 97.];
    place(engine.active.as_mut().unwrap(), feet);
    detonate(&mut engine);
    // Self damage keeps the full dose; the team rule never scales it.
    let expected = quarter_points(anchor_distance(feet), 1.);
    let dealt = 400 - health(engine.active.as_ref().unwrap());
    assert!(dealt.abs_diff(expected) <= 1, "{dealt} vs {expected}");
}

#[test]
fn teammates_take_frag_damage_through_the_friendly_fire_rule() {
    let expected = quarter_points(anchor_distance([100., 3., 103.]), 1.);
    let mut half = unconfigured(None, 0.5);
    same_team(&mut half);
    let mut half = half.with_static_world_frag_blast().unwrap();
    detonate(&mut half);
    let dealt = 400 - health(victim(&half));
    assert!(dealt.abs_diff(expected / 2) <= 1, "{dealt} vs {expected}");

    let mut immune = unconfigured(None, 0.);
    same_team(&mut immune);
    let mut immune = immune.with_static_world_frag_blast().unwrap();
    detonate(&mut immune);
    assert_eq!(health(victim(&immune)), 400);
}

#[test]
fn without_the_static_scene_a_sabotage_frag_still_does_no_damage() {
    let mut engine = unconfigured(None, 1.);
    detonate(&mut engine);
    assert_eq!(health(victim(&engine)), 400);
    assert!(
        events(&engine)
            .iter()
            .any(|row| { row["decoded"]["grenade_visibility_policy"] == "unknown_no_damage" })
    );
}

#[test]
fn the_static_scene_requires_its_inputs() {
    let collision = Err("static_world_frag_blast_requires_sabotage_collision");
    let mut no_world = fixture();
    no_world.item_catalog = frag_items();
    let no_world = enable_incoming_base(no_world, 1.);
    assert_eq!(
        no_world.with_static_world_frag_blast().map(|_| ()),
        collision
    );
    let mut other_map = fixture();
    other_map.item_catalog = frag_items();
    let other_map =
        enable_incoming_base(other_map.with_collision_world(arena("other_map", None)), 1.);
    assert_eq!(
        other_map.with_static_world_frag_blast().map(|_| ()),
        collision
    );
    let mut no_rules = fixture().with_collision_world(arena("synthetic_arena", None));
    no_rules.item_catalog = frag_items();
    assert_eq!(
        no_rules.with_static_world_frag_blast().map(|_| ()),
        Err("static_world_frag_blast_requires_character_damage_rules")
    );
    // The default fixture's combat metadata has no grenade damage authority.
    let no_grenades = enable_incoming_base(
        fixture().with_collision_world(arena("synthetic_arena", None)),
        1.,
    );
    assert_eq!(
        no_grenades.with_static_world_frag_blast().map(|_| ()),
        Err("static_world_frag_blast_requires_grenade_damage")
    );
    assert_eq!(qualified_items(&frag_items(), true), Ok(vec![FRAG]));
}

#[test]
fn a_registered_enemy_bot_in_range_takes_the_frag_dose() {
    let mut engine = crate::bot_registry::identity_tests::engine("Synthetic Frag");
    // The shared bot fixture leaves its mode unset; make it Sabotage.
    engine.spawn_catalog = Some(serde_json::from_value(json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC frag test",
        "active":{"map_name":"synthetic_arena","squad_index":0,"default_spawn":0,"available":[0]},
        "match_profile":{"game_type":0,"has_bunkers":false,"has_objectives":false},
        "maps":[{"name":"synthetic_arena","owner_faction":2,"spawn_count":1,
            "locations":[{"index":0,"token":1,"kind":"on_foot","position":{"x":2.,"y":0.,"z":2.}}],
            "squads":[{"index":0,"team":0,"platoon":0,"squad":0,"faction":null,"available":[0],"default_spawn":0},
            {"index":16,"team":1,"platoon":0,"squad":0,"faction":2,"available":[0],"default_spawn":0}]}]
    })).unwrap());
    let response = engine.handle(
        crate::Request::Reserve {
            v: crate::BRIDGE_VERSION,
            request_id: "synthetic-reserve-0".into(),
            reservation_id: "synthetic-reservation-0000".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic 0","faction":2,
                "head":0,"voice":0,"universe_id":26,"character_id":7}))
            .unwrap(),
        },
        std::time::Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    engine
        .reconcile_bot_population(std::time::Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let equipped = fixture().active.take().unwrap();
    engine.item_catalog = frag_items();
    let actor = engine.active.as_mut().unwrap();
    crate::spawn_progression::ready(actor);
    actor.current_inventory_handle = equipped.current_inventory_handle;
    actor.spawned_loadout = equipped.spawned_loadout;
    arm(actor);
    // Thrown from well outside the blast.
    place(actor, [-8., 0., -8.]);
    let team = actor.team();
    let kit = actor.equipped_loadout().unwrap();
    let target = engine
        .bot_combat_snapshots()
        .into_iter()
        .find(|bot| bot.team != team)
        .unwrap();
    let bot = engine
        .bot_registry
        .bots
        .get_mut(&target.object_index)
        .unwrap();
    bot.target.loadout = kit;
    bot.health = 400;
    bot.damage.health_quarter_points = 400;
    let feet = bot
        .target
        .runtime
        .unwrap()
        .physical_position()
        .map(|axis| axis as f32);
    engine.character_damage_rules = Some(
        crate::character_damage_rules::Rules::from_json(
            &json!({"schema_version":1,"application_id":20231,"map_name":"synthetic_arena",
                "game_type":0,"friendly_fire_multiplier":1.0,
                "provenance":{"spawn_catalog_sha256":"0".repeat(64),"source":"synthetic"}})
            .to_string(),
        )
        .unwrap(),
    );
    let mut engine = engine.with_static_world_frag_blast().unwrap();
    engine.match_started = true;
    let at = [feet[0], feet[1] + 0.05, feet[2] + 1.];
    detonate_at(&mut engine, at);
    assert_eq!(denials(&engine), Vec::<String>::new());
    let anchor = [
        feet[0],
        feet[1] + CHARACTER_BLAST_ANCHOR_HEIGHT_M[0],
        feet[2],
    ];
    let distance = (0..3)
        .map(|axis| (anchor[axis] - at[axis]).powi(2))
        .sum::<f32>()
        .sqrt();
    let expected = quarter_points(distance, 1.);
    let dealt = 400 - engine.bot_registry.bots[&target.object_index].health;
    assert!(expected > 0);
    assert!(dealt.abs_diff(expected) <= 1, "{dealt} vs {expected}");
}
