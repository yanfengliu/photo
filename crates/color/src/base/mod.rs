//! Exposure, highlight and shadow recovery, and base rendering:
//! scene-linear to display-linear (pipeline stage 5, DESIGN §5.4 and §5.5).
//!
//! Camera sources get a gain and a filmic base curve; raster sources, which
//! are already renderings, get a gain and (for positive exposure only) a
//! shoulder that keeps the brightest pixel on white. Curves are applied with
//! the hue-preserving RGB tone method of the DNG SDK (`RefBaselineRGBTone`).
//!
//! Highlights and Shadows act here, before the curve, as a scene-linear gain
//! looked up from an edge-aware base. With `c` the scalar scene-to-display
//! curve, `Z` the perceptual zone curve and `b` the base of a pixel, the
//! gain is `c⁻¹(decode(Z(b))) / c⁻¹(decode(b))`. A flat neutral area
//! therefore lands exactly where `Z` says, as it would if `Z` were applied
//! to the finished picture, but the detail riding on it is rendered through
//! the curve at the new operating point: pulling highlights down moves them
//! out of the flat shoulder and their texture comes back, instead of a
//! flattened texture merely being darkened. The structure (an exposure gain
//! driven by a smoothed luminance) is that of darktable's tone equalizer
//! (`src/iop/toneequal.c`); the base is smoothed by the caller, for which
//! the guided filter of He, Sun and Tang (2010) is the intended choice.

mod curve;

use photo_core::colorimetry::{WORKING_LUMA, luminance};
use photo_core::develop::{Profile, Tone};
use photo_core::image::SourceColor;

use crate::lut::Lut;
use crate::math::{max3, non_negative, scale3, toward_luminance};
use crate::tone::{ZoneCurve, decode_d, encode_perceptual};
use curve::{CURVES, SOFT_CLIP_KNEE, Shoulder, curve_index, shoulder_inverse, soft_clip_inverse};

pub use curve::{RAW_EXPOSURE_BIAS_EV, base_curve, exposure_gain, rgb_tone, soft_clip};

// ---- Tunable constants ----------------------------------------------------

/// Samples of the zone-gain table over the base, 0 to 1.
const ZONE_CELLS: usize = 2048;
/// The zone gain is evaluated no closer to white than this: at white itself
/// the inverse of a base curve is infinite, and the gain there is defined as
/// its limit.
const BASE_LIMIT: f64 = 1.0 - 1.0e-9;
/// Bounds on the zone gain (about ±6 stops), far beyond what the zone curve
/// can ask for; they only keep a degenerate base from producing an absurd
/// multiplier.
const ZONE_GAIN_RANGE: (f64, f64) = (1.0 / 64.0, 64.0);
/// Bounds on the total exposure gain, global and local together.
const GAIN_RANGE: (f32, f32) = (1.0e-6, 1.0e6);

// ---------------------------------------------------------------------------

/// Which scalar curve follows the gain.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Rendering {
    /// A raster source: gain and shoulder.
    Raster,
    /// A camera source with the curved profile at this index of [`CURVES`].
    Curve(usize),
    /// A camera source with the linear profile: gain and soft clip.
    Linear,
}

impl Rendering {
    /// The already-gained scene value that this rendering maps to display
    /// value `display`. `gain` only matters for a raster source, whose
    /// shoulder depends on it.
    fn inverse(self, gain: f64, display: f64) -> f64 {
        match self {
            Self::Raster if gain > 1.0 => shoulder_inverse(gain, display),
            Self::Raster => display.max(0.0),
            Self::Curve(index) => CURVES[index].exact.inverse(display),
            Self::Linear => soft_clip_inverse(display),
        }
    }
}

/// The scene-linear gain that moves a flat area whose perceptual display
/// value is `base` to where `zone` puts it. One wherever the zone curve is
/// the identity, and for a base that is not a positive number.
fn zone_gain(rendering: Rendering, gain: f64, zone: &ZoneCurve, base: f64) -> f64 {
    if base.is_nan() || base <= 0.0 {
        return 1.0;
    }
    let base = base.min(BASE_LIMIT);
    let target = zone.eval_d(base);
    if target == base {
        return 1.0;
    }
    let from = rendering.inverse(gain, decode_d(base));
    let to = rendering.inverse(gain, decode_d(target));
    if from > 0.0 && from.is_finite() && to.is_finite() {
        (to / from).clamp(ZONE_GAIN_RANGE.0, ZONE_GAIN_RANGE.1)
    } else {
        1.0
    }
}

