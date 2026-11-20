//! The Basic-panel tone curves as pure functions of a perceptual value
//! (DESIGN §5.5): the highlights/shadows zone curve, the whites/blacks
//! end-point curve and contrast.
//!
//! Every control is its own strictly increasing curve, and controls are
//! composed rather than summed, so any combination is increasing by
//! construction. The minimum slope of each curve follows from its constants
//! and is noted beside them.
//!
//! The zone bands are deliberately tight, following the owner's standing
//! rule of Gaussian-like bands about one stop wide (σ = 1 EV) with about
//! 1.5 EV of reach: at σ = √2 a single full-strength slider reads as a
//! global exposure shift rather than as targeted recovery.

use photo_core::meta::FileKind;

use crate::math::{slider_unit, smoothstep_d};

// ---- Tunable constants ----------------------------------------------------

/// Shadows: a bell that rises from black to its peak and is gone by 0.66,
/// so it never reaches the highlights.
const SHADOWS: Bell = Bell {
    lo: 0.0,
    peak: 0.32,
    hi: 0.66,
};
/// Perceptual shift at the peak for Shadows +100 and -100: +1.48 and -1.49
/// stops of display luminance. With smoothstep flanks (maximum slope
/// 1.5 / width) the curve's slope stays above `1 - 0.19 * 1.5 / 0.34 = 0.16`
/// and `1 - 0.12 * 1.5 / 0.32 = 0.44`.
const SHADOWS_LIFT: f64 = 0.19;
const SHADOWS_DROP: f64 = 0.12;

/// Highlights above zero: a bell from 0.45 to white with its peak at 0.80.
/// It vanishes at white, so the white point itself is left to Whites. The
/// slope stays above `1 - 0.10 * 1.5 / 0.20 = 0.25`.
const HIGHLIGHTS_LIFT_BELL: Bell = Bell {
    lo: 0.45,
    peak: 0.80,
    hi: 1.0,
};
const HIGHLIGHTS_LIFT: f64 = 0.10;

/// Highlights below zero for camera raw sources. The pull-down is complete
/// at 0.98 and returns to zero at white as a square root, so that the
/// headroom a base curve squeezes into the last half percent below white is
/// pulled back into the visible range, the brighter the more (2 EV of
/// headroom above the sensor's clip point land between display 0.69 and
/// 0.96 on the standard profile). The slope on the rise stays above
/// `1 - 0.22 * 1.5 / 0.53 = 0.38`; on the return it is above 1.
const RAW_RECOVERY: Shelf = Shelf {
    start: 0.45,
    full: 0.98,
    drop: 0.22,
    power: 0.5,
};
/// Highlights below zero for raster sources, which have no headroom: the
/// pull-down is complete at 0.85 and returns to zero at white along a
/// parabola, whose slope at white, `1 + 2 * 0.18 / 0.15 = 3.4`, is kept low
/// so that the top codes of an 8-bit file are not stretched into bands. The
/// slope on the rise stays above `1 - 0.18 * 1.5 / 0.40 = 0.33`.
const RASTER_RECOVERY: Shelf = Shelf {
    start: 0.45,
    full: 0.85,
    drop: 0.18,
    power: 1.0,
};

/// Blacks: the gain `1 + a (1 - v)^12`, largest at black and gone by about
/// 0.3. The perceptual shift `a v (1 - v)^12` peaks at `v = 1/13`. The
/// curve's slope is at least `1 - 0.16 a` for a lift and `1 - |a|` for a
/// crush, so `a` may range over (-1, 6).
const BLACKS_POWER: i32 = 12;
const BLACKS_LIFT: f64 = 2.0;
const BLACKS_CRUSH: f64 = 0.85;

/// Whites: the gain `1 + a v^4`, largest at white. The slope at white is
/// `1 + 5a`, so `a` must stay above -0.2.
const WHITES_POWER: i32 = 4;
const WHITES_PUSH: f64 = 0.25;
const WHITES_DULL: f64 = 0.16;

