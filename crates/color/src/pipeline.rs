//! The per-pixel pipeline compiled for one render.
//!
//! [`PixelPipeline`] is built once from the settings and the source's colour
//! description and then called per pixel. It exposes the point stages of
//! DESIGN §5.2 as separate steps, because the pipeline's spatial stages run
//! between them:
//!
//! | Step | Stage | In | Out |
//! | --- | --- | --- | --- |
//! | [`PixelPipeline::input`] | 3 | source-native RGB | white-balanced working RGB |
//! | [`PixelPipeline::tone_base`] | (5) | stage 3's output | the value to smooth into the base plane |
//! | [`PixelPipeline::scene_to_display`] | 5 | scene-linear + base | display-linear |
//! | [`PixelPipeline::tone`] | 6 | display-linear | display-linear |
//! | [`PixelPipeline::color`] | 7 | display-linear | display-linear |
//! | [`PixelPipeline::output_u8`] and friends | 11 | display-linear | encoded |
//!
//! Stage 5 holds exposure, Highlights and Shadows (a scene-linear gain
//! looked up from the edge-aware base) and the base curve; stage 6 holds
//! Whites, Blacks, Contrast and the tone curves.
//!
//! Every step takes the pixel's [`LocalDelta`]: the alpha-weighted sum of the
//! mask adjustments covering it, or `None` outside every mask. Without a
//! delta a step only reads tables built up front; with one, the affected
//! curves are evaluated directly.

use photo_core::colorimetry::{
    IDENTITY, Mat3, RgbSpace, WORKING_SPACE, mat3_apply, mat3_mul, rgb_to_rgb,
};
use photo_core::develop::{DevelopSettings, LocalAdjustments, WbPreset, WhiteBalance};
use photo_core::image::SourceColor;

use crate::base::{SceneDelta, SceneToDisplay};
use crate::camera::{CameraInput, camera_as_shot_temp_tint, pick_camera_white_balance};
use crate::color::ColorStage;
use crate::math::non_negative;
use crate::output::OutputTransform;
use crate::relative_wb::{
    RASTER_TEMP_LIMIT, adapt, pick_raster_white_balance, relative_wb_gains, relative_wb_matrix,
};
use crate::tone::{ToneDelta, ToneStage};

/// The sum of the mask adjustments covering one pixel, each weighted by the
/// mask's alpha at that pixel and its amount: `Σ alphaᵢ · amountᵢ · localᵢ`.
/// Units are those of [`LocalAdjustments`]: exposure in stops, the rest in
/// slider units. Temperature and tint are relative offsets for every kind
/// of source.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LocalDelta {
    /// Stops added to the exposure (stage 5).
    pub exposure: f32,
    /// Added to Contrast (stage 6).
    pub contrast: f32,
    /// Added to Highlights (stage 5).
    pub highlights: f32,
    /// Added to Shadows (stage 5).
    pub shadows: f32,
    /// Added to Whites (stage 6).
    pub whites: f32,
    /// Added to Blacks (stage 6).
    pub blacks: f32,
    /// Relative temperature offset, positive is warmer (stage 3).
    pub temp: f32,
    /// Relative tint offset, positive is more magenta (stage 3).
    pub tint: f32,
    /// Added to Saturation (stage 7).
    pub saturation: f32,
}

impl LocalDelta {
    /// Add one mask's adjustments, scaled by `weight` (the mask's alpha at
    /// this pixel times its amount as a fraction).
    #[inline]
    pub fn add(&mut self, adjust: &LocalAdjustments, weight: f32) {
        self.exposure += adjust.exposure * weight;
        self.contrast += adjust.contrast * weight;
        self.highlights += adjust.highlights * weight;
        self.shadows += adjust.shadows * weight;
        self.whites += adjust.whites * weight;
        self.blacks += adjust.blacks * weight;
        self.temp += adjust.temp * weight;
        self.tint += adjust.tint * weight;
        self.saturation += adjust.saturation * weight;
    }

