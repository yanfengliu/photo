//! Contracts of white balance for camera and raster sources (DESIGN §5.3).

use photo_color::camera::{
    camera_as_shot_temp_tint, camera_gains, camera_gains_for_temp_tint, camera_to_working,
    pick_camera_white_balance, reconstruct_highlights,
};
use photo_color::relative_wb::{
    RASTER_TEMP_LIMIT, adapt, pick_raster_white_balance, relative_wb_gains, relative_wb_matrix,
};
use photo_color::temperature::temp_tint_to_xy;
use photo_core::colorimetry::{Chromaticity, IDENTITY, Mat3, RgbSpace, mat3_apply, rgb_to_rgb};
use photo_core::develop::{WbPreset, WhiteBalance};
use photo_core::image::CameraColor;

/// A realistic D65 `ColorMatrix` (XYZ to camera) of a full-frame mirrorless
/// camera, with a daylight-ish as-shot balance.
fn camera() -> CameraColor {
    CameraColor {
        xyz_to_camera: [
            [0.6640, -0.1847, -0.0503],
            [-0.5238, 1.3010, 0.2474],
            [-0.0993, 0.1673, 0.6527],
        ],
        as_shot_gains: [2.31, 1.0, 1.58],
        baseline_exposure: 0.0,
    }
}

fn camera_response(camera: &CameraColor, xy: Chromaticity) -> [f32; 3] {
    let xyz = xy.to_xyz().map(|v| v as f32);
    mat3_apply(&camera.xyz_to_camera, xyz)
}

fn assert_neutral(rgb: [f32; 3], tolerance: f32) {
    let mean = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
    for channel in rgb {
        assert!(
            (channel / mean - 1.0).abs() < tolerance,
            "{rgb:?} is not neutral"
        );
    }
}

fn custom(temp: f32, tint: f32) -> WhiteBalance {
    WhiteBalance {
        preset: WbPreset::Custom,
        temp,
        tint,
    }
}

#[test]
fn the_camera_matrix_maps_balanced_white_to_working_white() {
    let matrix = camera_to_working(&camera());
    for level in [1.0, 0.18, 4.0] {
        let out = mat3_apply(&matrix, [level; 3]);
        for channel in out {
            assert!((channel - level).abs() < 2e-6 * level.max(1.0), "{out:?}");
        }
    }
}

#[test]
fn gains_for_an_illuminant_neutralise_the_cameras_response_to_it() {
    let camera = camera();
    for (kelvin, tint) in [
        (2850.0, 0.0),
        (4000.0, 20.0),
        (5500.0, 10.0),
        (6500.0, -15.0),
        (9000.0, 5.0),
    ] {
        let gains = camera_gains_for_temp_tint(&camera, kelvin, tint);
        assert_eq!(gains[1], 1.0, "green is the reference channel");
        let response =
            camera_response(&camera, temp_tint_to_xy(f64::from(kelvin), f64::from(tint)));
        assert_neutral(
            [
                response[0] * gains[0],
                response[1] * gains[1],
                response[2] * gains[2],
            ],
            1e-4,
        );
    }
}

#[test]
fn as_shot_uses_the_files_gains_and_ignores_the_numbers() {
    let camera = camera();
    let wb = WhiteBalance {
        preset: WbPreset::AsShot,
        temp: 2500.0,
        tint: 90.0,
    };
    assert_eq!(camera_gains(&camera, &wb), camera.as_shot_gains);
}

#[test]
fn as_shot_gains_round_trip_through_temperature_and_tint() {
    for as_shot_gains in [
        [2.31, 1.0, 1.58],
        [1.45, 1.0, 2.60],
        [2.80, 1.0, 1.25],
        [1.95, 1.0, 1.90],
    ] {
        let camera = CameraColor {
            as_shot_gains,
            ..camera()
        };
        let (temp, tint) = camera_as_shot_temp_tint(&camera);
        assert!((1667.0..=50_000.0).contains(&temp), "{temp}");
        let gains = camera_gains(&camera, &custom(temp, tint));
        for (back, original) in gains.iter().zip(as_shot_gains) {
            assert!(
                (back / original - 1.0).abs() < 3e-3,
                "{gains:?} vs {as_shot_gains:?}"
            );
        }
    }
}

#[test]
fn a_higher_kelvin_setting_renders_warmer() {
    // Telling the developer the light was bluer (higher kelvin) makes it
    // compensate toward red.
    let camera = camera();
    let warm = camera_gains(&camera, &custom(8000.0, 0.0));
    let cool = camera_gains(&camera, &custom(3500.0, 0.0));
    assert!(warm[0] / warm[2] > cool[0] / cool[2]);
    // Positive tint compensates a greener light with magenta: red and blue
    // rise relative to green.
    let magenta = camera_gains(&camera, &custom(5500.0, 60.0));
    let green = camera_gains(&camera, &custom(5500.0, -60.0));
    assert!(magenta[0] > green[0] && magenta[2] > green[2]);
}

