//! The tone stage compiled for per-pixel use (pipeline stage 6): the
//! display-referred tone controls.
//!
//! Whites and Blacks are a gain looked up from the pixel's own luminance.
//! Contrast, the parametric curve and the RGB point curve are one table
//! applied with the hue-preserving RGB tone method, and the per-channel
//! point curves follow, one table each. Highlights and Shadows are not here:
//! they act before the base curve, in stage 5.

use std::sync::OnceLock;

use photo_core::colorimetry::luminance;
use photo_core::develop::{Tone, ToneCurve};

use super::shape::{EndpointCurve, contrast_d};
use super::{DisplayCurves, GAMMA, decode_d, encode_d, encode_perceptual};
use crate::base::rgb_tone;
use crate::lut::{LinearCurveLut, Lut, extend_above_one};
use crate::math::{non_negative, scale3};

// ---- Tunable constants ----------------------------------------------------

/// Samples per table. Tables are rebuilt on every settings change, so this
/// trades build time (well under 0.1 ms each) against interpolation error
/// (below 1e-5 for the curves these controls can produce).
const CELLS: usize = 1024;
/// The whites/blacks gain table is indexed by the square root of linear
/// luminance and covers values up to this (linear 4.0); beyond it the gain
/// stays constant.
const GAIN_INDEX_MAX: f64 = 2.0;

// ---------------------------------------------------------------------------

/// Per-pixel additions to the global stage-6 sliders: the alpha-weighted sum
/// of the mask adjustments covering a pixel, in slider units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ToneDelta {
    /// Added to Contrast.
    pub contrast: f32,
    /// Added to Whites.
    pub whites: f32,
    /// Added to Blacks.
    pub blacks: f32,
}

impl ToneDelta {
    /// True when every field is zero.
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }
}

/// Stage 6 compiled for one set of tone settings: display-linear working RGB
/// in, display-linear working RGB out.
#[derive(Clone, Debug)]
pub struct ToneStage {
    sliders: Tone,
    curves: DisplayCurves,
    endpoint_gain: Option<Lut>,
    shared: Option<LinearCurveLut>,
    channels: [Option<LinearCurveLut>; 3],
    /// `decode(curves(v))` over perceptual `v`, for pixels whose contrast
    /// differs from the global one. Built on first use: most renders have
    /// no such pixel.
    after_contrast: OnceLock<Lut>,
}

impl ToneStage {
    /// Compile Whites, Blacks and Contrast from `tone` (its Exposure,
    /// Highlights and Shadows belong to stage 5 and are ignored here) and
    /// the Tone Curve panel. Only the tables that do something are built.
    pub fn new(tone: &Tone, curve: &ToneCurve) -> Self {
        let curves = DisplayCurves::new(tone, curve);
        let endpoints = curves.endpoints;
        let endpoint_gain = (!endpoints.is_identity()).then(|| {
            Lut::build(CELLS, GAIN_INDEX_MAX, |t| {
                endpoints.ratio_d(encode_d(t * t)).powf(GAMMA)
            })
        });
        let shared = (curves.contrast() != 0.0 || !curves.curves_are_identity()).then(|| {
            LinearCurveLut::build(CELLS, |linear| decode_d(curves.shared_d(encode_d(linear))))
        });
        let channels = [0, 1, 2].map(|index| {
            curves.channel_curve(index).map(|point| {
                LinearCurveLut::build(CELLS, |linear| decode_d(point.eval_d(encode_d(linear))))
            })
        });
        Self {
            sliders: *tone,
            curves,
            endpoint_gain,
            shared,
            channels,
            after_contrast: OnceLock::new(),
        }
    }

    /// True when [`Self::apply`] returns every non-negative input bit for
    /// bit, so the stage can be skipped.
    pub fn is_identity(&self) -> bool {
        self.endpoint_gain.is_none()
            && self.shared.is_none()
            && self.channels.iter().all(Option::is_none)
    }

