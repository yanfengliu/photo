//! Box blur with a running sum: constant cost per pixel whatever the radius.

use crate::buffer::{Plane, par_column_strips};

/// Columns handled together by one thread in the vertical pass. Narrow
/// enough that a strip's running sums stay in the first-level cache.
const STRIP_WIDTH: usize = 128;

/// Radii beyond this are treated as this: the blur has long since become
/// an average of the two opposite edges, and index arithmetic must not
/// overflow.
pub(crate) const MAX_RADIUS: f32 = 1.0e6;

/// The weights of a box of fractional radius `r`: 1 for offsets up to
/// `floor(r)`, the fractional part of `r` for the next offset out, and the
/// whole divided by `2r + 1`. An "extended box" in the sense of Gwosdek,
/// Grewenig, Bruhn and Weickert, "Theoretical Foundations of Gaussian
/// Convolution by Extended Box Filtering" (SSVM 2011).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BoxKernel {
    whole: usize,
    fraction: f64,
    inverse_norm: f64,
}

impl BoxKernel {
    /// The kernel for `radius`, or `None` when the radius is zero, negative
    /// or NaN, in which case the blur is the identity. Infinity is a large
    /// radius like any other and is held at [`MAX_RADIUS`].
    pub(crate) fn from_radius(radius: f32) -> Option<Self> {
        if radius.is_nan() || radius <= 0.0 {
            return None;
        }
        let radius = radius.min(MAX_RADIUS);
        let whole = radius.floor();
        Some(Self::new(
            whole as usize,
            f64::from(radius) - f64::from(whole),
        ))
    }

    /// The kernel with weight 1 out to `whole` and `fraction` beyond it.
    pub(crate) fn new(whole: usize, fraction: f64) -> Self {
        Self {
            whole,
            fraction,
            inverse_norm: 1.0 / (2.0 * (whole as f64 + fraction) + 1.0),
        }
    }

    /// The farthest offset with a non-zero weight.
    pub(crate) fn support(&self) -> usize {
        self.whole + usize::from(self.fraction > 0.0)
    }
}

/// Blur one line with edge replication. The window sum is kept in `f64`, so
/// rounding does not build up along the line and the result does not depend
/// measurably on where the line starts (contract R1).
fn blur_line(source: &[f32], out: &mut [f32], kernel: &BoxKernel) {
    let len = source.len();
    if len == 0 {
        return;
    }
    let (reach, last) = (kernel.whole, len - 1);

    // The window around sample 0: `reach` replicated samples to its left,
    // the real ones, and replicas of the last one if the line is short.
    let inside = reach.min(last);
    let sum = reach as f64 * f64::from(source[0])
        + source[..=inside].iter().map(|&v| f64::from(v)).sum::<f64>()
        + (reach - inside) as f64 * f64::from(source[last]);

    // Samples whose whole window, fringe included, lies inside the line
    // need no clamping.
    let interior_start = (reach + 1).min(len);
    let interior_end = len.saturating_sub(reach + 1).max(interior_start);
    let sum = blur_clamped(source, out, kernel, 0..interior_start, sum);
    let sum = blur_interior(source, out, kernel, interior_start..interior_end, sum);
    blur_clamped(source, out, kernel, interior_end..len, sum);
}

/// The ends of [`blur_line`], where the window hangs over an edge and reads
/// replicated samples. Takes and returns the running window sum.
fn blur_clamped(
    source: &[f32],
    out: &mut [f32],
    kernel: &BoxKernel,
    range: std::ops::Range<usize>,
    mut sum: f64,
) -> f64 {
    let (reach, last) = (kernel.whole as isize, source.len() as isize - 1);
    let at = |index: isize| f64::from(source[index.clamp(0, last) as usize]);
    for x in range {
        let (near, far) = (x as isize - reach, x as isize + reach + 1);
        out[x] = ((sum + kernel.fraction * (at(near - 1) + at(far))) * kernel.inverse_norm) as f32;
        sum += at(far) - at(near);
    }
    sum
}

/// The clamp-free middle of [`blur_line`]. Takes and returns the running
/// window sum.
fn blur_interior(
    source: &[f32],
    out: &mut [f32],
    kernel: &BoxKernel,
    range: std::ops::Range<usize>,
    mut sum: f64,
) -> f64 {
    if range.is_empty() {
        return sum;
    }
    let reach = kernel.whole;
    let leading = &source[range.start + reach + 1..range.end + reach + 1];
    let trailing = &source[range.start - reach..range.end - reach];
    let fringe = &source[range.start - reach - 1..range.end - reach - 1];
    let samples = out[range].iter_mut().zip(leading).zip(trailing).zip(fringe);
    for (((value, &lead), &trail), &before) in samples {
        let lead = f64::from(lead);
        *value =
            ((sum + kernel.fraction * (f64::from(before) + lead)) * kernel.inverse_norm) as f32;
        sum += lead - f64::from(trail);
    }
    sum
}

/// Blur every row of `source` into `out`.
pub(crate) fn blur_rows(source: &Plane, out: &mut Plane, kernel: &BoxKernel) {
    out.par_rows_mut(|y, row| blur_line(source.row(y), row, kernel));
}

