//! EXIF for fixtures: a description of what the file should claim about
//! itself, and the TIFF directories that say it.

use photo_core::meta::GpsPosition;
use photo_core::time::DateTime;

use crate::tiff::{Ifd, TiffWriter, Value};

/// What a fixture's EXIF block should say. Fields left `None` are not
/// written at all, so a reader must report them as missing.
///
/// The field names and units match `photo_core::meta::PhotoMetadata`, so a
/// test can compare the two directly.
#[derive(Clone, Debug, PartialEq)]
pub struct ExifSpec {
    /// EXIF orientation, 1 to 8. The fixture stores its pixels turned and
    /// mirrored so that a reader honouring this tag shows the scene upright.
    pub orientation: u8,
    /// Camera maker, written as `Make`.
    pub camera_make: Option<String>,
    /// Camera model, written as `Model`.
    pub camera_model: Option<String>,
    /// Lens name, written as `LensModel`.
    pub lens: Option<String>,
    /// Written as `DateTimeOriginal`, `DateTimeDigitized` and `DateTime`.
    pub capture_time: Option<DateTime>,
    /// Exposure time in seconds.
    pub shutter_seconds: Option<f32>,
    /// f-number.
    pub aperture: Option<f32>,
    /// ISO speed. Values above 65534 are written the way cameras do it: the
    /// 16-bit tag saturated and the real figure in the 32-bit `ISOSpeed` tag.
    pub iso: Option<u32>,
    /// Focal length of the lens in millimetres.
    pub focal_length_mm: Option<f32>,
    /// Focal length with the same angle of view on 35 mm film, in whole
    /// millimetres (the tag holds an integer).
    pub focal_length_35mm: Option<f32>,
    /// Exposure compensation in stops.
    pub exposure_bias_ev: Option<f32>,
    /// Whether the flash fired: bit 0 of the `Flash` tag.
    pub flash_fired: Option<bool>,
    /// Where the picture was taken, written as a GPS directory.
    pub gps: Option<GpsPosition>,
    /// The photographer, written as `Artist`.
    pub artist: Option<String>,
    /// The copyright notice, written as `Copyright`.
    pub copyright: Option<String>,
}

impl Default for ExifSpec {
    /// Upright and otherwise silent: only the orientation tag is written.
    fn default() -> Self {
        Self {
            orientation: 1,
            camera_make: None,
            camera_model: None,
            lens: None,
            capture_time: None,
            shutter_seconds: None,
            aperture: None,
            iso: None,
            focal_length_mm: None,
            focal_length_35mm: None,
            exposure_bias_ev: None,
            flash_fired: None,
            gps: None,
            artist: None,
            copyright: None,
        }
    }
}

impl ExifSpec {
    /// A fully populated, plausible record: every field set, upright.
    pub fn sample() -> Self {
        Self {
            orientation: 1,
            camera_make: Some("Photo Testkit".into()),
            camera_model: Some("Synthetic One".into()),
            lens: Some("Testkit 35mm F2".into()),
            capture_time: DateTime::new(2024, 3, 15, 7, 42, 10),
            shutter_seconds: Some(1.0 / 250.0),
            aperture: Some(5.6),
            iso: Some(200),
            focal_length_mm: Some(35.0),
            focal_length_35mm: Some(52.0),
            exposure_bias_ev: Some(-1.0 / 3.0),
            flash_fired: Some(false),
            gps: Some(GpsPosition {
                latitude: 47.556_25,
                longitude: -122.332_5,
                altitude_m: Some(56.5),
            }),
            artist: Some("Test Kit".into()),
            copyright: Some("(c) Photo Testkit".into()),
        }
    }

    /// [`Self::sample`] taken at `capture_time`.
    pub fn sample_at(capture_time: DateTime) -> Self {
        Self {
            capture_time: Some(capture_time),
            ..Self::sample()
        }
    }
}

mod tag {
    pub(super) const MAKE: u16 = 0x010f;
    pub(super) const MODEL: u16 = 0x0110;
    pub(super) const ORIENTATION: u16 = 0x0112;
    pub(super) const DATE_TIME: u16 = 0x0132;
    pub(super) const ARTIST: u16 = 0x013b;
    pub(super) const COPYRIGHT: u16 = 0x8298;
    pub(super) const EXIF_IFD: u16 = 0x8769;
    pub(super) const GPS_IFD: u16 = 0x8825;

