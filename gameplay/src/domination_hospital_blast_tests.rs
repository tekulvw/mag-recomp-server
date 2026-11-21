//! Hospital blast tests from authenticated frag launch and current operational meshes.
use super::*;
use crate::{replication::CharacterPosition, vehicle_exit::BunkerTransform};
use std::sync::Arc;

pub(crate) use super::super::hospital_rocket_helo_damage::tests::frag_terminal;
pub(crate) const TARGET: u16 = 5;
const POSITION: CharacterPosition = CharacterPosition {
    x: 2.,
    y: 2.5,
    z: 1.,
};

pub(crate) fn frag_fixture(enabled: bool) -> Engine<Vec<u8>> {
    frag_fixture_for_team(enabled, 0)
}

fn frag_fixture_for_team(enabled: bool, team: u16) -> Engine<Vec<u8>> {
    let mut engine = super::super::hospital_rocket_helo_damage::tests::frag_fixture(false);
    let squad = engine
        .spawn_catalog
        .as_ref()
        .unwrap()
        .for_squad(team * 16)
        .unwrap();
    engine.active.as_mut().unwrap().squad_catalog = Arc::new(squad);
    let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
    state.helo = None;
    state.collisions.remove_helo(0);
    install_hospital_fixture(&mut engine, enabled);
    engine
}

pub(crate) fn install_hospital_fixture(engine: &mut Engine<Vec<u8>>, enabled: bool) {
    let bunkers = engine.bunker_world.as_mut().unwrap();
    for (index, position) in [
        (TARGET, POSITION),
        (6, CharacterPosition { x: 20., ..POSITION }),
    ] {
        bunkers.set_synthetic_hospital_transform(
            index,
            BunkerTransform {
                position,
                rotation_xyzw: [0., 0., 0., 1.],
            },
        );
    }
    let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
    state.filter_rows[6] |= 1 << 22;
    state.filter_rows[24] |= 1 << 22;
    if enabled {
        state.hospital_blast_areas =
            Some(Arc::new(crate::hospital_blast_areas::Catalog::synthetic()));
    }
    let mesh = collision::CollisionWorld::from_bytes(
        &serde_json::to_vec(&json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic",
                "coverage":"provisional","provenance":["synthetic"],"unknowns":[]},
            "instances":[{"id":"hospital-side","material_index":255,
                "vertices_xyz":[[0.5,-2.,-2.],[0.5,2.,-2.],[0.5,0.,2.]],
                "triangles":[[0,1,2]]}]
        }))
        .unwrap(),
        Default::default(),
    )
    .unwrap();
    let assets = HospitalHulkAssets {
        by_faction: BTreeMap::from([(
            0,
            HospitalHulk {
                layer: 17,
                mesh: mesh.clone(),
                operational: Some(mesh),
                projectile: None,
                damage: HospitalDestruction {
                    raw_hp: 20.,
                    half_radius: 1.,
                    outer_radius: 2.,
                    material_penetration: 1.,
                    hulk_fade_seconds: 0.,
                },
            },
        )]),
    };
    for index in [TARGET, 6] {
        state
            .collisions
            .install_operational_hospital(bunkers.domination_bunker(index).unwrap(), &assets)
            .unwrap();
    }
    state.hospital_hulks = Some(Arc::new(assets));
    state.hospital_turret_yaws = Arc::new(BTreeMap::from([(TARGET, 0.), (6, 0.)]));
}

pub(crate) fn damage(engine: &Engine<Vec<u8>>) -> replication::VehicleDamage {
    engine
        .bunker_world
        .as_ref()
        .unwrap()
        .creations(1, 0)
        .into_iter()
        .find(|row| row.object_index == TARGET)
        .unwrap()
        .damage
}

