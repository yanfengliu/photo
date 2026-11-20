//! Metadata: what the file says about itself must come back unchanged.

use photo_core::meta::{GpsPosition, PhotoMetadata};
use photo_core::time::DateTime;
use photo_decode::{decode, read_metadata};
use photo_testkit::{self as kit, DngSpec, ExifSpec};

const W: u32 = 96;
const H: u32 = 64;

/// The metadata a reader should produce for a `W` x `H` fixture written
/// with `spec`.
fn expected(spec: &ExifSpec) -> PhotoMetadata {
    PhotoMetadata {
        width: W,
        height: H,
        capture_time: spec.capture_time,
        camera_make: spec.camera_make.clone(),
        camera_model: spec.camera_model.clone(),
        lens: spec.lens.clone(),
        focal_length_mm: spec.focal_length_mm,
        focal_length_35mm: spec.focal_length_35mm,
        aperture: spec.aperture,
        shutter_seconds: spec.shutter_seconds,
        iso: spec.iso,
        exposure_bias_ev: spec.exposure_bias_ev,
        flash_fired: spec.flash_fired,
        gps: spec.gps,
        artist: spec.artist.clone(),
        copyright: spec.copyright.clone(),
    }
}

/// Field-by-field comparison with a tolerance on the position, which is
/// stored as degrees, minutes and seconds.
fn assert_matches(actual: &PhotoMetadata, expected: &PhotoMetadata, context: &str) {
    let without_gps = |metadata: &PhotoMetadata| PhotoMetadata {
        gps: None,
        ..metadata.clone()
    };
    assert_eq!(without_gps(actual), without_gps(expected), "{context}");
    match (actual.gps, expected.gps) {
        (Some(got), Some(want)) => {
            assert!(
                (got.latitude - want.latitude).abs() < 1e-6,
                "{context}: {got:?}"
            );
            assert!(
                (got.longitude - want.longitude).abs() < 1e-6,
                "{context}: {got:?}"
            );
            assert_eq!(got.altitude_m, want.altitude_m, "{context}");
        }
        (None, None) => {}
        other => panic!("{context}: position mismatch {other:?}"),
    }
}

#[test]
fn exif_fields_round_trip_from_the_spec() {
    let dir = tempfile::tempdir().unwrap();
    let spec = ExifSpec::sample();
    let jpeg = kit::write_jpeg_with_exif(dir.path(), "full.jpg", W, H, &spec);
    let tiff = kit::write_tiff16_with_exif(dir.path(), "full.tif", W, H, &spec);
    for path in [jpeg, tiff] {
        let metadata = read_metadata(&path).unwrap();
        assert_matches(&metadata, &expected(&spec), &path.display().to_string());
        assert_eq!(
            metadata.camera_name().as_deref(),
            Some("Photo Testkit Synthetic One")
        );
    }
}

#[test]
fn decode_returns_the_same_metadata_as_read_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let spec = ExifSpec {
        orientation: 6,
        ..ExifSpec::sample()
    };
    let files = [
        kit::write_jpeg_with_exif(dir.path(), "a.jpg", W, H, &spec),
        kit::write_tiff16_with_exif(dir.path(), "a.tif", W, H, &spec),
        kit::write_png8(dir.path(), "a.png", W, H),
        kit::write_dng(dir.path(), "a.dng", W, H),
    ];
    for path in files {
        assert_eq!(
            decode(&path).unwrap().metadata,
            read_metadata(&path).unwrap(),
            "{}",
            path.display()
        );
    }
}

#[test]
fn files_without_exif_report_only_their_size() {
    let dir = tempfile::tempdir().unwrap();
    let bare = PhotoMetadata {
        width: W,
        height: H,
        ..PhotoMetadata::default()
    };
    for path in [
        kit::write_jpeg(dir.path(), "a.jpg", W, H),
        kit::write_png8(dir.path(), "a.png", W, H),
        kit::write_tiff16(dir.path(), "a.tif", W, H),
    ] {
        assert_eq!(read_metadata(&path).unwrap(), bare, "{}", path.display());
    }
}

