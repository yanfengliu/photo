//! EXIF through kamadak-exif: finding the block in a file and turning its
//! fields into [`PhotoMetadata`].

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use exif::{Exif, In, Reader, Tag, Value};
use photo_core::meta::{GpsPosition, PhotoMetadata};
use photo_core::time::DateTime;

use crate::orient::Orientation;
use crate::tiff_meta;

/// How a file carries its EXIF block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Container {
    /// JPEG, PNG or WebP: the block is one chunk, found by scanning the
    /// chunk headers.
    Chunked,
    /// TIFF and TIFF-based raw files: the tags are spread through the file's
    /// own directories and are collected sparsely.
    Tiff,
}

/// The parsed EXIF block of one file.
pub(crate) struct ExifData(Exif);

/// Reads the EXIF block of the file at `path`. `None` when there is none or
/// it cannot be parsed: metadata is never worth failing a decode for.
/// Fields that parse are kept even when others in the block are broken.
pub(crate) fn read(path: &Path, container: Container) -> Option<ExifData> {
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    let parsed = match container {
        Container::Chunked => {
            let file = File::open(path).ok()?;
            reader.read_from_container(&mut BufReader::new(file))
        }
        Container::Tiff => reader.read_raw(tiff_meta::metadata_block(path)?),
    };
    let exif = parsed.or_else(|error| {
        error.distill_partial_result(|skipped| {
            log::debug!(
                "{}: {} unreadable EXIF fields skipped",
                path.display(),
                skipped.len()
            );
        })
    });
    match exif {
        Ok(exif) => Some(ExifData(exif)),
        Err(error) => {
            log::debug!("{}: no usable EXIF ({error})", path.display());
            None
        }
    }
}

impl ExifData {
    /// The orientation tag; upright when it is missing or out of range.
    pub(crate) fn orientation(&self) -> Orientation {
        Orientation::from_exif(self.unsigned(Tag::Orientation).unwrap_or(1))
    }

    /// Fills every field of `metadata` that is still empty from this block.
    /// Fields already set, and the dimensions, are left alone.
    pub(crate) fn fill(&self, metadata: &mut PhotoMetadata) {
        fn or<T>(slot: &mut Option<T>, value: impl FnOnce() -> Option<T>) {
            if slot.is_none() {
                *slot = value();
            }
        }
        or(&mut metadata.capture_time, || self.capture_time());
        or(&mut metadata.camera_make, || self.text(Tag::Make));
        or(&mut metadata.camera_model, || self.text(Tag::Model));
        or(&mut metadata.lens, || self.text(Tag::LensModel));
        or(&mut metadata.focal_length_mm, || {
            positive(self.number(Tag::FocalLength)?)
        });
        or(&mut metadata.focal_length_35mm, || {
            positive(self.number(Tag::FocalLengthIn35mmFilm)?)
        });
        or(&mut metadata.aperture, || self.aperture());
        or(&mut metadata.shutter_seconds, || self.shutter_seconds());
        or(&mut metadata.iso, || self.iso());
        or(&mut metadata.exposure_bias_ev, || {
            finite(self.number(Tag::ExposureBiasValue)?)
        });
        or(&mut metadata.flash_fired, || {
            self.unsigned(Tag::Flash).map(|flash| flash & 1 == 1)
        });
        or(&mut metadata.gps, || self.gps());
        or(&mut metadata.artist, || self.text(Tag::Artist));
        or(&mut metadata.copyright, || self.text(Tag::Copyright));
    }

    fn value(&self, tag: Tag) -> Option<&Value> {
        self.0.get_field(tag, In::PRIMARY).map(|field| &field.value)
    }

    /// The first non-empty string of an ASCII field, trimmed.
    fn text(&self, tag: Tag) -> Option<String> {
        let Value::Ascii(strings) = self.value(tag)? else {
            return None;
        };
        strings
            .iter()
            .map(|bytes| clean_text(&String::from_utf8_lossy(bytes)))
            .find(|text| !text.is_empty())
    }

    fn unsigned(&self, tag: Tag) -> Option<u32> {
        self.value(tag)?.get_uint(0)
    }