/// Scale colour around luminance and bring any channel that went negative
/// back by desaturating. Neutral values are untouched.
#[inline(always)]
fn saturate(rgb: [f32; 3], factor: f32) -> [f32; 3] {
    let y = luminance(rgb);
    toward_luminance(
        [
            y + (rgb[0] - y) * factor,
            y + (rgb[1] - y) * factor,
            y + (rgb[2] - y) * factor,
        ],
        &WORKING_LUMA,
    )
}

/// A raster source: `rgb` times `gain * zone`, then the shoulder that
/// belongs to `gain` when it is above one.
#[inline(always)]
fn raster_render(rgb: [f32; 3], gain: f32, zone: f32) -> [f32; 3] {
    let rgb = non_negative(rgb);
    let factor = gain * zone;
    if gain <= 1.0 {
        return if factor == 1.0 {
            rgb
        } else {
            scale3(rgb, factor)
        };
    }
    let scaled = scale3(rgb, factor);
    let shoulder = Shoulder::new(gain);
    if max3(scaled) <= shoulder.knee {
        return scaled;
    }
    rgb_tone(scaled, |u| shoulder.eval(u))
}

/// Exposure for a raster source: `rgb` multiplied by `gain`, with the
/// highlight shoulder described on DESIGN §5.4 when `gain > 1`.
///
/// A gain of exactly 1 returns the input unchanged; a gain below 1 is a
/// pure multiplication (whites go grey); above 1, values under the knee are
/// a pure multiplication and the rest roll off so that 1.0 stays 1.0. The
/// result is never negative.
#[inline]
pub fn raster_exposure(rgb: [f32; 3], gain: f32) -> [f32; 3] {
    raster_render(rgb, gain, 1.0)
}

/// Per-pixel additions to the global Exposure, Highlights and Shadows: the
/// alpha-weighted sum of the mask adjustments covering a pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneDelta {
    /// Stops added to the exposure.
    pub exposure: f32,
    /// Added to Highlights.
    pub highlights: f32,
    /// Added to Shadows.
    pub shadows: f32,
}

/// The global zone curve and its gain as a table over the base.
#[derive(Clone, Debug)]
struct Zones {
    curve: ZoneCurve,
    gain: Lut,
}

/// Stage 5 compiled for one source and one set of tone settings:
/// white-balanced scene-linear working RGB in, display-linear working RGB
/// out (nominally 0 to 1).
#[derive(Clone, Debug)]
pub struct SceneToDisplay {
    rendering: Rendering,
    gain: f32,
    highlights: f32,
    shadows: f32,
    /// `None` when Highlights and Shadows are both zero.
    zones: Option<Zones>,
}

impl SceneToDisplay {
    /// Compile the stage from the profile and the Exposure, Highlights and
    /// Shadows sliders of `tone` (its other fields belong to stage 6).
    ///
    /// For camera sources with a curved profile the gain also includes
    /// [`RAW_EXPOSURE_BIAS_EV`] and the file's baseline exposure; the linear
    /// profile and raster sources apply the Exposure slider alone. The
    /// profile does not affect raster sources.
    pub fn new(profile: Profile, tone: &Tone, source: &SourceColor) -> Self {
        let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
        let exposure = finite(tone.exposure);
        let (rendering, gain) = match source {
            SourceColor::Rgb(_) => (Rendering::Raster, exposure_gain(exposure)),
            SourceColor::Camera(camera) => match curve_index(profile) {
                None => (Rendering::Linear, exposure_gain(exposure)),
                Some(index) => (
                    Rendering::Curve(index),
                    exposure_gain(exposure + RAW_EXPOSURE_BIAS_EV + finite(camera.baseline_exposure)),
                ),
            },
        };
        let (highlights, shadows) = (finite(tone.highlights), finite(tone.shadows));
        let curve = ZoneCurve::new(highlights, shadows);
        let zones = (!curve.is_identity()).then(|| Zones {
            curve,
            gain: Lut::build(ZONE_CELLS, 1.0, |base| {
                zone_gain(rendering, f64::from(gain), &curve, base)
            }),
        });
        Self {
            rendering,
            gain,
            highlights,
            shadows,
            zones,
        }
    }

