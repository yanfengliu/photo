//! The parametric tone curve: four region sliders and three splits.
//!
//! Each region is a bump centred in its own region that fades out across the
//! neighbouring one. The four bumps are applied one after another (shadows,
//! darks, lights, highlights), each as `v + shift * bell(v)`, and a slider at
//! ±100 uses exactly the largest shift that keeps its curve's slope at
//! [`MIN_SLOPE`], whatever the splits are. The result is therefore strictly
//! increasing for every combination of sliders and splits, with the end
//! points fixed.

use photo_core::develop::ParametricCurve;

use super::shape::Bell;
use crate::math::slider_unit;

// ---- Tunable constants ----------------------------------------------------

/// The slope each region's curve keeps at full deflection. A smoothstep
/// flank of width `w` has a maximum slope of `1.5 / w`, so the shift allowed
/// against that flank is `(1 - MIN_SLOPE) * w / 1.5`.
const MIN_SLOPE: f64 = 0.25;
/// Splits are kept this far from the ends and from each other.
const SPLIT_MARGIN: f64 = 0.05;

// ---------------------------------------------------------------------------

/// A compiled [`ParametricCurve`], evaluated on perceptual values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParametricShape {
    regions: [(Bell, f64); 4],
}

impl ParametricShape {
    /// Compile the curve. Splits that are out of order or too close are
    /// repaired (each is kept at least 5% from its neighbours and from the
    /// ends); slider values beyond ±100 are clamped.
    pub fn new(curve: &ParametricCurve) -> Self {
        let fraction = |split: f32, fallback: f64| {
            if split.is_finite() {
                f64::from(split) / 100.0
            } else {
                fallback
            }
        };
        let s1 = fraction(curve.split_shadows, 0.25).clamp(SPLIT_MARGIN, 1.0 - 3.0 * SPLIT_MARGIN);
        // `max` then `min` rather than `clamp` from here on: these bounds
        // are sums of margins and may cross by a rounding error.
        let s2 = fraction(curve.split_midtones, 0.5)
            .max(s1 + SPLIT_MARGIN)
            .min(1.0 - 2.0 * SPLIT_MARGIN);
        let s3 = fraction(curve.split_highlights, 0.75)
            .max(s2 + SPLIT_MARGIN)
            .min(1.0 - SPLIT_MARGIN);

        let region = |lo: f64, peak: f64, hi: f64, slider: f32| {
            let bell = Bell { lo, peak, hi };
            let amount = slider_unit(slider);
            // Raising the bump flattens its falling flank, lowering it the
            // rising one.
            let flank = if amount >= 0.0 { hi - peak } else { peak - lo };
            (bell, amount * (1.0 - MIN_SLOPE) * flank / 1.5)
        };
        Self {
            regions: [
                region(0.0, 0.5 * s1, s2, curve.shadows),
                region(0.0, 0.5 * (s1 + s2), s3, curve.darks),
                region(s1, 0.5 * (s2 + s3), 1.0, curve.lights),
                region(s2, 0.5 * (s3 + 1.0), 1.0, curve.highlights),
            ],
        }
    }

    /// True when all four region sliders are zero.
    pub fn is_identity(&self) -> bool {
        self.regions.iter().all(|(_, shift)| *shift == 0.0)
    }

    /// The perceptual value at the centre of each region (shadows, darks,
    /// lights, highlights), where that region's slider acts most.
    pub fn region_centres(&self) -> [f32; 4] {
        self.regions.map(|(bell, _)| bell.peak as f32)
    }

    /// The curve's value at perceptual `v`. Identity outside `(0, 1)`.
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
        self.regions
            .iter()
            .fold(v, |value, (bell, shift)| value + shift * bell.eval(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(sliders: [f32; 4], splits: [f32; 3]) -> ParametricCurve {
        ParametricCurve {
            shadows: sliders[0],
            darks: sliders[1],
            lights: sliders[2],
            highlights: sliders[3],
            split_shadows: splits[0],
            split_midtones: splits[1],
            split_highlights: splits[2],
        }
    }

    #[test]
    fn the_default_curve_is_the_identity() {
        let shape = ParametricShape::new(&ParametricCurve::default());
        assert!(shape.is_identity());
        for step in 0..=100 {
            let v = step as f32 / 100.0;
            assert_eq!(shape.eval(v), v);
        }
        assert_eq!(shape.region_centres(), [0.125, 0.375, 0.625, 0.875]);
    }

    #[test]
    fn each_region_slider_moves_its_own_region_most_and_in_its_direction() {
        let neutral = ParametricShape::new(&ParametricCurve::default());
        let centres = neutral.region_centres();
        for region in 0..4 {
            for amount in [-100.0, -40.0, 40.0, 100.0] {
                let mut sliders = [0.0; 4];
                sliders[region] = amount;
                let shape = ParametricShape::new(&curve(sliders, [25.0, 50.0, 75.0]));
                let shift = |v: f32| shape.eval(v) - v;
                let own = shift(centres[region]);
                assert!(
                    own * amount > 0.0 && own.abs() > 0.01,
                    "region {region} at {amount}: {own}"
                );
                for (other, centre) in centres.iter().enumerate() {
                    if other != region {
                        assert!(
                            own.abs() > shift(*centre).abs(),
                            "region {region} moves region {other} more"
                        );
                    }
                }
                assert_eq!(shape.eval(0.0), 0.0);
                assert_eq!(shape.eval(1.0), 1.0);
            }
        }
    }

    #[test]
    fn every_combination_of_extremes_and_splits_is_strictly_increasing() {
        let split_sets = [
            [25.0, 50.0, 75.0],
            [5.0, 10.0, 15.0],
            [85.0, 90.0, 95.0],
            [10.0, 50.0, 90.0],
            [0.0, 0.0, 0.0],
            [100.0, 100.0, 100.0],
            [80.0, 20.0, 50.0],
            [f32::NAN, 50.0, f32::INFINITY],
        ];
        for splits in split_sets {
            for mask in 0..16u32 {
                let pick = |bit: u32| {
                    if mask & (1 << bit) != 0 {
                        100.0
                    } else {
                        -100.0
                    }
                };
                let shape =
                    ParametricShape::new(&curve([pick(0), pick(1), pick(2), pick(3)], splits));
                let mut previous = shape.eval_d(0.0);
                for step in 1..=2000 {
                    let v = f64::from(step) / 2000.0;
                    let value = shape.eval_d(v);
                    let slope = (value - previous) * 2000.0;
                    assert!(
                        slope > 0.003,
                        "splits {splits:?}, sliders {mask:04b}: slope {slope} at {v}"
                    );
                    previous = value;
                }
                assert!((previous - 1.0).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn one_region_at_full_deflection_keeps_the_stated_minimum_slope() {
        for region in 0..4 {
            for amount in [-100.0, 100.0] {
                let mut sliders = [0.0; 4];
                sliders[region] = amount;
                let shape = ParametricShape::new(&curve(sliders, [30.0, 55.0, 70.0]));
                let mut least = f64::MAX;
                for step in 0..4000 {
                    let v = f64::from(step) / 4000.0;
                    least = least.min((shape.eval_d(v + 0.00025) - shape.eval_d(v)) * 4000.0);
                }
                assert!(
                    (least - MIN_SLOPE).abs() < 2e-3,
                    "region {region} at {amount}: least slope {least}"
                );
            }
        }
    }
}
