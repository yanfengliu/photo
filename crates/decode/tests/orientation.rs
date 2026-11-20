//! EXIF orientation: every stored layout must come out upright.

mod common;

use common::{linearised_8, mean_abs_diff};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader};
use photo_core::colorimetry::RgbSpace;
use photo_decode::{decode, embedded_preview, read_metadata};
use photo_testkit::{self as kit, ExifSpec, MARKER_RGB};

const W: u32 = 120;
const H: u32 = 80;

fn spec(orientation: u8) -> ExifSpec {
    ExifSpec {
        orientation,
        ..ExifSpec::sample()
    }
}

#[test]
fn all_eight_orientations_decode_upright() {
    let dir = tempfile::tempdir().unwrap();
    let upright = linearised_8(&kit::scene_with_marker(W, H), RgbSpace::Srgb);
    let probe = kit::marker_size(W, H) / 2;
    for orientation in 1..=8u8 {
        let path = kit::write_jpeg_with_exif(
            dir.path(),
            &format!("o{orientation}.jpg"),
            W,
            H,
            &spec(orientation),
        );

        // The fixture really stores a turned grid for the sideways cases.
        let stored = image::image_dimensions(&path).unwrap();
        assert_eq!(
            stored,
            if orientation >= 5 { (H, W) } else { (W, H) },
            "orientation {orientation}"
        );

        let decoded = decode(&path).unwrap();
        assert_eq!(
            (decoded.image.width, decoded.image.height),
            (W, H),
            "orientation {orientation}"
        );
        assert_eq!((decoded.metadata.width, decoded.metadata.height), (W, H));

        // The marker is back in the top-left corner and nowhere else.
        let [r, g, b] = decoded.image.pixel(probe, probe);
        assert!(
            r > 50000 && g < 3000 && b > 50000,
            "orientation {orientation}: marker is {:?}",
            [r, g, b]
        );
        for (x, y) in [
            (W - 1 - probe, probe),
            (probe, H - 1 - probe),
            (W - 1 - probe, H - 1 - probe),
        ] {
            let [r, g, b] = decoded.image.pixel(x, y);
            assert!(
                !(r > 50000 && g < 3000 && b > 50000),
                "orientation {orientation}: marker also at {x},{y}"
            );
        }

        // And the whole picture matches the upright scene, not just a corner.
        let error = mean_abs_diff(&decoded.image.pixels, &upright) / 65535.0;
        assert!(
            error < 0.012,
            "orientation {orientation}: mean linear error {error}"
        );
    }
}

#[test]
fn read_metadata_reports_upright_dimensions_without_decoding() {
    let dir = tempfile::tempdir().unwrap();
    for orientation in [1u8, 3, 6, 8] {
        let path = kit::write_jpeg_with_exif(
            dir.path(),
            &format!("m{orientation}.jpg"),
            W,
            H,
            &spec(orientation),
        );
        let metadata = read_metadata(&path).unwrap();
        assert_eq!(
            (metadata.width, metadata.height),
            (W, H),
            "orientation {orientation}"
        );
    }
}

#[test]
fn orientation_agrees_with_the_image_crate_on_lossless_pixels() {
    // An independent implementation of the same transform, compared exactly:
    // the 16-bit TIFF fixture is lossless, so any difference is a wrong pixel.
    let dir = tempfile::tempdir().unwrap();
    for orientation in 1..=8u8 {
        let path = kit::write_tiff16_with_exif(
            dir.path(),
            &format!("o{orientation}.tif"),
            37,
            23,
            &spec(orientation),
        );

        let mut decoder = ImageReader::open(&path).unwrap().into_decoder().unwrap();
        let tagged = decoder.orientation().unwrap();
        assert_eq!(tagged, Orientation::from_exif(orientation).unwrap());
        let mut reference = DynamicImage::from_decoder(decoder).unwrap();
        reference.apply_orientation(tagged);
        let reference = reference.into_rgb16();

        let decoded = decode(&path).unwrap().image;
        assert_eq!(
            (decoded.width, decoded.height),
            (37, 23),
            "orientation {orientation}"
        );
        assert_eq!((reference.width(), reference.height()), (37, 23));
        assert_eq!(
            decoded.pixels,
            common::linearised_16(&reference, RgbSpace::Srgb),
            "orientation {orientation}"
        );

        // The marker corner, exactly.
        let marker = MARKER_RGB.map(|code| common::linear_from_8(RgbSpace::Srgb, code));
        assert_eq!(decoded.pixel(0, 0), marker, "orientation {orientation}");
    }
}

#[test]
fn previews_are_turned_upright_as_well() {
    let dir = tempfile::tempdir().unwrap();
    for orientation in [1u8, 2, 6, 7] {
        let path = kit::write_jpeg_with_exif(
            dir.path(),
            &format!("p{orientation}.jpg"),
            W,
            H,
            &spec(orientation),
        );
        let preview = embedded_preview(&path, 60)
            .unwrap()
            .expect("raster files always preview");
        assert_eq!(
            (preview.width, preview.height),
            (60, 40),
            "orientation {orientation}"
        );
        let [r, g, b, a] = preview.pixel(3, 3);
        assert!(
            r > 200 && g < 70 && b > 200 && a == 255,
            "orientation {orientation}: {:?}",
            [r, g, b, a]
        );
        let [r, g, b, _] = preview.pixel(56, 36);
        assert!(!(r > 200 && g < 70 && b > 200), "orientation {orientation}");
    }
}

#[test]
fn a_file_without_an_orientation_tag_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = kit::write_jpeg(dir.path(), "plain.jpg", W, H);
    let decoded = decode(&path).unwrap();
    assert_eq!((decoded.image.width, decoded.image.height), (W, H));
    let expected = linearised_8(&kit::scene(W, H), RgbSpace::Srgb);
    assert!(mean_abs_diff(&decoded.image.pixels, &expected) / 65535.0 < 0.012);
}
