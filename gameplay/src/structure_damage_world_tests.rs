use super::*;
use serde_json::json;

fn rule() -> Rule {
    serde_json::from_value(json!({"object_index":11,"trigger_index":0,"trigger_type":4,
        "start_active":true,"damage_threshold":60.,"position":[10.,2.,10.],
        "box":{"centre":[10.,2.,10.],"axes":[[0.,0.,1.],[0.,1.,0.],[-1.,0.,0.]],"half_extents":[1.,0.02,2.]}})).unwrap()
}
fn blast() -> InstantaneousBlast {
    InstantaneousBlast {
        origins: vec![[12., 2., 10.]],
        inner_radius: 1.,
        outer_radius: 4.,
        damage: 45.,
        material_penetration: 0.,
    }
}
#[test]
fn eligible_trigger_uses_box_filter_and_full_raw_damage_then_removes_only_on_acceptance() {
    let mut world = World::new(&[rule()]).unwrap();
    assert!(
        world
            .collect(&blast(), 0, Default::default())
            .unwrap()
            .is_empty()
    );
    let hits = world.collect(&blast(), 1 << 6, Default::default()).unwrap();
    assert_eq!(hits, [(11, 0)]);
    assert_eq!(
        world.apply(hits[0], blast().damage, true).unwrap(),
        StructureDamageOutcome::Accumulated
    );
    assert_eq!(
        world.apply(hits[0], blast().damage, true).unwrap(),
        StructureDamageOutcome::Destroyed
    );
    assert_eq!(
        world.collect(&blast(), 1 << 6, Default::default()).unwrap(),
        hits
    );
    assert_eq!(
        world.apply(hits[0], blast().damage, true).unwrap(),
        StructureDamageOutcome::Ignored
    );
    world.remove_phantom(hits[0]).unwrap();
    assert!(
        world
            .collect(&blast(), 1 << 6, Default::default())
            .unwrap()
            .is_empty()
    );
}
#[test]
fn inactive_owner_and_unqualified_boxes_never_receive_damage() {
    let mut world = World::new(&[rule()]).unwrap();
    assert_eq!(
        world.apply((11, 0), 45., false).unwrap(),
        StructureDamageOutcome::Ignored
    );
    assert_eq!(
        world.apply((11, 0), 45., true).unwrap(),
        StructureDamageOutcome::Accumulated
    );
    let mut invalid = rule();
    invalid.trigger_type = 3;
    assert!(World::new(&[invalid]).is_err());
    let mut invalid = rule();
    invalid.volume.axes[0] = [1., 0., 0.];
    assert!(World::new(&[invalid]).is_err());
    assert!(World::new(&[rule(), rule()]).is_err());
}