    pub(super) const EXPOSURE_TIME: u16 = 0x829a;
    pub(super) const F_NUMBER: u16 = 0x829d;
    pub(super) const ISO: u16 = 0x8827;
    pub(super) const SENSITIVITY_TYPE: u16 = 0x8830;
    pub(super) const ISO_SPEED: u16 = 0x8833;
    pub(super) const EXIF_VERSION: u16 = 0x9000;
    pub(super) const DATE_TIME_ORIGINAL: u16 = 0x9003;
    pub(super) const DATE_TIME_DIGITIZED: u16 = 0x9004;
    pub(super) const EXPOSURE_BIAS: u16 = 0x9204;
    pub(super) const FLASH: u16 = 0x9209;
    pub(super) const FOCAL_LENGTH: u16 = 0x920a;
    pub(super) const FOCAL_LENGTH_35MM: u16 = 0xa405;
    pub(super) const LENS_MODEL: u16 = 0xa434;

    pub(super) const GPS_VERSION: u16 = 0x0000;
    pub(super) const GPS_LATITUDE_REF: u16 = 0x0001;
    pub(super) const GPS_LATITUDE: u16 = 0x0002;
    pub(super) const GPS_LONGITUDE_REF: u16 = 0x0003;
    pub(super) const GPS_LONGITUDE: u16 = 0x0004;
    pub(super) const GPS_ALTITUDE_REF: u16 = 0x0005;
    pub(super) const GPS_ALTITUDE: u16 = 0x0006;
}

/// Adds everything `spec` describes to a file under construction: the tags
/// that belong in the first directory go into `ifd0`, and the EXIF and GPS
/// directories are appended to `writer` and linked from it.
pub(crate) fn add_exif(writer: &mut TiffWriter, ifd0: &mut Ifd, spec: &ExifSpec) {
    assert!(
        (1..=8).contains(&spec.orientation),
        "EXIF orientation must be 1 to 8, got {}",
        spec.orientation
    );
    ifd0.short(tag::ORIENTATION, u16::from(spec.orientation));
    let texts = [
        (tag::MAKE, &spec.camera_make),
        (tag::MODEL, &spec.camera_model),
        (tag::ARTIST, &spec.artist),
        (tag::COPYRIGHT, &spec.copyright),
    ];
    for (tag, text) in texts {
        if let Some(text) = text {
            ifd0.ascii(tag, text);
        }
    }
    let stamp = spec.capture_time.map(exif_date_time);
    if let Some(stamp) = &stamp {
        ifd0.ascii(tag::DATE_TIME, stamp);
    }

    let mut exif = Ifd::new();
    exif.set(tag::EXIF_VERSION, Value::Undefined(b"0232".to_vec()));
    if let Some(stamp) = &stamp {
        exif.ascii(tag::DATE_TIME_ORIGINAL, stamp)
            .ascii(tag::DATE_TIME_DIGITIZED, stamp);
    }
    if let Some(seconds) = spec.shutter_seconds {
        exif.set(tag::EXPOSURE_TIME, Value::Rational(vec![rational(seconds)]));
    }
    if let Some(f_number) = spec.aperture {
        exif.set(tag::F_NUMBER, Value::Rational(vec![rational(f_number)]));
    }
    match spec.iso {
        Some(iso) if iso < u32::from(u16::MAX) => {
            exif.short(tag::ISO, iso as u16);
        }
        Some(iso) => {
            // SensitivityType 3: the ISOSpeed tag carries the value.
            exif.short(tag::ISO, u16::MAX)
                .short(tag::SENSITIVITY_TYPE, 3)
                .long(tag::ISO_SPEED, iso);
        }
        None => {}
    }
    if let Some(bias) = spec.exposure_bias_ev {
        exif.set(
            tag::EXPOSURE_BIAS,
            Value::SRational(vec![signed_rational(bias)]),
        );
    }
    if let Some(fired) = spec.flash_fired {
        // Bit 0 is "flash fired"; 0x10 alone is "off, compulsory mode".
        exif.short(tag::FLASH, if fired { 0x01 } else { 0x10 });
    }
    if let Some(focal) = spec.focal_length_mm {
        exif.set(tag::FOCAL_LENGTH, Value::Rational(vec![rational(focal)]));
    }
    if let Some(focal) = spec.focal_length_35mm {
        exif.short(tag::FOCAL_LENGTH_35MM, focal.round() as u16);
    }
    if let Some(lens) = &spec.lens {
        exif.ascii(tag::LENS_MODEL, lens);
    }
    let exif_offset = writer.ifd(&exif);
    ifd0.long(tag::EXIF_IFD, exif_offset);

    if let Some(position) = spec.gps {
        let gps_offset = writer.ifd(&gps_ifd(position));
        ifd0.long(tag::GPS_IFD, gps_offset);
    }
}

