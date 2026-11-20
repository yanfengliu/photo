//! Embedded previews: fast, small, upright, sRGB.

use photo_core::colorimetry::{RgbSpace, mat3_apply, rgb_to_rgb};
use photo_core::image::Rgba8Image;
use photo_decode::embedded_preview;
use photo_testkit::{self as kit, DngSpec, ExifSpec};

const W: u32 = 320;
const H: u32 = 240;

fn preview(path: &std::path::Path, max_edge: u32) -> Rgba8Image {
    embedded_preview(path, max_edge)
        .unwrap()
        .unwrap_or_else(|| panic!("{} has no preview", path.display()))
}

/// Mean absolute difference, in 8-bit code values, between a preview and
/// the sRGB scene resized to the same dimensions.
fn error_against_scene(preview: &Rgba8Image, upright_width: u32, upright_height: u32) -> f64 {
    let scene = kit::scene(upright_width, upright_height);
    let reference = image::imageops::thumbnail(&scene, preview.width, preview.height);
    let mut total = 0u64;
    for (got, want) in preview.pixels.chunks_exact(4).zip(reference.pixels()) {
        for (got, want) in got.iter().zip(want.0) {
            total += u64::from(got.abs_diff(want));
        }
    }
    total as f64 / f64::from(preview.width * preview.height * 3)
}

fn assert_well_formed(preview: &Rgba8Image) {
    assert_eq!(preview.space, RgbSpace::Srgb);
    assert_eq!(
        preview.pixels.len(),
        (preview.width * preview.height * 4) as usize
    );
    assert!(
        preview.pixels.chunks_exact(4).all(|pixel| pixel[3] == 255),
        "alpha must be opaque"
    );
}

#[test]
fn raster_previews_fit_the_requested_edge_and_keep_the_aspect() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        kit::write_jpeg(dir.path(), "a.jpg", W, H),
        kit::write_png8(dir.path(), "a.png", W, H),
        kit::write_png16(dir.path(), "b.png", W, H),
        kit::write_tiff16(dir.path(), "a.tif", W, H),
    ];
    for path in &files {
        for (max_edge, expected) in [(160, (160, 120)), (64, (64, 48)), (100, (100, 75))] {
            let small = preview(path, max_edge);
            assert_eq!(
                (small.width, small.height),
                expected,
                "{} at {max_edge}",
                path.display()
            );
            assert_well_formed(&small);
            let error = error_against_scene(&small, W, H);
            assert!(
                error < 6.0,
                "{} at {max_edge}: mean error {error}",
                path.display()
            );
        }
    }
}

#[test]
fn previews_are_never_enlarged() {
    let dir = tempfile::tempdir().unwrap();
    let path = kit::write_png8(dir.path(), "small.png", 40, 30);
    for max_edge in [40, 41, 4096, u32::MAX] {
        let same = preview(&path, max_edge);
        assert_eq!((same.width, same.height), (40, 30), "max edge {max_edge}");
        assert_eq!(error_against_scene(&same, 40, 30), 0.0);
    }
}

#[test]
fn a_zero_edge_is_treated_as_one_pixel() {
    let dir = tempfile::tempdir().unwrap();
    let path = kit::write_png8(dir.path(), "tiny.png", 40, 30);
    let dot = preview(&path, 0);
    assert_eq!((dot.width, dot.height), (1, 1));
    let sliver = preview(&kit::write_png8(dir.path(), "wide.png", 200, 2), 10);
    assert_eq!(
        (sliver.width, sliver.height),
        (10, 1),
        "no side collapses to zero"
    );
}

#[test]
fn portrait_previews_limit_the_long_edge() {
    let dir = tempfile::tempdir().unwrap();
    let path = kit::write_png8(dir.path(), "tall.png", 90, 180);
    let small = preview(&path, 60);
    assert_eq!((small.width, small.height), (30, 60));
}

