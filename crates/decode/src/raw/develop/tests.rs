//! Every sensor layout rawler can hand over, built in memory, since the
//! only raw file a test can synthesize on disk is a Bayer DNG.

mod layouts;

use std::collections::HashMap;

use photo_core::colorimetry::{Mat3, RgbSpace};
use rawler::cfa::{CFA, PlaneColor};
use rawler::decoders::Camera;
use rawler::imgop::xyz::Illuminant;
use rawler::imgop::{Dim2, Point, Rect};
use rawler::rawimage::{BlackLevel, WhiteLevel};

use super::*;

const BLACK: u16 = 256;
const WHITE: u16 = 4095;
const X_TRANS: &str = "GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG";
const D65_MATRIX: [f32; 9] = [
    0.6640, -0.1847, -0.0503, -0.5238, 1.3010, 0.2474, -0.0993, 0.1673, 0.6527,
];

/// The raw code for a linear value in 0..=1.
fn code(linear: f32) -> u16 {
    BLACK + (linear * f32::from(WHITE - BLACK)).round() as u16
}

/// What a linear value comes back as after the 12-bit code round trip.
fn expected(linear: f32) -> u16 {
    let stored = f32::from(code(linear) - BLACK) / f32::from(WHITE - BLACK);
    (stored * 65535.0).round() as u16
}

fn camera(pattern: &str, planes: &str) -> Camera {
    Camera {
        cfa: CFA::new(pattern),
        plane_color: PlaneColor::new(planes),
        color_matrix: HashMap::from([(Illuminant::D65, D65_MATRIX.to_vec())]),
        ..Camera::new()
    }
}

/// A raw image of single samples whose value at each site is decided by
/// `sample(row, column)`, in linear 0..=1.
fn mosaic(
    camera: Camera,
    width: usize,
    height: usize,
    gains: [f32; 4],
    sample: impl Fn(usize, usize) -> f32,
) -> RawImage {
    let data = (0..height)
        .flat_map(|row| (0..width).map(move |column| (row, column)))
        .map(|(row, column)| code(sample(row, column)))
        .collect();
    let photometric = if camera.cfa.is_valid() {
        RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera))
    } else {
        RawPhotometricInterpretation::BlackIsZero
    };
    RawImage::new_with_data(
        camera,
        RawImageData::Integer(data),
        width,
        height,
        1,
        gains,
        photometric,
        Some(BlackLevel::new(&[BLACK], 1, 1, 1)),
        Some(WhiteLevel::new(vec![u32::from(WHITE)])),
        false,
    )
}

fn develop_upright(raw: RawImage) -> SourceImage {
    develop(raw, Orientation::UPRIGHT, 0.0).expect("develops")
}

fn camera_color(image: &SourceImage) -> CameraColor {
    match image.color {
        SourceColor::Camera(color) => color,
        other => panic!("expected camera data, got {other:?}"),
    }
}

fn assert_close(actual: [u16; 3], wanted: [u16; 3], tolerance: u16, context: &str) {
    for (got, want) in actual.iter().zip(wanted) {
        assert!(
            got.abs_diff(want) <= tolerance,
            "{context}: {actual:?} vs {wanted:?}"
        );
    }
}

const GAINS: [f32; 4] = [2.2, 1.0, 1.6, f32::NAN];
const FLAT: [f32; 3] = [0.2, 0.5, 0.3];

#[test]
fn a_flat_bayer_field_comes_back_exactly_and_not_white_balanced() {
    for pattern in ["RGGB", "BGGR", "GRBG", "GBRG"] {
        let camera = camera(pattern, "RGB");
        let cfa = camera.cfa.clone();
        let raw = mosaic(camera, 24, 16, GAINS, |row, column| {
            FLAT[cfa.color_at(row, column)]
        });
        let image = develop_upright(raw);
        assert_eq!((image.width, image.height), (24, 16));
        let wanted = FLAT.map(expected);
        for y in 0..16 {
            for x in 0..24 {
                assert_close(
                    image.pixel(x, y),
                    wanted,
                    1,
                    &format!("{pattern} at {x},{y}"),
                );
            }
        }
        let color = camera_color(&image);
        assert_eq!(color.as_shot_gains, [2.2, 1.0, 1.6]);
        assert_eq!(
            color.xyz_to_camera,
            std::array::from_fn::<_, 3, _>(|r| std::array::from_fn(|c| D65_MATRIX[r * 3 + c]))
        );
    }
}

#[test]
fn every_measured_sample_survives_interpolation_untouched() {
    // A busy mosaic: whatever the interpolation makes of the two missing
    // channels, the measured one must come back as recorded.
    for pattern in ["RGGB", X_TRANS] {
        let camera = camera(pattern, "RGB");
        let cfa = camera.cfa.clone();
        let value = |row: usize, column: usize| ((row * 31 + column * 17) % 97) as f32 / 96.0;
        let image = develop_upright(mosaic(camera, 40, 30, GAINS, value));
        for y in 0..30usize {
            for x in 0..40usize {
                let channel = cfa.color_at(y, x);
                let got = image.pixel(x as u32, y as u32)[channel];
                assert!(
                    got.abs_diff(expected(value(y, x))) <= 1,
                    "{pattern} site {x},{y}: {got}"
                );
            }
        }
    }
}

