//! Whatever this crate writes, it reads back: exactly through the private
//! settings, and field for field through the Camera Raw mapping alone.

use photo_core::develop::{
    Adj, Crop, CropConstraint, CropRect, CurvePoint, DevelopSettings, Mask, MaskShape, Orientation,
    PointCurve, Profile, SETTINGS_FORMAT, Spot, SpotMode, Treatment, WbPreset,
};
use photo_core::geom::Point;
use photo_core::meta::{ColorLabel, FileKind, Flag, Rating};
use photo_core::time::DateTime;
use photo_xmp::{ImageInfo, XmpData, XmpError, parse, to_string};

const KINDS: [FileKind; 2] = [FileKind::Raw, FileKind::Raster];
const RAW: ImageInfo = ImageInfo::new(FileKind::Raw);

/// A curve point given on the 0 to 255 scale sidecars use.
fn level(x: u8, y: u8) -> CurvePoint {
    CurvePoint::new(f32::from(x) / 255.0, f32::from(y) / 255.0)
}

fn all_orientations() -> impl Iterator<Item = Orientation> {
    (0..4).flat_map(|turns| [false, true].map(|flip| Orientation::new(turns, flip)))
}

fn develop(settings: &DevelopSettings) -> XmpData {
    XmpData {
        develop: Some(settings.clone()),
        ..XmpData::default()
    }
}

/// Settings with every adjustment moved to a value that has no short decimal
/// form, plus every kind of non-scalar edit.
fn rich_settings(kind: FileKind) -> DevelopSettings {
    let mut settings = DevelopSettings::default_for(kind);
    for (index, adj) in Adj::all().enumerate() {
        let spec = adj.spec_for(kind);
        let fraction = 0.15 + 0.7 * ((index * 37 % 101) as f32 / 101.0);
        adj.set(&mut settings, spec.min + (spec.max - spec.min) * fraction);
    }
    settings.white_balance.preset = WbPreset::Custom;
    settings.treatment = Treatment::BlackAndWhite;
    settings.profile = Profile::Vivid;
    settings.tone_curve.rgb =
        PointCurve::new([level(0, 8), level(70, 52), level(180, 200), level(255, 250)]);
    settings.tone_curve.red = PointCurve::new([level(0, 0), level(128, 120), level(255, 255)]);
    settings.tone_curve.green = PointCurve::new([level(10, 0), level(245, 255)]);
    settings.tone_curve.blue = PointCurve::new([
        level(0, 20),
        level(127, 128),
        level(128, 131),
        level(255, 235),
    ]);
    settings.orientation = Orientation::new(3, true);
    settings.crop.rect = Some(CropRect {
        cx: 0.47,
        cy: 0.52,
        width: 0.61,
        height: 0.73,
    });
    settings.crop.constraint = CropConstraint::Ratio {
        width: 4.0,
        height: 5.0,
    };
    settings.spots.push(Spot {
        mode: SpotMode::Clone,
        target: Point::new(0.25, 0.75),
        source: Point::new(0.3, 0.7),
        radius: 0.02,
        feather: 35.0,
        opacity: 90.0,
    });
    let mut mask = Mask::from_shape(
        MaskShape::Linear {
            start: Point::new(0.5, 0.0),
            end: Point::new(0.5, 0.4),
        },
        1,
    );
    mask.name = "Sky <& \"glow\"> ]]> ☁".to_owned();
    mask.adjust.exposure = -0.6;
    settings.masks.push(mask);

    let mut sanitised = settings.clone();
    sanitised.sanitize_for(kind);
    assert_eq!(
        sanitised, settings,
        "the fixture must already be valid for {kind:?}"
    );
    settings
}

/// The part of `settings` the Camera Raw vocabulary can carry: everything but
/// the profile, the crop tool's aspect lock, healing spots and masks.
fn mapped_part(settings: &DevelopSettings) -> DevelopSettings {
    DevelopSettings {
        profile: Profile::default(),
        crop: Crop {
            constraint: CropConstraint::default(),
            ..settings.crop
        },
        spots: Vec::new(),
        masks: Vec::new(),
        ..settings.clone()
    }
}

/// `xml` as it would be after an application that does not know the private
/// namespace dropped the property.
fn without_private_settings(xml: &str) -> String {
    let start = xml
        .find("<photo:Settings>")
        .expect("the private settings are an element");
    let end = xml
        .find("</photo:Settings>")
        .expect("the element is closed")
        + "</photo:Settings>".len();
    format!("{}{}", &xml[..start], &xml[end..])
}

#[test]
fn every_setting_survives_exactly_through_the_private_form() {
    for kind in KINDS {
        let image = ImageInfo::new(kind);
        let data = develop(&rich_settings(kind));
        let xml = to_string(&data, image, None).unwrap();
        assert_eq!(parse(&xml, image).unwrap(), data, "{kind:?}");
    }
}