/// A complete EXIF block (the TIFF stream that follows `Exif\0\0` in a JPEG
/// APP1 segment) for `spec`.
pub(crate) fn exif_block(spec: &ExifSpec) -> Vec<u8> {
    let mut writer = TiffWriter::new();
    let mut ifd0 = Ifd::new();
    add_exif(&mut writer, &mut ifd0, spec);
    let first = writer.ifd(&ifd0);
    writer.finish(first)
}

fn gps_ifd(position: GpsPosition) -> Ifd {
    let mut gps = Ifd::new();
    gps.set(tag::GPS_VERSION, Value::Byte(vec![2, 3, 0, 0]))
        .ascii(
            tag::GPS_LATITUDE_REF,
            if position.latitude < 0.0 { "S" } else { "N" },
        )
        .set(
            tag::GPS_LATITUDE,
            Value::Rational(degrees_minutes_seconds(position.latitude)),
        )
        .ascii(
            tag::GPS_LONGITUDE_REF,
            if position.longitude < 0.0 { "W" } else { "E" },
        )
        .set(
            tag::GPS_LONGITUDE,
            Value::Rational(degrees_minutes_seconds(position.longitude)),
        );
    if let Some(altitude) = position.altitude_m {
        // Reference 1 means below sea level.
        gps.set(
            tag::GPS_ALTITUDE_REF,
            Value::Byte(vec![u8::from(altitude < 0.0)]),
        )
        .set(
            tag::GPS_ALTITUDE,
            Value::Rational(vec![rational(altitude.abs())]),
        );
    }
    gps
}

fn exif_date_time(time: DateTime) -> String {
    format!(
        "{:04}:{:02}:{:02} {:02}:{:02}:{:02}",
        time.year, time.month, time.day, time.hour, time.minute, time.second
    )
}

fn degrees_minutes_seconds(angle: f64) -> Vec<(u32, u32)> {
    const SECOND_PARTS: f64 = 10_000.0;
    let total_parts = (angle.abs() * 3600.0 * SECOND_PARTS).round() as u64;
    let parts_per_minute = 60 * SECOND_PARTS as u64;
    let degrees = total_parts / (60 * parts_per_minute);
    let minutes = total_parts / parts_per_minute % 60;
    let seconds = total_parts % parts_per_minute;
    vec![
        (degrees as u32, 1),
        (minutes as u32, 1),
        (seconds as u32, SECOND_PARTS as u32),
    ]
}

/// The simplest fraction that reads back as exactly `value`: `1/n` for the
/// usual shutter speeds, otherwise a decimal fraction.
fn rational(value: f32) -> (u32, u32) {
    assert!(
        value.is_finite() && value >= 0.0,
        "cannot write {value} as an unsigned rational"
    );
    let reciprocal = (1.0 / value).round();
    if value > 0.0 && value < 1.0 && reciprocal <= u32::MAX as f32 && 1.0 / reciprocal == value {
        return (1, reciprocal as u32);
    }
    let (numerator, denominator) = signed_rational(value);
    (numerator as u32, denominator as u32)
}

