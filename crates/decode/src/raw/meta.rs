//! Metadata of raw files: rawler's reading of the file first, kamadak-exif
//! for whatever rawler does not carry.

use std::path::Path;

use photo_core::meta::PhotoMetadata;
use photo_core::time::DateTime;
use rawler::decoders::{Decoder, RawDecodeParams, RawMetadata, WellKnownIFD};
use rawler::exif::{Exif, ExifGPS};
use rawler::formats::tiff::{Rational, SRational};
use rawler::rawsource::RawSource;
use rawler::tags::{DngTag, TiffCommonTag};

use crate::exif::{
    self as exif_block, Container, aperture_from_apex, clean_text, finite, position, positive,
    resolve_iso, shutter_from_apex, signed_degrees,
};

/// The camera names as the file spells them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CameraNames {
    pub(crate) make: String,
    pub(crate) model: String,
}

impl CameraNames {
    /// The `Make` and `Model` tags of a DNG's first directory, which is
    /// where rawler's DNG decoder takes them from when it decodes pixels.
    pub(crate) fn of_dng(decoder: &dyn Decoder) -> Self {
        let root = decoder.ifd(WellKnownIFD::Root).ok().flatten();
        let text = |tag: TiffCommonTag| {
            let entry = root.as_ref()?.get_entry(tag)?;
            entry.as_string().cloned()
        };
        Self {
            make: text(TiffCommonTag::Make).unwrap_or_default(),
            model: text(TiffCommonTag::Model).unwrap_or_default(),
        }
    }
}

/// rawler's metadata for the file, or `None` (logged) when it cannot be
/// read: a picture is worth decoding even if its metadata is not.
pub(crate) fn raw_metadata(decoder: &dyn Decoder, source: &RawSource) -> Option<RawMetadata> {
    match decoder.raw_metadata(source, &RawDecodeParams::default()) {
        Ok(metadata) => Some(metadata),
        Err(error) => {
            log::warn!(
                "{}: raw metadata unreadable ({error})",
                source.path().display()
            );
            None
        }
    }
}

/// The exposure offset a DNG asks renderers to apply (`BaselineExposure`),
/// 0.0 for other formats and when the tag is absent or not a number.
pub(crate) fn baseline_exposure(decoder: &dyn Decoder) -> f32 {
    let root = decoder.ifd(WellKnownIFD::Root).ok().flatten();
    let stops = root
        .as_ref()
        .and_then(|ifd| ifd.get_entry(DngTag::BaselineExposure))
        .and_then(|entry| entry.get_f32(0).ok().flatten());
    stops.filter(|stops| stops.is_finite()).unwrap_or(0.0)
}

/// Assembles the metadata of a raw file of upright size `width` x `height`.
///
/// Camera names are the file's own spelling (`names`), falling back to
/// rawler's cleaned-up ones. Everything rawler read from EXIF is used, and
/// fields it has no place for, or did not find, are then filled from the
/// file's EXIF directories through kamadak-exif when the file is
/// TIFF-structured.
pub(crate) fn photo_metadata(
    width: u32,
    height: u32,
    names: &CameraNames,
    raw: Option<&RawMetadata>,
    path: &Path,
) -> PhotoMetadata {
    let name = |own: &str, cleaned: Option<&String>| {
        Some(clean_text(own))
            .filter(|text| !text.is_empty())
            .or_else(|| {
                cleaned
                    .map(|text| clean_text(text))
                    .filter(|text| !text.is_empty())
            })
    };
    let mut metadata = PhotoMetadata {
        width,
        height,
        camera_make: name(&names.make, raw.map(|raw| &raw.make)),
        camera_model: name(&names.model, raw.map(|raw| &raw.model)),
        ..PhotoMetadata::default()
    };
    if let Some(raw) = raw {
        apply_exif(&raw.exif, &mut metadata);
    }
    if let Some(block) = exif_block::read(path, Container::Tiff) {
        block.fill(&mut metadata);
    }
    metadata
}

