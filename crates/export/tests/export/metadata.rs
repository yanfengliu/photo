//! Metadata: what each mode writes, read back with kamadak-exif and from
//! the XMP packet, in every format.

use std::fs;
use std::io::BufReader;
use std::path::Path;

use exif::{Context, Exif, In, Reader, Tag, Value};
use photo_core::meta::{GpsPosition, Rating};
use photo_core::time::DateTime;
use photo_develop::CancelToken;
use photo_export::{BitDepth, ExportAttributes, Format, MetadataMode, export};

use crate::common::{Fixture, icc_profile, written};

const FORMATS: [Format; 3] = [
    Format::Jpeg { quality: 90 },
    Format::Png {
        depth: BitDepth::Eight,
    },
    Format::Tiff {
        depth: BitDepth::Sixteen,
    },
];

/// A fixture whose original says everything a camera says.
fn described() -> Fixture {
    let mut fixture = Fixture::new(48, 32);
    let meta = &mut fixture.metadata;
    meta.camera_make = Some("Photo Testkit".into());
    meta.camera_model = Some("Synthetic One".into());
    meta.lens = Some("Testkit 35mm F2".into());
    meta.capture_time = DateTime::new(2024, 3, 15, 7, 42, 10);
    meta.shutter_seconds = Some(1.0 / 250.0);
    meta.aperture = Some(5.6);
    meta.iso = Some(200);
    meta.focal_length_mm = Some(35.0);
    meta.focal_length_35mm = Some(52.0);
    meta.exposure_bias_ev = Some(-1.0 / 3.0);
    meta.flash_fired = Some(false);
    meta.gps = Some(GpsPosition {
        latitude: 47.556_25,
        longitude: -122.332_5,
        altitude_m: Some(56.5),
    });
    meta.artist = Some("Camera Owner".into());
    meta.copyright = Some("(c) Camera Owner".into());
    fixture.attributes = ExportAttributes {
        title: Some("Harbour & <Boats>".into()),
        caption: Some("Morning light".into()),
        creator: Some("Ana Núñez".into()),
        copyright: Some("© 2024 Ana Núñez".into()),
        keywords: vec!["sea".into(), "  ".into(), "boats".into()],
        rating: Rating::new(4),
    };
    fixture
}

fn export_with(
    fixture: &Fixture,
    format: Format,
    include: MetadataMode,
    strip_location: bool,
) -> std::path::PathBuf {
    let mut settings = fixture.settings_out();
    settings.format = format;
    settings.metadata.include = include;
    settings.metadata.strip_location = strip_location;
    settings.naming.template = format!("{include:?}-{strip_location}");
    written(&export(&fixture.item(), &settings, &CancelToken::new()).unwrap()).0
}

fn exif_of(path: &Path) -> Option<Exif> {
    let mut file = BufReader::new(fs::File::open(path).unwrap());
    Reader::new().read_from_container(&mut file).ok()
}

fn text(exif: &Exif, tag: Tag) -> Option<String> {
    match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Ascii(parts) => Some(String::from_utf8_lossy(&parts[0]).into_owned()),
        other => panic!("{tag} is {other:?}"),
    }
}

fn rational(exif: &Exif, tag: Tag) -> Option<f64> {
    match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Rational(values) => Some(values[0].to_f64()),
        Value::SRational(values) => Some(values[0].to_f64()),
        other => panic!("{tag} is {other:?}"),
    }
}

fn number(exif: &Exif, tag: Tag) -> Option<u32> {
    exif.get_field(tag, In::PRIMARY)?.value.get_uint(0)
}

/// The XMP packet of an exported file, wherever its format keeps it.
fn xmp_of(path: &Path) -> Option<String> {
    let bytes = fs::read(path).unwrap();
    let packet = match path.extension().unwrap().to_str().unwrap() {
        "jpg" => {
            let signature = b"http://ns.adobe.com/xap/1.0/\0";
            let mut at = 2;
            loop {
                let (marker, length) = (
                    bytes[at + 1],
                    usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]])),
                );
                if marker == 0xda {
                    return None;
                }
                let payload = &bytes[at + 4..at + 2 + length];
                if marker == 0xe1 && payload.starts_with(signature) {
                    break payload[signature.len()..].to_vec();
                }
                at += 2 + length;
            }
        }
        "png" => {
            let mut at = 8;
            loop {
                let length = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
                let kind = &bytes[at + 4..at + 8];
                if kind == b"IEND" {
                    return None;
                }
                let data = &bytes[at + 8..at + 8 + length];
                let keyword = b"XML:com.adobe.xmp\0\0\0\0\0";
                if kind == b"iTXt" && data.starts_with(keyword) {
                    break data[keyword.len()..].to_vec();
                }
                at += 12 + length;
            }
        }
        _ => match &exif_of(path)?
            .get_field(Tag(Context::Tiff, 700), In::PRIMARY)?
            .value
        {
            Value::Byte(bytes) => bytes.clone(),
            other => panic!("XMP tag is {other:?}"),
        },
    };
    Some(String::from_utf8(packet).unwrap())
}

