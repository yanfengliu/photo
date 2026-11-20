//! Camera RAW: the synthetic Bayer DNG must come back as camera-native,
//! un-white-balanced linear data that a renderer can turn into the scene.

mod common;

use common::{linear_pixel, mean};
use photo_core::colorimetry::{Mat3, RgbSpace, mat3_apply, mat3_invert, mat3_mul};
use photo_core::image::{CameraColor, SourceColor, SourceImage};
use photo_decode::{decode, read_metadata};
use photo_testkit::{self as kit, DngSpec, ExifSpec, LinearScene};

const W: u32 = 320;
const H: u32 = 240;

fn camera_color(image: &SourceImage) -> CameraColor {
    match image.color {
        SourceColor::Camera(color) => color,
        other => panic!("expected camera-native data, got {other:?}"),
    }
}

/// Camera-native RGB to linear sRGB the way a renderer does it: apply the
/// white-balance gains, then the inverse of the colour matrix scaled so that
/// D65 white is (1, 1, 1), then XYZ to sRGB.
fn camera_to_scene(color: CameraColor) -> impl Fn([f32; 3]) -> [f32; 3] {
    let camera = kit::DngCamera {
        xyz_to_camera: color.xyz_to_camera,
        ..kit::dng_camera()
    };
    let balanced_to_xyz = mat3_invert(&camera.xyz_to_balanced_camera()).expect("invertible matrix");
    let to_scene: Mat3 = mat3_mul(&RgbSpace::Srgb.from_xyz(), &balanced_to_xyz);
    let gains = color.as_shot_gains;
    move |native| mat3_apply(&to_scene, std::array::from_fn(|c| native[c] * gains[c]))
}

/// What the sensor recorded for a scene colour, before the mosaic: the
/// fixture's documented model, clipped at saturation.
fn scene_to_camera() -> impl Fn([f32; 3]) -> [f32; 3] {
    let camera = kit::dng_camera();
    let to_balanced = mat3_mul(&camera.xyz_to_balanced_camera(), &RgbSpace::Srgb.to_xyz());
    move |linear| {
        let balanced = mat3_apply(&to_balanced, linear);
        std::array::from_fn(|c| (balanced[c] * camera.as_shot_neutral[c]).clamp(0.0, 1.0))
    }
}

/// Mean absolute difference between the decoded image and the scene as the
/// sensor saw it, with the scene shifted by `(dx, dy)` pixels.
fn sensor_error(image: &SourceImage, scene: &LinearScene, dx: i64, dy: i64) -> f64 {
    let expose = scene_to_camera();
    let (mut total, mut count) = (0.0f64, 0u64);
    for y in 2..image.height - 2 {
        for x in 2..image.width - 2 {
            let expected =
                expose(scene.pixel((i64::from(x) + dx) as u32, (i64::from(y) + dy) as u32));
            for (got, want) in linear_pixel(image, x, y).iter().zip(expected) {
                total += f64::from((got - want).abs());
                count += 1;
            }
        }
    }
    total / count as f64
}

#[test]
fn the_dng_decodes_with_the_requested_dimensions_in_both_layouts() {
    let dir = tempfile::tempdir().unwrap();
    for embed_previews in [true, false] {
        let spec = DngSpec {
            embed_previews,
            ..DngSpec::default()
        };
        let path = kit::write_dng_with(dir.path(), &format!("d{embed_previews}.dng"), W, H, &spec);
        let decoded = decode(&path).unwrap();
        assert_eq!((decoded.image.width, decoded.image.height), (W, H));
        assert_eq!(decoded.image.pixels.len(), (W * H * 3) as usize);
        assert_eq!((decoded.metadata.width, decoded.metadata.height), (W, H));
        let quick = read_metadata(&path).unwrap();
        assert_eq!((quick.width, quick.height), (W, H));
    }
}

#[test]
fn odd_and_small_sizes_decode_too() {
    let dir = tempfile::tempdir().unwrap();
    for (width, height) in [(127u32, 95u32), (33, 64), (16, 16)] {
        let path = kit::write_dng(dir.path(), &format!("s{width}x{height}.dng"), width, height);
        let decoded = decode(&path).unwrap();
        assert_eq!((decoded.image.width, decoded.image.height), (width, height));
        let quick = read_metadata(&path).unwrap();
        assert_eq!((quick.width, quick.height), (width, height));
    }
}

#[test]
fn the_colour_description_comes_from_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let decoded = decode(kit::write_dng(dir.path(), "c.dng", W, H)).unwrap();
    let color = camera_color(&decoded.image);
    let camera = kit::dng_camera();
    assert!(decoded.image.color.is_camera());

    // The D65 matrix, not the illuminant A decoy that comes first in the file.
    for (row, expected) in color.xyz_to_camera.iter().zip(camera.xyz_to_camera) {
        for (got, want) in row.iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "{:?}", color.xyz_to_camera);
        }
    }
    let gains = camera.as_shot_gains();
    assert_eq!(color.as_shot_gains[1], 1.0);
    for (got, want) in color.as_shot_gains.iter().zip(gains) {
        assert!(
            (got - want).abs() < 1e-4,
            "{:?} vs {gains:?}",
            color.as_shot_gains
        );
    }
    assert!((color.baseline_exposure - camera.baseline_exposure).abs() < 1e-6);
}

