//! SYNTHETIC transforms and offsets; no map catalog values or captured expectations.
use mag_gameplay::{
    replication::{CharacterPosition, CharacterRespawn},
    vehicle_exit::{BunkerExitPolicy, BunkerTransform, ExitCandidate},
};
fn policy() -> BunkerExitPolicy {
    BunkerExitPolicy {
        transform: BunkerTransform {
            position: CharacterPosition {
                x: 20.,
                y: 10.,
                z: 30.,
            },
            rotation_xyzw: [0., 0., 0., 1.],
        },
        candidates: vec![
            ExitCandidate {
                ranking_offset: [-2., 0., 0.],
                position_offset: [-3., -1., 0.],
            },
            ExitCandidate {
                ranking_offset: [2., 0., 0.],
                position_offset: [3., -1., 0.],
            },
        ],
    }
}
#[test]
fn explicit_candidates_choose_camera_facing_without_map_defaults() {
    let policy = policy();
    let left = policy.exit(90).unwrap();
    let right = policy.exit(270).unwrap();
    assert_eq!((left.candidate_index, right.candidate_index), (0, 1));
    assert_eq!(
        (left.position.x, left.position.y, left.position.z),
        (17., 9., 30.)
    );
    assert_eq!(
        (right.position.x, right.position.y, right.position.z),
        (23., 9., 30.)
    );
    assert_eq!((left.packed_yaw, right.packed_yaw), (256, 768));
    let fallback = policy.exit(u32::MAX).unwrap();
    assert_eq!(fallback.candidate_index, 0);
    assert_eq!(fallback.packed_yaw, 0);
}
#[test]
fn explicit_transform_rotates_model_offsets_and_rejects_invalid_geometry() {
    let mut policy = policy();
    policy.transform.rotation_xyzw = [
        0.,
        std::f32::consts::FRAC_1_SQRT_2,
        0.,
        std::f32::consts::FRAC_1_SQRT_2,
    ];
    let exit = policy.exit(u32::MAX).unwrap();
    assert!((exit.position.x - 20.).abs() < 0.0001);
    assert!((exit.position.z - 33.).abs() < 0.0001);
    // The f32 quaternion approximation is truncated by the wire yaw quantizer.
    assert_eq!(exit.packed_yaw, 255);
    policy.transform.rotation_xyzw = [0.; 4];
    assert_eq!(policy.validate(), Err("invalid_bunker_exit_policy"));
    policy.transform.rotation_xyzw = [0., 0., 0., 1.];
    policy.candidates[0].ranking_offset = [0.; 3];
    assert!(policy.exit(0).is_err());
    policy.candidates.clear();
    assert!(policy.exit(0).is_err());
}
#[test]
fn supplied_respawn_position_is_preserved_and_invalid_position_rejected() {
    let point = CharacterPosition {
        x: 24.,
        y: 8.,
        z: 12.,
    };
    let respawn = CharacterRespawn::at_position(17, point);
    assert_eq!(
        (respawn.position.x, respawn.position.y, respawn.position.z),
        (24., 8., 12.)
    );
    assert!(respawn.encode().is_ok());
    let invalid = CharacterPosition {
        x: f32::NAN,
        ..point
    };
    assert!(CharacterRespawn::at_position(17, invalid).encode().is_err());
}
