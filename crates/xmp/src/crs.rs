//! Develop settings in the Camera Raw vocabulary (`crs:`), under the names
//! its 2012 process version introduced.
//!
//! This is an exchange of slider positions and nothing more. A number means
//! the same slider at the same position on both sides, because the scales
//! follow the same conventions; what that position does to the picture is
//! decided by whichever application renders it. Nothing here makes this
//! application's rendering match another's.
//!
//! Local corrections, retouching and lens profiles are outside the mapping:
//! they are not read, and they are never written.

use std::borrow::Cow;

use photo_core::develop::{Adj, ColorBand, DevelopSettings, Treatment, WbPreset, WhiteBalance};
use photo_core::meta::FileKind;

use crate::rdf::{CRS, Form, Packet, Values};
use crate::values::{format_bool, format_number, parse_bool, parse_number};
use crate::{ImageInfo, curve, geometry};

/// Every scalar adjustment that has a Camera Raw property, and its name.
/// Supporting another one is one more row.
const SCALARS: &[(Adj, &str)] = &[
    (Adj::Exposure, "Exposure2012"),
    (Adj::Contrast, "Contrast2012"),
    (Adj::Highlights, "Highlights2012"),
    (Adj::Shadows, "Shadows2012"),
    (Adj::Whites, "Whites2012"),
    (Adj::Blacks, "Blacks2012"),
    (Adj::Texture, "Texture"),
    (Adj::Clarity, "Clarity2012"),
    (Adj::Dehaze, "Dehaze"),
    (Adj::Vibrance, "Vibrance"),
    (Adj::Saturation, "Saturation"),
    (Adj::CurveShadows, "ParametricShadows"),
    (Adj::CurveDarks, "ParametricDarks"),
    (Adj::CurveLights, "ParametricLights"),
    (Adj::CurveHighlights, "ParametricHighlights"),
    (Adj::CurveSplitShadows, "ParametricShadowSplit"),
    (Adj::CurveSplitMidtones, "ParametricMidtoneSplit"),
    (Adj::CurveSplitHighlights, "ParametricHighlightSplit"),
    (Adj::GradeShadowsHue, "SplitToningShadowHue"),
    (Adj::GradeShadowsSaturation, "SplitToningShadowSaturation"),
    (Adj::GradeShadowsLuminance, "ColorGradeShadowLum"),
    (Adj::GradeMidtonesHue, "ColorGradeMidtoneHue"),
    (Adj::GradeMidtonesSaturation, "ColorGradeMidtoneSat"),
    (Adj::GradeMidtonesLuminance, "ColorGradeMidtoneLum"),
    (Adj::GradeHighlightsHue, "SplitToningHighlightHue"),
    (
        Adj::GradeHighlightsSaturation,
        "SplitToningHighlightSaturation",
    ),
    (Adj::GradeHighlightsLuminance, "ColorGradeHighlightLum"),
    (Adj::GradeGlobalHue, "ColorGradeGlobalHue"),
    (Adj::GradeGlobalSaturation, "ColorGradeGlobalSat"),
    (Adj::GradeGlobalLuminance, "ColorGradeGlobalLum"),
    (Adj::GradeBlending, "ColorGradeBlending"),
    (Adj::GradeBalance, "SplitToningBalance"),
    (Adj::SharpenAmount, "Sharpness"),
    (Adj::SharpenRadius, "SharpenRadius"),
    (Adj::SharpenDetail, "SharpenDetail"),
    (Adj::SharpenMasking, "SharpenEdgeMasking"),
    (Adj::NoiseLuminance, "LuminanceSmoothing"),
    (Adj::NoiseLuminanceDetail, "LuminanceNoiseReductionDetail"),
    (
        Adj::NoiseLuminanceContrast,
        "LuminanceNoiseReductionContrast",
    ),
    (Adj::NoiseColor, "ColorNoiseReduction"),
    (Adj::NoiseColorDetail, "ColorNoiseReductionDetail"),
    (Adj::NoiseColorSmoothness, "ColorNoiseReductionSmoothness"),
    (Adj::LensDistortion, "LensManualDistortionAmount"),
    (Adj::LensVignetting, "VignetteAmount"),
    (Adj::LensVignettingMidpoint, "VignetteMidpoint"),
    (Adj::TransformVertical, "PerspectiveVertical"),
    (Adj::TransformHorizontal, "PerspectiveHorizontal"),
    (Adj::TransformRotate, "PerspectiveRotate"),
    (Adj::TransformAspect, "PerspectiveAspect"),
    (Adj::TransformScale, "PerspectiveScale"),
    (Adj::TransformXOffset, "PerspectiveX"),
    (Adj::TransformYOffset, "PerspectiveY"),
    (Adj::VignetteAmount, "PostCropVignetteAmount"),
    (Adj::VignetteMidpoint, "PostCropVignetteMidpoint"),
    (Adj::VignetteRoundness, "PostCropVignetteRoundness"),
    (Adj::VignetteFeather, "PostCropVignetteFeather"),
    (Adj::VignetteHighlights, "PostCropVignetteHighlightContrast"),
    (Adj::GrainAmount, "GrainAmount"),
    (Adj::GrainSize, "GrainSize"),
    (Adj::GrainRoughness, "GrainFrequency"),
    (Adj::CalibrationShadowsTint, "ShadowTint"),
    (Adj::CalibrationRedHue, "RedHue"),
    (Adj::CalibrationRedSaturation, "RedSaturation"),
    (Adj::CalibrationGreenHue, "GreenHue"),
    (Adj::CalibrationGreenSaturation, "GreenSaturation"),
    (Adj::CalibrationBlueHue, "BlueHue"),
    (Adj::CalibrationBlueSaturation, "BlueSaturation"),
];