#[test]
fn the_camera_raw_mapping_alone_carries_every_mapped_setting() {
    for kind in KINDS {
        for file_orientation in all_orientations() {
            let image = ImageInfo::new(kind).with_orientation(file_orientation);
            let settings = rich_settings(kind);
            let xml =
                without_private_settings(&to_string(&develop(&settings), image, None).unwrap());
            assert!(!xml.contains("<photo:Settings"));
            let read = parse(&xml, image).unwrap().develop.unwrap();
            assert_eq!(
                read,
                mapped_part(&settings),
                "{kind:?} {file_orientation:?}"
            );
        }
    }
}

#[test]
fn the_private_form_wins_over_the_camera_raw_mapping() {
    let settings = rich_settings(FileKind::Raw);
    let xml = to_string(&develop(&settings), RAW, None).unwrap();
    let edited = xml.replace(
        r#"crs:ConvertToGrayscale="True""#,
        r#"crs:ConvertToGrayscale="False""#,
    );
    assert_ne!(edited, xml);
    assert_eq!(parse(&edited, RAW).unwrap().develop, Some(settings));
}

#[test]
fn private_settings_that_do_not_parse_are_neither_read_nor_replaced() {
    let image = RAW.with_orientation(Orientation::new(1, false));
    let settings = rich_settings(FileKind::Raw);
    let xml = to_string(&develop(&settings), image, None).unwrap();
    let broken = xml.replace("<photo:Settings>{", "<photo:Settings>{{");
    assert_ne!(broken, xml);
    // Not the Camera Raw mapping in their place: a later write would make
    // that the photo's settings for good.
    let read = parse(&broken, image).unwrap();
    assert_eq!(read.develop, None);
    assert_eq!(read.unreadable_settings_format, Some(SETTINGS_FORMAT));
    let refused = to_string(&develop(&settings), image, Some(&broken));
    assert!(
        matches!(refused, Err(XmpError::UnreadableSettings { .. })),
        "{refused:?}"
    );
}

#[test]
fn curve_points_between_levels_are_stored_at_the_nearest_of_256_levels() {
    let mut settings = DevelopSettings::default_for(FileKind::Raster);
    settings.tone_curve.rgb = PointCurve::new([
        CurvePoint::new(0.0, 0.0),
        CurvePoint::new(0.5, 0.25),
        CurvePoint::new(1.0, 1.0),
    ]);
    let image = ImageInfo::new(FileKind::Raster);
    let xml = to_string(&develop(&settings), image, None).unwrap();
    assert!(xml.contains("<rdf:li>128, 64</rdf:li>"), "{xml}");
    let read = parse(&without_private_settings(&xml), image)
        .unwrap()
        .develop
        .unwrap();
    assert_eq!(
        read.tone_curve.rgb,
        PointCurve::new([level(0, 0), level(128, 64), level(255, 255)])
    );
    assert_eq!(parse(&xml, image).unwrap().develop, Some(settings));
}