#[test]
fn unusable_numbers_fall_back_to_the_as_shot_gains() {
    let camera = camera();
    assert_eq!(
        camera_gains(&camera, &custom(f32::NAN, 0.0)),
        camera.as_shot_gains
    );
    let singular = CameraColor {
        xyz_to_camera: [[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 1.0, 0.0]],
        ..camera
    };
    assert_eq!(
        camera_gains(&singular, &custom(5000.0, 0.0)),
        singular.as_shot_gains
    );
    assert_eq!(camera_to_working(&singular), IDENTITY);
    let (temp, tint) = camera_as_shot_temp_tint(&singular);
    assert!(temp.is_finite() && tint.is_finite());
}

#[test]
fn highlight_reconstruction_leaves_unclipped_pixels_alone() {
    let gains = [2.3, 1.0, 1.6];
    for raw in [[0.1, 0.3, 0.2], [0.0, 0.0, 0.0], [0.5, 0.89, 0.4]] {
        let balanced = [raw[0] * gains[0], raw[1] * gains[1], raw[2] * gains[2]];
        assert_eq!(reconstruct_highlights(raw, balanced), balanced);
    }
}

#[test]
fn a_fully_clipped_pixel_becomes_neutral_and_keeps_its_headroom() {
    let gains = [2.3, 1.0, 1.6];
    let out = reconstruct_highlights([1.0; 3], gains);
    assert_eq!(
        out, [2.3; 3],
        "blown highlights are neutral at the brightest channel, above 1.0"
    );
}

#[test]
fn highlight_reconstruction_blends_in_smoothly_and_monotonically() {
    let gains = [2.3f32, 1.0, 1.6];
    let mut previous = [0.0f32; 3];
    for step in 0..=400 {
        // A grey ramp through the clipping point: green clips first.
        let level = step as f32 / 400.0 * 1.4;
        let raw = [
            (level * 0.45).min(1.0),
            level.min(1.0),
            (level * 0.65).min(1.0),
        ];
        let balanced = [raw[0] * gains[0], raw[1] * gains[1], raw[2] * gains[2]];
        let out = reconstruct_highlights(raw, balanced);
        for channel in 0..3 {
            assert!(
                out[channel] >= previous[channel] - 1e-6,
                "channel {channel} dipped at {level}"
            );
            assert!(
                out[channel] - previous[channel] < 0.05,
                "channel {channel} jumped at {level}"
            );
            assert!(
                out[channel] >= balanced[channel],
                "reconstruction only raises channels"
            );
        }
        previous = out;
    }
}

#[test]
fn the_camera_picker_neutralises_its_sample() {
    let camera = camera();
    let matrix = camera_to_working(&camera);
    for (kelvin, tint) in [
        (3200.0, 8.0),
        (4500.0, 12.0),
        (5600.0, -20.0),
        (7200.0, 30.0),
    ] {
        // What the sensor records for a grey card under that light.
        let response = camera_response(&camera, temp_tint_to_xy(kelvin, tint));
        let sample = response.map(|v| v * 0.4);
        let wb = pick_camera_white_balance(&camera, sample).expect("a usable sample");
        assert_eq!(wb.preset, WbPreset::Custom);
        assert!(
            (f64::from(wb.temp) / kelvin - 1.0).abs() < 5e-3,
            "{} vs {kelvin}",
            wb.temp
        );
        assert!(
            (f64::from(wb.tint) - tint).abs() < 0.5,
            "{} vs {tint}",
            wb.tint
        );
        let gains = camera_gains(&camera, &wb);
        let balanced = [
            sample[0] * gains[0],
            sample[1] * gains[1],
            sample[2] * gains[2],
        ];
        assert_neutral(mat3_apply(&matrix, balanced), 3e-3);
    }
}

#[test]
fn the_pickers_refuse_samples_they_cannot_use() {
    let camera = camera();
    for sample in [
        [0.0; 3],
        [0.2, -0.1, 0.3],
        [f32::NAN, 0.2, 0.2],
        [0.3, 0.0, 0.3],
    ] {
        assert_eq!(
            pick_camera_white_balance(&camera, sample),
            None,
            "{sample:?}"
        );
        assert_eq!(
            pick_raster_white_balance(RgbSpace::Srgb, sample),
            None,
            "{sample:?}"
        );
    }
}