#[test]
fn everything_is_written_and_reads_back_in_every_format() {
    let fixture = described();
    for format in FORMATS {
        let path = export_with(&fixture, format, MetadataMode::All, false);
        let exif = exif_of(&path).unwrap_or_else(|| panic!("{format:?}: no EXIF"));
        assert_eq!(text(&exif, Tag::Make).as_deref(), Some("Photo Testkit"));
        assert_eq!(text(&exif, Tag::Model).as_deref(), Some("Synthetic One"));
        assert_eq!(
            text(&exif, Tag::LensModel).as_deref(),
            Some("Testkit 35mm F2")
        );
        assert_eq!(
            text(&exif, Tag::DateTimeOriginal).as_deref(),
            Some("2024:03:15 07:42:10")
        );
        assert_eq!(text(&exif, Tag::Artist).as_deref(), Some("Ana Núñez"));
        assert_eq!(
            text(&exif, Tag::Copyright).as_deref(),
            Some("© 2024 Ana Núñez")
        );
        assert!(text(&exif, Tag::Software).unwrap().starts_with("Photo "));
        assert_eq!(number(&exif, Tag::Orientation), Some(1));
        assert_eq!(number(&exif, Tag::PhotographicSensitivity), Some(200));
        assert_eq!(rational(&exif, Tag::ExposureTime), Some(1.0 / 250.0));
        assert!((rational(&exif, Tag::FNumber).unwrap() - 5.6).abs() < 1e-6);
        assert_eq!(rational(&exif, Tag::FocalLength), Some(35.0));
        assert!((rational(&exif, Tag::ExposureBiasValue).unwrap() + 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(number(&exif, Tag::FocalLengthIn35mmFilm), Some(52));
        assert_eq!(number(&exif, Tag::ColorSpace), Some(1), "sRGB is declared");
        assert!(exif.get_field(Tag::GPSLatitude, In::PRIMARY).is_some());

        // This app's own reader gets the camera record back.
        let read = photo_decode::read_metadata(&path).unwrap();
        let mut expected = fixture.metadata.clone();
        (expected.width, expected.height) = (48, 32);
        expected.artist = Some("Ana Núñez".into());
        expected.copyright = Some("© 2024 Ana Núñez".into());
        let gps = read.gps.unwrap();
        assert!(
            (gps.latitude - 47.556_25).abs() < 1e-6 && (gps.longitude + 122.332_5).abs() < 1e-6
        );
        assert_eq!(gps.altitude_m, Some(56.5));
        // Degrees, minutes and seconds do not add back up to the same bits.
        expected.gps = read.gps;
        assert_eq!(read, expected, "{format:?}");

        let xmp = xmp_of(&path).unwrap_or_else(|| panic!("{format:?}: no XMP"));
        for needle in [
            "<rdf:li xml:lang=\"x-default\">Harbour &amp; &lt;Boats&gt;</rdf:li>",
            "<rdf:li xml:lang=\"x-default\">Morning light</rdf:li>",
            "<dc:creator>\n    <rdf:Seq>\n     <rdf:li>Ana Núñez</rdf:li>",
            "<rdf:li xml:lang=\"x-default\">© 2024 Ana Núñez</rdf:li>",
            "<rdf:Bag>\n     <rdf:li>sea</rdf:li>\n     <rdf:li>boats</rdf:li>\n    </rdf:Bag>",
            "<xmp:Rating>4</xmp:Rating>",
        ] {
            assert!(
                xmp.contains(needle),
                "{format:?}: {needle} missing from\n{xmp}"
            );
        }
    }
}

#[test]
fn copyright_and_contact_writes_only_the_creator_and_the_notice() {
    let fixture = described();
    for format in FORMATS {
        let path = export_with(&fixture, format, MetadataMode::CopyrightAndContact, false);
        let exif = exif_of(&path).unwrap();
        assert_eq!(text(&exif, Tag::Artist).as_deref(), Some("Ana Núñez"));
        assert_eq!(
            text(&exif, Tag::Copyright).as_deref(),
            Some("© 2024 Ana Núñez")
        );
        for absent in [
            Tag::Make,
            Tag::Model,
            Tag::LensModel,
            Tag::Software,
            Tag::DateTimeOriginal,
            Tag::ExposureTime,
            Tag::PhotographicSensitivity,
            Tag::GPSLatitude,
        ] {
            assert!(
                exif.get_field(absent, In::PRIMARY).is_none(),
                "{format:?}: {absent}"
            );
        }
        let xmp = xmp_of(&path).unwrap();
        assert!(xmp.contains("Ana Núñez") && xmp.contains("© 2024 Ana Núñez"));
        for absent in ["dc:title", "dc:description", "dc:subject", "xmp:Rating>"] {
            assert!(!xmp.contains(absent), "{format:?}: {absent}");
        }
    }
}

#[test]
fn none_writes_no_metadata_but_keeps_the_profile() {
    let fixture = described();
    for format in FORMATS {
        let path = export_with(&fixture, format, MetadataMode::None, false);
        match exif_of(&path) {
            None => {}
            // A TIFF is itself a TIFF structure: only its image tags.
            Some(exif) => {
                assert!(matches!(format, Format::Tiff { .. }), "{format:?} has EXIF");
                for absent in [
                    Tag::Make,
                    Tag::Artist,
                    Tag::Copyright,
                    Tag::Software,
                    Tag::ExifVersion,
                    Tag::GPSLatitude,
                ] {
                    assert!(exif.get_field(absent, In::PRIMARY).is_none(), "{absent}");
                }
            }
        }
        assert_eq!(xmp_of(&path), None, "{format:?}");
        assert!(icc_profile(&path).is_some(), "{format:?}");
        let read = photo_decode::read_metadata(&path).unwrap();
        assert_eq!(
            (read.camera_make, read.capture_time, read.artist),
            (None, None, None)
        );
    }
}

#[test]
fn stripping_the_location_removes_only_the_location() {
    let fixture = described();
    for format in FORMATS {
        let path = export_with(&fixture, format, MetadataMode::All, true);
        let exif = exif_of(&path).unwrap();
        for absent in [
            Tag::GPSLatitude,
            Tag::GPSLongitude,
            Tag::GPSAltitude,
            Tag::GPSVersionID,
        ] {
            assert!(
                exif.get_field(absent, In::PRIMARY).is_none(),
                "{format:?}: {absent}"
            );
        }
        assert_eq!(text(&exif, Tag::Make).as_deref(), Some("Photo Testkit"));
        assert_eq!(photo_decode::read_metadata(&path).unwrap().gps, None);
    }
}

#[test]
fn the_originals_own_artist_and_copyright_fill_in_blank_attributes() {
    let mut fixture = described();
    fixture.attributes.creator = Some("   ".into());
    fixture.attributes.copyright = None;
    let path = export_with(
        &fixture,
        Format::Jpeg { quality: 80 },
        MetadataMode::All,
        false,
    );
    let exif = exif_of(&path).unwrap();
    assert_eq!(text(&exif, Tag::Artist).as_deref(), Some("Camera Owner"));
    assert_eq!(
        text(&exif, Tag::Copyright).as_deref(),
        Some("(c) Camera Owner")
    );
    let xmp = xmp_of(&path).unwrap();
    assert!(xmp.contains("<rdf:li>Camera Owner</rdf:li>"), "{xmp}");
}

#[test]
fn wide_gamut_exports_declare_an_uncalibrated_exif_colour_space() {
    let fixture = described();
    let mut settings = fixture.settings_out();
    settings.color_space = photo_core::colorimetry::RgbSpace::AdobeRgb;
    let (path, _, _) = written(&export(&fixture.item(), &settings, &CancelToken::new()).unwrap());
    assert_eq!(
        number(&exif_of(&path).unwrap(), Tag::ColorSpace),
        Some(0xffff)
    );
}
