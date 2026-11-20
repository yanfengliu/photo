//! The scalar curves of base rendering, each with its inverse: the filmic
//! base curve of the curved profiles, the soft clip of the linear profile
//! and the gain-aware shoulder of raster sources.
//!
//! The base curve is the skewed log-logistic sigmoid used by darktable's
//! sigmoid module (`src/iop/sigmoid.c`, `y = (x^n / (x^n + s))^k`). How it
//! is anchored (middle grey and the sensor clip point) and the three skew
//! values are our own choices, to be tuned by rendering.

use std::sync::LazyLock;

use photo_core::develop::Profile;

use crate::lut::Lut;
use crate::math::{max3, min3};
use crate::tone::encode_d;

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
pub(super) const SOFT_CLIP_KNEE: f32 = 0.8;
/// Exposure (global plus local) is clamped to this many stops either way.
const EXPOSURE_LIMIT_EV: f32 = 20.0;
/// Base-curve tables: `CURVE_CELLS` samples over
/// `t = x / (x + CURVE_INDEX_HALF)`, which maps all of `[0, inf)` onto
/// `[0, 1)` with middle grey near 0.42 and the two stops above the sensor's
/// clip point spread over a tenth of the table.
const CURVE_CELLS: usize = 2048;
const CURVE_INDEX_HALF: f64 = 0.25;
/// Scene values are held to this before a table lookup, so that an absurd
/// input cannot turn the index into NaN.
const SCENE_LIMIT: f32 = 1.0e12;

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
/// `RefBaselineRGBTone` in `dng_reference.cpp`). The largest and smallest
/// channel take the curve's value; the middle channel keeps its relative
/// position between them.
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
pub(super) struct Sigmoid {
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
    pub(super) fn eval(&self, x: f64) -> f64 {
        if x.is_nan() || x <= 0.0 {
            return 0.0;
        }
        // Written as 1 / (1 + s / x^n) so that overflow and underflow of the
        // power both land on the correct limit.
        (1.0 / (1.0 + self.scale / x.powf(self.power))).powf(self.skew)
    }

    /// The scene value that the curve maps to display value `y`: 0 at 0,
    /// growing without bound as `y` approaches 1, infinite from 1 on.
    pub(super) fn inverse(&self, y: f64) -> f64 {
        if y.is_nan() || y <= 0.0 {
            return 0.0;
        }
        if y >= 1.0 {
            return f64::INFINITY;
        }
        // With z = y^(1/k): x^n = s z / (1 - z). `exp_m1` keeps 1 - z exact
        // where y is within rounding of 1, which is where it matters.
        let log_z = y.ln() / self.skew;
        (self.scale * log_z.exp() / -log_z.exp_m1()).powf(1.0 / self.power)
    }
}

/// A base curve compiled into tables.
pub(super) struct CurveTable {
    pub(super) exact: Sigmoid,
    /// Display-linear value over the table index.
    display: Lut,
    /// The same curve as a perceptual value, for the tone base.
    perceptual: Lut,
    pub(super) saturation: f32,
}

impl CurveTable {
    fn new(look: &Look) -> Self {
        let exact = Sigmoid::new(look.skew);
        let at = |t: f64| {
            if t >= 1.0 {
                1.0
            } else {
                exact.eval(CURVE_INDEX_HALF * t / (1.0 - t))
            }
        };
        let display = Lut::build(CURVE_CELLS, 1.0, at);
        let perceptual = Lut::build(CURVE_CELLS, 1.0, |t| encode_d(at(t)));
        Self {
            exact,
            display,
            perceptual,
            saturation: look.saturation,
        }
    }

    #[inline(always)]
    fn index(x: f32) -> f32 {
        let x = x.min(SCENE_LIMIT);
        x / (x + CURVE_INDEX_HALF as f32)
    }

    /// The curve's display-linear value at scene value `x`.
    #[inline(always)]
    pub(super) fn eval(&self, x: f32) -> f32 {
        self.display.eval(Self::index(x))
    }

    /// The curve's value at `x` in perceptual terms, `eval(x)^(1/2.2)`.
    #[inline(always)]
    pub(super) fn eval_perceptual(&self, x: f32) -> f32 {
        self.perceptual.eval(Self::index(x))
    }
}

/// The three curved profiles, built once per process.
pub(super) static CURVES: LazyLock<[CurveTable; 3]> = LazyLock::new(|| {
    [
        CurveTable::new(&STANDARD),
        CurveTable::new(&NEUTRAL),
        CurveTable::new(&VIVID),
    ]
});