    /// True when every field is zero.
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }

    /// True when the delta changes stage 3 (temperature or tint).
    #[inline]
    pub fn affects_input(&self) -> bool {
        self.temp != 0.0 || self.tint != 0.0
    }

    /// True when the delta changes stage 5 (exposure, highlights or
    /// shadows).
    #[inline]
    pub fn affects_scene(&self) -> bool {
        self.exposure != 0.0 || self.highlights != 0.0 || self.shadows != 0.0
    }

    /// True when the delta changes stage 6 (contrast, whites or blacks).
    #[inline]
    pub fn affects_tone(&self) -> bool {
        self.contrast != 0.0 || self.whites != 0.0 || self.blacks != 0.0
    }

    /// True when the delta changes stage 7 (saturation).
    #[inline]
    pub fn affects_color(&self) -> bool {
        self.saturation != 0.0
    }

    fn scene(&self) -> SceneDelta {
        SceneDelta {
            exposure: self.exposure,
            highlights: self.highlights,
            shadows: self.shadows,
        }
    }

    fn tone(&self) -> ToneDelta {
        ToneDelta {
            contrast: self.contrast,
            whites: self.whites,
            blacks: self.blacks,
        }
    }
}

/// The relative temperature and tint a raster source is rendered with:
/// zero for "As Shot" (the file as it is), otherwise the setting's numbers
/// held to the slider range.
fn raster_temp_tint(white_balance: &WhiteBalance) -> (f32, f32) {
    if white_balance.preset == WbPreset::AsShot {
        return (0.0, 0.0);
    }
    let held = |value: f32| {
        if value.is_finite() {
            value.clamp(-RASTER_TEMP_LIMIT, RASTER_TEMP_LIMIT)
        } else {
            0.0
        }
    };
    (held(white_balance.temp), held(white_balance.tint))
}

/// The temperature and tint to show for a white-balance setting: the
/// camera's as-shot values (kelvin, DNG tint) or `(0, 0)` for a raster file
/// when the preset is "As Shot", otherwise the setting's own numbers.
pub fn displayed_white_balance(white_balance: &WhiteBalance, source: &SourceColor) -> (f32, f32) {
    match (white_balance.preset, source) {
        (WbPreset::AsShot, SourceColor::Camera(camera)) => camera_as_shot_temp_tint(camera),
        (WbPreset::AsShot, SourceColor::Rgb(_)) => (0.0, 0.0),
        _ => (white_balance.temp, white_balance.tint),
    }
}

/// The white-balance picker: the custom white balance that renders `sample`
/// neutral. `sample` is a source-native pixel (what [`PixelPipeline::input`]
/// takes): camera-native RGB for a camera source, linear RGB in the file's
/// space for a raster source. `None` when the sample has a non-positive or
/// non-finite channel.
pub fn pick_white_balance(source: &SourceColor, sample: [f32; 3]) -> Option<WhiteBalance> {
    match source {
        SourceColor::Camera(camera) => pick_camera_white_balance(camera, sample),
        SourceColor::Rgb(space) => pick_raster_white_balance(*space, sample),
    }
}

#[derive(Clone, Copy, Debug)]
enum Input {
    Camera(CameraInput),
    Raster {
        /// File space to working space; `None` when they are the same.
        to_working: Option<Mat3>,
        /// `to_working` followed by the global white balance, as one
        /// matrix; `None` when both are the identity.
        combined: Option<Mat3>,
        temp: f32,
        tint: f32,
    },
}

/// The colour math of one render, compiled. Building it is cheap enough to
/// do on every settings change (well under a millisecond); applying it never
/// allocates.
#[derive(Clone, Debug)]
pub struct PixelPipeline {
    input: Input,
    scene: SceneToDisplay,
    tone: ToneStage,
    color: ColorStage,
    output: OutputTransform,
    masks_need_base: bool,
}