    /// True when [`Self::apply`] returns every non-negative input bit for
    /// bit: a raster source with Exposure, Highlights and Shadows at zero.
    pub fn is_identity(&self) -> bool {
        self.rendering == Rendering::Raster && self.gain == 1.0 && self.zones.is_none()
    }

    /// True when the global Highlights or Shadows slider is active, i.e.
    /// [`Self::apply`] reads its `base` argument. (A [`SceneDelta`] with
    /// highlights or shadows reads it too.)
    pub fn needs_base(&self) -> bool {
        self.zones.is_some()
    }

    /// The value to smooth into the edge-aware base: the perceptual display
    /// value that a neutral pixel of this pixel's luminance gets from this
    /// stage with Highlights and Shadows at zero. It needs only the pixel's
    /// luminance and one table lookup, and is exact for a neutral pixel.
    ///
    /// `exposure_delta` is the pixel's local exposure in stops (zero outside
    /// every mask); pass the same value that the pixel is later rendered
    /// with. Passing the unsmoothed value back as `base` turns the zone
    /// curve into a plain global curve.
    #[inline]
    pub fn base_value(&self, rgb: [f32; 3], exposure_delta: f32) -> f32 {
        let gain = self.gain_with(exposure_delta);
        let scene = luminance(non_negative(rgb)) * gain;
        match self.rendering {
            Rendering::Curve(index) => CURVES[index].eval_perceptual(scene),
            Rendering::Linear => encode_perceptual(soft_clip(scene)),
            Rendering::Raster if gain > 1.0 => encode_perceptual(Shoulder::new(gain).eval(scene)),
            Rendering::Raster => encode_perceptual(scene),
        }
    }

    /// Apply the stage with the global settings. `base` is this pixel's
    /// value in the smoothed plane of [`Self::base_value`]; it is only read
    /// when [`Self::needs_base`] is true.
    ///
    /// The result is finite and non-negative for every input. A flat neutral
    /// area whose base equals its own [`Self::base_value`] `v` comes out at
    /// perceptual value `Z(v)`, the zone curve of the settings.
    #[inline]
    pub fn apply(&self, rgb: [f32; 3], base: f32) -> [f32; 3] {
        let zone = match &self.zones {
            Some(zones) => zones.gain.eval(base),
            None => 1.0,
        };
        self.render(rgb, self.gain, zone)
    }

    /// Apply the stage with `delta` added to the global Exposure, Highlights
    /// and Shadows for this pixel (each zone slider sum is clamped to the
    /// slider's own range). A zero delta is exactly [`Self::apply`]; a delta
    /// with highlights or shadows evaluates the zone gain directly instead
    /// of reading the global table.
    pub fn apply_with_delta(&self, rgb: [f32; 3], base: f32, delta: &SceneDelta) -> [f32; 3] {
        let gain = self.gain_with(delta.exposure);
        let changed = |local: f32| local != 0.0 && local.is_finite();
        let zone = if changed(delta.highlights) || changed(delta.shadows) {
            let add = |global: f32, local: f32| if changed(local) { global + local } else { global };
            let curve = ZoneCurve::new(
                add(self.highlights, delta.highlights),
                add(self.shadows, delta.shadows),
            );
            zone_gain(self.rendering, f64::from(gain), &curve, f64::from(base)) as f32
        } else {
            match &self.zones {
                None => 1.0,
                // A raster shoulder depends on the gain, so the table only
                // holds for the global exposure.
                Some(zones) if self.rendering == Rendering::Raster && gain != self.gain => {
                    zone_gain(self.rendering, f64::from(gain), &zones.curve, f64::from(base)) as f32
                }
                Some(zones) => zones.gain.eval(base),
            }
        };
        self.render(rgb, gain, zone)
    }

    /// The exposure gain with a local exposure in stops added.
    #[inline(always)]
    fn gain_with(&self, exposure_delta: f32) -> f32 {
        if exposure_delta == 0.0 || !exposure_delta.is_finite() {
            self.gain
        } else {
            (self.gain * exposure_gain(exposure_delta)).clamp(GAIN_RANGE.0, GAIN_RANGE.1)
        }
    }