/// As [`rational`], for values that may be negative; besides decimal
/// fractions it tries the small denominators exposure compensation is quoted
/// in (halves, thirds).
fn signed_rational(value: f32) -> (i32, i32) {
    const DENOMINATORS: [i32; 13] = [
        1, 10, 100, 2, 3, 4, 5, 6, 8, 1000, 10_000, 100_000, 1_000_000,
    ];
    assert!(value.is_finite(), "cannot write {value} as a rational");
    let fraction = |denominator: i32| ((value * denominator as f32).round() as i32, denominator);
    DENOMINATORS
        .into_iter()
        .map(fraction)
        .find(|&(numerator, denominator)| numerator as f32 / denominator as f32 == value)
        .unwrap_or_else(|| fraction(1_000_000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiff::read;

    #[test]
    fn rationals_read_back_as_the_value_that_was_asked_for() {
        assert_eq!(rational(1.0 / 250.0), (1, 250));
        assert_eq!(rational(1.0 / 8000.0), (1, 8000));
        assert_eq!(rational(0.4), (4, 10));
        assert_eq!(rational(30.0), (30, 1));
        assert_eq!(rational(5.6), (56, 10));
        assert_eq!(rational(0.0), (0, 1));
        assert_eq!(signed_rational(-1.0 / 3.0), (-1, 3));
        assert_eq!(signed_rational(2.0 / 3.0), (2, 3));
        assert_eq!(signed_rational(0.5), (5, 10));
        assert_eq!(signed_rational(-2.0), (-2, 1));
        for value in [1.0f32 / 250.0, 0.4, 5.6, 35.0, 1.4, 2.8, 0.001_25, 123.456] {
            let (n, d) = rational(value);
            assert_eq!(n as f32 / d as f32, value, "{value}");
        }
    }

    #[test]
    fn coordinates_split_into_degrees_minutes_and_seconds() {
        assert_eq!(
            degrees_minutes_seconds(47.556_25),
            [(47, 1), (33, 1), (225_000, 10_000)]
        );
        assert_eq!(
            degrees_minutes_seconds(-122.332_5),
            [(122, 1), (19, 1), (570_000, 10_000)]
        );
        assert_eq!(degrees_minutes_seconds(0.0), [(0, 1), (0, 1), (0, 10_000)]);
    }

    #[test]
    fn the_block_links_exif_and_gps_directories_from_the_first() {
        let block = exif_block(&ExifSpec {
            orientation: 6,
            ..ExifSpec::sample()
        });
        let ifd0 = read::ifd(&block, read::first_ifd(&block));
        assert_eq!(ifd0[&tag::ORIENTATION].u32_at(0), 6);
        assert_eq!(ifd0[&tag::MAKE].text(), "Photo Testkit");
        assert_eq!(ifd0[&tag::DATE_TIME].text(), "2024:03:15 07:42:10");

        let exif = read::ifd(&block, ifd0[&tag::EXIF_IFD].u32_at(0));
        assert_eq!(exif[&tag::DATE_TIME_ORIGINAL].text(), "2024:03:15 07:42:10");
        assert_eq!(exif[&tag::ISO].u32_at(0), 200);
        assert_eq!(exif[&tag::LENS_MODEL].text(), "Testkit 35mm F2");
        assert_eq!(exif[&tag::FLASH].u32_at(0) & 1, 0);
        assert_eq!(exif[&tag::FOCAL_LENGTH_35MM].u32_at(0), 52);

        let gps = read::ifd(&block, ifd0[&tag::GPS_IFD].u32_at(0));
        assert_eq!(gps[&tag::GPS_LATITUDE_REF].text(), "N");
        assert_eq!(gps[&tag::GPS_LONGITUDE_REF].text(), "W");
        assert_eq!(gps[&tag::GPS_ALTITUDE_REF].u32_at(0), 0);
    }

    #[test]
    fn an_empty_spec_writes_only_the_orientation() {
        let block = exif_block(&ExifSpec::default());
        let ifd0 = read::ifd(&block, read::first_ifd(&block));
        let tags: Vec<u16> = ifd0.keys().copied().collect();
        assert_eq!(tags, [tag::ORIENTATION, tag::EXIF_IFD]);
        let exif = read::ifd(&block, ifd0[&tag::EXIF_IFD].u32_at(0));
        assert_eq!(
            exif.keys().copied().collect::<Vec<_>>(),
            [tag::EXIF_VERSION]
        );
    }

    #[test]
    fn very_high_iso_saturates_the_short_tag_and_uses_the_long_one() {
        let block = exif_block(&ExifSpec {
            iso: Some(102_400),
            ..ExifSpec::default()
        });
        let ifd0 = read::ifd(&block, read::first_ifd(&block));
        let exif = read::ifd(&block, ifd0[&tag::EXIF_IFD].u32_at(0));
        assert_eq!(exif[&tag::ISO].u32_at(0), 65535);
        assert_eq!(exif[&tag::SENSITIVITY_TYPE].u32_at(0), 3);
        assert_eq!(exif[&tag::ISO_SPEED].u32_at(0), 102_400);
    }

    #[test]
    #[should_panic(expected = "1 to 8")]
    fn an_impossible_orientation_is_refused() {
        exif_block(&ExifSpec {
            orientation: 9,
            ..ExifSpec::default()
        });
    }
}