fn apply_exif(exif: &Exif, metadata: &mut PhotoMetadata) {
    let text = |value: &Option<String>| {
        value
            .as_deref()
            .map(clean_text)
            .filter(|text| !text.is_empty())
    };
    metadata.capture_time = [
        &exif.date_time_original,
        &exif.create_date,
        &exif.modify_date,
    ]
    .into_iter()
    .find_map(|stamp| DateTime::parse_exif(stamp.as_deref()?));
    metadata.lens = text(&exif.lens_model);
    metadata.focal_length_mm = exif.focal_length.and_then(unsigned).and_then(positive);
    metadata.aperture = exif
        .fnumber
        .and_then(unsigned)
        .and_then(positive)
        .or_else(|| positive(aperture_from_apex(unsigned(exif.aperture_value?)?)));
    metadata.shutter_seconds = exif
        .exposure_time
        .and_then(unsigned)
        .and_then(positive)
        .or_else(|| positive(shutter_from_apex(signed(exif.shutter_speed_value?)?)));
    metadata.iso = resolve_iso(
        exif.iso_speed_ratings.map(u32::from),
        exif.iso_speed.or(exif.recommended_exposure_index),
    );
    metadata.exposure_bias_ev = exif.exposure_bias.and_then(signed).and_then(finite);
    metadata.flash_fired = exif.flash.map(|flash| flash & 1 == 1);
    metadata.gps = exif.gps.as_ref().and_then(gps);
    metadata.artist = text(&exif.artist);
    metadata.copyright = text(&exif.copyright);
}

fn gps(gps: &ExifGPS) -> Option<photo_core::meta::GpsPosition> {
    let coordinate = |parts: &Option<[Rational; 3]>, reference: &Option<String>, negative: char| {
        let [degrees, minutes, seconds] = (*parts)?;
        let hemisphere = reference.as_deref()?.trim().chars().next()?;
        Some(signed_degrees(
            [unsigned(degrees)?, unsigned(minutes)?, unsigned(seconds)?],
            hemisphere.eq_ignore_ascii_case(&negative),
        ))
    };
    let latitude = coordinate(&gps.gps_latitude, &gps.gps_latitude_ref, 'S')?;
    let longitude = coordinate(&gps.gps_longitude, &gps.gps_longitude_ref, 'W')?;
    let altitude = gps.gps_altitude.and_then(unsigned);
    position(
        latitude,
        longitude,
        altitude,
        gps.gps_altitude_ref == Some(1),
    )
}

/// An unsigned rational as a float; `None` for a zero denominator.
fn unsigned(value: Rational) -> Option<f64> {
    (value.d != 0).then(|| f64::from(value.n) / f64::from(value.d))
}