#[test]
fn ordinary_frag_hospital_applies_the_vehicle_channel_at_its_current_body() {
    let mut engine = frag_fixture(true);
    let wire = frag_terminal(&engine, 200);
    crate::spawn_progression::dispatch(&mut engine, &wire, 1100).unwrap();
    let hit = damage(&engine);
    // (80 × .875 − 6) × .5 = 32 HP applied to the 200 HP Hospital.
    assert_eq!(hit.health_quarter_points, 672);
    assert_eq!(hit.last_location_and_type, 0xa2);
    assert_eq!(hit.last_attacker, 0x0380_0000);
    assert_eq!(hit.last_source_object, u32::MAX);
    assert_eq!(hit.last_user_data, (128 | 512) << 20);
}

fn event(engine: &Engine<Vec<u8>>) -> Value {
    engine
        .writer
        .split(|&byte| byte == b'\n')
        .rev()
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .find(|row| row["op"] == "event")
        .unwrap()
}

#[test]
fn ordinary_frag_hospital_uses_current_rotated_area_and_friendly_scaling() {
    let mut engine = frag_fixture_for_team(true, 1);
    let bunkers = engine.bunker_world.as_mut().unwrap();
    bunkers.set_synthetic_hospital_transform(
        TARGET,
        BunkerTransform {
            position: POSITION,
            rotation_xyzw: [0., 1., 0., 0.],
        },
    );
    let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
    state
        .collisions
        .install_operational_hospital(
            bunkers.domination_bunker(TARGET).unwrap(),
            state.hospital_hulks.as_deref().unwrap(),
        )
        .unwrap();
    let wire = frag_terminal(&engine, 200);
    crate::spawn_progression::dispatch(&mut engine, &wire, 1100).unwrap();
    let hit = damage(&engine);
    // (80 × .875 × .25 − 6) × .5 = 5.75 HP. Rear-half X after 180° rotation.
    assert_eq!(hit.health_quarter_points, 777);
    assert_eq!(hit.last_location_and_type, 0xa5);
    assert_eq!(
        event(&engine)["decoded"]["hospital_grenade_checks"][0]["friendly"],
        true
    );
}

#[test]
fn ordinary_frag_hospital_preserves_default_off_and_recorder_refusal() {
    let mut legacy = frag_fixture(false);
    let wire = frag_terminal(&legacy, 200);
    crate::spawn_progression::dispatch(&mut legacy, &wire, 1100).unwrap();
    assert_eq!(damage(&legacy).health_quarter_points, 800);
    let mut engine = frag_fixture(true);
    engine.active.as_mut().unwrap().latest_owner_sequence = 200;
    let stale = frag_terminal(&engine, 200);
    crate::spawn_progression::dispatch(&mut engine, &stale, 1100).unwrap();
    assert!(
        !engine
            .active
            .as_ref()
            .unwrap()
            .projectiles
            .get(3)
            .unwrap()
            .completed
    );
    let fresh = frag_terminal(&engine, 201);
    engine.max_log = engine.written;
    let queued = engine.active.as_ref().unwrap().pending_shared.clone();
    assert_eq!(
        crate::spawn_progression::dispatch(&mut engine, &fresh, 1101).unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(damage(&engine).health_quarter_points, 800);
    let source = engine.active.as_ref().unwrap();
    assert!(!source.projectiles.get(3).unwrap().completed);
    assert_eq!(source.pending_shared, queued);
}

#[test]
fn ordinary_frag_hospital_respects_cover_and_fails_an_inconsistent_live_mesh() {
    let mut covered = frag_fixture(true);
    covered.collision_world = Some(collision::CollisionWorld::from_bytes(
        &serde_json::to_vec(&json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":covered.map_name(),"build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":[]},
            "instances":[{"id":"cover","material_index":255,
                "vertices_xyz":[[2.75,0.,0.],[2.75,6.,0.],[2.75,3.,4.]],"triangles":[[0,1,2]],
                "collision":{"filter_info":9,"ray_child_filter_info":0,"convex_radius":0.,
                    "provenance":"synthetic","child_filter_policy":"uniform"}}]
        })).unwrap(), Default::default()).unwrap());
    let wire = frag_terminal(&covered, 200);
    crate::spawn_progression::dispatch(&mut covered, &wire, 1100).unwrap();
    assert_eq!(damage(&covered).health_quarter_points, 800);
    assert!(event(&covered)["gameplay_denial"].is_null());

    let mut engine = frag_fixture(true);
    engine
        .domination_gameplay
        .as_mut()
        .unwrap()
        .state
        .collisions
        .hospitals
        .get_mut(&TARGET)
        .unwrap()
        .pose
        .translation[0] += 1.;
    let wire = frag_terminal(&engine, 200);
    crate::spawn_progression::dispatch(&mut engine, &wire, 1100).unwrap();
    assert_eq!(damage(&engine).health_quarter_points, 800);
    let source = engine.active.as_ref().unwrap();
    assert!(source.projectiles.get(3).unwrap().completed);
    assert!(engine.projectile_cleanup_pending(source, 3));
    assert_eq!(
        event(&engine)["decoded"]["hospital_grenade_checks"][0]["error"],
        "domination_hospital_blast_mesh_mismatch"
    );
}

