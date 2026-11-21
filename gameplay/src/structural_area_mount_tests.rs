use super::*;
use crate::spawn_progression::tests::ground_vehicle_runtime::{
    area_vehicle_fixture, reinforced_domination_apc,
};

fn area() -> DamageWork {
    DamageWork::Area(AreaUpdate {
        objective_index: 120,
        objective_position: [100., 3., 100.],
        updated_state: ObjectiveState::Completed,
        world: 1,
        properties: BlastProperties {
            subtype: 0,
            full_damage_radius: 0.,
            half_damage_radius: 0.,
            damage_value: 0.,
            material_penetration: 0.,
            character_damage_over_time_radius: 5.,
            character_damage_over_time_value: 5.,
        },
    })
}

#[test]
fn hidden_ground_seat_does_not_block_the_structural_scan_cursor() {
    let mut engine = area_vehicle_fixture(Some(false));
    let mut state = AreaDamageState::default();
    let mut native = engine.owned_acquisition_characters();
    engine
        .stage_domination_structural_damage(
            &mut state,
            &mut native,
            Duration::from_millis(100),
            &[area()],
        )
        .unwrap();
    assert_eq!(state.cursors[&120], 16);
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .life
            .damage_health()
            .unwrap()
            .1,
        400
    );
    engine
        .stage_domination_structural_damage(
            &mut state,
            &mut native,
            Duration::from_millis(150),
            &[area()],
        )
        .unwrap();
    assert_eq!(state.cursors[&120], 32);
}

#[test]
fn exposed_ground_occupant_uses_current_vehicle_root_not_cached_on_foot_position() {
    let mut engine = area_vehicle_fixture(Some(true));
    engine.item_catalog = crate::human_kill_reward_tests::combat_items_for_ground_test(0.);
    let actor = engine.active.as_mut().unwrap();
    let mut loadout = actor
        .reservation
        .character
        .loadouts
        .as_ref()
        .unwrap()
        .loadout(&engine.item_catalog, 0)
        .unwrap();
    loadout.inventories[0].slots[12] = Some(3);
    actor.current_inventory_handle = Some(loadout.handle);
    actor.spawned_loadout = Some(loadout);
    let mut state = AreaDamageState::default();
    let mut native = engine.owned_acquisition_characters();
    engine
        .stage_domination_structural_damage(
            &mut state,
            &mut native,
            Duration::from_millis(100),
            &[area()],
        )
        .unwrap();
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .life
            .damage_health()
            .unwrap()
            .1
            < 400
    );
    assert_eq!(state.cursors[&120], 16);
}

#[test]
fn missing_or_invalid_ground_seat_visibility_stays_unqualified() {
    let mut engine = area_vehicle_fixture(None);
    let mut state = AreaDamageState::default();
    let mut native = engine.owned_acquisition_characters();
    assert!(
        engine
            .stage_domination_structural_damage(
                &mut state,
                &mut native,
                Duration::from_millis(100),
                &[area()]
            )
            .is_err()
    );
    let mut engine = area_vehicle_fixture(Some(false));
    engine
        .active
        .as_mut()
        .unwrap()
        .ground_seat
        .as_mut()
        .unwrap()
        .seat = 9;
    let mut state = AreaDamageState::default();
    let mut native = engine.owned_acquisition_characters();
    assert!(
        engine
            .stage_domination_structural_damage(
                &mut state,
                &mut native,
                Duration::from_millis(100),
                &[area()]
            )
            .is_err()
    );
}

#[test]
fn domination_apc_reinforcement_preserves_hidden_passenger_during_due_area_scan() {
    let mut engine = reinforced_domination_apc();
    let actor = engine.active.as_ref().unwrap();
    let seat = actor
        .ground_seat
        .expect("APC reinforcement must board a passenger");
    assert_eq!(seat.vehicle_index, 0);
    assert_eq!(seat.seat, 9);
    let health = actor
        .life
        .damage_health()
        .expect("spawned passenger must be alive");
    let mut state = AreaDamageState::default();
    let mut native = engine.owned_acquisition_characters();
    engine
        .stage_domination_structural_damage(
            &mut state,
            &mut native,
            Duration::from_millis(150),
            &[area()],
        )
        .unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.ground_seat, Some(seat));
    assert_eq!(actor.life.damage_health(), Some(health));
    assert_eq!(state.cursors[&120], 16);
}

#[test]
fn ground_turret_hidden_gunner_keeps_health_and_advances_structural_scan() {
    use crate::{
        ground_turrets::tests::{equip_synthetic_ground_turret, opt_in},
        spawn_progression::tests::ground_vehicle_runtime::domination_ground_driver::{
            driver_engine, ready_on_foot,
        },
    };
    let mut engine = driver_engine(8);
    opt_in(&mut engine, 8);
    ready_on_foot(&mut engine, 0, 10);
    equip_synthetic_ground_turret(&mut engine, 8);
    let mut state = AreaDamageState::default();
    let mut native = engine.owned_acquisition_characters();
    engine
        .stage_domination_structural_damage(
            &mut state,
            &mut native,
            Duration::from_millis(100),
            &[area()],
        )
        .unwrap();
    assert_eq!(state.cursors[&120], 16);
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.life.damage_health().unwrap().1, 400);
    assert_eq!(actor.ground_seat.unwrap().seat, 1);
}