/// Contrast amounts (global plus local) beyond this are clamped.
const CONTRAST_LIMIT: f64 = 400.0;

/// The perceptual value at which Blacks shifts pixels the most. Zone tests
/// sample at these four peaks, never at a boundary where an effect is zero
/// by construction.
pub const BLACKS_PEAK: f32 = 1.0 / 13.0;
/// The perceptual value at which Shadows shifts pixels the most.
pub const SHADOWS_PEAK: f32 = 0.32;
/// A perceptual value where Highlights acts at least 80% as strongly as
/// anywhere, in both directions and for both kinds of source (a lift peaks
/// at 0.80, a raster pull-down at 0.85, a raw pull-down at 0.98).
pub const HIGHLIGHTS_PEAK: f32 = 0.85;
/// A perceptual value just inside the range where Whites acts most (its
/// effect keeps growing all the way to white and beyond).
pub const WHITES_PEAK: f32 = 0.998;

// ---------------------------------------------------------------------------

/// A C1 bump: zero outside `(lo, hi)`, one at `peak`, smoothstep flanks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bell {
    pub(crate) lo: f64,
    pub(crate) peak: f64,
    pub(crate) hi: f64,
}

impl Bell {
    pub(crate) fn eval(&self, v: f64) -> f64 {
        if v <= self.lo || v >= self.hi {
            0.0
        } else if v <= self.peak {
            smoothstep_d((v - self.lo) / (self.peak - self.lo))
        } else {
            1.0 - smoothstep_d((v - self.peak) / (self.hi - self.peak))
        }
    }
}

/// The weight of the negative-Highlights pull-down: zero up to `start`,
/// rising (smoothstep) to one at `full`, then back to zero at white as
/// `(1 - t)^power (1 + power t)`, with `t` running from 0 at `full` to 1 at
/// white. The return leaves `full` flat, so the shelf is C1; with
/// `power = 1` it is the parabola `1 - t²` and has a finite slope at white,
/// below 1 its slope there is unbounded. The return only ever falls, so the
/// pull-down `v - drop * weight` rises with a slope of at least 1 there.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shelf {
    start: f64,
    full: f64,
    drop: f64,
    power: f64,
}

impl Shelf {
    fn weight(&self, v: f64) -> f64 {
        if v <= self.start || v >= 1.0 {
            0.0
        } else if v <= self.full {
            smoothstep_d((v - self.start) / (self.full - self.start))
        } else {
            let t = (v - self.full) / (1.0 - self.full);
            (1.0 - t).powf(self.power) * (1.0 + self.power * t)
        }
    }
}

fn signed(slider: f32, up: f64, down: f64) -> f64 {
    let amount = slider_unit(slider);
    amount * if amount >= 0.0 { up } else { down }
}

/// The Highlights and Shadows controls as one curve on perceptual values:
/// Shadows is applied first, Highlights to its result. It says where a flat
/// neutral area of perceptual value `v` lands; the pipeline realises it as a
/// scene-linear gain looked up from an edge-aware base (see
/// [`crate::base::SceneToDisplay`]).
///
/// Negative Highlights depends on the kind of source: a camera raw source
/// has headroom above the sensor's clip point, squeezed by the base curve
/// into the last fraction of a percent below white, and the curve reaches
/// into it; a raster file has none.
///
/// Identity outside `(0, 1)`, so black and white stay put; strictly
/// increasing for every pair of slider values. Slider values beyond ±100 are
/// clamped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoneCurve {
    shadows: f64,
    /// Signed shift at full weight: a lift when positive, a pull-down when
    /// negative (the two use different shapes).
    highlights: f64,
    recovery: Shelf,
}

