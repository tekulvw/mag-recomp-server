//! SYNTHETIC off-mesh stop regressions: a wide raised slab above open
//! ground, authored in code. No game catalogs, meshes or captures.
use super::*;
use sha2::{Digest, Sha256};

const SLAB_Y: f64 = 3.;
/// The slab spans x 10..20; the navmesh is eroded 0.6 m from its edges, so a
/// body 0.4 m in from the west edge stands on the slab but off the mesh.
const SLAB_WEST_X: f64 = 10.;
const BAND_X: f64 = SLAB_WEST_X + 0.4;

fn slab_engine(x: f64) -> (Engine<Vec<u8>>, u16) {
    let quads: [[f64; 5]; 2] = [
        // x0, x1, y, z0, z1
        [0., 30., 0., 0., 30.],
        [SLAB_WEST_X, 20., SLAB_Y, 5., 25.],
    ];
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for [x0, x1, y, z0, z1] in quads {
        let n = vertices.len() as u32;
        vertices.extend([[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]]);
        triangles.extend([[n, n + 2, n + 1], [n, n + 3, n + 2]]);
    }
    let (min, max) = ([0., -5., 0.], [30., 8., 30.]);
    let raw = serde_json::to_vec(&json!({
        "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
            "provenance":["synthetic slab"],"unknowns":["Not a game map"],"physics_regions":[
                {"min":min,"max":max,"ground_verified":true,"obstacles_verified":true,
                 "provenance":["synthetic slab over open ground"]}]},
        "instances":[{"id":"slab","vertices_xyz":vertices,"triangles":triangles}]
    }))
    .unwrap();
    let world = collision::CollisionWorld::from_bytes(&raw, Default::default()).unwrap();
    let vertices: Vec<[f32; 3]> = vertices.iter().map(|p| p.map(|v| v as f32)).collect();
    let mut hash = Sha256::new();
    hash.update(b"MAG-NAV-GEOMETRY-v1\0");
    hash.update((vertices.len() as u32).to_le_bytes());
    hash.update((triangles.len() as u32).to_le_bytes());
    for v in vertices.iter().flatten() {
        hash.update(v.to_le_bytes());
    }
    for i in triangles.iter().flatten() {
        hash.update(i.to_le_bytes());
    }
    let artifact = navmesh::bake(navmesh::BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: "synthetic_arena".into(),
        world_sha256: world.source_sha256().into(),
        geometry_sha256: format!("{:x}", hash.finalize()),
        bounds: navmesh::Bounds {
            min: [0., -1., 0.],
            max: [30., 6., 30.],
        },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices,
        triangles,
    })
    .unwrap();
    let mesh = navmesh::NavMesh::load(artifact, world.source_sha256(), "synthetic_arena").unwrap();
    let mut engine = crate::bot_registry::identity_tests::engine("Synthetic Slab");
    let mut config = engine.bot_registry.config.take().unwrap();
    config.slots[0].position = replication::CharacterPosition {
        x: x as f32,
        y: SLAB_Y as f32,
        z: 15.,
    };
    let mut engine = engine
        .with_collision_world(world)
        .with_navmesh_world(mesh)
        .with_bot_population(config, 2)
        .unwrap();
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let id = *engine
        .bot_registry
        .bots
        .iter()
        .find(|(_, b)| b.target.runtime.unwrap().position.y > 2.)
        .unwrap()
        .0;
    // Settle the bot where it stands, as a finished move leaves it: grounded,
    // phase complete, its navigation not aborted.
    let body = bot_physics::Body::default();
    let terrain =
        bot_physics::WorldTerrain::new(engine.collision_world.as_ref(), engine.map_name()).unwrap();
    let settled = terrain
        .advance(
            terrain.start_motion([x, SLAB_Y + 0.5, 15.], body).unwrap(),
            [0.; 3],
            1.,
            body,
        )
        .unwrap();
    assert!(settled.grounded && (settled.position[1] - SLAB_Y).abs() < 0.1);
    assert!((settled.position[0] - x).abs() < 0.05, "{settled:?}");
    let runtime = engine
        .bot_registry
        .bots
        .get_mut(&id)
        .unwrap()
        .target
        .runtime
        .as_mut()
        .unwrap();
    runtime.physics_motion = Some(settled);
    runtime.position = replication::CharacterPosition {
        x: settled.position[0] as f32,
        y: settled.position[1] as f32,
        z: settled.position[2] as f32,
    };
    runtime.phase = BotPhase::Complete;
    runtime.navmesh_state.navigation_aborted = false;
    (engine, id)
}

fn recovery_move(goal: [f64; 3]) -> development_bot_movement::BotMove {
    development_bot_movement::BotMove {
        mode: development_bot_movement::BotMoveMode::Navigation,
        destination: position(goal),
        duration_ms: 5000,
        update_ms: 250,
        yaw: None,
        sprint: false,
    }
}

