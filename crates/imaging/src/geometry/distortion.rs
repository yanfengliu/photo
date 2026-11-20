//! Manual lens distortion: a one-parameter radial model.
//!
//! With `r` the distance from the image centre divided by the half-diagonal
//! (so the corners sit at `r = 1`), a point at radius `r_c` in the corrected
//! frame is read from radius
//!
//! ```text
//! r_s = r_c * (1 + a * (1 - r_c^2))        for r_c <= 1
//! r_s = 1 + (1 - 2a) * (r_c - 1)           for r_c >  1
//! ```
//!
//! in the source. This is the single-coefficient polynomial radial model
//! (the `k1` term of Brown, "Decentering Distortion of Lenses", 1966),
//! renormalized so that the corners do not move, and continued past the
//! corners with its own slope there so it never folds back.
//!
//! `a = MAX_COEFFICIENT * distortion / 100`. The derivative `1 + a - 3a r^2`
//! stays between `1 - 2a` and `1 + a` on `0..=1`, so for `|a| <= 0.25` the
//! map is strictly increasing in radius everywhere, and therefore invertible.

/// `|a|` at `distortion = +-100`.
const MAX_COEFFICIENT: f64 = 0.25;

/// The radial map between the corrected frame and the source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Distortion {
    coefficient: f64,
    centre_x: f64,
    centre_y: f64,
    inverse_radius: f64,
}

impl Distortion {
    /// `amount` is the `distortion` slider, -100 to 100; out-of-range values
    /// are clamped and non-finite ones mean no distortion.
    pub(crate) fn new(source_width: f64, source_height: f64, amount: f32) -> Self {
        let amount = if amount.is_finite() {
            f64::from(amount).clamp(-100.0, 100.0)
        } else {
            0.0
        };
        Self {
            coefficient: MAX_COEFFICIENT * amount / 100.0,
            centre_x: source_width / 2.0,
            centre_y: source_height / 2.0,
            inverse_radius: 2.0 / source_width.hypot(source_height),
        }
    }

    /// True when the map is exactly the identity.
    pub(crate) fn is_identity(&self) -> bool {
        self.coefficient == 0.0
    }

    /// Where a corrected-frame position is read from in the source.
    #[inline]
    pub(crate) fn corrected_to_source(&self, x: f64, y: f64) -> (f64, f64) {
        if self.is_identity() {
            return (x, y);
        }
        let (dx, dy) = (x - self.centre_x, y - self.centre_y);
        let r2 = (dx * dx + dy * dy) * self.inverse_radius * self.inverse_radius;
        let a = self.coefficient;
        let gain = if r2 <= 1.0 {
            1.0 + a * (1.0 - r2)
        } else {
            let r = r2.sqrt();
            (1.0 + (1.0 - 2.0 * a) * (r - 1.0)) / r
        };
        (self.centre_x + dx * gain, self.centre_y + dy * gain)
    }

    /// Inverse of [`Self::corrected_to_source`].
    pub(crate) fn source_to_corrected(&self, x: f64, y: f64) -> (f64, f64) {
        if self.is_identity() {
            return (x, y);
        }
        let (dx, dy) = (x - self.centre_x, y - self.centre_y);
        let target = dx.hypot(dy) * self.inverse_radius;
        if target <= 0.0 {
            return (x, y);
        }
        let a = self.coefficient;
        let radius = if target >= 1.0 {
            1.0 + (target - 1.0) / (1.0 - 2.0 * a)
        } else {
            // Newton on f(r) = r (1 + a) - a r^3 - target, which is strictly
            // increasing with slope >= 1 - 2|a| >= 1/2 on 0..=1.
            let mut r = target;
            for _ in 0..24 {
                let step = (r * (1.0 + a) - a * r * r * r - target) / (1.0 + a - 3.0 * a * r * r);
                r = (r - step).clamp(0.0, 1.0);
                if step.abs() < 1e-15 {
                    break;
                }
            }
            r
        };
        let gain = radius / target;
        (self.centre_x + dx * gain, self.centre_y + dy * gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_distortion_is_exactly_the_identity() {
        let d = Distortion::new(6000.0, 4000.0, 0.0);
        assert!(d.is_identity());
        assert_eq!(d.corrected_to_source(1234.5, 17.5), (1234.5, 17.5));
        assert_eq!(d.source_to_corrected(1234.5, 17.5), (1234.5, 17.5));
        assert!(Distortion::new(10.0, 10.0, f32::NAN).is_identity());
    }

    #[test]
    fn the_inverse_undoes_the_forward_map_inside_and_outside_the_frame() {
        for amount in [-100.0, -30.0, 12.5, 100.0] {
            let d = Distortion::new(600.0, 400.0, amount);
            for (x, y) in [
                (300.0, 200.0),
                (10.0, 390.0),
                (599.0, 1.0),
                (-400.0, 900.0),
                (2000.0, 200.0),
            ] {
                let (sx, sy) = d.corrected_to_source(x, y);
                let (bx, by) = d.source_to_corrected(sx, sy);
                assert!(
                    (bx - x).abs() < 1e-9 && (by - y).abs() < 1e-9,
                    "{amount}: ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn the_map_is_continuous_and_smooth_across_the_corner_radius() {
        let d = Distortion::new(600.0, 400.0, 100.0);
        let half_diagonal = 300.0f64.hypot(200.0);
        let radius_at = |r: f64| {
            let (x, _) = d.corrected_to_source(300.0 + r * half_diagonal, 200.0);
            (x - 300.0) / half_diagonal
        };
        let (inside, outside) = (radius_at(1.0 - 1e-6), radius_at(1.0 + 1e-6));
        assert!(
            (outside - inside - 2e-6 * 0.5).abs() < 1e-9,
            "slope 1 - 2a = 0.5 on both sides"
        );
        assert!((radius_at(1.0) - 1.0).abs() < 1e-12);
    }
}
