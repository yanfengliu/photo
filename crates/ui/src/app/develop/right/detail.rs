//! The panels that are sliders alone: Detail (sharpening and noise
//! reduction), Lens Corrections, Transform, Effects (post-crop vignette and
//! grain) and Calibration.

use egui::Ui;
use photo_core::develop::Adj;

use super::{Editing, WIDE_LABEL, heading, slider, sliders, sliders_in};
use crate::app::Cx;

/// The Detail panel.
pub(super) fn detail(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    heading(ui, "Sharpening");
    sliders(
        ui,
        cx,
        editing,
        &[
            Adj::SharpenAmount,
            Adj::SharpenRadius,
            Adj::SharpenDetail,
            Adj::SharpenMasking,
        ],
        Some("Sharpening"),
    );
    heading(ui, "Noise Reduction");
    slider(ui, cx, editing, Adj::NoiseLuminance, Some("Noise"));
    sliders(
        ui,
        cx,
        editing,
        &[Adj::NoiseLuminanceDetail, Adj::NoiseLuminanceContrast],
        Some("Noise Luminance"),
    );
    slider(ui, cx, editing, Adj::NoiseColor, Some("Noise"));
    sliders(
        ui,
        cx,
        editing,
        &[Adj::NoiseColorDetail, Adj::NoiseColorSmoothness],
        Some("Noise Color"),
    );
}

/// The Lens Corrections panel.
pub(super) fn lens(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    heading(ui, "Distortion");
    slider(ui, cx, editing, Adj::LensDistortion, Some("Lens"));
    heading(ui, "Vignetting");
    slider(ui, cx, editing, Adj::LensVignetting, Some("Lens"));
    slider(
        ui,
        cx,
        editing,
        Adj::LensVignettingMidpoint,
        Some("Lens Vignetting"),
    );
}

/// The Transform panel.
pub(super) fn transform(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    sliders(
        ui,
        cx,
        editing,
        &[
            Adj::TransformVertical,
            Adj::TransformHorizontal,
            Adj::TransformRotate,
            Adj::TransformAspect,
            Adj::TransformScale,
            Adj::TransformXOffset,
            Adj::TransformYOffset,
        ],
        Some("Transform"),
    );
}

/// The Effects panel.
pub(super) fn effects(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    heading(ui, "Post-Crop Vignetting");
    sliders(
        ui,
        cx,
        editing,
        &[
            Adj::VignetteAmount,
            Adj::VignetteMidpoint,
            Adj::VignetteRoundness,
            Adj::VignetteFeather,
            Adj::VignetteHighlights,
        ],
        Some("Vignette"),
    );
    heading(ui, "Grain");
    sliders(
        ui,
        cx,
        editing,
        &[Adj::GrainAmount, Adj::GrainSize, Adj::GrainRoughness],
        Some("Grain"),
    );
}

/// The Calibration panel.
pub(super) fn calibration(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    heading(ui, "Shadows");
    sliders_in(
        ui,
        cx,
        editing,
        &[Adj::CalibrationShadowsTint],
        None,
        WIDE_LABEL,
    );
    for (primary, hue, saturation) in [
        (
            "Red Primary",
            Adj::CalibrationRedHue,
            Adj::CalibrationRedSaturation,
        ),
        (
            "Green Primary",
            Adj::CalibrationGreenHue,
            Adj::CalibrationGreenSaturation,
        ),
        (
            "Blue Primary",
            Adj::CalibrationBlueHue,
            Adj::CalibrationBlueSaturation,
        ),
    ] {
        heading(ui, primary);
        sliders_in(ui, cx, editing, &[hue, saturation], None, WIDE_LABEL);
    }
}
