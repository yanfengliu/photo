//! The tone curves as the Tone Curve panel draws them.
//!
//! The engine has no tone-curve sampler yet, and this crate may not reach
//! the colour crate that evaluates the curves, so this is a copy of that
//! evaluation for drawing only: `photo-color`'s `MonotoneCubic` (the
//! Fritsch–Carlson monotone cubic through the points) and `ParametricShape`
//! (four smoothstep bells over the regions the splits make). It never
//! touches a pixel. When the engine offers a sampler, the panel should draw
//! from it and this file should go.

use photo_core::develop::{CurvePoint, ParametricCurve};

/// The point curve through `points` (sorted by `x`, at least two) at `x`:
/// flat beyond the end points, never overshooting a point.
pub(crate) fn point_curve(points: &[CurvePoint], x: f32) -> f32 {
    let (xs, ys): (Vec<f64>, Vec<f64>) = points
        .iter()
        .map(|point| (f64::from(point.x), f64::from(point.y)))
        .unzip();
    if xs.len() < 2 {
        return x;
    }
    let x = f64::from(x);
    let last = xs.len() - 1;
    if x.is_nan() || x <= xs[0] {
        return ys[0] as f32;
    }
    if x >= xs[last] {
        return ys[last] as f32;
    }
    let tangents = fritsch_carlson(&xs, &ys);
    let i = xs.partition_point(|knot| *knot <= x) - 1;
    let h = xs[i + 1] - xs[i];
    let t = (x - xs[i]) / h;
    let (t2, t3) = (t * t, t * t * t);
    ((2.0 * t3 - 3.0 * t2 + 1.0) * ys[i]
        + (t3 - 2.0 * t2 + t) * h * tangents[i]
        + (-2.0 * t3 + 3.0 * t2) * ys[i + 1]
        + (t3 - t2) * h * tangents[i + 1]) as f32
}

/// Tangents for a monotone cubic Hermite interpolant (Fritsch and Carlson,
/// SIAM J. Numer. Anal. 17, 1980).
fn fritsch_carlson(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let n = xs.len();
    let secants: Vec<f64> = (0..n - 1)
        .map(|i| (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]))
        .collect();
    let mut tangents = vec![0.0; n];
    tangents[0] = secants[0];
    tangents[n - 1] = secants[n - 2];
    for i in 1..n - 1 {
        tangents[i] = if secants[i - 1] * secants[i] <= 0.0 {
            0.0
        } else {
            0.5 * (secants[i - 1] + secants[i])
        };
    }
    for i in 0..n - 1 {
        if secants[i] == 0.0 {
            tangents[i] = 0.0;
            tangents[i + 1] = 0.0;
            continue;
        }
        let alpha = tangents[i] / secants[i];
        let beta = tangents[i + 1] / secants[i];
        let radius = alpha * alpha + beta * beta;
        if radius > 9.0 {
            let tau = 3.0 / radius.sqrt();
            tangents[i] = tau * alpha * secants[i];
            tangents[i + 1] = tau * beta * secants[i];
        }
    }
    tangents
}

/// The slope each region keeps at full deflection.
const MIN_SLOPE: f64 = 0.25;
/// Splits are kept this far from the ends and from each other.
const SPLIT_MARGIN: f64 = 0.05;

/// A C1 bump: zero outside `(lo, hi)`, one at `peak`, smoothstep flanks.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bell {
    lo: f64,
    peak: f64,
    hi: f64,
}

impl Bell {
    fn eval(self, v: f64) -> f64 {
        let smoothstep = |t: f64| {
            let t = t.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        if v <= self.lo || v >= self.hi {
            0.0
        } else if v <= self.peak {
            smoothstep((v - self.lo) / (self.peak - self.lo))
        } else {
            1.0 - smoothstep((v - self.peak) / (self.hi - self.peak))
        }
    }
}

/// The parametric curve, compiled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Parametric {
    regions: [(Bell, f64); 4],
    /// The three splits as the curve uses them, 0 to 1.
    pub(crate) splits: [f32; 3],
}