/// Blur every column of `source` into `out`.
pub(crate) fn blur_columns(source: &Plane, out: &mut Plane, kernel: &BoxKernel) {
    let height = source.height();
    if height == 0 {
        return;
    }
    let (reach, last) = (kernel.whole as isize, height as isize - 1);
    let (fraction, inverse_norm) = (kernel.fraction, kernel.inverse_norm);
    par_column_strips(out, STRIP_WIDTH, |x0, rows| {
        let width = rows[0].len();
        let line = |y: isize| &source.row(y.clamp(0, last) as usize)[x0..x0 + width];
        let mut sums = vec![0.0f64; width];
        // Rows above the plane are replicas of row 0; count them at once.
        let inside = reach.min(last);
        for (sum, &v) in sums.iter_mut().zip(line(0)) {
            *sum = reach as f64 * f64::from(v);
        }
        for y in 0..=inside {
            for (sum, &v) in sums.iter_mut().zip(line(y)) {
                *sum += f64::from(v);
            }
        }
        for (sum, &v) in sums.iter_mut().zip(line(last)) {
            *sum += (reach - inside) as f64 * f64::from(v);
        }
        for (y, row) in rows.iter_mut().enumerate() {
            let y = y as isize;
            let (leading, trailing, fringe) =
                (line(y + reach + 1), line(y - reach), line(y - reach - 1));
            let samples = row
                .iter_mut()
                .zip(sums.iter_mut())
                .zip(leading)
                .zip(trailing)
                .zip(fringe);
            for ((((value, sum), &lead), &trail), &before) in samples {
                let lead = f64::from(lead);
                *value = ((*sum + fraction * (f64::from(before) + lead)) * inverse_norm) as f32;
                *sum += lead - f64::from(trail);
            }
        }
    });
}

/// Farthest pixel, in whole pixels, that can influence an output pixel of
/// [`box_blur`] with this radius: `ceil(radius)`, 0 for a radius of 0.
pub fn box_support(radius: f32) -> usize {
    BoxKernel::from_radius(radius).map_or(0, |kernel| kernel.support())
}

/// Box blur: the mean over a `(2 * radius + 1)` square.
///
/// `radius` is in pixels and may be fractional: the samples at
/// `floor(radius) + 1` are weighted by the fractional part, so the result
/// changes smoothly with the radius (and with the view scale it is
/// multiplied by). A radius of 0 or less, or NaN, returns the input
/// unchanged; above `1e6`, infinity included, it is treated as `1e6`.
///
/// Cost is constant per pixel for any radius. Edges are replicated. Support:
/// [`box_support`], that is `ceil(radius)` pixels.
pub fn box_blur(input: &Plane, radius: f32) -> Plane {
    let Some(kernel) = BoxKernel::from_radius(radius) else {
        return input.clone();
    };
    let mut scratch = Plane::new(input.width(), input.height());
    let mut out = Plane::new(input.width(), input.height());
    blur_rows(input, &mut scratch, &kernel);
    blur_columns(&scratch, &mut out, &kernel);
    out
}

/// A box blur that can be applied again and again without allocating.
pub(crate) struct BoxBlurrer {
    kernel: BoxKernel,
    scratch: Plane,
}

impl BoxBlurrer {
    pub(crate) fn new(kernel: BoxKernel, width: usize, height: usize) -> Self {
        Self {
            kernel,
            scratch: Plane::new(width, height),
        }
    }

    /// Blur `plane` in place. It must have the size given to [`Self::new`].
    pub(crate) fn apply(&mut self, plane: &mut Plane) {
        blur_rows(plane, &mut self.scratch, &self.kernel);
        blur_columns(&self.scratch, plane, &self.kernel);
    }

    /// A blurred copy of `plane`, which must have the size given to
    /// [`Self::new`].
    pub(crate) fn blurred(&mut self, plane: &Plane) -> Plane {
        let mut out = Plane::new(plane.width(), plane.height());
        blur_rows(plane, &mut self.scratch, &self.kernel);
        blur_columns(&self.scratch, &mut out, &self.kernel);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fractional_radius_weights_the_fringe() {
        let mut out = [0.0f32; 5];
        blur_line(
            &[0.0, 0.0, 1.0, 0.0, 0.0],
            &mut out,
            &BoxKernel::from_radius(0.5).unwrap(),
        );
        assert_eq!(out, [0.0, 0.25, 0.5, 0.25, 0.0]);
        blur_line(
            &[0.0, 0.0, 1.0, 0.0, 0.0],
            &mut out,
            &BoxKernel::from_radius(1.0).unwrap(),
        );
        let third = 1.0 / 3.0;
        assert_eq!(out, [0.0, third, third, third, 0.0]);
    }

    #[test]
    fn edges_are_replicated() {
        let mut out = [0.0f32; 4];
        blur_line(
            &[3.0, 0.0, 0.0, 9.0],
            &mut out,
            &BoxKernel::from_radius(1.0).unwrap(),
        );
        assert_eq!(out, [2.0, 1.0, 3.0, 6.0]);
    }

    #[test]
    fn a_radius_far_larger_than_the_line_tends_to_the_mean_of_its_ends() {
        let mut out = [0.0f32; 3];
        blur_line(
            &[2.0, 100.0, 4.0],
            &mut out,
            &BoxKernel::from_radius(5.0e5).unwrap(),
        );
        for value in out {
            assert!((value - 3.0).abs() < 1e-3, "{value}");
        }
    }

    #[test]
    fn support_is_the_radius_rounded_up() {
        assert_eq!(box_support(0.0), 0);
        assert_eq!(box_support(0.2), 1);
        assert_eq!(box_support(3.0), 3);
        assert_eq!(box_support(3.01), 4);
        assert_eq!(box_support(f32::NAN), 0);
    }
}
