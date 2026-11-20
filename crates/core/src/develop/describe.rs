//! Human-readable names for a change between two develop states, used as
//! history step labels when the caller has no better name (such as a preset
//! name or "Paste Settings").

use super::{Adj, DevelopSettings, GroupSet, SettingsGroup, Treatment};
use crate::meta::FileKind;

/// Describe how `after` differs from `before`, e.g. `Exposure +0.35`.
pub fn describe_change(before: &DevelopSettings, after: &DevelopSettings, kind: FileKind) -> String {
    if before == after {
        return "No Change".to_string();
    }
    if let Some(label) = structural_change(before, after) {
        return label;
    }
    let changed: Vec<Adj> = Adj::all().filter(|adj| adj.get(before) != adj.get(after)).collect();
    match changed.as_slice() {
        [adj] => {
            let spec = adj.spec_for(kind);
            format!("{} {}", adjustment_name(*adj), spec.format(adj.get(after)))
        }
        _ => group_change(before.differing_groups(after)),
    }
}

/// Changes that are not a single slider: treatment, profile, presets, and
/// edits to lists and shapes.
fn structural_change(before: &DevelopSettings, after: &DevelopSettings) -> Option<String> {
    let groups = before.differing_groups(after);
    let only = |group: SettingsGroup| groups == GroupSet::NONE.with(group);

    if only(SettingsGroup::Treatment) {
        if before.treatment != after.treatment {
            return Some(
                match after.treatment {
                    Treatment::BlackAndWhite => "Convert to Black & White",
                    Treatment::Color => "Convert to Color",
                }
                .to_string(),
            );
        }
        return Some(format!("Profile: {}", after.profile.label()));
    }
    if only(SettingsGroup::WhiteBalance) && before.white_balance.preset != after.white_balance.preset {
        return Some(format!("White Balance: {}", after.white_balance.preset.label()));
    }
    if only(SettingsGroup::Healing) {
        return Some(list_change("Spot Removal", before.spots.len(), after.spots.len()));
    }
    if only(SettingsGroup::Masking) {
        return Some(list_change("Mask", before.masks.len(), after.masks.len()));
    }
    if only(SettingsGroup::Crop) {
        if before.orientation != after.orientation {
            let mirrored = before.orientation.flip_horizontal != after.orientation.flip_horizontal;
            return Some(if mirrored { "Flip" } else { "Rotate" }.to_string());
        }
        if before.crop.rect != after.crop.rect {
            return Some("Crop Rectangle".to_string());
        }
        if before.crop.angle == after.crop.angle {
            return Some("Crop Aspect".to_string());
        }
    }
    if only(SettingsGroup::ToneCurve) && before.tone_curve.parametric == after.tone_curve.parametric {
        return Some("Point Curve".to_string());
    }
    None
}

fn list_change(noun: &str, before: usize, after: usize) -> String {
    match after.cmp(&before) {
        std::cmp::Ordering::Greater => format!("Add {noun}"),
        std::cmp::Ordering::Less => format!("Delete {noun}"),
        std::cmp::Ordering::Equal => format!("Update {noun}"),
    }
}

fn group_change(groups: GroupSet) -> String {
    let mut names = groups.iter().map(SettingsGroup::label);
    match (names.next(), names.next()) {
        (Some(single), None) => single.to_string(),
        _ => "Multiple Settings".to_string(),
    }
}

/// A slider's label, qualified by its panel where the bare label would be
/// ambiguous in a history list ("Amount" appears in three panels).
fn adjustment_name(adj: Adj) -> String {
    let spec = adj.spec();
    match adj {
        Adj::Temperature => "Temperature".to_string(),
        Adj::MixHue(_) => format!("{} Hue", spec.label),
        Adj::MixSaturation(_) => format!("{} Saturation", spec.label),
        Adj::MixLuminance(_) => format!("{} Luminance", spec.label),
        Adj::MixBw(_) => format!("B&W {}", spec.label),
        _ => match spec.group {
            SettingsGroup::ToneCurve => format!("Curve {}", spec.label),
            SettingsGroup::Sharpening => format!("Sharpening {}", spec.label),
            SettingsGroup::NoiseReduction => format!("Noise Reduction {}", spec.label),
            SettingsGroup::Vignette => format!("Vignette {}", spec.label),
            SettingsGroup::Grain => format!("Grain {}", spec.label),
            SettingsGroup::LensCorrections => format!("Lens {}", spec.label),
            SettingsGroup::Transform => format!("Transform {}", spec.label),
            SettingsGroup::ColorGrading => format!("{} {}", grading_range(adj), spec.label),
            SettingsGroup::Crop => "Crop Angle".to_string(),
            _ => spec.label.to_string(),
        },
    }
}