#[test]
fn an_empty_exif_block_adds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let spec = ExifSpec::default();
    let path = kit::write_jpeg_with_exif(dir.path(), "empty.jpg", W, H, &spec);
    assert_matches(
        &read_metadata(&path).unwrap(),
        &expected(&spec),
        "empty spec",
    );
}

#[test]
fn partial_specs_leave_the_other_fields_empty() {
    let dir = tempfile::tempdir().unwrap();
    let spec = ExifSpec {
        camera_model: Some("Lonely Model".into()),
        iso: Some(6400),
        capture_time: DateTime::new(2019, 12, 31, 23, 59, 58),
        ..ExifSpec::default()
    };
    let path = kit::write_jpeg_with_exif(dir.path(), "partial.jpg", W, H, &spec);
    let metadata = read_metadata(&path).unwrap();
    assert_matches(&metadata, &expected(&spec), "partial spec");
    assert_eq!(metadata.camera_name().as_deref(), Some("Lonely Model"));
}

#[test]
fn very_high_iso_is_read_from_the_long_tag() {
    let dir = tempfile::tempdir().unwrap();
    let spec = ExifSpec {
        iso: Some(102_400),
        ..ExifSpec::default()
    };
    for path in [
        kit::write_jpeg_with_exif(dir.path(), "iso.jpg", W, H, &spec),
        kit::write_tiff16_with_exif(dir.path(), "iso.tif", W, H, &spec),
    ] {
        assert_eq!(
            read_metadata(&path).unwrap().iso,
            Some(102_400),
            "{}",
            path.display()
        );
    }
}

#[test]
fn positions_keep_their_hemisphere_and_depth() {
    let dir = tempfile::tempdir().unwrap();
    let places = [
        GpsPosition {
            latitude: -33.856_784,
            longitude: 151.215_297,
            altitude_m: Some(4.0),
        },
        GpsPosition {
            latitude: 31.5,
            longitude: 35.475,
            altitude_m: Some(-430.5),
        },
        GpsPosition {
            latitude: 0.0,
            longitude: -0.001_5,
            altitude_m: None,
        },
    ];
    for (index, place) in places.into_iter().enumerate() {
        let spec = ExifSpec {
            gps: Some(place),
            ..ExifSpec::default()
        };
        let path = kit::write_jpeg_with_exif(dir.path(), &format!("gps{index}.jpg"), W, H, &spec);
        assert_matches(
            &read_metadata(&path).unwrap(),
            &expected(&spec),
            &format!("{place:?}"),
        );
    }
}

#[test]
fn slow_shutter_speeds_and_fired_flash_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let spec = ExifSpec {
        shutter_seconds: Some(2.5),
        aperture: Some(1.4),
        exposure_bias_ev: Some(0.7),
        flash_fired: Some(true),
        focal_length_mm: Some(85.0),
        ..ExifSpec::default()
    };
    let path = kit::write_jpeg_with_exif(dir.path(), "slow.jpg", W, H, &spec);
    assert_matches(
        &read_metadata(&path).unwrap(),
        &expected(&spec),
        "slow shutter",
    );
}

#[test]
fn dng_metadata_round_trips_through_the_raw_decoder() {
    let dir = tempfile::tempdir().unwrap();
    for embed_previews in [true, false] {
        let spec = DngSpec {
            exif: ExifSpec::sample(),
            embed_previews,
        };
        let path = kit::write_dng_with(dir.path(), &format!("m{embed_previews}.dng"), W, H, &spec);
        let metadata = read_metadata(&path).unwrap();
        assert_matches(
            &metadata,
            &expected(&spec.exif),
            &format!("previews: {embed_previews}"),
        );
    }
}

#[test]
fn dng_metadata_reports_upright_dimensions() {
    let dir = tempfile::tempdir().unwrap();
    for orientation in [1u8, 6, 8, 3] {
        let spec = DngSpec {
            exif: ExifSpec {
                orientation,
                ..ExifSpec::sample()
            },
            ..DngSpec::default()
        };
        let path = kit::write_dng_with(dir.path(), &format!("o{orientation}.dng"), W, H, &spec);
        let metadata = read_metadata(&path).unwrap();
        assert_eq!(
            (metadata.width, metadata.height),
            (W, H),
            "orientation {orientation}"
        );
    }
}