#[test]
fn levels_map_black_to_zero_and_white_to_one_and_clamp_beyond() {
    let camera = camera("RGGB", "RGB");
    let mut raw = mosaic(camera, 16, 32, GAINS, |_, _| 0.5);
    let RawImageData::Integer(data) = &mut raw.data else {
        unreachable!()
    };
    // Four bands of eight rows: below black, black, white, above white.
    for (band, value) in [0u16, BLACK, WHITE, 65535].into_iter().enumerate() {
        data[band * 8 * 16..(band + 1) * 8 * 16].fill(value);
    }
    let image = develop_upright(raw);
    assert_eq!(image.pixel(8, 3), [0; 3]);
    assert_eq!(image.pixel(8, 11), [0; 3]);
    assert_eq!(image.pixel(8, 19), [65535; 3]);
    assert_eq!(image.pixel(8, 27), [65535; 3]);
}

#[test]
fn per_site_black_and_white_levels_are_honoured() {
    let camera = camera("RGGB", "RGB");
    let blacks = [100u16, 200, 300, 400];
    let whites = [1100u32, 2200, 3300, 4400];
    let mut raw = mosaic(camera, 16, 16, [1.0, 1.0, 1.0, f32::NAN], |_, _| 0.0);
    raw.blacklevel = BlackLevel::new(&blacks, 2, 2, 1);
    raw.whitelevel = WhiteLevel::new(whites.to_vec());
    let RawImageData::Integer(data) = &mut raw.data else {
        unreachable!()
    };
    // Every site sits a quarter of the way up its own range.
    for (index, sample) in data.iter_mut().enumerate() {
        let site = (index / 16 % 2) * 2 + index % 16 % 2;
        *sample = blacks[site] + ((whites[site] - u32::from(blacks[site])) / 4) as u16;
    }
    let image = develop_upright(raw);
    for (x, y) in [(3, 3), (8, 8), (11, 4)] {
        assert_close(image.pixel(x, y), [16384; 3], 1, "quarter scale");
    }
}

#[test]
fn floating_point_raw_data_is_scaled_like_integer_data() {
    let camera = camera("RGGB", "RGB");
    let cfa = camera.cfa.clone();
    let integer = mosaic(camera, 16, 16, GAINS, |row, column| {
        FLAT[cfa.color_at(row, column)]
    });
    let mut float = integer.clone();
    float.data = RawImageData::Float(integer.pixels_u16().iter().map(|&v| f32::from(v)).collect());
    assert_eq!(develop_upright(float), develop_upright(integer));
}

#[test]
fn active_area_crop_and_orientation_select_and_turn_the_picture() {
    // A 40 x 30 sensor whose active area starts at (5, 3) and is 32 x 24;
    // the crop keeps 28 x 20 from (2, 2) inside it. The left half of the
    // crop is one flat colour and the right half another. Everything
    // outside the active area is saturated garbage that must never show.
    let (left, right) = ([0.6f32, 0.2, 0.1], [0.1f32, 0.3, 0.7]);
    let camera = camera("RGGB", "RGB");
    let cfa = camera.cfa.clone();
    let scene = move |row: usize, column: usize| {
        let inside = (5..37).contains(&column) && (3..27).contains(&row);
        match (inside, column < 5 + 2 + 14) {
            (false, _) => 1.0,
            (true, true) => left[cfa.color_at(row, column)],
            (true, false) => right[cfa.color_at(row, column)],
        }
    };
    let build = || {
        let mut raw = mosaic(camera.clone(), 40, 30, GAINS, &scene);
        raw.active_area = Some(Rect::new(Point::new(5, 3), Dim2::new(32, 24)));
        raw.crop_area = Some(Rect::new(Point::new(7, 5), Dim2::new(28, 20)));
        raw
    };

    let upright = develop_upright(build());
    assert_eq!((upright.width, upright.height), (28, 20));
    assert_close(upright.pixel(4, 10), left.map(expected), 1, "left half");
    assert_close(upright.pixel(23, 10), right.map(expected), 1, "right half");
    // Right at the crop edge: no bleed from the garbage border.
    assert_close(
        upright.pixel(0, 0),
        left.map(expected),
        1,
        "top-left corner",
    );
    assert_close(
        upright.pixel(27, 19),
        right.map(expected),
        1,
        "bottom-right corner",
    );

    // Orientation 6: a quarter turn clockwise puts the stored left on top.
    let turned = develop(build(), Orientation::from_exif(6), 0.0).unwrap();
    assert_eq!((turned.width, turned.height), (20, 28));
    assert_close(
        turned.pixel(10, 4),
        left.map(expected),
        1,
        "top after the turn",
    );
    assert_close(
        turned.pixel(10, 23),
        right.map(expected),
        1,
        "bottom after the turn",
    );
    // Orientation 3: upside down.
    let flipped = develop(build(), Orientation::from_exif(3), 0.0).unwrap();
    assert_close(
        flipped.pixel(4, 10),
        right.map(expected),
        1,
        "left after the half turn",
    );
}