impl Parametric {
    pub(crate) fn new(curve: &ParametricCurve) -> Self {
        let fraction = |split: f32, fallback: f64| {
            if split.is_finite() {
                f64::from(split) / 100.0
            } else {
                fallback
            }
        };
        let s1 = fraction(curve.split_shadows, 0.25).clamp(SPLIT_MARGIN, 1.0 - 3.0 * SPLIT_MARGIN);
        let s2 = fraction(curve.split_midtones, 0.5)
            .max(s1 + SPLIT_MARGIN)
            .min(1.0 - 2.0 * SPLIT_MARGIN);
        let s3 = fraction(curve.split_highlights, 0.75)
            .max(s2 + SPLIT_MARGIN)
            .min(1.0 - SPLIT_MARGIN);
        let region = |lo: f64, peak: f64, hi: f64, slider: f32| {
            let amount = if slider.is_finite() {
                (f64::from(slider) / 100.0).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let flank = if amount >= 0.0 { hi - peak } else { peak - lo };
            (Bell { lo, peak, hi }, amount * (1.0 - MIN_SLOPE) * flank / 1.5)
        };
        Self {
            regions: [
                region(0.0, 0.5 * s1, s2, curve.shadows),
                region(0.0, 0.5 * (s1 + s2), s3, curve.darks),
                region(s1, 0.5 * (s2 + s3), 1.0, curve.lights),
                region(s2, 0.5 * (s3 + 1.0), 1.0, curve.highlights),
            ],
            splits: [s1 as f32, s2 as f32, s3 as f32],
        }
    }

    /// The curve's value at `v`.
    pub(crate) fn eval(&self, v: f32) -> f32 {
        let v = f64::from(v);
        if v.is_nan() {
            return 0.0;
        }
        if v <= 0.0 || v >= 1.0 {
            return v as f32;
        }
        self.regions
            .iter()
            .fold(v, |value, (bell, shift)| value + shift * bell.eval(value)) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(pairs: &[(f32, f32)]) -> Vec<CurvePoint> {
        pairs.iter().map(|(x, y)| CurvePoint::new(*x, *y)).collect()
    }

    #[test]
    fn the_point_curve_passes_through_its_points_and_is_flat_beyond_them() {
        let curve = points(&[(0.1, 0.05), (0.3, 0.2), (0.55, 0.7), (0.9, 0.95)]);
        for point in &curve {
            assert!((point_curve(&curve, point.x) - point.y).abs() < 1.0e-6);
        }
        assert_eq!(point_curve(&curve, 0.0), 0.05);
        assert_eq!(point_curve(&curve, 1.0), 0.95);
        let identity = points(&[(0.0, 0.0), (1.0, 1.0)]);
        for step in 0..=10 {
            let x = step as f32 / 10.0;
            assert!((point_curve(&identity, x) - x).abs() < 1.0e-6);
        }
    }

    #[test]
    fn rising_points_give_a_rising_curve_without_overshoot() {
        let curve = points(&[(0.0, 0.0), (0.5, 0.5), (0.52, 0.98), (1.0, 1.0)]);
        let samples: Vec<f32> = (0..=1000)
            .map(|step| point_curve(&curve, step as f32 / 1000.0))
            .collect();
        assert!(samples.windows(2).all(|pair| pair[1] >= pair[0] - 1.0e-6));
        assert!(samples.iter().all(|value| (0.0..=1.0).contains(value)));
    }

    #[test]
    fn the_default_parametric_curve_is_the_identity_with_splits_at_the_quarters() {
        let shape = Parametric::new(&ParametricCurve::default());
        assert_eq!(shape.splits, [0.25, 0.5, 0.75]);
        for step in 0..=20 {
            let v = step as f32 / 20.0;
            assert_eq!(shape.eval(v), v);
        }
    }

    #[test]
    fn each_region_moves_its_own_part_of_the_curve_most() {
        let centres = [0.125, 0.375, 0.625, 0.875];
        for region in 0..4 {
            let mut curve = ParametricCurve::default();
            match region {
                0 => curve.shadows = 100.0,
                1 => curve.darks = 100.0,
                2 => curve.lights = 100.0,
                _ => curve.highlights = 100.0,
            }
            let shape = Parametric::new(&curve);
            let lift = |v: f32| shape.eval(v) - v;
            let own = lift(centres[region]);
            assert!(own > 0.01, "region {region}: {own}");
            for (other, centre) in centres.iter().enumerate() {
                if other != region {
                    assert!(own > lift(*centre), "region {region} moves {other} more");
                }
            }
            assert_eq!((shape.eval(0.0), shape.eval(1.0)), (0.0, 1.0));
        }
    }
}