impl PixelPipeline {
    /// Compile `settings` for a source with the given colour description.
    /// Output is encoded as sRGB until [`Self::with_output_space`] says
    /// otherwise.
    pub fn new(settings: &DevelopSettings, source: &SourceColor) -> Self {
        let input = match source {
            SourceColor::Camera(camera) => {
                Input::Camera(CameraInput::new(camera, &settings.white_balance))
            }
            SourceColor::Rgb(space) => {
                let to_working =
                    (*space != WORKING_SPACE).then(|| rgb_to_rgb(*space, WORKING_SPACE));
                let (temp, tint) = raster_temp_tint(&settings.white_balance);
                let balance = relative_wb_matrix(temp, tint);
                let combined = match (to_working, balance == IDENTITY) {
                    (None, true) => None,
                    (Some(matrix), true) => Some(matrix),
                    (None, false) => Some(balance),
                    (Some(matrix), false) => Some(mat3_mul(&balance, &matrix)),
                };
                Input::Raster {
                    to_working,
                    combined,
                    temp,
                    tint,
                }
            }
        };
        let masks_need_base = settings.masks.iter().any(|mask| {
            !mask.is_inert() && (mask.adjust.highlights != 0.0 || mask.adjust.shadows != 0.0)
        });
        Self {
            input,
            scene: SceneToDisplay::new(settings.profile, &settings.tone, source),
            tone: ToneStage::new(&settings.tone, &settings.tone_curve),
            color: ColorStage::new(settings),
            output: OutputTransform::new(RgbSpace::Srgb),
            masks_need_base,
        }
    }

    /// The same pipeline encoding its output in `space`.
    pub fn with_output_space(mut self, space: RgbSpace) -> Self {
        self.output = OutputTransform::new(space);
        self
    }

    /// The colour space the output steps encode into.
    pub fn output_space(&self) -> RgbSpace {
        self.output.space()
    }

    /// Stage 3: a source-native pixel to white-balanced linear working RGB.
    ///
    /// Camera sources: white-balance gains, highlight reconstruction, then
    /// the camera-to-working matrix; values above 1.0 are kept. Raster
    /// sources: file space to working space, then the relative white
    /// balance. A delta's `temp` and `tint` are added to the global values
    /// for raster sources and applied as a further relative adaptation for
    /// camera sources. The result is never negative.
    #[inline]
    pub fn input(&self, source: [f32; 3], delta: Option<&LocalDelta>) -> [f32; 3] {
        let local = match delta {
            Some(delta) if delta.affects_input() => Some((delta.temp, delta.tint)),
            _ => None,
        };
        match (&self.input, local) {
            (Input::Camera(camera), None) => camera.apply(source),
            (Input::Camera(camera), Some((temp, tint))) => {
                adapt(camera.apply(source), relative_wb_gains(temp, tint))
            }
            (Input::Raster { combined, .. }, None) => match combined {
                Some(matrix) => non_negative(mat3_apply(matrix, source)),
                None => non_negative(source),
            },
            (
                Input::Raster {
                    to_working,
                    temp,
                    tint,
                    ..
                },
                Some((local_temp, local_tint)),
            ) => {
                let working = match to_working {
                    Some(matrix) => mat3_apply(matrix, source),
                    None => source,
                };
                adapt(
                    working,
                    relative_wb_gains(temp + local_temp, tint + local_tint),
                )
            }
        }
    }

    /// True when [`Self::input`] without a delta returns every non-negative
    /// pixel bit for bit: a raster source already in working space with no
    /// white-balance change.
    pub fn input_is_identity(&self) -> bool {
        matches!(self.input, Input::Raster { combined: None, .. })
    }

    /// The value to smooth into the edge-aware base plane for Highlights and
    /// Shadows, from this pixel's [`Self::input`] output: the perceptual
    /// display value a neutral pixel of its luminance would get from stage
    /// 5 with Highlights and Shadows at zero (see
    /// [`SceneToDisplay::base_value`]). Luminance and one table lookup; only
    /// the delta's `exposure` is read.
    #[inline]
    pub fn tone_base(&self, rgb: [f32; 3], delta: Option<&LocalDelta>) -> f32 {
        self.scene
            .base_value(rgb, delta.map_or(0.0, |delta| delta.exposure))
    }

    /// True when the base plane has to be computed: the global Highlights or
    /// Shadows slider is active, or an active mask adjusts either. When it
    /// is false, [`Self::scene_to_display`] ignores its `base` argument.
    pub fn tone_needs_base(&self) -> bool {
        self.scene.needs_base() || self.masks_need_base
    }

