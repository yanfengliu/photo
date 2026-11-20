//! Point curves: a smooth curve through user-placed control points.
//!
//! Interpolation is the monotone piecewise cubic of F. N. Fritsch and
//! R. E. Carlson, "Monotone Piecewise Cubic Interpolation", SIAM J. Numer.
//! Anal. 17 (1980): a cubic Hermite spline whose tangents are limited so the
//! curve never overshoots its control points. Wherever the points rise the
//! curve rises; a curve drawn with a bump keeps the bump but gains no
//! ripples.

use photo_core::develop::{CurvePoint, PointCurve};

/// A compiled [`PointCurve`].
#[derive(Clone, Debug, PartialEq)]
pub struct MonotoneCubic {
    xs: Vec<f64>,
    ys: Vec<f64>,
    tangents: Vec<f64>,
}

impl MonotoneCubic {
    /// Compile a point curve. The curve passes through every point, is C1,
    /// and is constant to the left of the first point and to the right of
    /// the last.
    pub fn new(curve: &PointCurve) -> Self {
        Self::through(curve.points())
    }

    /// Compile a curve only when it does something: `None` for the identity.
    pub fn non_identity(curve: &PointCurve) -> Option<Self> {
        (!curve.is_identity()).then(|| Self::new(curve))
    }

    fn through(points: &[CurvePoint]) -> Self {
        let mut xs: Vec<f64> = Vec::with_capacity(points.len());
        let mut ys: Vec<f64> = Vec::with_capacity(points.len());
        for point in points {
            let (x, y) = (f64::from(point.x), f64::from(point.y));
            // `PointCurve` guarantees finite, strictly increasing x; this
            // keeps the compiled form safe even if that is ever violated.
            if x.is_finite() && y.is_finite() && xs.last().is_none_or(|last| x > *last) {
                xs.push(x);
                ys.push(y);
            }
        }
        if xs.len() < 2 {
            xs = vec![0.0, 1.0];
            ys = vec![0.0, 1.0];
        }
        let tangents = fritsch_carlson(&xs, &ys);
        Self { xs, ys, tangents }
    }

    /// The curve's value at `x`.
    pub fn eval(&self, x: f32) -> f32 {
        self.eval_d(f64::from(x)) as f32
    }

    pub(crate) fn eval_d(&self, x: f64) -> f64 {
        let last = self.xs.len() - 1;
        if x.is_nan() || x <= self.xs[0] {
            return self.ys[0];
        }
        if x >= self.xs[last] {
            return self.ys[last];
        }
        // Index of the segment that contains x.
        let i = self.xs.partition_point(|knot| *knot <= x) - 1;
        let h = self.xs[i + 1] - self.xs[i];
        let t = (x - self.xs[i]) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * self.ys[i]
            + (t3 - 2.0 * t2 + t) * h * self.tangents[i]
            + (-2.0 * t3 + 3.0 * t2) * self.ys[i + 1]
            + (t3 - t2) * h * self.tangents[i + 1]
    }
}

/// Tangents for a monotone cubic Hermite interpolant (Fritsch-Carlson).
fn fritsch_carlson(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let n = xs.len();
    let secants: Vec<f64> = (0..n - 1)
        .map(|i| (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]))
        .collect();
    let mut tangents = vec![0.0; n];
    tangents[0] = secants[0];
    tangents[n - 1] = secants[n - 2];
    for i in 1..n - 1 {
        // A local extremum of the data gets a flat tangent.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(points: &[(f32, f32)]) -> MonotoneCubic {
        MonotoneCubic::new(&PointCurve::new(
            points.iter().map(|(x, y)| CurvePoint::new(*x, *y)),
        ))
    }

    #[test]
    fn the_default_curve_is_the_identity() {
        assert!(MonotoneCubic::non_identity(&PointCurve::default()).is_none());
        let cubic = MonotoneCubic::new(&PointCurve::default());
        for step in 0..=100 {
            let x = f64::from(step) / 100.0;
            assert!((cubic.eval_d(x) - x).abs() < 1e-15);
        }
    }

    #[test]
    fn the_curve_passes_through_its_points_and_is_flat_outside_them() {
        let points = [(0.1, 0.05), (0.3, 0.2), (0.55, 0.7), (0.9, 0.95)];
        let cubic = curve(&points);
        for (x, y) in points {
            assert!((cubic.eval(x) - y).abs() < 1e-6);
        }
        assert_eq!(cubic.eval(0.0), 0.05);
        assert_eq!(cubic.eval(1.0), 0.95);
        assert_eq!(cubic.eval(7.0), 0.95);
        assert_eq!(cubic.eval(f32::NAN), 0.05);
    }

    #[test]
    fn rising_points_give_a_rising_curve_without_overshoot() {
        let sets: [&[(f32, f32)]; 4] = [
            &[(0.0, 0.0), (0.1, 0.01), (0.2, 0.6), (0.3, 0.61), (1.0, 1.0)],
            &[(0.0, 0.0), (0.5, 0.5), (0.52, 0.98), (1.0, 1.0)],
            &[(0.0, 0.2), (0.25, 0.2), (0.75, 0.8), (1.0, 0.8)],
            &[(0.0, 0.0), (0.004, 0.5), (0.5, 0.5), (1.0, 1.0)],
        ];
        for points in sets {
            let cubic = curve(points);
            let mut previous = cubic.eval_d(0.0);
            for step in 1..=5000 {
                let value = cubic.eval_d(f64::from(step) / 5000.0);
                assert!(value >= previous - 1e-12, "{points:?} dips at step {step}");
                assert!((0.0..=1.0).contains(&value));
                previous = value;
            }
        }
    }

    #[test]
    fn a_bump_stays_inside_the_range_of_its_points() {
        let cubic = curve(&[(0.0, 0.0), (0.4, 0.9), (0.6, 0.3), (1.0, 1.0)]);
        for step in 0..=1000 {
            let x = f64::from(step) / 1000.0;
            let value = cubic.eval_d(x);
            assert!((0.0..=1.0).contains(&value));
            if (0.4..=0.6).contains(&x) {
                assert!(
                    (0.3 - 1e-9..=0.9 + 1e-9).contains(&value),
                    "overshoot at {x}: {value}"
                );
            }
        }
    }

    #[test]
    fn the_curve_is_c1_across_its_knots() {
        let cubic = curve(&[(0.0, 0.0), (0.3, 0.2), (0.7, 0.85), (1.0, 1.0)]);
        for knot in [0.3, 0.7] {
            let below = (cubic.eval_d(knot) - cubic.eval_d(knot - 1e-6)) / 1e-6;
            let above = (cubic.eval_d(knot + 1e-6) - cubic.eval_d(knot)) / 1e-6;
            assert!(
                (below - above).abs() < 1e-4,
                "kink at {knot}: {below} vs {above}"
            );
        }
    }
}
