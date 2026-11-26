// SYNTHETIC: every world here is authored in code; no game catalogs or captures.
use super::*;
use serde_json::json;

const STATIC_WORLD: u32 = 9;
const VEHICLE_ONLY: u32 = 11;

/// Rows where the projectile and blast queries collide with the static world
/// layer only, as retail's do among the static layers.
fn rows() -> [u32; 32] {
    let mut rows = [0; 32];
    rows[PROJECTILE_QUERY_LAYER] = 1 | 1 << STATIC_WORLD;
    rows[BLAST_QUERY_LAYER] = 1 | 1 << STATIC_WORLD;
    rows
}

/// A floor, then a sheet across it at x = 10 on `layer` (None: no metadata).
fn sheet(layer: Option<u32>) -> CollisionWorld {
    let mut sheet = json!({"id":"sheet","vertices_xyz":[[10.,0.,0.],[10.,0.,10.],[10.,4.,10.],[10.,4.,0.]],
        "triangles":[[0,1,2],[0,2,3]]});
    if let Some(layer) = layer {
        sheet["collision"] = json!({"filter_info":layer,"ray_child_filter_info":0,"convex_radius":0.,
            "provenance":"synthetic","child_filter_policy":"uniform"});
    }
    CollisionWorld::from_bytes(
        json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":["Not a game map"],"physics_regions":[
                    {"min":[-1.,-2.,-1.],"max":[21.,6.,11.],"ground_verified":true,
                     "obstacles_verified":true,"provenance":["synthetic sheet"]}]},
            "instances":[sheet]
        })
        .to_string()
        .as_bytes(),
        CollisionLimits::default(),
    )
    .unwrap()
}

const FROM: [f64; 3] = [5., 1.5, 5.];
const TO: [f64; 3] = [15., 1.5, 5.];

fn sight(world: &CollisionWorld) -> bool {
    crate::bot_perception::clear_view(
        crate::bot_firearms::SightGeometry {
            world: Some(world),
            structures: None,
            map: "synthetic",
            prefetch: None,
        },
        FROM,
        TO,
        &mut 4,
        &mut 10_000,
    )
    .unwrap()
}

fn shot(world: &CollisionWorld) -> Result<(), &'static str> {
    crate::bot_firearms::visible(
        crate::bot_firearms::SightGeometry {
            world: Some(world),
            structures: None,
            map: "synthetic",
            prefetch: None,
        },
        FROM,
        TO,
        1,
        2,
        &[],
        &mut 4,
        &mut 10_000,
    )
}

fn blast_samples(world: &CollisionWorld) -> u8 {
    let at = |p: [f64; 3]| crate::replication::CharacterPosition {
        x: p[0] as f32,
        y: 0.,
        z: p[2] as f32,
    };
    match crate::blast_visibility::query_blast_visibility(
        Some(world),
        "synthetic",
        at(FROM),
        at(TO),
    ) {
        crate::blast_visibility::BlastVisibility::Queried {
            visible_samples, ..
        } => visible_samples,
        _ => panic!("blast query not run"),
    }
}

#[test]
fn rays_cross_layers_their_retail_row_passes_through() {
    let vehicle_only = sheet(Some(VEHICLE_ONLY)).with_filter_rows(Some(rows()));
    assert!(sight(&vehicle_only));
    assert_eq!(shot(&vehicle_only), Ok(()));
    assert_eq!(blast_samples(&vehicle_only), 3);
    let mut budget = usize::MAX;
    let row = vehicle_only.query_row(PROJECTILE_QUERY_LAYER);
    assert!(
        vehicle_only
            .raycast_row_bounded(FROM, TO, row, &mut budget)
            .unwrap()
            .nearest
            .is_none()
    );
    // The plain query still reports the sheet.
    assert!(
        vehicle_only
            .raycast_segment(FROM, TO)
            .unwrap()
            .nearest
            .is_some()
    );
}