/// Position of a curved profile in [`CURVES`]; `None` for the linear one.
pub(super) fn curve_index(profile: Profile) -> Option<usize> {
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
    let over = x.min(SCENE_LIMIT) - SOFT_CLIP_KNEE;
    let room = 1.0 - SOFT_CLIP_KNEE;
    (SOFT_CLIP_KNEE + room * over / (room + over)).min(1.0 - f32::EPSILON)
}

/// Inverse of [`soft_clip`]: infinite from 1 on.
pub(super) fn soft_clip_inverse(y: f64) -> f64 {
    let knee = f64::from(SOFT_CLIP_KNEE);
    if y.is_nan() || y <= knee {
        return y.max(0.0);
    }
    if y >= 1.0 {
        return f64::INFINITY;
    }
    knee + (1.0 - knee) * (y - knee) / (1.0 - y)
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
pub(super) struct Shoulder {
    gain: f32,
    pub(super) knee: f32,
    span: f32,
}

impl Shoulder {
    #[inline(always)]
    pub(super) fn new(gain: f32) -> Self {
        let knee = 1.0 / gain;
        Self {
            gain,
            knee,
            span: gain - knee,
        }
    }

    #[inline(always)]
    pub(super) fn eval(&self, u: f32) -> f32 {
        if u <= self.knee {
            u
        } else {
            let over = u.min(SCENE_LIMIT) - self.knee;
            self.knee + over * self.span / (self.span + self.gain * over)
        }
    }
}

/// Inverse of the shoulder for `gain > 1`: the already-gained value that
/// lands on display value `y`. Infinite at and beyond the shoulder's
/// asymptote (which lies above 1).
pub(super) fn shoulder_inverse(gain: f64, y: f64) -> f64 {
    let knee = 1.0 / gain;
    if y.is_nan() || y <= knee {
        return y.max(0.0);
    }
    let span = gain - knee;
    let over = y - knee;
    let room = span - gain * over;
    if room <= 0.0 {
        return f64::INFINITY;
    }
    knee + span * over / room
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
    fn every_curve_inverts_exactly() {
        for curve in CURVES.iter() {
            for step in 1..=400 {
                // From deep shadow to four stops above the clip point.
                let x = 1.0e-4 * 1.03f64.powi(step);
                let back = curve.exact.inverse(curve.exact.eval(x));
                assert!((back / x - 1.0).abs() < 1e-6, "sigmoid at {x}: {back}");
            }
            assert_eq!(curve.exact.inverse(0.0), 0.0);
            assert!(curve.exact.inverse(1.0).is_infinite());
            // Within rounding of white the inverse is large but finite.
            assert!(curve.exact.inverse(1.0 - 1.0e-12).is_finite());
        }
        for step in 0..=300 {
            let x = f64::from(step) * 0.05;
            let y = f64::from(soft_clip(x as f32));
            if y < 0.9999 {
                assert!(
                    (soft_clip_inverse(y) - x).abs() < 2e-3 * x.max(1.0),
                    "soft clip at {x}"
                );
            }
        }
        assert!(soft_clip_inverse(1.0).is_infinite());
        for gain in [1.05f32, 2.0, 8.0, 32.0] {
            let shoulder = Shoulder::new(gain);
            for step in 0..=200 {
                let u = gain * step as f32 / 200.0;
                let back = shoulder_inverse(f64::from(gain), f64::from(shoulder.eval(u)));
                assert!(
                    (back - f64::from(u)).abs() < 2e-3 * f64::from(u.max(1.0)),
                    "shoulder {gain} at {u}: {back}"
                );
            }
            assert!(shoulder_inverse(f64::from(gain), 5.0).is_infinite());
        }
    }

    #[test]
    fn the_tables_track_the_exact_curves() {
        for curve in CURVES.iter() {
            for step in 0..=6000 {
                let x = step as f32 / 1000.0;
                let exact = curve.exact.eval(f64::from(x));
                assert!((curve.eval(x) - exact as f32).abs() < 4e-6, "at {x}");
                if step >= 5 {
                    let perceptual = encode_d(exact) as f32;
                    assert!(
                        (curve.eval_perceptual(x) - perceptual).abs() < 2e-5,
                        "at {x}"
                    );
                }
            }
            assert!(curve.eval(f32::MAX) <= 1.0 && curve.eval(f32::INFINITY) <= 1.0);
            assert_eq!(curve.eval_perceptual(0.0), 0.0);
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