#[test]
fn a_finished_move_resting_in_the_eroded_band_is_an_offmesh_stop() {
    let (engine, id) = slab_engine(BAND_X);
    let mesh = engine.navmesh_world.as_ref().unwrap();
    let target = &engine.bot_registry.bots[&id].target;
    let here = target.runtime.unwrap().physical_position();
    // No route can start here: the start does not project onto the mesh.
    assert!(mesh.component_id(here).is_err());
    assert_eq!(
        mesh.route(here, [15., SLAB_Y, 15.], 1.).map(|r| r.status),
        Ok(navmesh::Status::InvalidStart)
    );
    assert!(target.supported_offmesh_stop(mesh));
}

#[test]
fn a_bot_on_the_mesh_or_still_moving_is_not_an_offmesh_stop() {
    let (engine, id) = slab_engine(15.);
    let mesh = engine.navmesh_world.clone().unwrap();
    assert!(
        !engine.bot_registry.bots[&id]
            .target
            .supported_offmesh_stop(&mesh)
    );

    let (mut engine, id) = slab_engine(BAND_X);
    let mesh = engine.navmesh_world.clone().unwrap();
    let runtime = engine
        .bot_registry
        .bots
        .get_mut(&id)
        .unwrap()
        .target
        .runtime
        .as_mut()
        .unwrap();
    // Mid-route a body may brush the band; that is not a stop.
    runtime.phase = BotPhase::Relocating;
    assert!(
        !engine.bot_registry.bots[&id]
            .target
            .supported_offmesh_stop(&mesh)
    );
    // Once its navigation stops early, it is.
    let runtime = engine
        .bot_registry
        .bots
        .get_mut(&id)
        .unwrap()
        .target
        .runtime
        .as_mut()
        .unwrap();
    runtime.navmesh_state.navigation_aborted = true;
    assert!(
        engine.bot_registry.bots[&id]
            .target
            .supported_offmesh_stop(&mesh)
    );
}

#[test]
fn the_nearest_floor_mesh_point_is_on_the_same_floor_and_close() {
    let (engine, id) = slab_engine(BAND_X);
    let mesh = engine.navmesh_world.as_ref().unwrap();
    let here = engine.bot_registry.bots[&id]
        .target
        .runtime
        .unwrap()
        .physical_position();
    let policy = ObjectivePolicy::bundled();
    let nearest = nearest_floor_mesh_point(mesh, here, 2. * policy.recovery_m).unwrap();
    // On the slab, not the open ground 3 m below whose mesh is as close.
    assert!((nearest[1] - SLAB_Y).abs() < 0.2, "{nearest:?}");
    let reach = (nearest[0] - here[0]).hypot(nearest[2] - here[2]);
    assert!(reach < 0.5, "{nearest:?} from {here:?}");
    assert!(
        nearest[0] > here[0],
        "inward, away from the edge: {nearest:?}"
    );
    assert!(mesh.component_id(nearest).is_ok());
}

#[test]
fn a_finished_move_in_the_eroded_band_walks_back_onto_the_mesh() {
    let (mut engine, id) = slab_engine(BAND_X);
    let mesh = engine.navmesh_world.clone().unwrap();
    let here = engine.bot_registry.bots[&id]
        .target
        .runtime
        .unwrap()
        .physical_position();
    let policy = ObjectivePolicy::bundled();
    let nearest = nearest_floor_mesh_point(&mesh, here, 2. * policy.recovery_m).unwrap();
    let map = engine.map_name().to_string();
    let world = engine.collision_world.clone().unwrap();
    let actors = crate::actor_collision::ActorGrid::default();
    let target = &mut engine.bot_registry.bots.get_mut(&id).unwrap().target;
    // Before, recovery demanded an aborted navigation; a bot whose last move
    // finished in the band retried routes that could never start.
    target
        .begin_registry_ground_recovery(
            &world,
            mesh.clone(),
            &map,
            recovery_move(nearest),
            Duration::ZERO,
            2. * policy.recovery_m,
            policy.drop_entry_buffer_m,
            crate::actor_collision::Query {
                grid: &actors,
                actor: id,
            },
            policy.drop_traffic_query_budget,
        )
        .expect("a supported stop beside the mesh walks back onto it");
    assert!(target.registry_ground_recovery());
    let end = point(target.registry_move_destination().unwrap());
    assert!(mesh.component_id(end).is_ok(), "{end:?}");
}

#[test]
fn ground_recovery_still_refuses_a_bot_on_the_mesh() {
    let (mut engine, id) = slab_engine(15.);
    let mesh = engine.navmesh_world.clone().unwrap();
    let map = engine.map_name().to_string();
    let world = engine.collision_world.clone().unwrap();
    let policy = ObjectivePolicy::bundled();
    let actors = crate::actor_collision::ActorGrid::default();
    let target = &mut engine.bot_registry.bots.get_mut(&id).unwrap().target;
    assert_eq!(
        target.begin_registry_ground_recovery(
            &world,
            mesh,
            &map,
            recovery_move([16., SLAB_Y, 15.]),
            Duration::ZERO,
            2. * policy.recovery_m,
            policy.drop_entry_buffer_m,
            crate::actor_collision::Query {
                grid: &actors,
                actor: id,
            },
            policy.drop_traffic_query_budget,
        ),
        Err("bot_ground_recovery_requires_supported_offmesh_stop")
    );
}
