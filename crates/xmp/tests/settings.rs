//! What travels with the develop settings: the format of the private
//! settings, which keeps this build from overwriting settings a newer build
//! wrote, and the Camera Raw header that tells other applications which
//! process the slider values belong to.

use std::fs;

use photo_core::develop::{DevelopSettings, Mask, MaskShape, SETTINGS_FORMAT};
use photo_core::geom::Point;
use photo_core::meta::{FileKind, Rating};
use photo_xmp::{ImageInfo, XmpData, XmpError, parse, read_sidecar, to_string, write_sidecar};

const RAW: ImageInfo = ImageInfo::new(FileKind::Raw);

/// A packet with `attributes` on its one description and `content` in it.
fn packet(attributes: &str, content: &str) -> String {
    format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" xmlns:photo="http://ns.photo.invalid/1.0/" {attributes}>{content}</rdf:Description></rdf:RDF></x:xmpmeta>"#
    )
}

/// A sidecar as a newer build writes it: settings in a format this build
/// does not know, using a value it has no name for, beside Camera Raw
/// properties that carry only part of them.
fn newer() -> (String, u32) {
    let format = SETTINGS_FORMAT + 1;
    let xml = packet(
        &format!(r#"xmp:Rating="2" crs:Exposure2012="+1.00" photo:SettingsFormat="{format}""#),
        r#"<photo:Settings>{"treatment":"sepia","tone":{"exposure":1.0}}</photo:Settings>"#,
    );
    (xml, format)
}

fn develop(settings: DevelopSettings) -> XmpData {
    XmpData {
        develop: Some(settings),
        ..XmpData::default()
    }
}

#[test]
fn settings_written_here_carry_their_format() {
    let xml = to_string(
        &develop(DevelopSettings::default_for(FileKind::Raw)),
        RAW,
        None,
    )
    .unwrap();
    let marked = format!(r#"photo:SettingsFormat="{SETTINGS_FORMAT}""#);
    assert!(xml.contains(&marked), "{xml}");
    let read = parse(&xml, RAW).unwrap();
    assert_eq!(read.newer_settings_format, None);
    assert!(read.develop.is_some());
}

#[test]
fn settings_of_a_newer_format_are_reported_rather_than_read() {
    let (xml, format) = newer();
    let read = parse(&xml, RAW).unwrap();
    // Neither the private settings, which this build would misread, nor the
    // Camera Raw properties in their place, which a later write would make
    // the photo's settings for good.
    assert_eq!(read.develop, None);
    assert_eq!(read.newer_settings_format, Some(format));
    assert_eq!(read.rating, Rating::new(2));
}

#[test]
fn develop_settings_are_not_written_over_settings_of_a_newer_format() {
    let (xml, format) = newer();
    let data = XmpData {
        rating: Rating::new(4),
        ..develop(DevelopSettings::default_for(FileKind::Raw))
    };
    let refused = to_string(&data, RAW, Some(&xml));
    assert!(
        matches!(refused, Err(XmpError::NewerSettings { format: found }) if found == format),
        "{refused:?}"
    );

    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("IMG_0001.xmp");
    fs::write(&path, &xml).unwrap();
    let refused = write_sidecar(&path, &data, RAW);
    assert!(matches!(refused, Err(XmpError::NewerSettings { .. })));
    assert_eq!(fs::read_to_string(&path).unwrap(), xml);
    let error = refused.unwrap_err().to_string();
    assert!(error.contains(&format.to_string()), "{error}");
}

#[test]
fn everything_else_is_still_written_beside_settings_of_a_newer_format() {
    let (xml, _) = newer();
    let data = XmpData {
        rating: Rating::new(4),
        ..parse(&xml, RAW).unwrap()
    };
    let updated = to_string(&data, RAW, Some(&xml)).unwrap();
    assert_eq!(parse(&updated, RAW).unwrap(), data);
    let (_, format) = newer();
    for kept in [
        r#"<photo:Settings>{"treatment":"sepia","tone":{"exposure":1.0}}</photo:Settings>"#
            .to_owned(),
        format!(r#"photo:SettingsFormat="{format}""#),
        r#"crs:Exposure2012="+1.00""#.to_owned(),
    ] {
        assert!(updated.contains(&kept), "{kept} is missing from\n{updated}");
    }
    assert!(!updated.contains("crs:ProcessVersion"), "{updated}");
}

#[test]
fn settings_of_this_format_or_an_earlier_one_are_read() {
    let settings = r#"<photo:Settings>{"tone":{"exposure":1.5}}</photo:Settings>"#;
    let current = format!(r#"photo:SettingsFormat="{SETTINGS_FORMAT}""#);
    for attributes in [
        "",
        current.as_str(),
        r#"photo:SettingsFormat="0""#,
        r#"photo:SettingsFormat=" +1 ""#,
        r#"photo:SettingsFormat="soon""#,
    ] {
        let read = parse(&packet(attributes, settings), RAW).unwrap();
        let exposure = read.develop.map(|develop| develop.tone.exposure);
        assert_eq!(exposure, Some(1.5), "{attributes}");
        assert_eq!(read.newer_settings_format, None, "{attributes}");
    }
    let far = r#"photo:SettingsFormat="99999999999999999999""#;
    let read = parse(&packet(far, settings), RAW).unwrap();
    assert_eq!(read.newer_settings_format, Some(u32::MAX));
}

#[test]
fn a_number_that_is_not_finite_is_refused_rather_than_written_as_null() {
    let mut exposure = DevelopSettings::default_for(FileKind::Raw);
    exposure.tone.exposure = f32::NAN;
    let mut feather = DevelopSettings::default_for(FileKind::Raw);
    let shape = MaskShape::Radial {
        center: Point::new(0.5, 0.5),
        radius_x: 0.2,
        radius_y: 0.1,
        angle: 0.0,
        feather: f32::INFINITY,
        invert: false,
    };
    feather.masks.push(Mask::from_shape(shape, 1));
    for settings in [exposure, feather] {
        let result = to_string(&develop(settings), RAW, None);
        assert!(matches!(result, Err(XmpError::NotFinite)), "{result:?}");
    }
}

#[test]
fn camera_raw_settings_say_which_process_their_names_belong_to() {
    let xml = to_string(
        &develop(DevelopSettings::default_for(FileKind::Raw)),
        RAW,
        None,
    )
    .unwrap();
    for expected in [
        r#"crs:Version="15.4""#,
        r#"crs:ProcessVersion="11.0""#,
        r#"crs:HasSettings="True""#,
    ] {
        assert!(xml.contains(expected), "{expected} is missing from\n{xml}");
    }
    let metadata = to_string(&XmpData::default(), RAW, None).unwrap();
    assert!(!metadata.contains("crs:"), "{metadata}");
}

#[test]
fn a_process_of_the_same_generation_is_kept_and_an_older_one_replaced() {
    let data = develop(DevelopSettings::default_for(FileKind::Raw));
    for (header, version, process) in [
        (
            r#"crs:Version="7.0" crs:ProcessVersion="6.7""#,
            "7.0",
            "6.7",
        ),
        (
            r#"crs:Version="16.0" crs:ProcessVersion="15.4""#,
            "16.0",
            "15.4",
        ),
        (r#"crs:ProcessVersion="10.0""#, "15.4", "10.0"),
        (
            r#"crs:Version="6.0" crs:ProcessVersion="5.7""#,
            "15.4",
            "11.0",
        ),
        (
            r#"crs:Version="4.0" crs:ProcessVersion="5.0""#,
            "15.4",
            "11.0",
        ),
        (
            r#"crs:ProcessVersion="old" crs:HasSettings="False""#,
            "15.4",
            "11.0",
        ),
    ] {
        let updated = to_string(&data, RAW, Some(&packet(header, ""))).unwrap();
        for expected in [
            format!(r#"crs:Version="{version}""#),
            format!(r#"crs:ProcessVersion="{process}""#),
            r#"crs:HasSettings="True""#.to_owned(),
        ] {
            assert!(
                updated.contains(&expected),
                "{header}: {expected} in\n{updated}"
            );
        }
        assert_eq!(to_string(&data, RAW, Some(&updated)).unwrap(), updated);
    }
}

#[test]
fn a_sidecar_file_with_newer_settings_is_read_and_left_alone() {
    let (xml, format) = newer();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("IMG_0001.xmp");
    fs::write(&path, &xml).unwrap();
    let read = read_sidecar(&path, RAW).unwrap().unwrap();
    assert_eq!(read.newer_settings_format, Some(format));
    write_sidecar(&path, &read, RAW).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), xml);
}