fn wire_with_frag_reports(
    engine: &Engine<Vec<u8>>,
    terminals: &[(u8, CharacterPosition)],
    turret: Option<hospital_turret_wire::Fields>,
) -> Vec<u8> {
    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    let player = engine.active.as_ref().unwrap().reservation.player;
    let mask = terminals
        .iter()
        .fold(0, |mask, (slot, _)| mask | (1 << (16 + slot)));
    replication::UpdateHeader {
        sequence: 200,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: mask | (u32::from(turret.is_some()) << 13),
        timestamp_ms: 1100,
    }
    .encode(&mut bits)
    .unwrap();
    if let Some(fields) = turret {
        bits.write(1, 1).unwrap();
        bits.write(TARGET.into(), 16).unwrap();
        for field in 0..25 {
            if !fields.write_field(field, None, &mut bits).unwrap() {
                bits.write(0, 1).unwrap();
            }
        }
        bits.write(0, 1).unwrap();
    }
    for &(_, position) in terminals {
        bits.write(1, 1).unwrap();
        bits.write(player.into(), 16).unwrap();
        projectile::ProjectileOwnerFields {
            state: Some(2),
            position: Some(position),
            rotation_xyzw: None,
        }
        .encode_fields(&mut bits)
        .unwrap();
        bits.write(0, 1).unwrap();
    }
    bits.finish()
}

#[test]
fn ordinary_frag_hospital_accumulates_same_event_fuses_into_one_final_batch() {
    let mut engine = frag_fixture(true);
    let launch = crate::hospital_rocket_runtime::tests::launch_wire(&engine, 0, 4);
    assert!(
        !crate::spawn_progression::dispatch(&mut engine, &launch, 1000)
            .unwrap()
            .is_empty()
    );
    let wire = wire_with_frag_reports(
        &engine,
        &[
            (0, CharacterPosition { x: 3., ..POSITION }),
            (3, CharacterPosition { z: 0., ..POSITION }),
        ],
        None,
    );
    crate::spawn_progression::dispatch(&mut engine, &wire, 1100).unwrap();
    let audit = event(&engine);
    let rows = audit["decoded"]["hospital_grenade_checks"]
        .as_array()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["health_before_quarter_points"], 800);
    assert_eq!(rows[0]["health_after_quarter_points"], 672);
    assert_eq!(rows[1]["health_before_quarter_points"], 672);
    assert_eq!(rows[1]["health_after_quarter_points"], 544);
    let source = engine.active.as_ref().unwrap();
    let packets = source
        .pending_shared
        .iter()
        .filter(|wire| {
            replication::ObjectUpdate::decode_with_profile(wire, engine.wire_profile)
                .is_ok_and(|update| update.header.object_type_mask == 1 << 13)
        })
        .collect::<Vec<_>>();
    let [packet] = packets.as_slice() else {
        panic!("one final field18 batch expected")
    };
    let mut bits = replication::BitReader::with_profile(packet, engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(TARGET.into()));
    assert_eq!(bits.read(18), Ok(0));
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(8), Ok(0xa5));
    assert_eq!(bits.read(16).unwrap() & 0x0fff, 544);
    assert_eq!(bits.read(32), Ok(0x0380_0000));
    assert_eq!(bits.read(32), Ok(u32::MAX));
    assert_eq!(bits.read(32), Ok(128)); // Away from the final origin along +Z.
    assert_eq!(bits.read(7), Ok(0)); // Six remaining fields, then the list terminator.
}