impl ZoneCurve {
    /// Build from the two slider values (-100 to 100) for a source of the
    /// given kind.
    pub fn new(highlights: f32, shadows: f32, kind: FileKind) -> Self {
        let recovery = match kind {
            FileKind::Raw => RAW_RECOVERY,
            FileKind::Raster => RASTER_RECOVERY,
        };
        Self {
            shadows: signed(shadows, SHADOWS_LIFT, SHADOWS_DROP),
            highlights: signed(highlights, HIGHLIGHTS_LIFT, recovery.drop),
            recovery,
        }
    }

    /// True when both sliders are zero.
    pub fn is_identity(&self) -> bool {
        self.shadows == 0.0 && self.highlights == 0.0
    }

    /// The curve's value at perceptual `v`.
    pub fn eval(&self, v: f32) -> f32 {
        self.eval_d(f64::from(v)) as f32
    }

    pub(crate) fn eval_d(&self, v: f64) -> f64 {
        if v.is_nan() {
            return 0.0;
        }
        if v <= 0.0 || v >= 1.0 {
            return v;
        }
        let lifted = v + self.shadows * SHADOWS.eval(v);
        let weight = if self.highlights >= 0.0 {
            HIGHLIGHTS_LIFT_BELL.eval(lifted)
        } else {
            self.recovery.weight(lifted)
        };
        lifted + self.highlights * weight
    }
}

/// The Whites and Blacks controls as one curve on perceptual values: Blacks
/// is applied first, Whites to its result. Both are gains on the pixel's own
/// value, so black stays black. Whites above zero carries the top end past
/// 1.0 (it clips at output); above 1.0 the curve continues along its slope
/// at white. Strictly increasing for every pair of slider values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EndpointCurve {
    blacks: f64,
    whites: f64,
}

impl EndpointCurve {
    /// Build from the two slider values (-100 to 100).
    pub fn new(whites: f32, blacks: f32) -> Self {
        Self {
            blacks: signed(blacks, BLACKS_LIFT, BLACKS_CRUSH),
            whites: signed(whites, WHITES_PUSH, WHITES_DULL),
        }
    }

    /// True when both sliders are zero.
    pub fn is_identity(&self) -> bool {
        self.blacks == 0.0 && self.whites == 0.0
    }

    /// The curve's value at perceptual `v`.
    pub fn eval(&self, v: f32) -> f32 {
        self.eval_d(f64::from(v)) as f32
    }

    pub(crate) fn eval_d(&self, v: f64) -> f64 {
        if v.is_nan() {
            return 0.0;
        }
        v * self.ratio_d(v)
    }

    fn blacks_ratio(&self, v: f64) -> f64 {
        if v >= 1.0 {
            1.0
        } else {
            1.0 + self.blacks * (1.0 - v.max(0.0)).powi(BLACKS_POWER)
        }
    }

    fn whites_ratio(&self, v: f64) -> f64 {
        if v <= 1.0 {
            1.0 + self.whites * v.max(0.0).powi(WHITES_POWER)
        } else {
            let end_slope = 1.0 + f64::from(WHITES_POWER + 1) * self.whites;
            (1.0 + self.whites + end_slope * (v - 1.0)) / v
        }
    }

    /// `eval(v) / v`, finite at zero: the perceptual gain on the pixel.
    pub(crate) fn ratio_d(&self, v: f64) -> f64 {
        if v.is_nan() {
            return 1.0;
        }
        let blacks = self.blacks_ratio(v);
        blacks * self.whites_ratio(v * blacks)
    }
}

/// Contrast as the rational sigmoid `u' = u (1 + a) / (1 + a |u|)` on
/// `u = 2v - 1`, with `1 + a = 2^(amount / 100)`.
///
/// The pivot `v = 0.5` and both end points are fixed; the slope is `1 + a`
/// at the pivot and `1 / (1 + a)` at the ends, so it is bounded and positive
/// for every amount. Outside `[0, 1]` the curve continues along its end
/// slope. An amount of zero returns `v` unchanged.
pub fn contrast_curve(v: f32, amount: f32) -> f32 {
    if amount == 0.0 {
        return v;
    }
    contrast_d(f64::from(v), f64::from(amount)) as f32
}