    /// Apply the stage to one pixel. The result is finite and non-negative
    /// for every input. Values above 1.0 are kept, not clipped.
    #[inline]
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut rgb = non_negative(rgb);
        if let Some(table) = &self.endpoint_gain {
            rgb = scale3(rgb, table.eval(luminance(rgb).sqrt()));
        }
        self.finish(rgb)
    }

    /// Apply the stage with `delta` added to the global sliders for this
    /// pixel (Whites and Blacks sums are clamped to the sliders' range,
    /// Contrast is not). A zero delta is exactly [`Self::apply`]. Otherwise
    /// the end-point and contrast curves are evaluated directly for the
    /// summed values instead of being read from the global tables, which is
    /// several times slower and agrees with the tables to about 1e-4.
    pub fn apply_with_delta(&self, rgb: [f32; 3], delta: &ToneDelta) -> [f32; 3] {
        if delta.is_zero() {
            return self.apply(rgb);
        }
        let mut rgb = non_negative(rgb);
        let changed = |local: f32| local != 0.0 && local.is_finite();
        if changed(delta.whites) || changed(delta.blacks) {
            let add = |global: f32, local: f32| {
                if changed(local) {
                    global + local
                } else {
                    global
                }
            };
            let endpoints = EndpointCurve::new(
                add(self.sliders.whites, delta.whites),
                add(self.sliders.blacks, delta.blacks),
            );
            let own = f64::from(encode_perceptual(luminance(rgb)));
            rgb = scale3(rgb, endpoints.ratio_d(own).powf(GAMMA) as f32);
        } else if let Some(table) = &self.endpoint_gain {
            rgb = scale3(rgb, table.eval(luminance(rgb).sqrt()));
        }

        if !changed(delta.contrast) {
            return self.finish(rgb);
        }
        let contrast = self.curves.contrast() + f64::from(delta.contrast);
        // Contrast depends on the pixel, but what follows it does not: the
        // parametric and point curves and the decoding come from one table,
        // built the first time a pixel needs it.
        let after_contrast = self
            .after_contrast
            .get_or_init(|| Lut::build(CELLS, 1.0, |v| decode_d(self.curves.curves_d(v))));
        let toned = rgb_tone(rgb, |linear| {
            if linear < 1.0 {
                let v = contrast_d(f64::from(encode_perceptual(linear)), contrast);
                after_contrast.eval(v as f32)
            } else {
                // Over-range values follow the same rule as the global
                // table. Its slope is a difference of two nearby values, so
                // it is taken from the exact curve, not from a table.
                extend_above_one(CELLS, f64::from(linear), |linear| {
                    decode_d(self.curves.curves_d(contrast_d(encode_d(linear), contrast)))
                }) as f32
            }
        });
        self.channel_curves(non_negative(toned))
    }

    /// The shared curve, then the per-channel curves.
    #[inline(always)]
    fn finish(&self, rgb: [f32; 3]) -> [f32; 3] {
        let rgb = match &self.shared {
            Some(table) => rgb_tone(rgb, |linear| table.eval(linear)),
            None => rgb,
        };
        self.channel_curves(rgb)
    }

    #[inline(always)]
    fn channel_curves(&self, mut rgb: [f32; 3]) -> [f32; 3] {
        for (value, table) in rgb.iter_mut().zip(&self.channels) {
            if let Some(table) = table {
                *value = table.eval(*value);
            }
        }
        rgb
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_tables_that_are_needed_get_built() {
        let neutral = ToneStage::new(&Tone::default(), &ToneCurve::default());
        assert!(neutral.is_identity());
        let contrast = ToneStage::new(
            &Tone {
                contrast: 20.0,
                ..Default::default()
            },
            &ToneCurve::default(),
        );
        assert!(contrast.shared.is_some() && contrast.endpoint_gain.is_none());
        // Exposure, Highlights and Shadows belong to stage 5.
        let stage_five = ToneStage::new(
            &Tone {
                exposure: 2.0,
                highlights: -50.0,
                shadows: 50.0,
                ..Default::default()
            },
            &ToneCurve::default(),
        );
        assert!(stage_five.is_identity());
    }

    #[test]
    fn the_tables_agree_with_direct_evaluation() {
        let tone = Tone {
            contrast: 55.0,
            whites: 40.0,
            blacks: -60.0,
            ..Default::default()
        };
        let stage = ToneStage::new(&tone, &ToneCurve::default());
        // Deltas that change nothing numerically but force the direct paths.
        let nudges = [
            ToneDelta {
                contrast: 1.0e-6,
                ..Default::default()
            },
            ToneDelta {
                whites: 1.0e-6,
                ..Default::default()
            },
        ];
        for nudge in nudges {
            for step in 0..=400 {
                let rgb = [
                    step as f32 / 400.0,
                    step as f32 / 500.0,
                    step as f32 / 300.0,
                ];
                let (fast, direct) = (stage.apply(rgb), stage.apply_with_delta(rgb, &nudge));
                for channel in 0..3 {
                    assert!(
                        (fast[channel] - direct[channel]).abs() < 1e-4,
                        "{rgb:?}: {fast:?} vs {direct:?}"
                    );
                }
            }
        }
    }
}