/// The adjustment a per-band family has for one band.
type BandFamily = fn(ColorBand) -> Adj;

/// The four per-band families, each with the property name its band names
/// are appended to.
const BAND_FAMILIES: [(BandFamily, &str); 4] = [
    (Adj::MixHue, "HueAdjustment"),
    (Adj::MixSaturation, "SaturationAdjustment"),
    (Adj::MixLuminance, "LuminanceAdjustment"),
    (Adj::MixBw, "GrayMixer"),
];

/// Band names as Camera Raw spells them, indexed by [`ColorBand::index`].
const BAND_NAMES: [&str; ColorBand::COUNT] = [
    "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta",
];

/// Values of `crs:WhiteBalance`.
const WHITE_BALANCE_NAMES: [(WbPreset, &str); 9] = [
    (WbPreset::AsShot, "As Shot"),
    (WbPreset::Auto, "Auto"),
    (WbPreset::Daylight, "Daylight"),
    (WbPreset::Cloudy, "Cloudy"),
    (WbPreset::Shade, "Shade"),
    (WbPreset::Tungsten, "Tungsten"),
    (WbPreset::Fluorescent, "Fluorescent"),
    (WbPreset::Flash, "Flash"),
    (WbPreset::Custom, "Custom"),
];

const WHITE_BALANCE: &str = "WhiteBalance";
const GRAYSCALE: &str = "ConvertToGrayscale";

/// The process version declared beside the properties written here. The
/// 2012 names they carry (`Exposure2012` and the rest) are the ones Camera
/// Raw reads from process version 6.7 onwards; 11.0 is the one beside them
/// in both test fixtures written in the form Adobe software writes. Without
/// a process version, Adobe software may take the settings for an older
/// process, which reads other names.
const PROCESS_VERSION: &str = "11.0";
/// The Camera Raw version declared with [`PROCESS_VERSION`], as in the same
/// fixtures.
const CAMERA_RAW_VERSION: &str = "15.4";
/// The first process version whose properties carry the 2012 names. One
/// from there on already agrees with the names written here, and is kept.
const FIRST_2012_PROCESS: f64 = 6.7;

/// Every adjustment the table maps, with its property name.
fn mapped() -> impl Iterator<Item = (Adj, Cow<'static, str>)> {
    let scalars = SCALARS
        .iter()
        .map(|&(adj, name)| (adj, Cow::Borrowed(name)));
    let bands = BAND_FAMILIES.iter().flat_map(|&(family, prefix)| {
        ColorBand::ALL.into_iter().map(move |band| {
            (
                family(band),
                Cow::Owned(format!("{prefix}{}", BAND_NAMES[band.index()])),
            )
        })
    });
    scalars.chain(bands)
}

/// The temperature and tint properties for a kind of file. A raster file's
/// white balance is an offset from the file as it is, which Camera Raw keeps
/// apart from the absolute kelvin and tint of a RAW file.
const fn white_balance_properties(kind: FileKind) -> (&'static str, &'static str) {
    match kind {
        FileKind::Raw => ("Temperature", "Tint"),
        FileKind::Raster => ("IncrementalTemperature", "IncrementalTint"),
    }
}

fn number(crs: &Values, name: &str) -> Option<f32> {
    crs.get(name).and_then(|text| parse_number(text))
}

