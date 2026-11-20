use super::*;

/// Adobe's D65 matrix for a Sony ILCE-7RM3, and a different one for A.
const D65_MATRIX: [f32; 9] = [
    0.6640, -0.1847, -0.0503, -0.5238, 1.3010, 0.2474, -0.0993, 0.1673, 0.6527,
];
const A_MATRIX: [f32; 9] = [
    0.9847, -0.3091, -0.0928, -0.8485, 1.6345, 0.2225, -0.0715, 0.0595, 0.7103,
];
/// The D65 matrix rawler ships for the Canon PowerShot G1, a CYGM camera
/// whose planes are ordered green, magenta, cyan, yellow.
const G1_MATRIX: [[f32; 3]; 4] = [
    [-0.5686, 1.03, 0.2223],
    [0.4725, -0.1157, 0.4383],
    [-0.6128, 1.0783, 0.6163],
    [-0.2688, 1.2093, 0.0604],
];

fn matrices(entries: &[(Illuminant, &[f32])]) -> HashMap<Illuminant, Vec<f32>> {
    entries
        .iter()
        .map(|(illuminant, matrix)| (*illuminant, matrix.to_vec()))
        .collect()
}

fn as_mat3(flat: &[f32]) -> Mat3 {
    std::array::from_fn(|row| std::array::from_fn(|column| flat[row * 3 + column]))
}

fn sensor_response(xyz_to_sensor: &[[f32; 3]; 4], xyz: [f32; 3]) -> [f32; 4] {
    std::array::from_fn(|channel| (0..3).map(|i| xyz_to_sensor[channel][i] * xyz[i]).sum())
}

fn folded(fold: &FourColorFold, sensor: [f32; 4]) -> [f32; 3] {
    std::array::from_fn(|row| {
        (0..4)
            .map(|channel| fold.fold[row][channel] * sensor[channel])
            .sum()
    })
}

#[test]
fn the_d65_matrix_is_preferred_over_any_other() {
    let both = matrices(&[(Illuminant::A, &A_MATRIX), (Illuminant::D65, &D65_MATRIX)]);
    assert_eq!(nearest_d65_matrix(&both, 3), Some(&D65_MATRIX[..]));
    assert_eq!(camera_matrix(&both), as_mat3(&D65_MATRIX));
    // Same temperature, different name: D65 still wins.
    let twins = matrices(&[
        (Illuminant::CloudyWeather, &A_MATRIX),
        (Illuminant::D65, &D65_MATRIX),
        (Illuminant::C, &A_MATRIX),
    ]);
    assert_eq!(nearest_d65_matrix(&twins, 3), Some(&D65_MATRIX[..]));
}

#[test]
fn without_d65_the_nearest_calibration_in_mired_is_used_as_it_is() {
    let warm = matrices(&[(Illuminant::A, &A_MATRIX), (Illuminant::D50, &D65_MATRIX)]);
    assert_eq!(nearest_d65_matrix(&warm, 3), Some(&D65_MATRIX[..]));
    // 7500 K is 20 mired from D65, 5500 K is 28: the cooler one is nearer.
    let split = matrices(&[(Illuminant::D55, &A_MATRIX), (Illuminant::D75, &D65_MATRIX)]);
    assert_eq!(nearest_d65_matrix(&split, 3), Some(&D65_MATRIX[..]));
    // An illuminant of unknown temperature is the last resort, but a resort.
    let unknown = matrices(&[(Illuminant::Unknown, &D65_MATRIX)]);
    assert_eq!(nearest_d65_matrix(&unknown, 3), Some(&D65_MATRIX[..]));
    let known = matrices(&[
        (Illuminant::Unknown, &A_MATRIX),
        (Illuminant::A, &D65_MATRIX),
    ]);
    assert_eq!(nearest_d65_matrix(&known, 3), Some(&D65_MATRIX[..]));
}

