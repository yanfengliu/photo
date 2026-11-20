//! The tone stage compiled for per-pixel use (pipeline stage 6).
//!
//! Highlights and Shadows are an exposure gain looked up from an edge-aware
//! base of perceptual luminance, `gain = (Z(b) / b)^2.2`, which is the
//! structure of darktable's tone equalizer (`src/iop/toneequal.c`); the base
//! is filtered by the caller (guided filter of He, Sun and Tang, 2010).
//! Whites and Blacks are a gain looked up from the pixel's own luminance.
//! Contrast, the parametric curve and the RGB point curve are one table
//! applied with the hue-preserving RGB tone method, and the per-channel
//! point curves follow, one table each.

use std::sync::OnceLock;

use photo_core::colorimetry::luminance;
use photo_core::develop::{Tone, ToneCurve};

use super::shape::{EndpointCurve, ZoneCurve, contrast_d};
use super::{GAMMA, ToneCurves, decode_d, encode_d, encode_perceptual};
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

/// Per-pixel additions to the global tone sliders: the alpha-weighted sum of
/// the mask adjustments covering a pixel, in slider units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ToneDelta {
    /// Added to Contrast.
    pub contrast: f32,
    /// Added to Highlights.
    pub highlights: f32,
    /// Added to Shadows.
    pub shadows: f32,
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
    curves: ToneCurves,
    zone_gain: Option<Lut>,
    endpoint_gain: Option<Lut>,
    shared: Option<LinearCurveLut>,
    channels: [Option<LinearCurveLut>; 3],
    /// `decode(curves(v))` over perceptual `v`, for pixels whose contrast
    /// differs from the global one. Built on first use: most renders have
    /// no such pixel.
    after_contrast: OnceLock<Lut>,
}

impl ToneStage {
    /// Compile the tone sliders (exposure is handled by stage 5 and ignored
    /// here) and the Tone Curve panel. Only the tables that do something are
    /// built.
    pub fn new(tone: &Tone, curve: &ToneCurve) -> Self {
        let curves = ToneCurves::new(tone, curve);
        let zone = *curves.zone();
        let endpoints = *curves.endpoints();
        let zone_gain =
            (!zone.is_identity()).then(|| Lut::build(CELLS, 1.0, |b| zone.ratio_d(b).powf(GAMMA)));
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
            zone_gain,
            endpoint_gain,
            shared,
            channels,
            after_contrast: OnceLock::new(),
        }
    }

    /// True when [`Self::apply`] returns every non-negative input bit for
    /// bit, so the stage can be skipped.
    pub fn is_identity(&self) -> bool {
        self.zone_gain.is_none()
            && self.endpoint_gain.is_none()
            && self.shared.is_none()
            && self.channels.iter().all(Option::is_none)
    }

    /// True when the global Highlights or Shadows slider is active, i.e.
    /// [`Self::apply`] reads its `base` argument. (A [`ToneDelta`] with
    /// highlights or shadows reads it too.)
    pub fn needs_base(&self) -> bool {
        self.zone_gain.is_some()
    }

    /// The curves this stage was compiled from.
    pub fn curves(&self) -> &ToneCurves {
        &self.curves
    }

    /// Apply the stage to one pixel. `base` is the edge-aware base for this
    /// pixel: [`super::tone_base_value`] of the stage's input, filtered by
    /// the caller (pass it unfiltered for a plain global curve). It is only
    /// read when [`Self::needs_base`] is true.
    ///
    /// The result is finite and non-negative for every input. Values above
    /// 1.0 are kept, not clipped.
    #[inline]
    pub fn apply(&self, rgb: [f32; 3], base: f32) -> [f32; 3] {
        let mut rgb = non_negative(rgb);
        if self.zone_gain.is_some() || self.endpoint_gain.is_some() {
            let y = luminance(rgb);
            let mut gain = 1.0;
            if let Some(table) = &self.zone_gain {
                gain = table.eval(base);
            }
            if let Some(table) = &self.endpoint_gain {
                gain *= table.eval((y * gain).sqrt());
            }
            rgb = scale3(rgb, gain);
        }
        self.finish(rgb)
    }

    /// Apply the stage with `delta` added to the global sliders for this
    /// pixel (each sum is clamped to the slider's own range, contrast
    /// excepted). A zero delta is exactly [`Self::apply`]. Otherwise the
    /// zone, end-point and contrast curves are evaluated directly for the
    /// summed values instead of being read from the global tables, which is
    /// several times slower and agrees with the tables to about 1e-4.
    pub fn apply_with_delta(&self, rgb: [f32; 3], base: f32, delta: &ToneDelta) -> [f32; 3] {
        if delta.is_zero() {
            return self.apply(rgb, base);
        }
        let rgb = non_negative(rgb);
        let add = |global: f32, local: f32| {
            if local.is_finite() {
                global + local
            } else {
                global
            }
        };
        let zone = ZoneCurve::new(
            add(self.sliders.highlights, delta.highlights),
            add(self.sliders.shadows, delta.shadows),
        );
        let endpoints = EndpointCurve::new(
            add(self.sliders.whites, delta.whites),
            add(self.sliders.blacks, delta.blacks),
        );
        let zone_ratio = zone.ratio_d(f64::from(base));
        let own = f64::from(encode_perceptual(luminance(rgb))) * zone_ratio;
        let gain = (zone_ratio * endpoints.ratio_d(own)).powf(GAMMA) as f32;
        let rgb = scale3(rgb, gain);

        if delta.contrast == 0.0 || !delta.contrast.is_finite() {
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
    use crate::tone::tone_base_value;

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
        assert!(
            contrast.shared.is_some()
                && contrast.zone_gain.is_none()
                && contrast.endpoint_gain.is_none()
        );
        assert!(!contrast.needs_base());
        // Exposure belongs to stage 5.
        let exposure = ToneStage::new(
            &Tone {
                exposure: 2.0,
                ..Default::default()
            },
            &ToneCurve::default(),
        );
        assert!(exposure.is_identity());
    }

    #[test]
    fn the_tables_agree_with_direct_evaluation() {
        let tone = Tone {
            exposure: 0.0,
            contrast: 55.0,
            highlights: -80.0,
            shadows: 70.0,
            whites: 40.0,
            blacks: -60.0,
        };
        let stage = ToneStage::new(&tone, &ToneCurve::default());
        // A delta that changes nothing numerically but forces the direct path.
        let nudge = ToneDelta {
            contrast: 1.0e-6,
            ..Default::default()
        };
        for step in 0..=400 {
            let rgb = [
                step as f32 / 400.0,
                step as f32 / 500.0,
                step as f32 / 300.0,
            ];
            let base = tone_base_value(rgb);
            let (fast, direct) = (
                stage.apply(rgb, base),
                stage.apply_with_delta(rgb, base, &nudge),
            );
            for channel in 0..3 {
                assert!(
                    (fast[channel] - direct[channel]).abs() < 1e-4,
                    "{rgb:?}: {fast:?} vs {direct:?}"
                );
            }
        }
    }
}