/// A signed rational as a float; `None` for a zero denominator.
fn signed(value: SRational) -> Option<f64> {
    (value.d != 0).then(|| f64::from(value.n) / f64::from(value.d))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rational(n: u32, d: u32) -> Option<Rational> {
        Some(Rational::new(n, d))
    }

    #[test]
    fn every_exposure_field_is_carried_over() {
        let exif = Exif {
            date_time_original: Some("2024:03:15 07:42:10".into()),
            exposure_time: rational(1, 250),
            fnumber: rational(56, 10),
            iso_speed_ratings: Some(200),
            exposure_bias: Some(SRational::new(-1, 3)),
            flash: Some(0x10),
            focal_length: rational(35, 1),
            lens_model: Some(" Testkit 35mm F2 \0".into()),
            artist: Some("Test Kit".into()),
            copyright: Some(String::new()),
            ..Exif::default()
        };
        let mut metadata = PhotoMetadata::default();
        apply_exif(&exif, &mut metadata);
        assert_eq!(metadata.capture_time, DateTime::new(2024, 3, 15, 7, 42, 10));
        assert_eq!(metadata.shutter_seconds, Some(1.0 / 250.0));
        assert_eq!(metadata.aperture, Some(5.6));
        assert_eq!(metadata.iso, Some(200));
        assert_eq!(metadata.exposure_bias_ev, Some(-1.0 / 3.0));
        assert_eq!(metadata.flash_fired, Some(false));
        assert_eq!(metadata.focal_length_mm, Some(35.0));
        assert_eq!(metadata.lens.as_deref(), Some("Testkit 35mm F2"));
        assert_eq!(metadata.artist.as_deref(), Some("Test Kit"));
        assert_eq!(metadata.copyright, None, "an empty string is no copyright");
    }

    #[test]
    fn apex_values_stand_in_for_missing_plain_ones() {
        let exif = Exif {
            aperture_value: rational(4, 1),
            shutter_speed_value: Some(SRational::new(7, 1)),
            create_date: Some("2020:01:02 03:04:05".into()),
            ..Exif::default()
        };
        let mut metadata = PhotoMetadata::default();
        apply_exif(&exif, &mut metadata);
        assert_eq!(metadata.aperture, Some(4.0));
        assert_eq!(metadata.shutter_seconds, Some(1.0 / 128.0));
        assert_eq!(metadata.capture_time, DateTime::new(2020, 1, 2, 3, 4, 5));
    }

    #[test]
    fn zero_denominators_and_zero_readings_are_missing_values() {
        let exif = Exif {
            exposure_time: rational(1, 0),
            fnumber: rational(0, 10),
            focal_length: rational(0, 0),
            exposure_bias: Some(SRational::new(1, 0)),
            iso_speed_ratings: Some(0),
            date_time_original: Some("0000:00:00 00:00:00".into()),
            ..Exif::default()
        };
        let mut metadata = PhotoMetadata::default();
        apply_exif(&exif, &mut metadata);
        assert_eq!(metadata, PhotoMetadata::default());
    }

    #[test]
    fn saturated_iso_defers_to_the_long_tags() {
        let mut exif = Exif {
            iso_speed_ratings: Some(65535),
            recommended_exposure_index: Some(102_400),
            ..Exif::default()
        };
        let mut metadata = PhotoMetadata::default();
        apply_exif(&exif, &mut metadata);
        assert_eq!(metadata.iso, Some(102_400));
        exif.iso_speed = Some(204_800);
        apply_exif(&exif, &mut metadata);
        assert_eq!(metadata.iso, Some(204_800));
    }

    #[test]
    fn positions_need_both_coordinates_and_their_hemispheres() {
        let dms = |d, m, s| {
            Some([
                Rational::new(d, 1),
                Rational::new(m, 1),
                Rational::new(s, 10),
            ])
        };
        let full = ExifGPS {
            gps_latitude: dms(33, 51, 244),
            gps_latitude_ref: Some("S".into()),
            gps_longitude: dms(151, 12, 551),
            gps_longitude_ref: Some("E".into()),
            gps_altitude: rational(125, 10),
            gps_altitude_ref: Some(1),
            ..ExifGPS::default()
        };
        let place = gps(&full).unwrap();
        assert!((place.latitude + 33.856_777_8).abs() < 1e-6, "{place:?}");
        assert!((place.longitude - 151.215_305_6).abs() < 1e-6, "{place:?}");
        assert_eq!(place.altitude_m, Some(-12.5));

        let no_reference = ExifGPS {
            gps_latitude_ref: None,
            ..full.clone()
        };
        assert_eq!(gps(&no_reference), None);
        let no_longitude = ExifGPS {
            gps_longitude: None,
            ..full
        };
        assert_eq!(gps(&no_longitude), None);
    }

    #[test]
    fn the_files_own_camera_names_win_over_the_cleaned_ones() {
        let raw = RawMetadata {
            make: "Sony".into(),
            model: "ILCE-7RM3".into(),
            ..RawMetadata::default()
        };
        let own = CameraNames {
            make: "SONY ".into(),
            model: String::new(),
        };
        let path = Path::new("definitely/not/a/file.arw");
        let metadata = photo_metadata(60, 40, &own, Some(&raw), path);
        assert_eq!(metadata.camera_make.as_deref(), Some("SONY"));
        assert_eq!(metadata.camera_model.as_deref(), Some("ILCE-7RM3"));
        assert_eq!((metadata.width, metadata.height), (60, 40));
        let bare = photo_metadata(1, 1, &CameraNames::default(), None, path);
        assert_eq!((bare.camera_make, bare.camera_model), (None, None));
    }
}
