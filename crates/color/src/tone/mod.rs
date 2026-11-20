//! Tone (DESIGN §5.5): highlights and shadows, whites and blacks, contrast,
//! the parametric curve and the point curves.
//!
//! All curves here are defined on perceptual values `v = Y^(1/2.2)` with the
//! pivot at `v = 0.5`. The pure curve functions are public so they can be
//! tested and plotted; [`ToneStage`] compiles them into tables for the
//! per-pixel path.

mod parametric;
mod point;
mod shape;
mod stage;

use std::sync::LazyLock;

use photo_core::colorimetry::luminance;
use photo_core::develop::{Tone, ToneCurve};

use crate::lut::Lut;
use crate::math::non_negative;

pub use parametric::ParametricShape;
pub use point::MonotoneCubic;
pub use shape::{
    BLACKS_PEAK, EndpointCurve, HIGHLIGHTS_PEAK, SHADOWS_PEAK, WHITES_PEAK, ZoneCurve,
    contrast_curve,
};
pub use stage::{ToneDelta, ToneStage};

/// The exponent of the perceptual encoding the tone controls work in.
pub const PERCEPTUAL_GAMMA: f32 = 2.2;
const GAMMA: f64 = 2.2;

/// The fast encoder's table covers linear values up to this; brighter ones
/// fall back to `powf`.
const ENCODE_TABLE_MAX: f32 = 16.0;

/// `linear^(1/2.2)` sampled over the fourth root of the linear value, which
/// keeps the table accurate down to black (the encoding itself has an
/// infinite slope there) for the price of two square roots per lookup.
static ENCODE: LazyLock<Lut> = LazyLock::new(|| Lut::build(2048, 2.0, |t| t.powf(4.0 / GAMMA)));

pub(crate) fn encode_d(linear: f64) -> f64 {
    if linear > 0.0 {
        linear.powf(1.0 / GAMMA)
    } else {
        0.0
    }
}

pub(crate) fn decode_d(v: f64) -> f64 {
    if v > 0.0 { v.powf(GAMMA) } else { 0.0 }
}

/// Linear light to the perceptual value `linear^(1/2.2)`, from a table:
/// accurate to about 3e-6 and monotonic. Zero for negative or NaN input.
#[inline]
pub fn encode_perceptual(linear: f32) -> f32 {
    if linear > ENCODE_TABLE_MAX {
        linear.min(f32::MAX).powf(1.0 / PERCEPTUAL_GAMMA)
    } else if linear > 0.0 {
        ENCODE.eval(linear.sqrt().sqrt())
    } else {
        0.0
    }
}

/// Perceptual value back to linear light, `v^2.2` (exact). Zero for
/// negative or NaN input.
#[inline]
pub fn decode_perceptual(v: f32) -> f32 {
    if v > 0.0 {
        v.powf(PERCEPTUAL_GAMMA)
    } else {
        0.0
    }
}

/// The value the pipeline filters to obtain the edge-aware base for
/// Highlights and Shadows: the perceptual luminance of a display-linear
/// working RGB pixel. Feed the filtered plane back as the `base` argument of
/// [`ToneStage::apply`]; passing this value unfiltered turns the zone curve
/// into a plain global curve.
#[inline]
pub fn tone_base_value(rgb: [f32; 3]) -> f32 {
    encode_perceptual(luminance(non_negative(rgb)))
}

/// Every tone curve of a set of settings, as functions of a perceptual
/// value. This is what a curve editor plots and what [`ToneStage`] samples.
#[derive(Clone, Debug, PartialEq)]
pub struct ToneCurves {
    zone: ZoneCurve,
    endpoints: EndpointCurve,
    contrast: f64,
    parametric: ParametricShape,
    point: Option<MonotoneCubic>,
    channels: [Option<MonotoneCubic>; 3],
}