    /// The first value of a numeric field of any type.
    fn number(&self, tag: Tag) -> Option<f64> {
        numbers(self.value(tag)?).first().copied()
    }

    fn capture_time(&self) -> Option<DateTime> {
        [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
            .into_iter()
            .find_map(|tag| DateTime::parse_exif(&self.text(tag)?))
    }

    fn aperture(&self) -> Option<f32> {
        self.number(Tag::FNumber)
            .and_then(positive)
            .or_else(|| positive(aperture_from_apex(self.number(Tag::ApertureValue)?)))
    }

    fn shutter_seconds(&self) -> Option<f32> {
        self.number(Tag::ExposureTime)
            .and_then(positive)
            .or_else(|| positive(shutter_from_apex(self.number(Tag::ShutterSpeedValue)?)))
    }

    fn iso(&self) -> Option<u32> {
        let extended = [
            Tag::ISOSpeed,
            Tag::RecommendedExposureIndex,
            Tag::StandardOutputSensitivity,
        ];
        resolve_iso(
            self.unsigned(Tag::PhotographicSensitivity),
            extended.into_iter().find_map(|tag| self.unsigned(tag)),
        )
    }

    fn gps(&self) -> Option<GpsPosition> {
        let coordinate = |value: Tag, reference: Tag, negative: char| {
            let parts = numbers(self.value(value)?);
            let &[degrees, minutes, seconds] = parts.as_slice() else {
                return None;
            };
            let hemisphere = self.text(reference)?.chars().next()?;
            Some(signed_degrees(
                [degrees, minutes, seconds],
                hemisphere.eq_ignore_ascii_case(&negative),
            ))
        };
        let latitude = coordinate(Tag::GPSLatitude, Tag::GPSLatitudeRef, 'S')?;
        let longitude = coordinate(Tag::GPSLongitude, Tag::GPSLongitudeRef, 'W')?;
        let below_sea_level = self.unsigned(Tag::GPSAltitudeRef) == Some(1);
        let altitude = self.number(Tag::GPSAltitude);
        position(latitude, longitude, altitude, below_sea_level)
    }
}

/// Every value of a numeric field as `f64`; empty for non-numeric fields.
fn numbers(value: &Value) -> Vec<f64> {
    match value {
        Value::Byte(v) => v.iter().map(|&x| f64::from(x)).collect(),
        Value::Short(v) => v.iter().map(|&x| f64::from(x)).collect(),
        Value::Long(v) => v.iter().map(|&x| f64::from(x)).collect(),
        Value::SByte(v) => v.iter().map(|&x| f64::from(x)).collect(),
        Value::SShort(v) => v.iter().map(|&x| f64::from(x)).collect(),
        Value::SLong(v) => v.iter().map(|&x| f64::from(x)).collect(),
        Value::Rational(v) => v.iter().map(|x| x.to_f64()).collect(),
        Value::SRational(v) => v.iter().map(|x| x.to_f64()).collect(),
        Value::Float(v) => v.iter().map(|&x| f64::from(x)).collect(),
        Value::Double(v) => v.clone(),
        _ => Vec::new(),
    }
}

/// Trims the padding EXIF writers leave around strings.
pub(crate) fn clean_text(text: &str) -> String {
    text.trim_matches(|c: char| c.is_whitespace() || c == '\0')
        .to_string()
}

/// A finite value, narrowed to `f32`.
pub(crate) fn finite(value: f64) -> Option<f32> {
    value.is_finite().then_some(value as f32)
}

/// A finite value above zero, narrowed to `f32`. EXIF uses zero (and zero
/// denominators) for "unknown".
pub(crate) fn positive(value: f64) -> Option<f32> {
    finite(value).filter(|&v| v > 0.0)
}

/// f-number from an APEX aperture value: `N = 2^(Av / 2)`.
pub(crate) fn aperture_from_apex(apex: f64) -> f64 {
    (apex / 2.0).exp2()
}

/// Exposure time in seconds from an APEX shutter speed: `t = 2^(-Tv)`.
pub(crate) fn shutter_from_apex(apex: f64) -> f64 {
    (-apex).exp2()
}

/// The ISO speed from the 16-bit tag and the 32-bit tags that replace it
/// when it saturates (EXIF 2.3 `SensitivityType`): cameras write 65535 in
/// the short tag once the real figure no longer fits.
pub(crate) fn resolve_iso(short: Option<u32>, extended: Option<u32>) -> Option<u32> {
    match short.filter(|&iso| iso > 0) {
        Some(iso) if iso < u32::from(u16::MAX) => Some(iso),
        saturated => extended.filter(|&iso| iso > 0).or(saturated),
    }
}

/// Decimal degrees from degrees, minutes and seconds.
pub(crate) fn signed_degrees([degrees, minutes, seconds]: [f64; 3], negative: bool) -> f64 {
    let magnitude = degrees + minutes / 60.0 + seconds / 3600.0;
    if negative { -magnitude } else { magnitude }
}

/// A position, if the coordinates are on the globe.
pub(crate) fn position(
    latitude: f64,
    longitude: f64,
    altitude: Option<f64>,
    below_sea_level: bool,
) -> Option<GpsPosition> {
    let on_globe = latitude.abs() <= 90.0 && longitude.abs() <= 180.0;
    on_globe.then(|| GpsPosition {
        latitude,
        longitude,
        altitude_m: altitude
            .and_then(finite)
            .map(|metres| if below_sea_level { -metres } else { metres }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_prefers_the_short_tag_until_it_saturates() {
        assert_eq!(resolve_iso(Some(400), None), Some(400));
        assert_eq!(resolve_iso(Some(400), Some(51_200)), Some(400));
        assert_eq!(resolve_iso(Some(65_535), Some(102_400)), Some(102_400));
        assert_eq!(resolve_iso(Some(65_535), None), Some(65_535));
        assert_eq!(resolve_iso(None, Some(204_800)), Some(204_800));
        assert_eq!(resolve_iso(Some(0), None), None);
        assert_eq!(resolve_iso(Some(0), Some(0)), None);
        assert_eq!(resolve_iso(None, None), None);
    }

    #[test]
    fn apex_values_convert_to_the_numbers_photographers_use() {
        assert!((aperture_from_apex(5.0) - 5.657).abs() < 1e-3);
        assert!((aperture_from_apex(2.0) - 2.0).abs() < 1e-9);
        assert!((shutter_from_apex(8.0) - 1.0 / 256.0).abs() < 1e-12);
        assert!((shutter_from_apex(-1.0) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn zero_and_non_finite_readings_count_as_missing() {
        assert_eq!(positive(0.0), None);
        assert_eq!(positive(-2.0), None);
        assert_eq!(positive(f64::NAN), None);
        assert_eq!(positive(f64::INFINITY), None);
        assert_eq!(positive(2.8), Some(2.8));
        assert_eq!(finite(-0.5), Some(-0.5));
        assert_eq!(finite(f64::NEG_INFINITY), None);
    }

    #[test]
    fn coordinates_combine_and_take_their_sign_from_the_hemisphere() {
        assert!((signed_degrees([47.0, 33.0, 22.5], false) - 47.55625).abs() < 1e-12);
        assert!((signed_degrees([122.0, 19.0, 57.0], true) + 122.3325).abs() < 1e-12);
        let place = position(-33.86, 151.21, Some(12.0), true).unwrap();
        assert_eq!(place.altitude_m, Some(-12.0));
        assert_eq!(position(1.0, 2.0, None, false).unwrap().altitude_m, None);
        assert_eq!(position(91.0, 0.0, None, false), None);
        assert_eq!(position(0.0, 180.5, None, false), None);
        assert_eq!(position(f64::NAN, 0.0, None, false), None);
    }

    #[test]
    fn text_is_trimmed_of_padding() {
        assert_eq!(clean_text("  SONY \0\0"), "SONY");
        assert_eq!(clean_text("\0"), "");
        assert_eq!(clean_text("FE 24-70mm F2.8 GM"), "FE 24-70mm F2.8 GM");
    }
}