#[test]
fn white_balanced_matrix_converted_colours_match_the_scene() {
    let dir = tempfile::tempdir().unwrap();
    let decoded = decode(kit::write_dng(dir.path(), "p.dng", W, H)).unwrap();
    let to_scene = camera_to_scene(camera_color(&decoded.image));
    for patch in kit::patches(W, H) {
        let native = mean(&decoded.image, patch.x, patch.y, patch.width, patch.height);
        let recovered = to_scene(native);
        for (got, want) in recovered.iter().zip(patch.linear_srgb) {
            assert!(
                (got - want).abs() < 1e-3,
                "{}: {recovered:?} vs {:?}",
                patch.name,
                patch.linear_srgb
            );
        }
    }
}

#[test]
fn the_data_is_not_white_balanced() {
    // A neutral grey must keep the camera's own channel ratios: the as-shot
    // neutral, far from equal channels.
    let dir = tempfile::tempdir().unwrap();
    let decoded = decode(kit::write_dng(dir.path(), "n.dng", W, H)).unwrap();
    let neutral = kit::dng_camera().as_shot_neutral;
    let (x, y, width, height) = kit::grey_ramp(W, H);
    // Two thirds along the ramp: bright but far from clipping.
    let native = mean(&decoded.image, x + width * 2 / 3, y + 2, 4, height - 4);
    assert!(native[1] > 0.2 && native[1] < 0.8, "{native:?}");
    assert!(
        (native[0] / native[1] - neutral[0]).abs() < 0.002,
        "{native:?} vs {neutral:?}"
    );
    assert!(
        (native[2] / native[1] - neutral[2]).abs() < 0.002,
        "{native:?} vs {neutral:?}"
    );
    // And multiplying by the reported gains is what makes it neutral.
    let gains = camera_color(&decoded.image).as_shot_gains;
    let balanced: [f32; 3] = std::array::from_fn(|c| native[c] * gains[c]);
    assert!(
        (balanced[0] - balanced[1]).abs() < 0.002 && (balanced[2] - balanced[1]).abs() < 0.002,
        "{balanced:?}"
    );
}

#[test]
fn black_and_white_levels_are_applied_and_the_sun_clips_at_one() {
    let dir = tempfile::tempdir().unwrap();
    let decoded = decode(kit::write_dng(dir.path(), "l.dng", W, H)).unwrap();
    // The sun: every channel saturated, stored as exactly 1.0.
    let (sun_x, sun_y) = ((0.76 * W as f32) as u32, (0.2 * H as f32) as u32);
    assert_eq!(decoded.image.pixel(sun_x, sun_y), [SourceImage::WHITE; 3]);
    // The black end of the grey ramp: an unsubtracted black level would
    // leave about 3 % here.
    let (x, y, _, height) = kit::grey_ramp(W, H);
    let dark = mean(&decoded.image, x + 3, y + 2, 3, height - 4);
    assert!(dark.iter().all(|&c| c < 0.004), "{dark:?}");
    // Nothing exceeds 1.0 and the image is not blank.
    let brightest = decoded.image.pixels.iter().copied().max().unwrap();
    let dimmest = decoded.image.pixels.iter().copied().min().unwrap();
    assert_eq!(brightest, SourceImage::WHITE);
    assert!(dimmest < 600, "darkest sample {dimmest}");
}

#[test]
fn the_decoded_image_is_aligned_with_the_scene_to_the_pixel() {
    let dir = tempfile::tempdir().unwrap();
    let decoded = decode(kit::write_dng(dir.path(), "a.dng", W, H)).unwrap();
    let scene = kit::scene_linear(W, H);
    let aligned = sensor_error(&decoded.image, &scene, 0, 0);
    assert!(aligned < 0.005, "mean error against the scene is {aligned}");
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let shifted = sensor_error(&decoded.image, &scene, dx, dy);
        assert!(
            shifted > aligned * 1.5,
            "a shift of ({dx}, {dy}) fits as well: {shifted} vs {aligned}"
        );
    }
}

#[test]
fn the_raw_orientation_is_applied_to_the_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let scene = kit::scene_linear(W, H);
    let upright = decode(kit::write_dng(dir.path(), "o1.dng", W, H)).unwrap();
    let reference = sensor_error(&upright.image, &scene, 0, 0);
    for orientation in 2..=8u8 {
        let spec = DngSpec {
            exif: ExifSpec {
                orientation,
                ..ExifSpec::sample()
            },
            embed_previews: orientation % 2 == 0,
        };
        let path = kit::write_dng_with(dir.path(), &format!("o{orientation}.dng"), W, H, &spec);
        let decoded = decode(&path).unwrap();
        assert_eq!(
            (decoded.image.width, decoded.image.height),
            (W, H),
            "orientation {orientation}"
        );
        let error = sensor_error(&decoded.image, &scene, 0, 0);
        assert!(
            error < reference * 1.5 + 1e-4,
            "orientation {orientation}: error {error} vs upright {reference}"
        );
        assert_eq!(camera_color(&decoded.image), camera_color(&upright.image));
    }
}

#[test]
fn decoding_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let path = kit::write_dng(dir.path(), "d.dng", 160, 120);
    let first = decode(&path).unwrap();
    let second = decode(&path).unwrap();
    assert_eq!(first, second);
}