#[test]
fn matrices_of_the_wrong_shape_are_not_candidates() {
    let four_colour: Vec<f32> = G1_MATRIX.iter().flatten().copied().collect();
    let mixed = matrices(&[(Illuminant::D65, &four_colour), (Illuminant::A, &A_MATRIX)]);
    assert_eq!(nearest_d65_matrix(&mixed, 3), Some(&A_MATRIX[..]));
    assert_eq!(nearest_d65_matrix(&mixed, 4), Some(&four_colour[..]));
    assert_eq!(nearest_d65_matrix(&HashMap::new(), 3), None);
}

#[test]
fn an_unusable_matrix_falls_back_to_srgb_primaries() {
    let srgb = RgbSpace::Srgb.from_xyz();
    assert_eq!(camera_matrix(&HashMap::new()), srgb);
    let singular = [1.0, 2.0, 3.0, 2.0, 4.0, 6.0, 0.0, 1.0, 0.0];
    assert_eq!(
        camera_matrix(&matrices(&[(Illuminant::D65, &singular)])),
        srgb
    );
    let mut broken = D65_MATRIX;
    broken[4] = f32::NAN;
    assert_eq!(
        camera_matrix(&matrices(&[(Illuminant::D65, &broken)])),
        srgb
    );
    let zeros = [0.0; 9];
    assert_eq!(camera_matrix(&matrices(&[(Illuminant::D65, &zeros)])), srgb);
}

#[test]
fn gains_are_normalized_to_green_whatever_scale_the_file_uses() {
    let matrix = as_mat3(&D65_MATRIX);
    assert_eq!(as_shot_gains([2.0, 1.0, 1.5], &matrix), [2.0, 1.0, 1.5]);
    let scaled = as_shot_gains([2000.0, 1024.0, 1536.0], &matrix);
    assert!(
        (scaled[0] - 1.953_125).abs() < 1e-6 && scaled[1] == 1.0 && (scaled[2] - 1.5).abs() < 1e-6
    );
}

#[test]
fn missing_or_broken_coefficients_fall_back_to_the_daylight_balance() {
    let matrix = as_mat3(&D65_MATRIX);
    let daylight = as_shot_gains([f32::NAN; 3], &matrix);
    // The gains neutralize the camera's response to D65.
    let d65 = Chromaticity::D65.to_xyz().map(|v| v as f32);
    let response = mat3_apply(&matrix, d65);
    let balanced: [f32; 3] = std::array::from_fn(|c| response[c] * daylight[c]);
    assert_eq!(daylight[1], 1.0);
    assert!((balanced[0] - balanced[1]).abs() < 1e-5 && (balanced[2] - balanced[1]).abs() < 1e-5);
    assert!(daylight[0] > 2.0 && daylight[2] > 1.2, "{daylight:?}");

    for broken in [
        [0.0, 1.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, -2.0],
        [f32::INFINITY, 1.0, 1.0],
        [1.0, f32::NAN, 1.0],
        [1.0e9, 1.0, 1.0],
    ] {
        assert_eq!(as_shot_gains(broken, &matrix), daylight, "{broken:?}");
    }
    // With no usable matrix either, the balance is left alone.
    assert_eq!(as_shot_gains([0.0; 3], &[[0.0; 3]; 3]), [1.0; 3]);
    assert_eq!(as_shot_gains([f32::NAN; 3], &[[f32::NAN; 3]; 3]), [1.0; 3]);
}

