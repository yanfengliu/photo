//! Which metadata an export carries, as EXIF directories and an XMP packet,
//! following [`MetadataMode`].
//!
//! EXIF text fields are written as UTF-8 in the `ASCII` type, as most
//! writers do; the XMP packet carries the same text as proper Unicode.

use photo_core::colorimetry::RgbSpace;
use photo_core::meta::GpsPosition;
use photo_core::time::DateTime;

use crate::ifd::{Ifd, TiffStream, Value};
use crate::item::ExportItem;
use crate::settings::{ExportSettings, MetadataMode};
use crate::xmp::{XmpFields, packet};

/// EXIF tag numbers (EXIF 2.32, TIFF 6.0).
mod tag {
    pub(super) const MAKE: u16 = 0x010f;
    pub(super) const MODEL: u16 = 0x0110;
    pub(super) const ORIENTATION: u16 = 0x0112;
    pub(super) const SOFTWARE: u16 = 0x0131;
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
    pub(super) const COLOR_SPACE: u16 = 0xa001;
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

/// The metadata of one export.
#[derive(Clone, Debug, Default)]
pub(crate) struct Metadata {
    /// Whether an EXIF block is written at all.
    exif: bool,
    /// Tags of the first directory.
    primary: Ifd,
    /// Tags of the EXIF directory.
    camera: Ifd,
    /// Tags of the GPS directory.
    gps: Ifd,
    /// The XMP packet, when there is anything to say.
    pub(crate) xmp: Option<String>,
}

impl Metadata {
    /// What `settings` lets `item` carry.
    pub(crate) fn new(item: &ExportItem, settings: &ExportSettings) -> Self {
        let mode = settings.metadata.include;
        if mode == MetadataMode::None {
            return Self::default();
        }
        let attributes = &item.attributes;
        let meta = item.metadata;
        let creator = text(&attributes.creator).or_else(|| text(&meta.artist));
        let rights = text(&attributes.copyright).or_else(|| text(&meta.copyright));
        let all = mode == MetadataMode::All;

        let mut primary = Ifd::new();
        primary.set(tag::ORIENTATION, Value::Short(vec![1]));
        set_text(&mut primary, tag::ARTIST, creator.as_deref());
        set_text(&mut primary, tag::COPYRIGHT, rights.as_deref());
        let (mut camera, mut gps) = (Ifd::new(), Ifd::new());
        if all {
            set_text(&mut primary, tag::MAKE, text(&meta.camera_make).as_deref());
            set_text(
                &mut primary,
                tag::MODEL,
                text(&meta.camera_model).as_deref(),
            );
            let software = format!("Photo {}", env!("CARGO_PKG_VERSION"));
            set_text(&mut primary, tag::SOFTWARE, Some(&software));
            camera = camera_directory(item, settings.color_space);
            if !settings.metadata.strip_location
                && let Some(position) = meta.gps
            {
                gps = gps_directory(position);
            }
        }

        let keywords: Vec<&str> = attributes
            .keywords
            .iter()
            .map(|keyword| keyword.trim())
            .filter(|keyword| !keyword.is_empty())
            .collect();
        let title = text(&attributes.title);
        let description = text(&attributes.caption);
        let rating = attributes.rating.stars();
        let fields = XmpFields {
            title: all.then_some(title.as_deref()).flatten(),
            description: all.then_some(description.as_deref()).flatten(),
            creator: creator.as_deref(),
            rights: rights.as_deref(),
            keywords: if all { &keywords } else { &[] },
            rating: (all && rating > 0).then_some(rating),
        };
        Self {
            exif: true,
            primary,
            camera,
            gps,
            xmp: packet(&fields),
        }
    }

    /// The EXIF block as JPEG's `APP1` and PNG's `eXIf` carry it: a TIFF
    /// stream (without the `Exif\0\0` prefix). `None` when no EXIF is
    /// written.
    pub(crate) fn exif_block(&self) -> Option<Vec<u8>> {
        if !self.exif {
            return None;
        }
        let mut stream = TiffStream::new();
        let mut primary = self.primary.clone();
        self.link_children(&mut primary, &mut stream);
        let first = stream.directory(&primary);
        Some(stream.finish(first))
    }

