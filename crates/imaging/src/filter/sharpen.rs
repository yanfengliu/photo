//! The pieces an unsharp mask is assembled from.
//!
//! Sharpening is `value + amount * weight * core(detail)`: a detail layer,
//! a coring function that leaves small differences alone, and a per-pixel
//! weight that keeps flat areas from being sharpened. The amounts and how
//! sliders map to them belong to the pipeline; this module supplies the
//! detail layer ([`high_pass`]), the edge measure a weight is built from
//! ([`gradient_magnitude`]), and coring is
//! [`soft_threshold`](super::soft_threshold).

use crate::buffer::Plane;

use super::gaussian::gaussian_blur;

/// The unsharp mask's detail layer: `input - gaussian_blur(input, sigma)`.
///
/// Zero where the plane is flat or changes linearly over a few `sigma`,
/// positive on the bright side of an edge and negative on the dark side.
/// Adding it back to the blurred plane gives the input, to within rounding.
/// `sigma <= 0` gives all zeros. Edges are replicated. Support:
/// [`gaussian_support`](super::gaussian_support)`(sigma)`.
pub fn high_pass(input: &Plane, sigma: f32) -> Plane {
    let mut out = gaussian_blur(input, sigma);
    out.par_rows_mut(|y, row| {
        for (value, &original) in row.iter_mut().zip(input.row(y)) {
            *value = original - *value;
        }
    });
    out
}

/// Length of the local gradient, in plane units per pixel, by the Sobel
/// operator (Sobel and Feldman, 1968) scaled so that a ramp of slope `s`
/// per pixel reads `s` in any direction.
///
/// Always non-negative; zero on flat areas. Edges are replicated. Support:
/// 1 pixel.
pub fn gradient_magnitude(input: &Plane) -> Plane {
    let (width, height) = (input.width(), input.height());
    let mut out = Plane::new(width, height);
    out.par_rows_mut(|y, row| {
        let (above, here, below) = (
            input.row(y.saturating_sub(1)),
            input.row(y),
            input.row((y + 1).min(height - 1)),
        );
        for (x, value) in row.iter_mut().enumerate() {
            let (left, right) = (x.saturating_sub(1), (x + 1).min(width - 1));
            let across = (above[right] + 2.0 * here[right] + below[right])
                - (above[left] + 2.0 * here[left] + below[left]);
            let down = (below[left] + 2.0 * below[x] + below[right])
                - (above[left] + 2.0 * above[x] + above[right]);
            *value = across.hypot(down) / 8.0;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_high_pass_of_a_step_is_signed_and_local() {
        let step = Plane::from_fn(40, 3, |x, _| if x < 20 { 0.25 } else { 0.75 });
        let detail = high_pass(&step, 1.5);
        assert!(detail.get(19, 1) < -0.05 && detail.get(20, 1) > 0.05);
        assert_eq!(detail.get(5, 1), 0.0);
        assert_eq!(high_pass(&step, 0.0), Plane::new(40, 3));
    }

    #[test]
    fn the_gradient_at_a_border_uses_the_replicated_edge() {
        let ramp = Plane::from_fn(4, 4, |x, _| x as f32);
        let magnitude = gradient_magnitude(&ramp);
        // One-sided at the border: half the interior slope.
        assert_eq!(magnitude.get(0, 2), 0.5);
        assert_eq!(magnitude.get(1, 2), 1.0);
        assert_eq!(magnitude.get(3, 0), 0.5);
    }
}
