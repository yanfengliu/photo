//! Exposure and base rendering: scene-linear to display-linear (DESIGN §5.4).
//!
//! Camera sources get a gain and a filmic base curve; raster sources, which
//! are already renderings, get a gain and (for positive exposure only) a
//! shoulder that keeps the brightest pixel on white. Curves are applied with
//! the hue-preserving RGB tone method of the DNG SDK (`RefBaselineRGBTone`
//! in `dng_reference.cpp`): the largest and smallest channel go through the
//! curve and the middle one keeps its relative position between them.
//!
//! The base curve is the skewed log-logistic sigmoid used by darktable's
//! sigmoid module (`src/iop/sigmoid.c`, `y = (x^n / (x^n + s))^k`). How it
//! is anchored (middle grey and the sensor clip point) and the three skew
//! values are our own choices, to be tuned by rendering.

use std::sync::LazyLock;

use photo_core::colorimetry::{WORKING_LUMA, luminance};
use photo_core::develop::Profile;
use photo_core::image::SourceColor;

use crate::lut::Lut;
use crate::math::{max3, min3, non_negative, scale3, toward_luminance};

// ---- Tunable constants ----------------------------------------------------

/// Stops added to camera sources before the base curve. Carried over from
/// the previous app's calibration against the embedded previews of 35 Sony
/// ARW files: a raw value of 0.1 lands on middle grey.
pub const RAW_EXPOSURE_BIAS_EV: f32 = 0.85;
/// Scene middle grey, and the display-linear value the base curves give it.
const MID_GREY_IN: f64 = 0.18;
const MID_GREY_OUT: f64 = 0.19;
/// Display-linear value of the sensor clip point (raw 1.0 after the bias).
const CLIP_OUT: f64 = 0.99;

/// What distinguishes the curved profiles. `skew` is the outer exponent `k`
/// of the sigmoid: with both anchors fixed, a larger skew means more
/// contrast through middle grey (log-log slope about 1.12 / 1.25 / 1.42 for
/// 0.28 / 0.35 / 0.45). `saturation` scales colour around luminance after
/// the curve.
struct Look {
    skew: f64,
    saturation: f32,
}
const STANDARD: Look = Look {
    skew: 0.35,
    saturation: 1.0,
};
const NEUTRAL: Look = Look {
    skew: 0.28,
    saturation: 0.92,
};
const VIVID: Look = Look {
    skew: 0.45,
    saturation: 1.12,
};

/// The linear profile, and nothing else, clips softly above this value.
const SOFT_CLIP_KNEE: f32 = 0.8;
/// Exposure (global plus local) is clamped to this many stops either way.
const EXPOSURE_LIMIT_EV: f32 = 20.0;
/// Base-curve table: `CURVE_CELLS` samples over `t = x / (x + CURVE_INDEX_HALF)`,
/// which maps all of `[0, inf)` onto `[0, 1)` with middle grey near 0.42.
const CURVE_CELLS: usize = 2048;
const CURVE_INDEX_HALF: f64 = 0.25;

// ---------------------------------------------------------------------------

/// `2^ev`, the scene-linear gain of an exposure change in stops. Clamped to
/// ±20 stops; a non-finite `ev` counts as zero.
#[inline]
pub fn exposure_gain(ev: f32) -> f32 {
    if ev.is_finite() {
        ev.clamp(-EXPOSURE_LIMIT_EV, EXPOSURE_LIMIT_EV).exp2()
    } else {
        1.0
    }
}

/// Apply a tone curve to an RGB value without shifting its hue (DNG SDK
/// `RefBaselineRGBTone`). The largest and smallest channel take the curve's
/// value; the middle channel keeps its relative position between them.
///
/// For an increasing curve the order of the channels is preserved, equal
/// channels stay equal, and a neutral value stays neutral.
#[inline(always)]
pub fn rgb_tone(rgb: [f32; 3], curve: impl Fn(f32) -> f32) -> [f32; 3] {
    let (hi, lo) = (max3(rgb), min3(rgb));
    let top = curve(hi);
    let span = hi - lo;
    if span.is_nan() || span <= 0.0 {
        return [top; 3];
    }
    let bottom = curve(lo);
    let slope = (top - bottom) / span;
    [
        bottom + (rgb[0] - lo) * slope,
        bottom + (rgb[1] - lo) * slope,
        bottom + (rgb[2] - lo) * slope,
    ]
}

/// The sigmoid `y = (x^n / (x^n + s))^k`, solved so that it passes through
/// the two anchors for a given skew.
struct Sigmoid {
    power: f64,
    scale: f64,
    skew: f64,
}

impl Sigmoid {
    fn new(skew: f64) -> Self {
        let clip_in = 2.0f64.powf(f64::from(RAW_EXPOSURE_BIAS_EV));
        let odds = |out: f64| {
            let z = out.powf(1.0 / skew);
            z / (1.0 - z)
        };
        let (odds_grey, odds_clip) = (odds(MID_GREY_OUT), odds(CLIP_OUT));
        let power = (odds_clip / odds_grey).ln() / (clip_in / MID_GREY_IN).ln();
        Self {
            power,
            scale: MID_GREY_IN.powf(power) / odds_grey,
            skew,
        }
    }