    /// Adds the EXIF tags to a TIFF file's own first directory, writing the
    /// EXIF and GPS directories into `stream`.
    pub(crate) fn add_to(&self, primary: &mut Ifd, stream: &mut TiffStream) {
        if !self.exif {
            return;
        }
        primary.merge(&self.primary);
        self.link_children(primary, stream);
    }

    fn link_children(&self, primary: &mut Ifd, stream: &mut TiffStream) {
        if !self.camera.is_empty() {
            let offset = stream.directory(&self.camera);
            primary.set(tag::EXIF_IFD, Value::Long(vec![offset]));
        }
        if !self.gps.is_empty() {
            let offset = stream.directory(&self.gps);
            primary.set(tag::GPS_IFD, Value::Long(vec![offset]));
        }
    }
}

/// The trimmed text, or `None` when blank.
fn text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn set_text(ifd: &mut Ifd, tag: u16, value: Option<&str>) {
    if let Some(value) = value {
        ifd.set(tag, Value::Ascii(value.to_string()));
    }
}

/// The EXIF directory: exposure, lens and capture time.
fn camera_directory(item: &ExportItem, space: RgbSpace) -> Ifd {
    let meta = item.metadata;
    let mut ifd = Ifd::new();
    ifd.set(tag::EXIF_VERSION, Value::Undefined(b"0232".to_vec()));
    // 1 declares sRGB; anything else is "uncalibrated", left to the ICC
    // profile.
    let color_space = if space == RgbSpace::Srgb { 1 } else { 0xffff };
    ifd.set(tag::COLOR_SPACE, Value::Short(vec![color_space]));
    let positive = |value: Option<f32>| value.filter(|v| v.is_finite() && *v > 0.0);
    if let Some(seconds) = positive(meta.shutter_seconds).and_then(rational) {
        ifd.set(tag::EXPOSURE_TIME, Value::Rational(vec![seconds]));
    }
    if let Some(f_number) = positive(meta.aperture).and_then(rational) {
        ifd.set(tag::F_NUMBER, Value::Rational(vec![f_number]));
    }
    if let Some(iso) = meta.iso.filter(|&iso| iso > 0) {
        ifd.set(
            tag::ISO,
            Value::Short(vec![iso.min(u32::from(u16::MAX)) as u16]),
        );
        if iso > u32::from(u16::MAX) {
            // EXIF 2.3: the 16-bit tag saturates and the ISO speed tag holds
            // the real figure.
            ifd.set(tag::SENSITIVITY_TYPE, Value::Short(vec![3]));
            ifd.set(tag::ISO_SPEED, Value::Long(vec![iso]));
        }
    }
    if let Some(time) = meta.capture_time {
        let stamp = exif_time(time);
        ifd.set(tag::DATE_TIME_ORIGINAL, Value::Ascii(stamp.clone()));
        ifd.set(tag::DATE_TIME_DIGITIZED, Value::Ascii(stamp));
    }
    if let Some(bias) = meta.exposure_bias_ev.filter(|v| v.is_finite())
        && let Some((numerator, denominator)) = rational(bias.abs())
    {
        let numerator = numerator.min(i32::MAX as u32) as i32;
        let signed = if bias < 0.0 { -numerator } else { numerator };
        ifd.set(
            tag::EXPOSURE_BIAS,
            Value::SRational(vec![(signed, denominator as i32)]),
        );
    }
    if let Some(fired) = meta.flash_fired {
        ifd.set(tag::FLASH, Value::Short(vec![u16::from(fired)]));
    }
    if let Some(length) = positive(meta.focal_length_mm).and_then(rational) {
        ifd.set(tag::FOCAL_LENGTH, Value::Rational(vec![length]));
    }
    if let Some(length) = positive(meta.focal_length_35mm) {
        let length = length.round().min(f32::from(u16::MAX)) as u16;
        ifd.set(tag::FOCAL_LENGTH_35MM, Value::Short(vec![length]));
    }
    set_text(&mut ifd, tag::LENS_MODEL, text(&meta.lens).as_deref());
    ifd
}

/// The GPS directory for `position`.
fn gps_directory(position: GpsPosition) -> Ifd {
    let mut ifd = Ifd::new();
    if !(position.latitude.is_finite() && position.longitude.is_finite()) {
        return ifd;
    }
    ifd.set(tag::GPS_VERSION, Value::Byte(vec![2, 3, 0, 0]));
    let hemisphere = |value: f64, positive: &str, negative: &str| {
        Value::Ascii(if value < 0.0 { negative } else { positive }.to_string())
    };
    ifd.set(
        tag::GPS_LATITUDE_REF,
        hemisphere(position.latitude, "N", "S"),
    );
    ifd.set(tag::GPS_LATITUDE, degrees(position.latitude));
    ifd.set(
        tag::GPS_LONGITUDE_REF,
        hemisphere(position.longitude, "E", "W"),
    );
    ifd.set(tag::GPS_LONGITUDE, degrees(position.longitude));
    if let Some(altitude) = position.altitude_m.filter(|v| v.is_finite()) {
        ifd.set(
            tag::GPS_ALTITUDE_REF,
            Value::Byte(vec![u8::from(altitude < 0.0)]),
        );
        let centimetres = (f64::from(altitude.abs()) * 100.0)
            .round()
            .min(f64::from(u32::MAX));
        ifd.set(
            tag::GPS_ALTITUDE,
            Value::Rational(vec![(centimetres as u32, 100)]),
        );
    }
    ifd
}

/// Degrees, minutes and seconds (to a thousandth) of an angle's magnitude.
fn degrees(angle: f64) -> Value {
    let angle = angle.abs().min(360.0);
    let whole = angle.floor();
    let minutes = ((angle - whole) * 60.0).floor();
    let thousandths = ((angle - whole - minutes / 60.0) * 3_600_000.0)
        .round()
        .max(0.0);
    Value::Rational(vec![
        (whole as u32, 1),
        (minutes as u32, 1),
        (thousandths as u32, 1000),
    ])
}

fn exif_time(time: DateTime) -> String {
    format!(
        "{:04}:{:02}:{:02} {:02}:{:02}:{:02}",
        time.year, time.month, time.day, time.hour, time.minute, time.second
    )
}

/// The simplest fraction within one part in ten thousand of `value`
/// (continued fractions), with numerator and denominator in `u32`: `1/250`
/// for 0.004, `14/5` for 2.8. `None` for values that are not finite and
/// non-negative or are too large.
fn rational(value: f32) -> Option<(u32, u32)> {
    let value = f64::from(value);
    if !value.is_finite() || value < 0.0 || value >= f64::from(u32::MAX) {
        return None;
    }
    let tolerance = value * 1e-4;
    let (mut numerator, mut denominator) = (value.floor(), 1.0);
    let (mut previous_numerator, mut previous_denominator) = (1.0, 0.0);
    let mut rest = value - value.floor();
    while (numerator / denominator - value).abs() > tolerance && rest > 1e-12 {
        let inverse = 1.0 / rest;
        let term = inverse.floor();
        let next = (
            term * numerator + previous_numerator,
            term * denominator + previous_denominator,
        );
        if next.0 > f64::from(u32::MAX) || next.1 > f64::from(u32::MAX) {
            break;
        }
        (previous_numerator, previous_denominator) = (numerator, denominator);
        (numerator, denominator) = next;
        rest = inverse - term;
    }
    Some((numerator as u32, denominator as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_numbers_become_the_fractions_cameras_write() {
        assert_eq!(rational(1.0 / 250.0), Some((1, 250)));
        assert_eq!(rational(1.0 / 8000.0), Some((1, 8000)));
        assert_eq!(rational(1.0 / 3.0), Some((1, 3)));
        assert_eq!(rational(2.8), Some((14, 5)));
        assert_eq!(rational(5.6), Some((28, 5)));
        assert_eq!(rational(30.0), Some((30, 1)));
        assert_eq!(rational(0.0), Some((0, 1)));
        assert_eq!(rational(f32::NAN), None);
        assert_eq!(rational(-1.0), None);
    }

    #[test]
    fn angles_split_into_degrees_minutes_and_seconds() {
        let Value::Rational(parts) = degrees(-122.3325) else {
            panic!("rational expected");
        };
        assert_eq!(parts, vec![(122, 1), (19, 1), (57_000, 1000)]);
    }
}
