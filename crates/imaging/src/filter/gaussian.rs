//! Gaussian blur approximated by three box passes.
//!
//! Repeated box filtering converges on a Gaussian (Wells, "Efficient
//! Synthesis of Gaussian Filters by Cascaded Uniform Filters", IEEE PAMI
//! 1986). Three passes give a quadratic B-spline: the same variance as the
//! Gaussian it stands in for, a peak 6% lower, and strictly finite support.
//! The boxes have fractional radii (see [`BoxKernel`]) chosen so the variance
//! is exact for any `sigma`, not only those an integer box width can hit.

use crate::buffer::Plane;

use super::box_blur::{BoxKernel, MAX_RADIUS, blur_columns, blur_rows};

const PASSES: usize = 3;

/// The box whose variance is a third of `sigma^2`, or `None` when `sigma`
/// is zero, negative or NaN. Anything above [`MAX_RADIUS`], infinity
/// included, is held there.
///
/// A box with weight 1 out to `n` and `f` at `n + 1` has variance
/// `(n (n + 1) (2n + 1) / 3 + 2 f (n + 1)^2) / (2n + 1 + 2f)`. `n` is the
/// largest integer whose plain box (`f = 0`, variance `n (n + 1) / 3`) is
/// not already too wide, and `f` follows by solving for the target.
fn kernel_for_sigma(sigma: f32) -> Option<BoxKernel> {
    if sigma.is_nan() || sigma <= 0.0 {
        return None;
    }
    let target = f64::from(sigma.min(MAX_RADIUS)).powi(2) / PASSES as f64;
    let whole = ((-1.0 + (1.0 + 12.0 * target).sqrt()) / 2.0)
        .floor()
        .max(0.0);
    let plain = whole * (whole + 1.0) / 3.0;
    let fraction =
        (2.0 * whole + 1.0) * (target - plain) / (2.0 * ((whole + 1.0).powi(2) - target));
    Some(BoxKernel::new(whole as usize, fraction.clamp(0.0, 1.0)))
}

/// Farthest pixel that can influence an output pixel of [`gaussian_blur`]:
/// three times the support of one of its box passes. Close to `3 * sigma`
/// for large `sigma`, never less than `2.1 * sigma` (it steps up by 3
/// whenever the box grows by a sample), and 0 for `sigma <= 0`.
pub fn gaussian_support(sigma: f32) -> usize {
    kernel_for_sigma(sigma).map_or(0, |kernel| PASSES * kernel.support())
}

/// Gaussian blur of standard deviation `sigma` pixels, approximated by three
/// box passes per axis.
///
/// The result has unit gain and a variance of exactly `sigma^2` for any
/// positive `sigma`, fractional values included. Its shape is a quadratic
/// B-spline rather than a true Gaussian: within 6% of the Gaussian's peak
/// everywhere, and exactly zero beyond [`gaussian_support`]. A `sigma` of 0
/// or less, or NaN, returns the input unchanged; above `1e6`, infinity
/// included, it is treated as `1e6`.
///
/// Cost is constant per pixel for any `sigma`. Edges are replicated.
pub fn gaussian_blur(input: &Plane, sigma: f32) -> Plane {
    let Some(kernel) = kernel_for_sigma(sigma) else {
        return input.clone();
    };
    let mut scratch = Plane::new(input.width(), input.height());
    let mut out = Plane::new(input.width(), input.height());
    blur_rows(input, &mut scratch, &kernel);
    blur_columns(&scratch, &mut out, &kernel);
    for _ in 1..PASSES {
        blur_rows(&out, &mut scratch, &kernel);
        blur_columns(&scratch, &mut out, &kernel);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_box_changes_continuously_with_sigma() {
        // Just below and just above the sigma at which the whole part of
        // the radius steps from 1 to 2, the supports differ by one sample
        // whose weight is close to zero.
        let step = (3.0f64 * 2.0).sqrt() as f32;
        let below = kernel_for_sigma(step - 1e-3).unwrap();
        let above = kernel_for_sigma(step + 1e-3).unwrap();
        assert_eq!((below.support(), above.support()), (2, 3));
        assert_eq!(gaussian_support(step - 1e-3), 6);
    }

    #[test]
    fn tiny_sigmas_still_blur_a_little() {
        let kernel = kernel_for_sigma(0.3).unwrap();
        assert_eq!(kernel.support(), 1);
        assert!(kernel_for_sigma(0.0).is_none());
        assert!(kernel_for_sigma(f32::NAN).is_none());
    }

    #[test]
    fn an_infinite_sigma_is_the_largest_sigma() {
        assert_eq!(kernel_for_sigma(f32::INFINITY), kernel_for_sigma(MAX_RADIUS));
        assert_eq!(gaussian_support(f32::INFINITY), gaussian_support(1.0e30));
    }

    #[test]
    fn support_is_between_two_point_one_and_three_and_a_bit_sigma() {
        for step in 1..4000 {
            let sigma = step as f32 * 0.01;
            let ratio = gaussian_support(sigma) as f32 / sigma;
            assert!(ratio >= 2.12, "sigma {sigma}: support is only {ratio} sigma");
            assert!(sigma < 1.0 || ratio <= 3.0 + 3.0 / sigma, "sigma {sigma}: {ratio}");
        }
    }
}