    /// Stage 5: exposure, Highlights and Shadows, and the base curve,
    /// scene-linear to display-linear (see [`SceneToDisplay`]). `base` is
    /// this pixel's value in the smoothed plane of [`Self::tone_base`]. A
    /// delta's `exposure` is added in stops, its `highlights` and `shadows`
    /// to the sliders.
    #[inline]
    pub fn scene_to_display(
        &self,
        rgb: [f32; 3],
        base: f32,
        delta: Option<&LocalDelta>,
    ) -> [f32; 3] {
        match delta {
            Some(delta) if delta.affects_scene() => {
                self.scene.apply_with_delta(rgb, base, &delta.scene())
            }
            _ => self.scene.apply(rgb, base),
        }
    }

    /// True when [`Self::scene_to_display`] without a delta is the identity:
    /// a raster source with Exposure, Highlights and Shadows at zero.
    pub fn scene_to_display_is_identity(&self) -> bool {
        self.scene.is_identity()
    }

    /// Stage 6: Whites, Blacks, Contrast and the tone curves (see
    /// [`ToneStage`]). A delta's `contrast`, `whites` and `blacks` are added
    /// to the sliders.
    #[inline]
    pub fn tone(&self, rgb: [f32; 3], delta: Option<&LocalDelta>) -> [f32; 3] {
        match delta {
            Some(delta) if delta.affects_tone() => self.tone.apply_with_delta(rgb, &delta.tone()),
            _ => self.tone.apply(rgb),
        }
    }

    /// True when [`Self::tone`] without a delta is the identity.
    pub fn tone_is_identity(&self) -> bool {
        self.tone.is_identity()
    }

    /// Stage 7: colour (see [`ColorStage`]). A delta's `saturation` is added
    /// to the global Saturation.
    #[inline]
    pub fn color(&self, rgb: [f32; 3], delta: Option<&LocalDelta>) -> [f32; 3] {
        match delta {
            Some(delta) if delta.affects_color() => {
                self.color.apply_with_saturation(rgb, delta.saturation)
            }
            _ => self.color.apply(rgb),
        }
    }

    /// True when [`Self::color`] without a delta is the identity.
    pub fn color_is_identity(&self) -> bool {
        self.color.is_identity()
    }

    /// Stage 11 for the screen: gamut-map, encode and quantize to 8 bits
    /// through a lookup table.
    #[inline]
    pub fn output_u8(&self, rgb: [f32; 3]) -> [u8; 3] {
        self.output.encode_u8(rgb)
    }

    /// Stage 11 for export: gamut-map, encode exactly and quantize to 16
    /// bits.
    #[inline]
    pub fn output_u16(&self, rgb: [f32; 3]) -> [u16; 3] {
        self.output.encode_u16(rgb)
    }

    /// Stage 11 without quantization: encoded values in 0 to 1.
    #[inline]
    pub fn output_f32(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.output.encode(rgb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raster_white_balance_is_folded_into_one_matrix() {
        let settings = DevelopSettings {
            white_balance: WhiteBalance {
                preset: WbPreset::Custom,
                temp: 25.0,
                tint: -10.0,
            },
            ..Default::default()
        };
        let pipeline = PixelPipeline::new(&settings, &SourceColor::Rgb(RgbSpace::Srgb));
        let rgb = [0.3, 0.5, 0.2];
        let two_steps = mat3_apply(
            &relative_wb_matrix(25.0, -10.0),
            mat3_apply(&rgb_to_rgb(RgbSpace::Srgb, WORKING_SPACE), rgb),
        );
        for (a, b) in pipeline.input(rgb, None).iter().zip(two_steps) {
            assert!((a - b).abs() < 1e-6);
        }
        // A delta of zero temperature and tint takes the same path.
        let zero = LocalDelta {
            exposure: 1.0,
            ..Default::default()
        };
        assert_eq!(pipeline.input(rgb, Some(&zero)), pipeline.input(rgb, None));
    }

    #[test]
    fn raster_white_balance_numbers_outside_the_slider_range_are_held() {
        let stray = WhiteBalance {
            preset: WbPreset::Custom,
            temp: 5600.0,
            tint: f32::NAN,
        };
        assert_eq!(raster_temp_tint(&stray), (RASTER_TEMP_LIMIT, 0.0));
        assert_eq!(raster_temp_tint(&WhiteBalance::default()), (0.0, 0.0));
    }
}
