//! Colour grading: a tint and a lightness offset for shadows, midtones and
//! highlights, plus a global one, applied in Oklab.
//!
//! The three ranges are weights over Oklab lightness that sum to one. A tint
//! adds chroma in proportion to lightness, which keeps black black and is
//! close to multiplying the linear colour by a fixed tint, the way a gel
//! over a light would.

use photo_core::colorimetry::{RgbSpace, WORKING_SPACE, mat3_apply, rgb_to_rgb};
use photo_core::develop::{ColorGrading, GradeWheel};

use crate::math::{slider_unit, smoothstep};
use crate::oklab::working_to_oklab;

// ---- Tunable constants ----------------------------------------------------

/// Oklab chroma added per unit of lightness at wheel saturation 100.
const TINT_RANGE: f32 = 0.15;
/// Relative lightness change at wheel luminance ±100.
const LUMINANCE_RANGE: f32 = 0.25;
/// The lightness around which shadows hand over to highlights at balance 0,
/// and how far balance ±100 moves it (positive balance lowers it, giving
/// more of the tonal range to the highlights).
const PIVOT: f32 = 0.5;
const BALANCE_RANGE: f32 = 0.25;
/// Half-width of the transitions between ranges at blending 0 and 100.
const BLEND_MIN: f32 = 0.05;
const BLEND_MAX: f32 = 0.33;

// ---------------------------------------------------------------------------

/// The two transitions that split lightness into three ranges.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ranges {
    /// Start of the shadows-to-midtones transition and one over its width.
    shadows_start: f32,
    shadows_scale: f32,
    /// Start of the midtones-to-highlights transition and one over its width.
    highlights_start: f32,
    highlights_scale: f32,
}

impl Ranges {
    fn new(blending: f32, balance: f32) -> Self {
        let blend = if blending.is_finite() {
            (blending / 100.0).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let pivot = PIVOT - BALANCE_RANGE * slider_unit(balance) as f32;
        let half_width = BLEND_MIN + (BLEND_MAX - BLEND_MIN) * blend;
        // The shadows transition is centred a third of the way below the
        // pivot, the highlights one a third of the way above it. Neither
        // may run past black or white, so the ranges are pure at the ends.
        let shadows_centre = pivot * (2.0 / 3.0);
        let highlights_centre = pivot + (1.0 - pivot) / 3.0;
        let shadows_half = half_width.min(shadows_centre);
        let highlights_half = half_width.min(1.0 - highlights_centre);
        Self {
            shadows_start: shadows_centre - shadows_half,
            shadows_scale: 0.5 / shadows_half,
            highlights_start: highlights_centre - highlights_half,
            highlights_scale: 0.5 / highlights_half,
        }
    }

    /// `[shadows, midtones, highlights]` at a lightness. The highlights
    /// transition starts and ends later than the shadows one, so the
    /// midtones weight (their difference) is never negative.
    #[inline(always)]
    fn weights(&self, lightness: f32) -> [f32; 3] {
        let past_shadows = smoothstep((lightness - self.shadows_start) * self.shadows_scale);
        let into_highlights =
            smoothstep((lightness - self.highlights_start) * self.highlights_scale);
        [
            1.0 - past_shadows,
            past_shadows - into_highlights,
            into_highlights,
        ]
    }
}

/// The weights of the shadows, midtones and highlights ranges at an Oklab
/// lightness (0 to 1), for `blending` 0 to 100 and `balance` -100 to 100.
/// They are smooth, lie in 0 to 1 and sum to one; shadows is 1 at black and
/// highlights is 1 at white. More blending widens the overlaps; positive
/// balance moves the split down, toward the highlights.
pub fn grading_weights(lightness: f32, blending: f32, balance: f32) -> [f32; 3] {
    Ranges::new(blending, balance).weights(lightness)
}

/// One wheel compiled: the tint as an `(a, b)` offset per unit of lightness
/// and the relative lightness change.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Wheel {
    a: f32,
    b: f32,
    lightness: f32,
}