#[test]
fn ordinary_frag_hospital_does_not_overwrite_same_event_valid_turret_aim() {
    let mut engine = frag_fixture_for_team(true, 1);
    let active = engine.active.as_mut().unwrap();
    active.mounted_bunker = Some(TARGET);
    active.mounted_seat = Some(0);
    let current = crate::hospital_turrets::Runtime::new(5, Some(6));
    let (turret, definition) = current.mount(active).unwrap();
    let fields = hospital_turret_wire::Fields {
        yaw: Some(hospital_turret_wire::Yaw::new(128).unwrap()),
        primary: Some(replication::InventoryItem {
            inventory_update_id: definition.version,
            gear_slot: 0,
            ammo_pool: 0,
            ammo_count: 11,
            reloading: false,
        }),
        ..Default::default()
    };
    let (expected, _) = turret.report(fields).unwrap();
    engine
        .bunker_world
        .as_mut()
        .unwrap()
        .hospital_turrets
        .insert(TARGET, turret);
    let wire = wire_with_frag_reports(
        &engine,
        &[(3, CharacterPosition { x: 3., ..POSITION })],
        Some(fields),
    );
    let actor = engine.active.as_ref().unwrap();
    let input = events::EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: actor.reservation.world,
        player_id: actor.reservation.player,
        transport: Transport::Udp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(&wire),
    };
    let decoded = DecodedEvent::with_hospital_ownership(
        &wire,
        input.player_id,
        engine.wire_profile,
        false,
        true,
    );
    engine
        .handle_event(input, &decoded, Duration::from_millis(1100), 1)
        .unwrap();
    assert_eq!(damage(&engine).health_quarter_points, 777);
    assert_eq!(
        engine.bunker_world.as_ref().unwrap().hospital_turrets[&TARGET],
        expected,
        "{}",
        event(&engine)
    );
}

#[test]
fn ordinary_frag_hospital_defers_lethal_state_until_vehicle_update() {
    let mut engine = frag_fixture(true);
    engine.item_catalog = crate::human_kill_reward_tests::vehicle_grenade_items_with_cover(800.);
    let wire = frag_terminal(&engine, 200);
    crate::spawn_progression::dispatch(&mut engine, &wire, 1100).unwrap();
    let bunkers = engine.bunker_world.as_mut().unwrap();
    let original = bunkers
        .creations(1, 0)
        .into_iter()
        .find(|row| row.object_index == TARGET)
        .unwrap();
    assert_eq!(original.damage.health_quarter_points, 0);
    assert_eq!(original.state, replication::VehicleState::Operational);
    assert!(
        engine
            .domination_gameplay
            .as_ref()
            .unwrap()
            .state
            .collisions
            .hospitals
            .contains_key(&TARGET)
    );
    let updates = bunkers
        .advance_domination_hospital_damage(Duration::from_millis(1200))
        .unwrap();
    let [(update, attacker)] = updates.as_slice() else {
        panic!("one deferred Hospital update")
    };
    assert_eq!(update.object_index(), TARGET);
    assert!(update.kills_occupants());
    assert!(update.destroys_turret());
    assert_eq!(*attacker, 0x0380_0000);
    let rebuild = bunkers
        .apply_domination_cycle(original.spawn_point_index, true, u32::MAX)
        .unwrap();
    assert!(rebuild.restores_turret());
    assert_eq!(damage(&engine).health_quarter_points, 800);
}