#[test]
fn raster_white_balance_at_zero_is_exactly_the_identity() {
    assert_eq!(relative_wb_matrix(0.0, 0.0), IDENTITY);
    assert_eq!(relative_wb_gains(0.0, 0.0), [1.0; 3]);
    for rgb in [
        [0.25, 0.5, 0.75],
        [1.0e-6, 0.0, 16.0],
        [0.333, 0.333, 0.333],
    ] {
        let out = adapt(rgb, relative_wb_gains(0.0, 0.0));
        for (a, b) in out.iter().zip(rgb) {
            assert!((a - b).abs() <= 4e-6 * b.max(1.0), "{out:?} vs {rgb:?}");
        }
    }
}

#[test]
fn raster_white_balance_is_continuous_at_zero() {
    let near = relative_wb_matrix(0.02, -0.02);
    for (row, identity_row) in near.iter().zip(IDENTITY) {
        for (value, expected) in row.iter().zip(identity_row) {
            assert!((value - expected).abs() < 1e-3, "{near:?}");
        }
    }
}

#[test]
fn raster_temperature_warms_and_tint_goes_magenta() {
    let grey = [0.4f32; 3];
    let warm = mat3_apply(&relative_wb_matrix(40.0, 0.0), grey);
    assert!(warm[0] > warm[1] && warm[1] > warm[2], "{warm:?}");
    let cool = mat3_apply(&relative_wb_matrix(-40.0, 0.0), grey);
    assert!(cool[0] < cool[1] && cool[1] < cool[2], "{cool:?}");
    let magenta = mat3_apply(&relative_wb_matrix(0.0, 40.0), grey);
    assert!(
        magenta[1] < magenta[0] && magenta[1] < magenta[2],
        "{magenta:?}"
    );
    let green = mat3_apply(&relative_wb_matrix(0.0, -40.0), grey);
    assert!(green[1] > green[0] && green[1] > green[2], "{green:?}");
    // More slider, more shift.
    let warmer = mat3_apply(&relative_wb_matrix(80.0, 0.0), grey);
    assert!(warmer[0] / warmer[2] > warm[0] / warm[2]);
}

#[test]
fn the_matrix_and_the_cone_gains_are_the_same_transform() {
    for (temp, tint) in [(35.0, -20.0), (-100.0, 100.0), (100.0, -100.0)] {
        let matrix = relative_wb_matrix(temp, tint);
        let gains = relative_wb_gains(temp, tint);
        for rgb in [[0.2, 0.5, 0.7], [0.9, 0.1, 0.3], [0.5, 0.5, 0.5]] {
            let a = mat3_apply(&matrix, rgb).map(|v| v.max(0.0));
            let b = adapt(rgb, gains);
            for (x, y) in a.iter().zip(b) {
                assert!((x - y).abs() < 2e-5, "{a:?} vs {b:?}");
            }
        }
    }
}

#[test]
fn adaptation_never_returns_a_negative_channel() {
    for temp in [-100.0, -60.0, 60.0, 100.0, 400.0, -400.0] {
        for tint in [-100.0, 0.0, 100.0, 400.0] {
            let gains = relative_wb_gains(temp, tint);
            for rgb in [
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 1.0, 0.0],
                [0.0, 0.02, 1.0],
            ] {
                let out = adapt(rgb, gains);
                assert!(
                    out.iter().all(|v| v.is_finite() && *v >= 0.0),
                    "{temp}/{tint}: {out:?}"
                );
            }
        }
    }
}

#[test]
fn the_raster_picker_neutralises_its_sample() {
    for space in [RgbSpace::Srgb, RgbSpace::DisplayP3, RgbSpace::Rec2020] {
        let to_working: Mat3 = rgb_to_rgb(space, RgbSpace::Rec2020);
        for sample in [
            [0.50, 0.45, 0.38],
            [0.20, 0.22, 0.26],
            [0.61, 0.60, 0.52],
            [0.30, 0.33, 0.31],
        ] {
            let wb = pick_raster_white_balance(space, sample).expect("a usable sample");
            assert_eq!(wb.preset, WbPreset::Custom);
            assert!(wb.temp.abs() < RASTER_TEMP_LIMIT && wb.tint.abs() < RASTER_TEMP_LIMIT);
            let working = mat3_apply(&to_working, sample);
            assert_neutral(adapt(working, relative_wb_gains(wb.temp, wb.tint)), 2e-3);
        }
    }
}

#[test]
fn the_raster_picker_stays_inside_the_slider_range() {
    let wb = pick_raster_white_balance(RgbSpace::Srgb, [0.9, 0.2, 0.02]).expect("a usable sample");
    assert!(wb.temp.abs() <= RASTER_TEMP_LIMIT && wb.tint.abs() <= RASTER_TEMP_LIMIT);
    // A warm cast needs a cooling correction.
    assert!(wb.temp < 0.0);
}
