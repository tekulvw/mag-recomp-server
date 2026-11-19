//! The existing native capture-hold exit handshake is part of the core loop.
use super::*;

fn expected_exit() -> String {
    protocol::encode_hex(
        &protocol::ObjectiveInteraction {
            target_object_id: 0x0c80_0000 | u32::from(FIRST_CAPTURE),
            kind: protocol::ObjectiveInteractionKind::ActivatingOnExit,
            actor_index: 0,
            trigger_index: 0,
        }
        .encode(),
    )
}

#[test]
fn completed_acquisition_hold_sends_exit_to_the_current_holder_once() {
    let mut e = capture_fixture();
    start_capture(&mut e, 0, 100);
    let result = e
        .tick_acquisition_objectives(Duration::from_millis(3200), 2)
        .unwrap();
    assert_eq!(
        result
            .iter()
            .filter(|m| m.payload_hex == expected_exit())
            .count(),
        1
    );
    assert!(
        e.tick_acquisition_objectives(Duration::from_millis(3300), 3)
            .unwrap()
            .iter()
            .all(|m| m.payload_hex != expected_exit())
    );
}

#[test]
fn range_cancelled_acquisition_hold_preserves_the_native_exit_handshake() {
    let mut e = capture_fixture();
    start_capture(&mut e, 0, 100);
    e.active.as_mut().unwrap().tracking.server_position(
        CharacterPosition {
            x: 130.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        250,
    );
    let result = e
        .tick_acquisition_objectives(Duration::from_millis(300), 2)
        .unwrap();
    assert_eq!(
        result
            .iter()
            .filter(|m| m.payload_hex == expected_exit())
            .count(),
        1
    );
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(FIRST_CAPTURE),
        Some(ObjectiveState::Deactivated)
    );
}
