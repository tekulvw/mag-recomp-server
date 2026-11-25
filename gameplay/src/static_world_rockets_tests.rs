//! SYNTHETIC Sabotage launcher checks. The arena, launcher rows and loadouts
//! are invented; no game files or captures are used.
use super::*;
use crate::{
    human_kill_reward_tests::{enable_incoming_base, fixture as human_fixture, victim},
    spawn_progression::dispatch,
};

const LAUNCH: [f32; 3] = [100., 4.5, 100.];

/// A flat synthetic floor at y=3 with an optional wall across +Z.
fn arena(map_name: &str, wall_z: Option<f64>) -> CollisionWorld {
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

fn catalog() -> Catalog {
    infantry_rockets::tests::synthetic_catalog()
}

fn variant(faction: u8) -> infantry_rockets::Variant {
    catalog().variants()[usize::from(faction)]
}

fn equip(actor: &mut Active, item: Option<u16>) {
    let kit = actor.spawned_loadout.as_mut().unwrap();
    kit.inventories[0].slots[usize::from(infantry_rockets::GEAR_SLOT)] = item;
}

/// Two humans on opposing teams; the shooter carries `faction`'s launcher.
fn sabotage(faction: u8, wall_z: Option<f64>) -> Engine<Vec<u8>> {
    let engine = human_fixture().with_collision_world(arena("synthetic_arena", wall_z));
    let mut engine = enable_incoming_base(engine, 1.0)
        .with_static_world_infantry_rockets(catalog())
        .unwrap();
    engine.match_started = true;
    let actor = engine.active.as_mut().unwrap();
    actor.reservation.character.faction = faction;
    equip(actor, Some(variant(faction).item_id));
    place_victim(&mut engine, [100., 3., 120.], 400);
    engine
}

fn place_victim(engine: &mut Engine<Vec<u8>>, feet: [f32; 3], health: u16) {
    let peer = engine.peers.get_mut("synthetic-reservation-0000").unwrap();
    peer.tracking.server_position(
        replication::CharacterPosition {
            x: feet[0],
            y: feet[1],
            z: feet[2],
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    peer.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: health,
    };
    peer.last_damage.health_quarter_points = health;
}

/// Authenticated `0210` launch along +Z from `LAUNCH` at full power.
fn fire(engine: &mut Engine<Vec<u8>>, slot: u16, now_ms: u64) -> Vec<OutboundMessage> {
    fire_from(engine, LAUNCH, [0., 0., 0., 1.], slot, now_ms)
}

fn fire_from(
    engine: &mut Engine<Vec<u8>>,
    origin: [f32; 3],
    rotation_xyzw: [f32; 4],
    slot: u16,
    now_ms: u64,
) -> Vec<OutboundMessage> {
    let player = engine.active.as_ref().unwrap().reservation.player;
    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    bits.write(0x0210, 16).unwrap();
    for value in origin.into_iter().chain(rotation_xyzw).chain([0.; 3]) {
        bits.write(value.to_bits(), 32).unwrap();
    }
    for (value, width) in [
        (u32::from(infantry_rockets::GEAR_SLOT), 8),
        (511, 10),
        (u32::MAX, 32),
        (0x0380_0000 | u32::from(player), 32),
        (u32::from(player), 16),
        (u32::from(slot), 16),
        (0, 6),
    ] {
        bits.write(value, width).unwrap();
    }
    dispatch(engine, &bits.finish(), now_ms).unwrap()
}

fn records(engine: &Engine<Vec<u8>>, operation: &str) -> Vec<Value> {
    engine
        .writer
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter(|row| row["op"] == operation)
        .collect()
}

fn denials(engine: &Engine<Vec<u8>>) -> Vec<String> {
    records(engine, "event")
        .into_iter()
        .filter_map(|row| row["gameplay_denial"].as_str().map(str::to_owned))
        .collect()
}

fn fly(engine: &mut Engine<Vec<u8>>, from_ms: u64) {
    for step in 1..=20 {
        engine
            .tick_infantry_rockets(Duration::from_millis(from_ms + step * 50), 1)
            .unwrap();
    }
}

fn victim_health(engine: &Engine<Vec<u8>>) -> u16 {
    victim(engine)
        .life
        .damage_health()
        .map_or(0, |(_, health)| health)
}

/// The authored dose after the native falloff, in quarter points before armour.
fn falloff(faction: u8, distance: f32) -> f32 {
    let blast = variant(faction).blast;
    blast.character_hp
        * armor::native_radial_scale(distance, blast.half_radius_m, blast.outer_radius_m).unwrap()
}

#[test]
fn every_faction_launcher_hits_an_enemy_directly_with_authored_damage() {
    for faction in 0..3 {
        let mut engine = sabotage(faction, None);
        assert!(!fire(&mut engine, 1, 1000).is_empty(), "{faction}");
        assert_eq!(denials(&engine), Vec::<String>::new());
        fly(&mut engine, 1000);
        let terminal = &records(&engine, "infantry_rocket_terminal")[0];
        assert_eq!(terminal["terminal"], "contact", "{terminal}");
        assert!(
            terminal["contact"]["obstacle"]
                .as_str()
                .unwrap()
                .contains(CHARACTER_ROOT)
        );
        // Contact one reach short of the body; the anchor is 2 m above the feet.
        let reach = body_radius() + variant(faction).flight.radius_m;
        let z = terminal["position"][2].as_f64().unwrap();
        assert!((z - (120. - reach)).abs() < 1e-6, "{z}");
        let distance = (0.5_f32.powi(2) + (reach as f32).powi(2)).sqrt();
        let expected = (falloff(faction, distance) * 4.).floor() as u16;
        let dealt = 400 - victim_health(&engine);
        assert!(
            dealt.abs_diff(expected) <= 4,
            "{dealt} vs {expected} {terminal}"
        );
    }
}

#[test]
fn a_rocket_that_strikes_the_world_splashes_a_nearby_enemy_with_falloff() {
    let mut engine = sabotage(1, Some(140.));
    // Beside the flight line, out of direct reach, 2 m from the wall.
    place_victim(&mut engine, [102.5, 3., 138.], 400);
    fire(&mut engine, 1, 1000);
    fly(&mut engine, 1000);
    let terminal = &records(&engine, "infantry_rocket_terminal")[0];
    assert_eq!(terminal["terminal"], "contact");
    assert!(
        terminal["contact"]["obstacle"]
            .as_str()
            .unwrap()
            .contains("wall")
    );
    let position: Vec<f32> = terminal["position"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap() as f32)
        .collect();
    let distance =
        ((102.5 - position[0]).powi(2) + (5. - position[1]).powi(2) + (138. - position[2]).powi(2))
            .sqrt();
    let expected = (falloff(1, distance) * 4.).floor() as u16;
    let dealt = 400 - victim_health(&engine);
    assert!(
        dealt > 0 && dealt.abs_diff(expected) <= 4,
        "{dealt} vs {expected}"
    );
    assert!(f32::from(dealt) < falloff(1, 0.) * 4.);
}

#[test]
fn splash_stops_at_the_outer_radius_and_behind_static_geometry() {
    let world = arena("synthetic_arena", Some(140.));
    let blast = super::Variant::Rocket(variant(0)).blast();
    let body = |feet: [f64; 3], id: u32| Body {
        target: Target {
            character_id: 0x0380_0000 | id,
            generation: 1,
            world: 1,
        },
        feet,
        anchor_height: 2.,
    };
    let origin = [100., 5., 130.];
    let inside = body([100., 3., 131.], 1);
    let edge = body([100., 3., 130. + f64::from(blast.outer_radius_m)], 2);
    let covered = body([100., 3., 141.], 3);
    let effects = static_blast(
        &world,
        &[inside, edge, covered],
        origin,
        blast,
        0x0380_0000,
        0x1080_0000,
    )
    .unwrap();
    assert_eq!(effects.len(), 1);
    let (target, Request::HospitalRocketExplosion { raw_hp, .. }) = effects[0] else {
        panic!("rocket dose expected");
    };
    assert_eq!(target, inside.target);
    assert_eq!(raw_hp, falloff(0, 1.));
    // The covered body is in range from a blast on the near side of the wall.
    let open = static_blast(&world, &[covered], [100., 5., 141.5], blast, 0, 0x1080_0000);
    assert_eq!(open.unwrap().len(), 1);
}

#[test]
fn teammates_take_rocket_damage_through_the_friendly_fire_rule() {
    let mut enemy = enable_incoming_base(sabotage(2, None), 0.5);
    fire(&mut enemy, 1, 1000);
    fly(&mut enemy, 1000);
    let enemy_damage = 400 - victim_health(&enemy);

    let mut friendly = sabotage(2, None);
    let squads = Arc::clone(&victim(&friendly).squad_catalog);
    friendly.active.as_mut().unwrap().squad_catalog = squads;
    let mut friendly = enable_incoming_base(friendly, 0.5);
    fire(&mut friendly, 1, 1000);
    fly(&mut friendly, 1000);
    let friendly_damage = 400 - victim_health(&friendly);
    assert!(enemy_damage > 0);
    assert!(
        friendly_damage.abs_diff(enemy_damage / 2) <= 2,
        "{friendly_damage} vs {enemy_damage}"
    );

    let mut immune = sabotage(2, None);
    let squads = Arc::clone(&victim(&immune).squad_catalog);
    immune.active.as_mut().unwrap().squad_catalog = squads;
    let mut immune = enable_incoming_base(immune, 0.0);
    fire(&mut immune, 1, 1000);
    fly(&mut immune, 1000);
    assert_eq!(victim_health(&immune), 400);
}

#[test]
fn unsupported_launchers_are_refused_with_a_detailed_code() {
    let mut empty = sabotage(1, None);
    equip(empty.active.as_mut().unwrap(), None);
    fire(&mut empty, 1, 1000);
    assert_eq!(denials(&empty), ["infantry_rocket_gear_empty"]);

    let mut other = sabotage(1, None);
    // The shooter's rifle is not a launcher row.
    equip(other.active.as_mut().unwrap(), Some(2));
    fire(&mut other, 1, 1000);
    assert_eq!(denials(&other), ["infantry_rocket_weapon_unqualified"]);

    let mut foreign = sabotage(1, None);
    equip(foreign.active.as_mut().unwrap(), Some(variant(0).item_id));
    fire(&mut foreign, 1, 1000);
    assert_eq!(denials(&foreign), ["infantry_rocket_weapon_unqualified"]);
    assert_eq!(victim_health(&foreign), 400);
}

#[test]
fn the_static_scene_requires_sabotage_and_a_collision_world_for_its_map() {
    let refusal = Err("static_world_infantry_rockets_require_sabotage_collision");
    assert_eq!(
        human_fixture()
            .with_collision_world(arena("synthetic_arena", None))
            .with_static_world_infantry_rockets(catalog())
            .map(|_| ()),
        Err("static_world_infantry_rockets_require_character_damage_rules")
    );
    assert_eq!(
        human_fixture()
            .with_static_world_infantry_rockets(catalog())
            .map(|_| ()),
        refusal
    );
    assert_eq!(
        human_fixture()
            .with_collision_world(arena("other_map", None))
            .with_static_world_infantry_rockets(catalog())
            .map(|_| ()),
        refusal
    );
}

#[test]
fn first_contact_finds_the_earliest_approach_and_ignores_misses() {
    let core = [[0., 0., 10.], [0., 2., 10.]];
    let delta = [0., 0., 20.];
    let hit = first_contact([0., 1., 0.], delta, core, 0.5).unwrap();
    assert!((hit - 9.5 / 20.).abs() < 1e-9, "{hit}");
    assert_eq!(first_contact([3., 1., 0.], delta, core, 0.5), None);
    assert_eq!(first_contact([0., 1., 10.2], delta, core, 0.5), Some(0.));
}

#[test]
fn a_registered_enemy_bot_takes_the_authored_direct_hit() {
    let mut engine = crate::bot_registry::identity_tests::engine("Synthetic Rocket");
    // The shared bot fixture leaves its mode unset; make it Sabotage.
    engine.spawn_catalog = Some(serde_json::from_value(json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC rocket test",
        "active":{"map_name":"synthetic_arena","squad_index":0,"default_spawn":0,"available":[0]},
        "match_profile":{"game_type":0,"has_bunkers":false,"has_objectives":false},
        "maps":[{"name":"synthetic_arena","owner_faction":2,"spawn_count":1,
            "locations":[{"index":0,"token":1,"kind":"on_foot","position":{"x":2.,"y":0.,"z":2.}}],
            "squads":[{"index":0,"team":0,"platoon":0,"squad":0,"faction":null,"available":[0],"default_spawn":0},
            {"index":16,"team":1,"platoon":0,"squad":0,"faction":2,"available":[0],"default_spawn":0}]}]
    })).unwrap());
    let response = engine.handle(
        crate::Request::Reserve {
            v: BRIDGE_VERSION,
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
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let mut armed = human_fixture();
    let equipped = armed.active.take().unwrap();
    engine.item_catalog = armed.item_catalog;
    let actor = engine.active.as_mut().unwrap();
    crate::spawn_progression::ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    actor.current_inventory_handle = equipped.current_inventory_handle;
    actor.spawned_loadout = equipped.spawned_loadout;
    equip(actor, Some(variant(2).item_id));
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
    let feet = bot.target.runtime.unwrap().physical_position();
    engine.character_damage_rules = Some(
        crate::character_damage_rules::Rules::from_json(
            &json!({"schema_version":1,"application_id":20231,"map_name":"synthetic_arena",
                "game_type":0,"friendly_fire_multiplier":1.0,
                "provenance":{"spawn_catalog_sha256":"0".repeat(64),"source":"synthetic"}})
            .to_string(),
        )
        .unwrap(),
    );
    let mut engine = engine
        .with_static_world_infantry_rockets(catalog())
        .unwrap();
    engine.match_started = true;
    // Fired back along -Z from 12 m beyond the bot.
    let origin = [feet[0] as f32, feet[1] as f32 + 1.5, feet[2] as f32 + 12.];
    let sent = fire_from(&mut engine, origin, [0., 1., 0., 0.], 1, 1000);
    assert!(
        !sent.is_empty(),
        "{:?} {}",
        denials(&engine),
        String::from_utf8_lossy(&engine.writer)
            .lines()
            .last()
            .unwrap_or_default()
    );
    assert_eq!(denials(&engine), Vec::<String>::new());
    fly(&mut engine, 1000);
    let terminal = &records(&engine, "infantry_rocket_terminal")[0];
    assert_eq!(terminal["terminal"], "contact", "{terminal}");
    let reach = (body_radius() + variant(2).flight.radius_m) as f32;
    let expected = (falloff(2, (0.5_f32.powi(2) + reach.powi(2)).sqrt()) * 4.).floor() as u16;
    let dealt = 400 - engine.bot_registry.bots[&target.object_index].health;
    assert!(
        dealt.abs_diff(expected) <= 4,
        "{dealt} vs {expected} {terminal}"
    );
}

/// A registry worker's Sabotage profile with its own arena world and
/// `friendly_fire` rules, selected before two humans are admitted.
fn registry_sabotage(faction: u8, friendly_fire: f32) -> Engine<Vec<u8>> {
    use crate::match_profiles::static_world::tests::{ARMED, selected_arena_engine};
    let items = Arc::new(infantry_rockets::tests::derivable_items(true));
    let mut engine = selected_arena_engine(items, friendly_fire, None, ARMED);
    // Selection installed rows derived from the worker's catalog; the shared
    // human fixture then admits and arms on its own synthetic items.
    engine.item_catalog = crate::spawn_progression::engine().item_catalog;
    let mut engine = crate::human_kill_reward_tests::configured_fixture(engine);
    engine.match_started = true;
    let actor = engine.active.as_mut().unwrap();
    actor.reservation.character.faction = faction;
    equip(actor, Some(variant(faction).item_id));
    place_victim(&mut engine, [100., 3., 120.], 400);
    engine
}

#[test]
fn a_registry_sabotage_profile_rocket_damages_by_its_own_friendly_fire_rule() {
    let mut enemy = registry_sabotage(2, 0.5);
    assert!(
        !fire(&mut enemy, 1, 1000).is_empty(),
        "{:?}",
        denials(&enemy)
    );
    assert_eq!(denials(&enemy), Vec::<String>::new());
    fly(&mut enemy, 1000);
    let terminal = &records(&enemy, "infantry_rocket_terminal")[0];
    assert_eq!(terminal["terminal"], "contact", "{terminal}");
    assert!(
        terminal["contact"]["obstacle"]
            .as_str()
            .unwrap()
            .contains(CHARACTER_ROOT)
    );
    let reach = (body_radius() + variant(2).flight.radius_m) as f32;
    let expected = (falloff(2, (0.5_f32.powi(2) + reach.powi(2)).sqrt()) * 4.).floor() as u16;
    let enemy_damage = 400 - victim_health(&enemy);
    assert!(
        enemy_damage > 0 && enemy_damage.abs_diff(expected) <= 4,
        "{enemy_damage} vs {expected}"
    );

    // The profile's own rules halve a teammate's dose.
    let mut friendly = registry_sabotage(2, 0.5);
    let squads = Arc::clone(&victim(&friendly).squad_catalog);
    friendly.active.as_mut().unwrap().squad_catalog = squads;
    fire(&mut friendly, 1, 1000);
    fly(&mut friendly, 1000);
    let friendly_damage = 400 - victim_health(&friendly);
    assert!(
        friendly_damage.abs_diff(enemy_damage / 2) <= 2,
        "{friendly_damage} vs {enemy_damage}"
    );
}