/// The white balance the properties describe, or `None` when the sidecar
/// has none of them.
fn read_white_balance(crs: &Values, kind: FileKind) -> Option<WhiteBalance> {
    let (temperature, tint) = white_balance_properties(kind);
    let name = crs.get(WHITE_BALANCE).map(|text| text.trim());
    let temp = number(crs, temperature);
    let tint = number(crs, tint);
    if name.is_none() && temp.is_none() && tint.is_none() {
        return None;
    }
    // Numbers under a missing or unfamiliar name are a balance set by hand.
    let preset = WHITE_BALANCE_NAMES
        .iter()
        .find(|(_, known)| name.is_some_and(|name| name.eq_ignore_ascii_case(known)))
        .map_or(WbPreset::Custom, |(preset, _)| *preset);
    if preset == WbPreset::AsShot {
        return Some(WhiteBalance::default());
    }
    let illuminant = match kind {
        FileKind::Raw => preset.illuminant(),
        FileKind::Raster => None,
    };
    let fallback = match kind {
        FileKind::Raw => illuminant.map(|(kelvin, _)| kelvin),
        // A relative balance that names only a tint leaves the temperature alone.
        FileKind::Raster => tint.map(|_| 0.0),
    };
    Some(match temp.or(fallback) {
        Some(temp) => WhiteBalance {
            preset,
            temp,
            tint: tint.or(illuminant.map(|(_, tint)| tint)).unwrap_or(0.0),
        },
        // Without a temperature there is nothing to apply.
        None => WhiteBalance::default(),
    })
}

fn write_white_balance(
    packet: &mut Packet,
    crs: &Values,
    white_balance: &WhiteBalance,
    kind: FileKind,
) {
    let as_shot = white_balance.preset == WbPreset::AsShot;
    let wanted = if as_shot {
        WhiteBalance::default()
    } else {
        *white_balance
    };
    if crs.contains_key(WHITE_BALANCE) && read_white_balance(crs, kind) == Some(wanted) {
        return;
    }
    let name = WHITE_BALANCE_NAMES
        .iter()
        .find(|(preset, _)| *preset == wanted.preset)
        .map_or("Custom", |(_, name)| *name);
    packet.set_text(CRS.property(WHITE_BALANCE), name, Form::Attribute);
    let (temperature, tint) = white_balance_properties(kind);
    if as_shot {
        // The numbers of an earlier balance would contradict the name.
        packet.remove(CRS.property(temperature));
        packet.remove(CRS.property(tint));
    } else {
        put_number(
            packet,
            crs,
            temperature,
            wanted.temp,
            is_signed(Adj::Temperature, kind),
        );
        put_number(packet, crs, tint, wanted.tint, is_signed(Adj::Tint, kind));
    }
}

fn is_signed(adj: Adj, kind: FileKind) -> bool {
    adj.spec_for(kind).min < 0.0
}

/// Sets a numeric property unless it already holds that number, so a value
/// another application wrote keeps its spelling until it changes.
fn put_number(packet: &mut Packet, crs: &Values, name: &str, value: f32, signed: bool) {
    let property = CRS.property(name);
    match format_number(value, signed) {
        Some(_) if number(crs, name) == Some(value) => {}
        Some(text) => packet.set_text(property, &text, Form::Attribute),
        None => packet.remove(property),
    }
}

/// Reads the settings the Camera Raw properties describe, on top of the
/// starting settings for the kind of file. `None` when the sidecar holds
/// none of the mapped properties.
pub(crate) fn read(packet: &Packet, image: ImageInfo) -> Option<DevelopSettings> {
    let crs = packet.plain_values(CRS);
    let mut settings = DevelopSettings::default_for(image.kind);
    let mut found = false;
    for (adj, name) in mapped() {
        if let Some(value) = number(&crs, &name) {
            adj.set(&mut settings, value);
            found = true;
        }
    }
    if let Some(white_balance) = read_white_balance(&crs, image.kind) {
        settings.white_balance = white_balance;
        found = true;
    }
    if let Some(grayscale) = crs.get(GRAYSCALE).and_then(|text| parse_bool(text)) {
        settings.treatment = if grayscale {
            Treatment::BlackAndWhite
        } else {
            Treatment::Color
        };
        found = true;
    }
    found |= curve::read(packet, &mut settings.tone_curve);
    found |= geometry::read(packet, &crs, image, &mut settings);
    found.then_some(settings)
}

/// Writes the properties that say what the settings are: the Camera Raw and
/// process versions, unless the packet declares a process whose properties
/// already carry the 2012 names, and `crs:HasSettings`.
fn write_header(packet: &mut Packet, crs: &Values) {
    let process = crs
        .get("ProcessVersion")
        .and_then(|text| parse_number::<f64>(text));
    if process.is_none_or(|process| process < FIRST_2012_PROCESS) {
        // An older Camera Raw cannot have written the newer process.
        packet.set_text(CRS.property("Version"), CAMERA_RAW_VERSION, Form::Attribute);
        let property = CRS.property("ProcessVersion");
        packet.set_text(property, PROCESS_VERSION, Form::Attribute);
    } else if !crs.contains_key("Version") {
        packet.set_text(CRS.property("Version"), CAMERA_RAW_VERSION, Form::Attribute);
    }
    if crs.get("HasSettings").and_then(|text| parse_bool(text)) != Some(true) {
        packet.set_text(
            CRS.property("HasSettings"),
            format_bool(true),
            Form::Attribute,
        );
    }
}