#[test]
fn a_folded_four_colour_sensor_behaves_like_the_camera_it_claims_to_be() {
    let coefficients =
        sensor_response(&G1_MATRIX, [0.9642, 1.0, 0.8251]).map(|response| 1.0 / response);
    let fold = fold_four_colors(&G1_MATRIX, coefficients).expect("an invertible matrix");
    // For any colour, folding the sensor's four signals gives what the
    // virtual camera's matrix predicts from XYZ.
    for xyz in [
        [0.9505, 1.0, 1.089],
        [0.4, 0.2, 0.05],
        [0.1, 0.3, 0.6],
        [0.02, 0.02, 0.02],
    ] {
        let actual = folded(&fold, sensor_response(&G1_MATRIX, xyz));
        let predicted = mat3_apply(&fold.xyz_to_camera, xyz);
        for (a, p) in actual.iter().zip(predicted) {
            assert!((a - p).abs() < 2e-4, "{xyz:?}: {actual:?} vs {predicted:?}");
        }
    }
    // The matrix is the Bradford cone matrix up to scale, so scaling the
    // channels is a Bradford adaptation.
    let scale = fold.xyz_to_camera[0][0] / BRADFORD_CONES[0][0];
    for (row, cones) in fold.xyz_to_camera.iter().zip(BRADFORD_CONES) {
        for (value, cone) in row.iter().zip(cones) {
            assert!((value - cone * scale).abs() < 1e-6);
        }
    }
    // A saturated sensor peaks at exactly 1.0.
    let saturated = folded(&fold, [1.0; 4]);
    let peak = saturated.into_iter().fold(f32::MIN, f32::max);
    assert!((peak - 1.0).abs() < 1e-5, "{saturated:?}");
}

#[test]
fn folded_gains_neutralize_the_as_shot_white() {
    // The coefficients are those of a D50 white; after folding, the gains
    // must make that white neutral in the virtual camera.
    let white = [0.9642, 1.0, 0.8251];
    let coefficients = sensor_response(&G1_MATRIX, white).map(|response| 1.0 / response);
    let fold = fold_four_colors(&G1_MATRIX, coefficients).unwrap();
    let seen = folded(&fold, sensor_response(&G1_MATRIX, white));
    let balanced: [f32; 3] = std::array::from_fn(|c| seen[c] * fold.as_shot_gains[c]);
    assert_eq!(fold.as_shot_gains[1], 1.0);
    assert!(
        (balanced[0] - balanced[1]).abs() < 1e-4 && (balanced[2] - balanced[1]).abs() < 1e-4,
        "{balanced:?}"
    );

    // Without usable coefficients the fold balances for D65 instead.
    let fallback = fold_four_colors(&G1_MATRIX, [f32::NAN; 4]).unwrap();
    let d65 = folded(
        &fallback,
        sensor_response(&G1_MATRIX, [0.9505, 1.0, 1.0891]),
    );
    let balanced: [f32; 3] = std::array::from_fn(|c| d65[c] * fallback.as_shot_gains[c]);
    assert!(
        (balanced[0] - balanced[1]).abs() < 1e-3 && (balanced[2] - balanced[1]).abs() < 1e-3,
        "{balanced:?}"
    );
    assert_eq!(
        fallback.fold, fold.fold,
        "the fold itself does not depend on the balance"
    );
}

#[test]
fn a_sensor_whose_matrix_cannot_be_inverted_cannot_be_folded() {
    let flat = [[0.3, 0.5, 0.2]; 4];
    assert_eq!(fold_four_colors(&flat, [1.0; 4]), None);
    assert_eq!(fold_four_colors(&[[0.0; 3]; 4], [1.0; 4]), None);
    assert_eq!(fold_four_colors(&[[f32::NAN; 3]; 4], [1.0; 4]), None);
}

#[test]
fn the_monochrome_camera_renders_equal_channels_as_d65_grey() {
    let camera_to_xyz = mat3_invert(&monochrome_matrix()).unwrap();
    let xyz = mat3_apply(&camera_to_xyz, [0.5; 3]);
    let d65 = Chromaticity::D65.to_xyz().map(|v| v as f32 * 0.5);
    for (got, want) in xyz.iter().zip(d65) {
        assert!((got - want).abs() < 1e-4, "{xyz:?}");
    }
    // Its daylight balance is neutral, as it must be for a grey sensor.
    for gain in as_shot_gains([f32::NAN; 3], &monochrome_matrix()) {
        assert!((gain - 1.0).abs() < 1e-5, "{gain}");
    }
}