#[test]
fn static_world_unlabelled_or_rowless_sheets_still_block() {
    for world in [
        sheet(Some(STATIC_WORLD)).with_filter_rows(Some(rows())),
        sheet(None).with_filter_rows(Some(rows())),
        // Without rows every triangle blocks, as before.
        sheet(Some(VEHICLE_ONLY)),
    ] {
        assert!(!sight(&world));
        assert_eq!(shot(&world), Err("world_blocked"));
        assert_eq!(blast_samples(&world), 0);
    }
}

/// A mode's placed structures (bunkers, field hospitals) are not part of the
/// static map. Bot sight and shots must still stop at them, filtered by the
/// same projectile row; a structure without a filter binding is solid.
#[test]
fn placed_structures_block_bot_sight_and_shots_on_the_projectile_row() {
    use crate::bot_firearms::Structures;
    use crate::domination_structural_world::SequenceCollisionState;
    // The static sheet lets projectiles through, so only the structure decides.
    let world = sheet(Some(VEHICLE_ONLY)).with_filter_rows(Some(rows()));
    let (from, to) = ([5., 3., 5.], [15., 0.5, 5.]);
    let check = |structures: Option<Structures<'_>>| {
        let sight = crate::bot_perception::clear_view(
            crate::bot_firearms::SightGeometry {
                world: Some(&world),
                structures,
                map: "synthetic",
                prefetch: None,
            },
            from,
            to,
            &mut 4,
            &mut 10_000,
        );
        let shot = crate::bot_firearms::visible(
            crate::bot_firearms::SightGeometry {
                world: Some(&world),
                structures,
                map: "synthetic",
                prefetch: None,
            },
            from,
            to,
            1,
            2,
            &[],
            &mut 4,
            &mut 10_000,
        );
        (sight, shot)
    };
    assert_eq!(check(None), (Ok(true), Ok(())));
    for (tag, blocked) in [
        (Some(STATIC_WORLD as u8), true),
        (None, true),
        (Some(VEHICLE_ONLY as u8), false),
    ] {
        let mut sequences = SequenceCollisionState::default();
        // A roof slab the sight line descends through at x = 10.
        sequences.synthetic_structure_roof(7, [10., 1.75, 5.], 3., tag);
        let structures = Some(Structures {
            sequences: &sequences,
            row: rows()[PROJECTILE_QUERY_LAYER],
        });
        let expected = if blocked {
            (Ok(false), Err("structure_blocked"))
        } else {
            (Ok(true), Ok(()))
        };
        assert_eq!(check(structures), expected, "{tag:?}");
        // Elsewhere on the map the structure hides nothing.
        let mut away = SequenceCollisionState::default();
        away.synthetic_structure_roof(7, [10., 1.75, 50.], 3., tag);
        let structures = Some(Structures {
            sequences: &away,
            row: rows()[PROJECTILE_QUERY_LAYER],
        });
        assert_eq!(check(structures), (Ok(true), Ok(())), "{tag:?}");
    }
}

#[test]
fn filter_rows_load_from_a_list_or_a_configuration() {
    let directory = std::env::temp_dir().join(format!("mag-filter-rows-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let list = directory.join("rows.json");
    std::fs::write(&list, serde_json::to_vec(&rows()).unwrap()).unwrap();
    assert_eq!(CollisionWorld::load_filter_rows(&list).unwrap(), rows());
    let nested = directory.join("config.json");
    std::fs::write(&nested, json!({"setup":{"filter_rows":rows()}}).to_string()).unwrap();
    assert_eq!(CollisionWorld::load_filter_rows(&nested).unwrap(), rows());
    let short = directory.join("short.json");
    std::fs::write(&short, json!({"filter_rows":[1,2,3]}).to_string()).unwrap();
    assert!(CollisionWorld::load_filter_rows(&short).is_err());
    std::fs::remove_dir_all(&directory).unwrap();
}