/// Writes every mapped setting, touching only the properties whose value
/// differs from what the packet already says.
pub(crate) fn write(packet: &mut Packet, settings: &DevelopSettings, image: ImageInfo) {
    let crs = packet.plain_values(CRS);
    write_header(packet, &crs);
    for (adj, name) in mapped() {
        put_number(
            packet,
            &crs,
            &name,
            adj.get(settings),
            is_signed(adj, image.kind),
        );
    }
    write_white_balance(packet, &crs, &settings.white_balance, image.kind);
    let grayscale = settings.treatment == Treatment::BlackAndWhite;
    if crs.get(GRAYSCALE).and_then(|text| parse_bool(text)) != Some(grayscale) {
        packet.set_text(
            CRS.property(GRAYSCALE),
            format_bool(grayscale),
            Form::Attribute,
        );
    }
    curve::write(packet, &settings.tone_curve);
    geometry::write(packet, &crs, image, settings);
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// Adjustments that travel outside the table: white balance with its
    /// preset, the crop angle with the crop rectangle.
    const ELSEWHERE: [Adj; 3] = [Adj::Temperature, Adj::Tint, Adj::CropAngle];

    #[test]
    fn every_adjustment_is_mapped_exactly_once_or_handled_elsewhere() {
        let mut seen = HashSet::new();
        for (adj, _) in mapped() {
            assert!(seen.insert(adj), "{adj:?} is mapped twice");
            assert!(!ELSEWHERE.contains(&adj), "{adj:?} is mapped twice over");
        }
        for adj in Adj::all() {
            assert!(
                seen.contains(&adj) || ELSEWHERE.contains(&adj),
                "{adj:?} has no Camera Raw property: add a row or list it as handled elsewhere"
            );
        }
    }

    #[test]
    fn property_names_are_unique() {
        let mut names = HashSet::new();
        for (_, name) in mapped() {
            assert!(names.insert(name.clone()), "{name} is used twice");
        }
        for reserved in [
            WHITE_BALANCE,
            GRAYSCALE,
            "Temperature",
            "Tint",
            "CropAngle",
            "HasCrop",
        ] {
            assert!(!names.contains(reserved), "{reserved} is used twice");
        }
        assert_eq!(names.len(), SCALARS.len() + 4 * ColorBand::COUNT);
    }

    #[test]
    fn every_white_balance_preset_has_a_name() {
        for preset in WbPreset::ALL {
            let names = WHITE_BALANCE_NAMES
                .iter()
                .filter(|(known, _)| *known == preset);
            assert_eq!(names.count(), 1, "{preset:?}");
        }
    }

    #[test]
    fn a_named_illuminant_supplies_the_numbers_a_raw_sidecar_leaves_out() {
        let crs = Values::from([(WHITE_BALANCE.to_owned(), "Tungsten".to_owned())]);
        let expected = WhiteBalance {
            preset: WbPreset::Tungsten,
            temp: 2850.0,
            tint: 0.0,
        };
        assert_eq!(read_white_balance(&crs, FileKind::Raw), Some(expected));
        assert_eq!(
            read_white_balance(&crs, FileKind::Raster),
            Some(WhiteBalance::default())
        );
        assert_eq!(read_white_balance(&Values::new(), FileKind::Raw), None);
    }

    #[test]
    fn numbers_without_a_known_name_are_a_custom_balance() {
        let unnamed = Values::from([("Temperature".to_owned(), "4800".to_owned())]);
        let custom = WhiteBalance {
            preset: WbPreset::Custom,
            temp: 4800.0,
            tint: 0.0,
        };
        assert_eq!(read_white_balance(&unnamed, FileKind::Raw), Some(custom));

        let auto_without_numbers = Values::from([(WHITE_BALANCE.to_owned(), "Auto".to_owned())]);
        assert_eq!(
            read_white_balance(&auto_without_numbers, FileKind::Raw),
            Some(WhiteBalance::default())
        );

        let tint_only = Values::from([("IncrementalTint".to_owned(), "-6".to_owned())]);
        let relative = WhiteBalance {
            preset: WbPreset::Custom,
            temp: 0.0,
            tint: -6.0,
        };
        assert_eq!(
            read_white_balance(&tint_only, FileKind::Raster),
            Some(relative)
        );
    }
}