fn grading_range(adj: Adj) -> &'static str {
    match adj {
        Adj::GradeShadowsHue | Adj::GradeShadowsSaturation | Adj::GradeShadowsLuminance => "Shadows Grade",
        Adj::GradeMidtonesHue | Adj::GradeMidtonesSaturation | Adj::GradeMidtonesLuminance => {
            "Midtones Grade"
        }
        Adj::GradeHighlightsHue | Adj::GradeHighlightsSaturation | Adj::GradeHighlightsLuminance => {
            "Highlights Grade"
        }
        Adj::GradeGlobalHue | Adj::GradeGlobalSaturation | Adj::GradeGlobalLuminance => "Global Grade",
        _ => "Grading",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::develop::{ColorBand, CropRect, CurvePoint, Mask, MaskShape, PointCurve, Profile, WbPreset};
    use crate::geom::Point;

    fn describe(edit: impl FnOnce(&mut DevelopSettings)) -> String {
        let before = DevelopSettings::default();
        let mut after = before.clone();
        edit(&mut after);
        describe_change(&before, &after, FileKind::Raster)
    }

    #[test]
    fn a_single_slider_is_named_with_its_new_value() {
        assert_eq!(describe(|s| s.tone.exposure = 0.35), "Exposure +0.35");
        assert_eq!(describe(|s| s.tone.contrast = -12.0), "Contrast -12");
        assert_eq!(describe(|s| s.detail.sharpening.amount = 60.0), "Sharpening Amount 60");
        assert_eq!(describe(|s| s.effects.vignette.amount = -20.0), "Vignette Amount -20");
        assert_eq!(
            describe(|s| s.color_mixer.saturation[ColorBand::Blue.index()] = 15.0),
            "Blue Saturation +15"
        );
        assert_eq!(describe(|s| s.crop.angle = 2.5), "Crop Angle +2.50");
    }

    #[test]
    fn raw_temperature_reads_in_kelvin() {
        let mut before = DevelopSettings::default();
        before.white_balance.preset = WbPreset::Custom;
        before.white_balance.temp = 5000.0;
        let mut after = before.clone();
        after.white_balance.temp = 5600.0;
        assert_eq!(describe_change(&before, &after, FileKind::Raw), "Temperature 5600");
    }

    #[test]
    fn structural_changes_get_their_own_names() {
        assert_eq!(
            describe(|s| s.treatment = Treatment::BlackAndWhite),
            "Convert to Black & White"
        );
        assert_eq!(describe(|s| s.profile = Profile::Vivid), "Profile: Vivid");
        assert_eq!(
            describe(|s| s.white_balance.preset = WbPreset::Auto),
            "White Balance: Auto"
        );
        assert_eq!(describe(|s| s.orientation = s.orientation.rotated_cw()), "Rotate");
        assert_eq!(describe(|s| s.orientation = s.orientation.flipped_horizontal()), "Flip");
        assert_eq!(
            describe(|s| {
                s.crop.rect = Some(CropRect {
                    cx: 0.5,
                    cy: 0.5,
                    width: 0.5,
                    height: 0.5,
                });
            }),
            "Crop Rectangle"
        );
        assert_eq!(
            describe(|s| {
                s.tone_curve.rgb = PointCurve::new([
                    CurvePoint::new(0.0, 0.0),
                    CurvePoint::new(0.5, 0.6),
                    CurvePoint::new(1.0, 1.0),
                ]);
            }),
            "Point Curve"
        );
        assert_eq!(
            describe(|s| {
                s.masks.push(Mask::from_shape(
                    MaskShape::Linear {
                        start: Point::new(0.5, 0.0),
                        end: Point::new(0.5, 0.5),
                    },
                    1,
                ));
            }),
            "Add Mask"
        );
    }

    #[test]
    fn several_sliders_collapse_to_their_group_or_a_generic_label() {
        assert_eq!(
            describe(|s| {
                s.color_grading.shadows.hue = 220.0;
                s.color_grading.shadows.saturation = 30.0;
            }),
            "Color Grading"
        );
        assert_eq!(
            describe(|s| {
                s.tone.exposure = 1.0;
                s.presence.clarity = 10.0;
            }),
            "Multiple Settings"
        );
    }

    #[test]
    fn identical_settings_are_no_change() {
        assert_eq!(describe(|_| {}), "No Change");
    }
}