pub(crate) fn contrast_d(v: f64, amount: f64) -> f64 {
    if v.is_nan() {
        return 0.0;
    }
    if amount == 0.0 || amount.is_nan() {
        return v;
    }
    let steepness = (amount.clamp(-CONTRAST_LIMIT, CONTRAST_LIMIT) / 100.0).exp2();
    if v >= 1.0 {
        return 1.0 + (v - 1.0) / steepness;
    }
    if v <= 0.0 {
        return v / steepness;
    }
    let u = 2.0 * v - 1.0;
    0.5 * (u * steepness / (1.0 + (steepness - 1.0) * u.abs()) + 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [FileKind; 2] = [FileKind::Raster, FileKind::Raw];

    fn least_slope(curve: &ZoneCurve) -> f64 {
        let steps = 20_000;
        (0..steps)
            .map(|step| {
                let v = f64::from(step) / f64::from(steps);
                (curve.eval_d(v + 1.0 / f64::from(steps)) - curve.eval_d(v)) * f64::from(steps)
            })
            .fold(f64::MAX, f64::min)
    }

    #[test]
    fn a_bell_is_one_at_its_peak_and_zero_outside() {
        for bell in [SHADOWS, HIGHLIGHTS_LIFT_BELL] {
            assert_eq!(bell.eval(bell.peak), 1.0);
            assert_eq!(bell.eval(bell.lo), 0.0);
            assert_eq!(bell.eval(bell.hi), 0.0);
            assert_eq!(bell.eval(bell.hi + 0.5), 0.0);
        }
    }

    #[test]
    fn the_recovery_shelves_are_c1_and_keep_white_and_their_stated_slopes() {
        for shelf in [RAW_RECOVERY, RASTER_RECOVERY] {
            assert_eq!(shelf.weight(shelf.start), 0.0);
            assert_eq!(shelf.weight(shelf.full), 1.0);
            assert_eq!(shelf.weight(1.0), 0.0);
            assert_eq!(shelf.weight(0.2), 0.0);
            // C1 where the rise meets the return: both are flat there.
            let h = 1e-7;
            let before = (shelf.weight(shelf.full) - shelf.weight(shelf.full - h)) / h;
            let after = (shelf.weight(shelf.full + h) - shelf.weight(shelf.full)) / h;
            assert!(before.abs() < 1e-3 && after.abs() < 1e-3, "{before} {after}");
        }
        let raster = ZoneCurve::new(-100.0, 0.0, FileKind::Raster);
        let h = 1e-7;
        let at_white = (raster.eval_d(1.0) - raster.eval_d(1.0 - h)) / h;
        let stated = 1.0 + 2.0 * RASTER_RECOVERY.drop / (1.0 - RASTER_RECOVERY.full);
        assert!((at_white - stated).abs() < 1e-3, "{at_white} vs {stated}");
        // The raw shelf keeps steepening toward white.
        let raw = ZoneCurve::new(-100.0, 0.0, FileKind::Raw);
        let slope = |v: f64| (raw.eval_d(v + 1e-9) - raw.eval_d(v)) / 1e-9;
        assert!(slope(0.9999) > slope(0.999) && slope(0.999) > slope(0.99));
        for curve in [raster, raw] {
            assert_eq!(curve.eval_d(1.0), 1.0);
            assert_eq!(curve.eval_d(1.3), 1.3);
        }
    }

    #[test]
    fn every_zone_curve_keeps_its_stated_minimum_slope() {
        // One slider alone keeps the minimum stated beside its constants.
        let alone = |highlights: f32, shadows: f32, kind| {
            least_slope(&ZoneCurve::new(highlights, shadows, kind))
        };
        for kind in KINDS {
            assert!((alone(0.0, 100.0, kind) - 0.162).abs() < 2e-3);
            assert!((alone(0.0, -100.0, kind) - 0.437).abs() < 2e-3);
            assert!((alone(100.0, 0.0, kind) - 0.25).abs() < 2e-3);
        }
        assert!((alone(-100.0, 0.0, FileKind::Raw) - 0.377).abs() < 2e-3);
        assert!((alone(-100.0, 0.0, FileKind::Raster) - 0.325).abs() < 2e-3);
        // Composed, the slopes multiply where the two bands overlap: never
        // below the product of the two smallest minima.
        for kind in KINDS {
            for highlights in [-100.0, 0.0, 100.0] {
                for shadows in [-100.0, 0.0, 100.0] {
                    let least = alone(highlights, shadows, kind);
                    assert!(least > 0.162 * 0.325, "{kind:?} {highlights}/{shadows}: {least}");
                }
            }
        }
    }

    #[test]
    fn the_stated_peaks_are_where_the_controls_shift_most() {
        let argmax = |f: &dyn Fn(f64) -> f64| {
            (1..10_000)
                .map(|step| f64::from(step) / 10_000.0)
                .max_by(|a, b| f(*a).abs().total_cmp(&f(*b).abs()))
                .unwrap()
        };
        let shadows = ZoneCurve::new(0.0, 100.0, FileKind::Raster);
        let found = argmax(&|v| shadows.eval_d(v) - v);
        assert!((found - f64::from(SHADOWS_PEAK)).abs() < 2e-3, "{found}");
        // Highlights peaks at a different place in each direction and for
        // each kind of source; the published sample point is well inside
        // all of them.
        let lift = ZoneCurve::new(100.0, 0.0, FileKind::Raster);
        assert!((argmax(&|v| lift.eval_d(v) - v) - HIGHLIGHTS_LIFT_BELL.peak).abs() < 2e-3);
        let sample = f64::from(HIGHLIGHTS_PEAK);
        assert!(lift.eval_d(sample) - sample > 0.8 * HIGHLIGHTS_LIFT);
        for (kind, shelf) in [(FileKind::Raw, RAW_RECOVERY), (FileKind::Raster, RASTER_RECOVERY)] {
            let pull = ZoneCurve::new(-100.0, 0.0, kind);
            assert!((argmax(&|v| pull.eval_d(v) - v) - shelf.full).abs() < 2e-3, "{kind:?}");
            assert!(sample - pull.eval_d(sample) > 0.8 * shelf.drop, "{kind:?}");
        }
        for blacks in [
            EndpointCurve::new(0.0, 100.0),
            EndpointCurve::new(0.0, -100.0),
        ] {
            assert!((argmax(&|v| blacks.eval_d(v) - v) - f64::from(BLACKS_PEAK)).abs() < 2e-3);
        }
        let whites = EndpointCurve::new(100.0, 0.0);
        assert!(argmax(&|v| whites.eval_d(v) - v) > f64::from(WHITES_PEAK));
    }

    #[test]
    fn the_endpoint_curve_is_c1_at_white() {
        for whites in [-100.0, 100.0] {
            let curve = EndpointCurve::new(whites, 0.0);
            let below = (curve.eval_d(1.0) - curve.eval_d(1.0 - 1e-6)) / 1e-6;
            let above = (curve.eval_d(1.0 + 1e-6) - curve.eval_d(1.0)) / 1e-6;
            assert!((below - above).abs() < 1e-3, "{below} vs {above}");
        }
    }

    #[test]
    fn contrast_is_its_own_inverse_at_the_opposite_amount() {
        for step in 0..=20 {
            let v = f64::from(step) / 20.0;
            let back = contrast_d(contrast_d(v, 70.0), -70.0);
            assert!((back - v).abs() < 1e-12);
        }
    }
}