#[test]
fn numbers_are_written_the_way_other_applications_write_them() {
    let mut settings = DevelopSettings::default_for(FileKind::Raw);
    settings.white_balance.preset = WbPreset::Custom;
    settings.white_balance.temp = 5600.0;
    settings.white_balance.tint = 12.0;
    settings.tone.exposure = 0.35;
    settings.tone.contrast = -12.0;
    settings.detail.sharpening.radius = 1.5;
    let xml = to_string(&develop(&settings), RAW, None).unwrap();
    for expected in [
        r#"crs:WhiteBalance="Custom""#,
        r#"crs:Temperature="5600""#,
        r#"crs:Tint="+12""#,
        r#"crs:Exposure2012="+0.35""#,
        r#"crs:Contrast2012="-12""#,
        r#"crs:Highlights2012="0""#,
        r#"crs:Sharpness="40""#,
        r#"crs:SharpenRadius="1.5""#,
        r#"crs:ConvertToGrayscale="False""#,
        r#"crs:HasCrop="False""#,
        "<rdf:li>0, 0</rdf:li>",
        "<rdf:li>255, 255</rdf:li>",
    ] {
        assert!(xml.contains(expected), "{expected} is missing from\n{xml}");
    }

    let mut relative = DevelopSettings::default_for(FileKind::Raster);
    relative.white_balance = settings.white_balance;
    relative.white_balance.temp = 18.0;
    let xml = to_string(&develop(&relative), ImageInfo::new(FileKind::Raster), None).unwrap();
    assert!(xml.contains(r#"crs:IncrementalTemperature="+18""#), "{xml}");
    assert!(xml.contains(r#"crs:IncrementalTint="+12""#), "{xml}");
    assert!(!xml.contains("crs:Temperature"), "{xml}");
}

#[test]
fn local_corrections_retouching_and_lens_profiles_are_never_emitted() {
    for kind in KINDS {
        let xml = to_string(&develop(&rich_settings(kind)), ImageInfo::new(kind), None).unwrap();
        let mirror = without_private_settings(&xml);
        for forbidden in [
            "Correction",
            "Retouch",
            "LensProfile",
            "MaskGroup",
            "Paint",
            "Gradient",
        ] {
            assert!(
                !mirror.contains(forbidden),
                "{forbidden} appears in\n{mirror}"
            );
        }
    }
}

#[test]
fn a_new_packet_is_wrapped_and_well_formed() {
    let xml = to_string(&develop(&rich_settings(FileKind::Raw)), RAW, None).unwrap();
    assert!(xml.starts_with("<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\""));
    assert!(xml.ends_with("</x:xmpmeta>\n<?xpacket end=\"w\"?>\n"));
    assert!(xml.contains(r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">"#));
    assert!(xml.contains(r#"xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/""#));
    assert!(
        !xml.contains("adobe.com/photo"),
        "the private namespace must not look like Adobe's"
    );
    assert_eq!(xml.matches("<rdf:Description").count(), 1);
}

#[test]
fn writing_twice_is_the_same_as_writing_once() {
    for kind in KINDS {
        let image = ImageInfo::new(kind).with_orientation(Orientation::new(1, false));
        let data = XmpData {
            rating: Rating::new(2),
            flag: Flag::Rejected,
            keywords: vec!["one".to_owned(), "two".to_owned()],
            ..develop(&rich_settings(kind))
        };
        let once = to_string(&data, image, None).unwrap();
        assert_eq!(to_string(&data, image, Some(&once)).unwrap(), once);
    }
}

#[test]
fn empty_data_makes_an_empty_but_valid_packet() {
    let xml = to_string(&XmpData::default(), RAW, None).unwrap();
    assert!(!xml.contains("rdf:Description"), "{xml}");
    assert_eq!(parse(&xml, RAW).unwrap(), XmpData::default());
}

#[test]
fn descriptive_metadata_round_trips_including_awkward_text() {
    let data = XmpData {
        rating: Rating::new(3),
        flag: Flag::Picked,
        label: ColorLabel::Blue,
        title: "Tom & Jerry <\"quoted\"> 'single' ]]> ünïcödé 日本語 🎞".to_owned(),
        caption: "line one\nline two\ttabbed\r\nwindows  two spaces ".to_owned(),
        creators: vec![
            "Ada Example".to_owned(),
            "O'Neil & Sons <studio>".to_owned(),
        ],
        copyright: "© 2026 Ada Example — \"all rights\"".to_owned(),
        keywords: vec![
            "blue hour".to_owned(),
            "R&D".to_owned(),
            "<tag>".to_owned(),
            "naïve café".to_owned(),
            "  padded  ".to_owned(),
        ],
        hierarchical_keywords: vec![
            vec![
                "Places".to_owned(),
                "Zürich".to_owned(),
                "Old & New".to_owned(),
            ],
            vec!["<top>".to_owned()],
        ],
        capture_time: DateTime::new(2026, 11, 19, 20, 44, 5),
        develop: None,
        unreadable_settings_format: None,
    };
    for kind in KINDS {
        let image = ImageInfo::new(kind);
        let xml = to_string(&data, image, None).unwrap();
        assert_eq!(parse(&xml, image).unwrap(), data, "{xml}");
    }
}

#[test]
fn characters_xml_cannot_hold_are_dropped_instead_of_breaking_the_file() {
    let data = XmpData {
        title: "bell\u{7}less\u{0}".to_owned(),
        keywords: vec!["ok\u{1b}".to_owned()],
        ..XmpData::default()
    };
    let read = parse(&to_string(&data, RAW, None).unwrap(), RAW).unwrap();
    assert_eq!(read.title, "bellless");
    assert_eq!(read.keywords, ["ok"]);
}

#[test]
fn every_label_round_trips_by_name() {
    for label in ColorLabel::ALL {
        let data = XmpData {
            label,
            ..XmpData::default()
        };
        let xml = to_string(&data, RAW, None).unwrap();
        assert_eq!(xml.contains("xmp:Label"), label != ColorLabel::None);
        assert!(
            label == ColorLabel::None || xml.contains(&format!(r#"xmp:Label="{}""#, label.name()))
        );
        assert_eq!(parse(&xml, RAW).unwrap().label, label);
    }
}

#[test]
fn hierarchical_keywords_are_written_with_bar_separators() {
    let data = XmpData {
        hierarchical_keywords: vec![vec!["A".to_owned(), "B".to_owned(), "C".to_owned()]],
        ..XmpData::default()
    };
    let xml = to_string(&data, RAW, None).unwrap();
    assert!(xml.contains("<rdf:li>A|B|C</rdf:li>"), "{xml}");
    assert!(
        xml.contains(r#"xmlns:lr="http://ns.adobe.com/lightroom/1.0/""#),
        "{xml}"
    );
}