impl Wheel {
    fn new(wheel: &GradeWheel) -> Self {
        let saturation = if wheel.saturation.is_finite() {
            (wheel.saturation / 100.0).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (a, b) = wheel_direction(wheel.hue);
        Self {
            a: a * saturation * TINT_RANGE,
            b: b * saturation * TINT_RANGE,
            lightness: slider_unit(wheel.luminance) as f32 * LUMINANCE_RANGE,
        }
    }
}

/// The unit Oklab `(a, b)` direction of a wheel hue. Wheel hues are the
/// familiar HSV ones (0 red, 120 green, 240 blue), so the direction is the
/// Oklab hue of the fully saturated sRGB colour at that HSV hue.
fn wheel_direction(hue_degrees: f32) -> (f32, f32) {
    let hue = if hue_degrees.is_finite() {
        hue_degrees.rem_euclid(360.0)
    } else {
        0.0
    };
    let sector = hue / 60.0;
    let rising = sector.fract();
    let encoded = match sector as u32 % 6 {
        0 => [1.0, rising, 0.0],
        1 => [1.0 - rising, 1.0, 0.0],
        2 => [0.0, 1.0, rising],
        3 => [0.0, 1.0 - rising, 1.0],
        4 => [rising, 0.0, 1.0],
        _ => [1.0, 0.0, 1.0 - rising],
    };
    let linear = encoded.map(|c| RgbSpace::Srgb.decode(c));
    let lab = working_to_oklab(mat3_apply(
        &rgb_to_rgb(RgbSpace::Srgb, WORKING_SPACE),
        linear,
    ));
    let chroma = lab[1].hypot(lab[2]);
    if chroma > 0.0 {
        (lab[1] / chroma, lab[2] / chroma)
    } else {
        (1.0, 0.0)
    }
}

/// Colour grading compiled for per-pixel use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Grading {
    ranges: Ranges,
    shadows: Wheel,
    midtones: Wheel,
    highlights: Wheel,
    global: Wheel,
}

impl Grading {
    /// `None` when no wheel has saturation or luminance.
    pub(super) fn new(grading: &ColorGrading) -> Option<Self> {
        (!grading.is_identity()).then(|| Self {
            ranges: Ranges::new(grading.blending, grading.balance),
            shadows: Wheel::new(&grading.shadows),
            midtones: Wheel::new(&grading.midtones),
            highlights: Wheel::new(&grading.highlights),
            global: Wheel::new(&grading.global),
        })
    }

    /// Grade an Oklab colour. Lightness never goes negative.
    #[inline(always)]
    pub(super) fn apply(&self, lab: [f32; 3]) -> [f32; 3] {
        let lightness = lab[0];
        let [s, m, h] = self.ranges.weights(lightness);
        let mix = |pick: fn(&Wheel) -> f32| {
            s * pick(&self.shadows)
                + m * pick(&self.midtones)
                + h * pick(&self.highlights)
                + pick(&self.global)
        };
        [
            lightness * (1.0 + mix(|wheel| wheel.lightness)).max(0.0),
            lab[1] + lightness * mix(|wheel| wheel.a),
            lab[2] + lightness * mix(|wheel| wheel.b),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oklab::hue_degrees;

    #[test]
    fn wheel_hues_follow_the_hsv_circle() {
        // Oklab hues of the sRGB primaries, in degrees.
        for (wheel, oklab) in [
            (0.0, 29.2),
            (120.0, 142.5),
            (240.0, 264.1),
            (360.0, 29.2),
            (-120.0, 264.1),
        ] {
            let (a, b) = wheel_direction(wheel);
            assert!((a.hypot(b) - 1.0).abs() < 1e-5);
            assert!(
                (hue_degrees(a, b) - oklab).abs() < 0.3,
                "wheel {wheel}: {}",
                hue_degrees(a, b)
            );
        }
        assert_eq!(wheel_direction(f32::NAN), wheel_direction(0.0));
    }

    #[test]
    fn the_midtones_weight_is_never_negative() {
        for blending in [0.0, 25.0, 50.0, 100.0, 400.0, f32::NAN] {
            for balance in [-300.0, -100.0, -30.0, 0.0, 30.0, 100.0, 300.0] {
                let ranges = Ranges::new(blending, balance);
                for step in 0..=1000 {
                    let weights = ranges.weights(step as f32 / 1000.0);
                    assert!(
                        weights.iter().all(|w| *w >= -1e-6),
                        "{blending}/{balance}: {weights:?}"
                    );
                }
                assert_eq!(ranges.weights(0.0), [1.0, 0.0, 0.0]);
                let white = ranges.weights(1.0);
                assert!(
                    white[0] == 0.0 && white[1].abs() < 1e-5 && white[2] > 1.0 - 1e-5,
                    "{white:?}"
                );
            }
        }
    }
}