    /// Strictly increasing from 0 at 0 toward (never reaching) 1.
    fn eval(&self, x: f64) -> f64 {
        if x.is_nan() || x <= 0.0 {
            return 0.0;
        }
        // Written as 1 / (1 + s / x^n) so that overflow and underflow of the
        // power both land on the correct limit.
        (1.0 / (1.0 + self.scale / x.powf(self.power))).powf(self.skew)
    }
}

/// A base curve compiled into a table.
struct CurveTable {
    exact: Sigmoid,
    table: Lut,
    saturation: f32,
}

impl CurveTable {
    fn new(look: &Look) -> Self {
        let exact = Sigmoid::new(look.skew);
        let table = Lut::build(CURVE_CELLS, 1.0, |t| {
            if t >= 1.0 {
                1.0
            } else {
                exact.eval(CURVE_INDEX_HALF * t / (1.0 - t))
            }
        });
        Self {
            exact,
            table,
            saturation: look.saturation,
        }
    }

    #[inline(always)]
    fn eval(&self, x: f32) -> f32 {
        let x = x.min(1.0e12);
        self.table.eval(x / (x + CURVE_INDEX_HALF as f32))
    }
}

/// The three curved profiles, built once per process.
static CURVES: LazyLock<[CurveTable; 3]> = LazyLock::new(|| {
    [
        CurveTable::new(&STANDARD),
        CurveTable::new(&NEUTRAL),
        CurveTable::new(&VIVID),
    ]
});

/// Position of a curved profile in [`CURVES`]; `None` for the linear one.
fn curve_index(profile: Profile) -> Option<usize> {
    match profile {
        Profile::Standard => Some(0),
        Profile::Neutral => Some(1),
        Profile::Vivid => Some(2),
        Profile::Linear => None,
    }
}

/// The base curve of a profile, evaluated exactly: scene-linear (after the
/// exposure gain and bias) to display-linear. For the curved profiles it
/// maps 0 to 0 and middle grey 0.18 to 0.19, has a power-law toe, reaches
/// 0.99 at the sensor clip point `2^0.85`, and approaches one without ever
/// reaching it. For [`Profile::Linear`] it is [`soft_clip`]. Strictly
/// increasing wherever `f32` can still resolve the difference.
pub fn base_curve(profile: Profile, x: f32) -> f32 {
    match curve_index(profile) {
        Some(index) => (CURVES[index].exact.eval(f64::from(x)) as f32).min(1.0 - f32::EPSILON),
        None => soft_clip(x),
    }
}

/// The soft clip of the linear profile: identity up to 0.8, then a rational
/// roll-off that is C1 at the knee and approaches 1 without reaching it.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    if x.is_nan() || x <= SOFT_CLIP_KNEE {
        return x.max(0.0);
    }
    let over = x.min(1.0e12) - SOFT_CLIP_KNEE;
    let room = 1.0 - SOFT_CLIP_KNEE;
    (SOFT_CLIP_KNEE + room * over / (room + over)).min(1.0 - f32::EPSILON)
}

/// The gain-aware shoulder for raster sources brightened by `gain > 1`.
///
/// A value `u` (already multiplied by the gain) is left alone below the knee
/// `1/gain`. Above it the response is the Möbius segment that leaves the
/// knee with slope one and maps the brightest possible input, `u = gain`, to
/// exactly 1. In terms of the original pixel `x = u / gain`:
///
/// ```text
/// y = g·x                                           for x <= 1/g²
/// y = (x·g·(g² + g - 1) - 1) / (g³·x + g² - g - 1)  otherwise
/// ```
///
/// This is strictly increasing in `x`, C1 at the knee, and (because
/// `dy/dg` has the sign of `(1 - x)·(2g - 1 + g³(g - 2)x) >= 0` on the
/// segment) non-decreasing in the gain; the knee tends to 1 as the gain
/// tends to 1, so the response is continuous in exposure through zero.
///
/// It is evaluated in the equivalent form `knee + d·a / (a + g·d)` with
/// `d = u - knee` and `a = g - knee`, which has no cancellation for gains
/// close to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shoulder {
    gain: f32,
    knee: f32,
    span: f32,
}

impl Shoulder {
    #[inline(always)]
    fn new(gain: f32) -> Self {
        let knee = 1.0 / gain;
        Self {
            gain,
            knee,
            span: gain - knee,
        }
    }

    #[inline(always)]
    fn eval(&self, u: f32) -> f32 {
        if u <= self.knee {
            u
        } else {
            let over = u - self.knee;
            self.knee + over * self.span / (self.span + self.gain * over)
        }
    }
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
    let rgb = non_negative(rgb);
    if gain == 1.0 {
        return rgb;
    }
    let scaled = scale3(rgb, gain);
    if gain < 1.0 {
        return scaled;
    }
    let shoulder = Shoulder::new(gain);
    if max3(scaled) <= shoulder.knee {
        return scaled;
    }
    rgb_tone(scaled, |u| shoulder.eval(u))
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Rendering {
    /// A raster source: gain and shoulder.
    Raster,
    /// A camera source with the curved profile at this index of [`CURVES`].
    Curve(usize),
    /// A camera source with the linear profile: gain and soft clip.
    Linear,
}

/// Stage 5 compiled for one source and one global exposure: white-balanced
/// working RGB in, display-linear working RGB out (nominally 0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneToDisplay {
    rendering: Rendering,
    gain: f32,
}

