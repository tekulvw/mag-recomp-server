use mag_gameplay::acquisition_volume_geometry::*;

#[test]
fn volume_box_uses_world_y_half_height_and_native_angle_factor() {
    let input = Transform {
        translation: [1., 2., 3.],
        rotation_degrees: [10., 20., 30.],
        scale: [4., 6., 8.],
    };
    let parameters = input.parameters().unwrap();
    assert_eq!(parameters.center, [1., 5., 3.]);
    assert_eq!(parameters.half_extents, [2., 3., 4.]);
    assert_eq!(parameters.margin.to_bits(), 0x3d4ccccd);
    for (degrees, radians) in input
        .rotation_degrees
        .into_iter()
        .zip(parameters.rotation_radians)
    {
        assert_eq!(
            radians.to_bits(),
            (degrees * f32::from_bits(0x3c8efa35)).to_bits()
        );
    }
    // Center is lifted on WORLD Y, not along the rotated local Y basis.
    let rotated = Transform {
        rotation_degrees: [0., 0., 90.],
        ..input
    }
    .parameters()
    .unwrap();
    assert_eq!(rotated.center, [1., 5., 3.]);
}

#[test]
fn scale_signs_zero_and_unsupported_numeric_results_are_not_silently_repaired() {
    let input = Transform {
        translation: [0.; 3],
        rotation_degrees: [0.; 3],
        scale: [-2., 0., 8.],
    };
    let p = input.parameters().unwrap();
    assert_eq!(p.half_extents, [-1., 0., 4.]); // No abs/clamp. Query support is separate.
    for invalid in [
        Transform {
            translation: [0., f32::INFINITY, 0.],
            ..input
        },
        Transform {
            rotation_degrees: [f32::NAN, 0., 0.],
            ..input
        },
        Transform {
            scale: [1., f32::INFINITY, 1.],
            ..input
        },
        Transform {
            translation: [0., f32::MAX, 0.],
            scale: [1., f32::MAX, 1.],
            ..input
        },
    ] {
        assert!(invalid.parameters().is_err());
    }
}

#[test]
fn raw_filter6_uses_current_layer_row_and_ignores_target_upper_bits() {
    let initial = [0u32, 6, 10, 21, 22, 23]
        .into_iter()
        .fold(0u32, |row, layer| row | (1 << layer));
    for layer in 0..32 {
        let allowed = [0, 6, 10, 21, 22, 23].contains(&layer);
        assert_eq!(filter6_allows(initial, layer), allowed);
        assert_eq!(filter6_allows(initial, 0xffffffe0 | layer), allowed);
    }
    assert!(!filter6_allows(0, 6)); // Current filter state is required, not presumed immutable.
    assert!(filter6_allows(1 << 31, u32::MAX));
}