impl ToneCurves {
    /// Compile the Basic-panel tone sliders (exposure is not part of the
    /// tone curve) and the Tone Curve panel.
    pub fn new(tone: &Tone, curve: &ToneCurve) -> Self {
        Self {
            zone: ZoneCurve::new(tone.highlights, tone.shadows),
            endpoints: EndpointCurve::new(tone.whites, tone.blacks),
            contrast: if tone.contrast.is_finite() {
                f64::from(tone.contrast)
            } else {
                0.0
            },
            parametric: ParametricShape::new(&curve.parametric),
            point: MonotoneCubic::non_identity(&curve.rgb),
            channels: [
                MonotoneCubic::non_identity(&curve.red),
                MonotoneCubic::non_identity(&curve.green),
                MonotoneCubic::non_identity(&curve.blue),
            ],
        }
    }

    /// The composite tone curve at perceptual `v`: highlights and shadows
    /// (with the base equal to the value itself), whites and blacks,
    /// contrast, the parametric curve and the RGB point curve, in that
    /// order. This is the response of a neutral pixel; the per-channel
    /// curves are separate ([`Self::channel`]). Exactly `v` when every
    /// control is neutral.
    pub fn eval(&self, v: f32) -> f32 {
        let v = f64::from(v);
        self.shared_d(self.endpoints.eval_d(self.zone.eval_d(v))) as f32
    }

    /// The highlights/shadows zone curve alone.
    pub fn zone(&self) -> &ZoneCurve {
        &self.zone
    }

    /// The whites/blacks end-point curve alone.
    pub fn endpoints(&self) -> &EndpointCurve {
        &self.endpoints
    }

    /// The parametric curve alone.
    pub fn parametric(&self) -> &ParametricShape {
        &self.parametric
    }

    /// The red (0), green (1) or blue (2) point curve at perceptual `v`.
    /// Other indices, and channels without a curve, return `v`.
    pub fn channel(&self, index: usize, v: f32) -> f32 {
        match self.channels.get(index) {
            Some(Some(curve)) => curve.eval(v),
            _ => v,
        }
    }

    pub(crate) fn channel_curve(&self, index: usize) -> Option<&MonotoneCubic> {
        self.channels[index].as_ref()
    }

    pub(crate) fn contrast(&self) -> f64 {
        self.contrast
    }

    /// True when the parametric curve and the RGB point curve do nothing.
    pub(crate) fn curves_are_identity(&self) -> bool {
        self.parametric.is_identity() && self.point.is_none()
    }

    /// The parametric curve followed by the RGB point curve.
    pub(crate) fn curves_d(&self, v: f64) -> f64 {
        let v = self.parametric.eval_d(v);
        match &self.point {
            Some(point) => point.eval_d(v),
            None => v,
        }
    }

    /// Contrast, the parametric curve and the RGB point curve: the part of
    /// the composite that is applied with the hue-preserving RGB method.
    pub(crate) fn shared_d(&self, v: f64) -> f64 {
        self.curves_d(shape::contrast_d(v, self.contrast))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fast_encoder_is_monotonic_and_meets_the_exact_one_at_the_table_edge() {
        let mut previous = -1.0;
        for step in 0..=20_000 {
            let linear = (step as f32 / 5000.0).powi(2);
            let value = encode_perceptual(linear);
            assert!(value >= previous, "dip at {linear}");
            previous = value;
        }
        let below = encode_perceptual(ENCODE_TABLE_MAX);
        let above = encode_perceptual(ENCODE_TABLE_MAX * 1.000_001);
        assert!((below - above).abs() < 1e-5);
        assert_eq!(encode_perceptual(f32::NAN), 0.0);
        assert!(encode_perceptual(f32::INFINITY).is_finite());
    }

    #[test]
    fn the_composite_is_the_documented_composition() {
        let tone = Tone {
            exposure: 0.0,
            contrast: 40.0,
            highlights: -30.0,
            shadows: 20.0,
            whites: 10.0,
            blacks: -15.0,
        };
        let mut curve = ToneCurve::default();
        curve.parametric.darks = 50.0;
        let curves = ToneCurves::new(&tone, &curve);
        for step in 0..=20 {
            let v = step as f32 / 20.0;
            let expected = curves.parametric().eval(contrast_curve(
                curves.endpoints().eval(curves.zone().eval(v)),
                40.0,
            ));
            assert!((curves.eval(v) - expected).abs() < 1e-6);
        }
    }
}