impl SceneToDisplay {
    /// `exposure` is the global Exposure slider in stops. For camera sources
    /// with a curved profile the gain also includes
    /// [`RAW_EXPOSURE_BIAS_EV`] and the file's baseline exposure; the linear
    /// profile and raster sources apply the slider alone. The profile does
    /// not affect raster sources.
    pub fn new(profile: Profile, exposure: f32, source: &SourceColor) -> Self {
        let exposure = if exposure.is_finite() { exposure } else { 0.0 };
        match source {
            SourceColor::Rgb(_) => Self {
                rendering: Rendering::Raster,
                gain: exposure_gain(exposure),
            },
            SourceColor::Camera(camera) => match curve_index(profile) {
                None => Self {
                    rendering: Rendering::Linear,
                    gain: exposure_gain(exposure),
                },
                Some(index) => {
                    let baseline = if camera.baseline_exposure.is_finite() {
                        camera.baseline_exposure
                    } else {
                        0.0
                    };
                    Self {
                        rendering: Rendering::Curve(index),
                        gain: exposure_gain(exposure + RAW_EXPOSURE_BIAS_EV + baseline),
                    }
                }
            },
        }
    }

    /// True when [`Self::apply`] returns every non-negative input bit for
    /// bit: a raster source at zero exposure.
    pub fn is_identity(&self) -> bool {
        self.rendering == Rendering::Raster && self.gain == 1.0
    }

    /// Apply the stage with the global exposure.
    #[inline]
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.render(rgb, self.gain)
    }

    /// Apply the stage with `exposure_delta` stops added to the global
    /// exposure (the alpha-weighted sum of mask exposures for this pixel).
    /// A delta of zero is exactly [`Self::apply`].
    #[inline]
    pub fn apply_with_exposure(&self, rgb: [f32; 3], exposure_delta: f32) -> [f32; 3] {
        if exposure_delta == 0.0 || !exposure_delta.is_finite() {
            return self.render(rgb, self.gain);
        }
        let gain = (self.gain * exposure_gain(exposure_delta)).clamp(1.0e-6, 1.0e6);
        self.render(rgb, gain)
    }

    #[inline(always)]
    fn render(&self, rgb: [f32; 3], gain: f32) -> [f32; 3] {
        match self.rendering {
            Rendering::Raster => raster_exposure(rgb, gain),
            Rendering::Linear => {
                let scaled = scale3(non_negative(rgb), gain);
                if max3(scaled) <= SOFT_CLIP_KNEE {
                    scaled
                } else {
                    rgb_tone(scaled, soft_clip)
                }
            }
            Rendering::Curve(index) => {
                let curve = &CURVES[index];
                let toned = rgb_tone(scale3(non_negative(rgb), gain), |x| curve.eval(x));
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

    #[test]
    fn the_sigmoid_passes_through_both_anchors_for_any_skew() {
        for skew in [0.2, 0.28, 0.35, 0.45, 0.8, 1.0] {
            let sigmoid = Sigmoid::new(skew);
            assert!((sigmoid.eval(MID_GREY_IN) - MID_GREY_OUT).abs() < 1e-12);
            let clip = 2.0f64.powf(f64::from(RAW_EXPOSURE_BIAS_EV));
            assert!((sigmoid.eval(clip) - CLIP_OUT).abs() < 1e-12);
            assert_eq!(sigmoid.eval(0.0), 0.0);
            assert!(sigmoid.eval(1.0e300) <= 1.0 && sigmoid.eval(1.0e-300) >= 0.0);
        }
    }

    #[test]
    fn the_tables_track_the_exact_curves() {
        for curve in CURVES.iter() {
            for step in 0..=6000 {
                let x = step as f32 / 1000.0;
                let exact = curve.exact.eval(f64::from(x)) as f32;
                assert!((curve.eval(x) - exact).abs() < 4e-6, "at {x}");
            }
            assert!(curve.eval(f32::MAX) <= 1.0 && curve.eval(f32::INFINITY) <= 1.0);
        }
    }

    #[test]
    fn the_shoulder_is_c1_at_its_knee() {
        for gain in [1.05f32, 2.0, 8.0, 32.0] {
            let shoulder = Shoulder::new(gain);
            // The shoulder of a small gain is short and bends quickly, so the
            // slope is measured very close to the knee.
            let h = shoulder.knee * 1e-4;
            let slope =
                (shoulder.eval(shoulder.knee + 2.0 * h) - shoulder.eval(shoulder.knee + h)) / h;
            assert!(
                (slope - 1.0).abs() < 0.02,
                "gain {gain}: slope {slope} just above the knee"
            );
        }
    }
}