#[test]
fn the_dng_preview_is_the_embedded_jpeg_not_a_develop() {
    let dir = tempfile::tempdir().unwrap();
    let path = kit::write_dng(dir.path(), "p.dng", W, H);
    let embedded = kit::dng_preview_size(W, H);

    // Asking for more than the embedded picture holds returns it as it is:
    // half the size of the raw image, which a develop would not produce.
    let full = preview(&path, 4096);
    assert_eq!((full.width, full.height), embedded);
    assert_well_formed(&full);
    // The embedded picture is the camera's sRGB rendering. Raw sensor data
    // shown as if it were sRGB would be dark and green, far from this.
    let error = error_against_scene(&full, W, H);
    assert!(error < 6.0, "mean error {error}");

    let small = preview(&path, 80);
    assert_eq!((small.width, small.height), (80, 60));
    assert!(error_against_scene(&small, W, H) < 6.0);
}

#[test]
fn the_dng_preview_follows_the_raw_orientation() {
    let dir = tempfile::tempdir().unwrap();
    for orientation in [3u8, 5, 6, 8] {
        let spec = DngSpec {
            exif: ExifSpec {
                orientation,
                ..ExifSpec::sample()
            },
            ..DngSpec::default()
        };
        let path = kit::write_dng_with(dir.path(), &format!("o{orientation}.dng"), W, H, &spec);
        let small = preview(&path, 100);
        assert_eq!(
            (small.width, small.height),
            (100, 75),
            "orientation {orientation}"
        );
        let error = error_against_scene(&small, W, H);
        assert!(error < 6.0, "orientation {orientation}: mean error {error}");
    }
}

#[test]
fn a_raw_file_without_an_embedded_picture_has_no_preview() {
    let dir = tempfile::tempdir().unwrap();
    let spec = DngSpec {
        embed_previews: false,
        ..DngSpec::default()
    };
    let path = kit::write_dng_with(dir.path(), "bare.dng", 64, 48, &spec);
    assert_eq!(embedded_preview(&path, 256).unwrap(), None);
}

#[test]
fn wide_gamut_previews_are_converted_to_srgb() {
    let dir = tempfile::tempdir().unwrap();
    let path = kit::write_tagged(dir.path(), "wide.png", W, H, RgbSpace::ProPhoto);
    let small = preview(&path, W);
    assert_well_formed(&small);
    assert_eq!((small.width, small.height), (W, H));

    // The file's code values are the scene's, labelled as ProPhoto. Check a
    // few patches against the same conversion done by hand.
    let scene = kit::scene(W, H);
    let to_srgb = rgb_to_rgb(RgbSpace::ProPhoto, RgbSpace::Srgb);
    for patch in kit::patches(W, H) {
        let (x, y) = (patch.x + patch.width / 2, patch.y + patch.height / 2);
        let code = scene.get_pixel(x, y).0;
        let linear = code.map(|c| RgbSpace::ProPhoto.decode(f32::from(c) / 255.0));
        let expected = mat3_apply(&to_srgb, linear)
            .map(|c| (RgbSpace::Srgb.encode(c.clamp(0.0, 1.0)) * 255.0).round() as u8);
        let got = small.pixel(x, y);
        for c in 0..3 {
            assert!(
                got[c].abs_diff(expected[c]) <= 2,
                "{}: {got:?} vs {expected:?}",
                patch.name
            );
        }
        // The conversion is not a no-op: these colours move.
        assert_ne!([got[0], got[1], got[2]], code, "{}", patch.name);
    }
}

#[test]
fn transparent_previews_are_shown_on_white() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("alpha.png");
    image::RgbaImage::from_fn(8, 8, |x, _| {
        image::Rgba([0, 0, 0, if x < 4 { 255 } else { 0 }])
    })
    .save(&path)
    .unwrap();
    let shown = preview(&path, 8);
    assert_eq!(shown.pixel(0, 0), [0, 0, 0, 255]);
    assert_eq!(shown.pixel(7, 7), [255, 255, 255, 255]);
}