#[test]
fn missing_white_balance_falls_back_to_daylight_without_touching_the_data() {
    let build = |gains: [f32; 4]| {
        let camera = camera("RGGB", "RGB");
        let cfa = camera.cfa.clone();
        mosaic(camera, 16, 16, gains, move |row, column| {
            FLAT[cfa.color_at(row, column)]
        })
    };
    let with = develop_upright(build(GAINS));
    let without = develop_upright(build([f32::NAN; 4]));
    for (a, b) in with.pixels.iter().zip(&without.pixels) {
        assert!(a.abs_diff(*b) <= 1);
    }
    let daylight = camera_color(&without).as_shot_gains;
    assert_eq!(daylight[1], 1.0);
    assert!(daylight[0] > 2.0 && daylight[2] > 1.2, "{daylight:?}");
    let zeroed = develop_upright(build([0.0, 0.0, 0.0, 0.0]));
    assert_eq!(camera_color(&zeroed).as_shot_gains, daylight);
}

#[test]
fn the_baseline_exposure_is_passed_on_unless_it_is_nonsense() {
    let build = || mosaic(camera("RGGB", "RGB"), 16, 16, GAINS, |_, _| 0.5);
    let exposure = |stops: f32| {
        camera_color(&develop(build(), Orientation::UPRIGHT, stops).unwrap()).baseline_exposure
    };
    assert_eq!(exposure(0.25), 0.25);
    assert_eq!(exposure(-1.5), -1.5);
    assert_eq!(exposure(f32::NAN), 0.0);
    assert_eq!(exposure(f32::INFINITY), 0.0);
    assert_eq!(exposure(99.0), 0.0);
}

#[test]
fn a_missing_colour_matrix_is_replaced_not_fatal() {
    let mut camera = camera("RGGB", "RGB");
    camera.color_matrix.clear();
    let image = develop_upright(mosaic(camera, 16, 16, GAINS, |_, _| 0.5));
    let matrix: Mat3 = camera_color(&image).xyz_to_camera;
    assert_eq!(matrix, RgbSpace::Srgb.from_xyz());
}

#[test]
fn layouts_that_cannot_be_developed_are_typed_errors() {
    let unsupported = |raw: RawImage| {
        matches!(
            develop(raw, Orientation::UPRIGHT, 0.0),
            Err(DecodeError::Unsupported(_))
        )
    };
    let corrupt = |raw: RawImage| {
        matches!(
            develop(raw, Orientation::UPRIGHT, 0.0),
            Err(DecodeError::Corrupt(_))
        )
    };

    // Too small for rawler's Bayer interpolation to be safe.
    assert!(unsupported(mosaic(
        camera("RGGB", "RGB"),
        6,
        6,
        GAINS,
        |_, _| 0.5
    )));
    assert!(unsupported(mosaic(
        camera("RGGB", "RGB"),
        7,
        40,
        GAINS,
        |_, _| 0.5
    )));
    // Four filters but only three planes described.
    assert!(unsupported(mosaic(
        camera("GMYC", "RGB"),
        16,
        16,
        GAINS,
        |_, _| 0.5
    )));
    // Four filters and no four-row matrix.
    assert!(unsupported(mosaic(
        camera("GMYC", "GMCY"),
        16,
        16,
        GAINS,
        |_, _| 0.5
    )));
    // Two samples per pixel.
    let mut odd = mosaic(camera("RGGB", "RGB"), 16, 16, GAINS, |_, _| 0.5);
    odd.cpp = 2;
    odd.width = 8;
    assert!(unsupported(odd));

    // Fewer samples than the header claims.
    let mut short = mosaic(camera("RGGB", "RGB"), 16, 16, GAINS, |_, _| 0.5);
    short.height = 17;
    assert!(corrupt(short));
    // A white level at or below black.
    let mut flat = mosaic(camera("RGGB", "RGB"), 16, 16, GAINS, |_, _| 0.5);
    flat.whitelevel = WhiteLevel::new(vec![u32::from(BLACK)]);
    assert!(corrupt(flat));
    let mut none = mosaic(camera("RGGB", "RGB"), 16, 16, GAINS, |_, _| 0.5);
    none.whitelevel = WhiteLevel::new(Vec::new());
    assert!(corrupt(none));
    // An active area off the sensor.
    let mut lost = mosaic(camera("RGGB", "RGB"), 16, 16, GAINS, |_, _| 0.5);
    lost.active_area = Some(Rect::new(Point::new(16, 0), Dim2::new(8, 8)));
    assert!(corrupt(lost));
}