    #[inline(always)]
    fn render(&self, rgb: [f32; 3], gain: f32, zone: f32) -> [f32; 3] {
        match self.rendering {
            Rendering::Raster => raster_render(rgb, gain, zone),
            Rendering::Linear => {
                let scaled = scale3(non_negative(rgb), gain * zone);
                if max3(scaled) <= SOFT_CLIP_KNEE {
                    scaled
                } else {
                    rgb_tone(scaled, soft_clip)
                }
            }
            Rendering::Curve(index) => {
                let curve = &CURVES[index];
                let toned = rgb_tone(scale3(non_negative(rgb), gain * zone), |x| curve.eval(x));
                if curve.saturation == 1.0 {
                    toned
                } else {
                    saturate(toned, curve.saturation)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tone::encode_d;
    use photo_core::colorimetry::RgbSpace;
    use photo_core::image::CameraColor;

    fn camera() -> SourceColor {
        SourceColor::Camera(CameraColor {
            xyz_to_camera: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            as_shot_gains: [1.0; 3],
            baseline_exposure: 0.0,
        })
    }

    #[test]
    fn the_zone_gain_puts_a_flat_area_where_the_zone_curve_says() {
        let zone = ZoneCurve::new(-100.0, 100.0);
        let renderings = [
            (Rendering::Raster, 1.0),
            (Rendering::Raster, 4.0),
            (Rendering::Curve(0), 1.0),
            (Rendering::Curve(2), 1.0),
            (Rendering::Linear, 1.0),
        ];
        for (rendering, gain) in renderings {
            for step in 1..1000 {
                let base = f64::from(step) / 1000.0;
                let from = rendering.inverse(gain, decode_d(base));
                let k = zone_gain(rendering, gain, &zone, base);
                // Rendering the scaled scene value must give decode(Z(base)):
                // check it through the inverse, which is exact.
                let landed = rendering.inverse(gain, decode_d(zone.eval_d(base)));
                assert!(
                    (k * from / landed - 1.0).abs() < 1e-9,
                    "{rendering:?} gain {gain} at base {base}"
                );
            }
            // Outside (0, 1) the zone curve is the identity; at white the
            // gain is its finite limit.
            assert_eq!(zone_gain(rendering, gain, &zone, 0.0), 1.0);
            assert_eq!(zone_gain(rendering, gain, &zone, f64::NAN), 1.0);
            let at_white = zone_gain(rendering, gain, &zone, 1.0);
            assert!(at_white > 0.0 && at_white <= 1.0, "{rendering:?}: {at_white}");
            assert_eq!(zone_gain(rendering, gain, &zone, 7.0), at_white);
        }
    }

    #[test]
    fn above_the_clip_point_highlight_recovery_is_a_uniform_exposure_cut() {
        // Every scene value from the sensor's clip point upward gets nearly
        // the same gain, so recovered highlights keep their own gradation.
        let zone = ZoneCurve::new(-100.0, 0.0);
        let sigmoid = &CURVES[0].exact;
        let clip = 2.0f64.powf(f64::from(RAW_EXPOSURE_BIAS_EV));
        let gains: Vec<f64> = [1.0, 1.5, 2.0, 4.0, 16.0]
            .iter()
            .map(|multiple| {
                let base = encode_d(sigmoid.eval(clip * multiple));
                zone_gain(Rendering::Curve(0), 1.0, &zone, base)
            })
            .collect();
        for gain in &gains {
            // Between one and one and a half stops down.
            assert!((0.35..0.5).contains(gain), "{gains:?}");
        }
        let spread = gains.iter().cloned().fold(0.0, f64::max) / gains.iter().cloned().fold(9.0, f64::min);
        assert!(spread < 1.1, "{gains:?}");
    }

    #[test]
    fn only_active_zones_build_a_table() {
        let neutral = SceneToDisplay::new(Profile::Standard, &Tone::default(), &camera());
        assert!(neutral.zones.is_none() && !neutral.needs_base() && !neutral.is_identity());
        let raster = SourceColor::Rgb(RgbSpace::Srgb);
        assert!(SceneToDisplay::new(Profile::Standard, &Tone::default(), &raster).is_identity());
        let shadows = Tone {
            shadows: 20.0,
            ..Default::default()
        };
        let stage = SceneToDisplay::new(Profile::Standard, &shadows, &raster);
        assert!(stage.needs_base() && !stage.is_identity());
        // Contrast, whites and blacks belong to stage 6.
        let other = Tone {
            contrast: 50.0,
            whites: 50.0,
            blacks: 50.0,
            ..Default::default()
        };
        assert!(SceneToDisplay::new(Profile::Standard, &other, &raster).is_identity());
    }
}
