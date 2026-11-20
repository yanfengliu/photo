//! Running minimum over a square window.
//!
//! The van Herk / Gil-Werman algorithm (van Herk, "A fast algorithm for
//! local minimum and maximum filters on rectangular and octagonal kernels",
//! Pattern Recognition Letters 1992; Gil and Werman, "Computing 2-D min,
//! median, and max filters", IEEE PAMI 1993): cut the line into blocks as
//! long as the window, take running minima forward and backward inside each
//! block, and every window is the minimum of one value from each. Three
//! comparisons per sample whatever the radius.

use crate::buffer::{Plane, par_column_strips};

/// Columns handled together by one thread in the vertical pass.
const STRIP_WIDTH: usize = 64;

/// Forward and backward block minima of a line padded by `radius` samples
/// of `+inf` on both sides. `value(p)` is padded sample `p`.
fn block_minima(
    padded: usize,
    window: usize,
    value: impl Fn(usize) -> f32,
    forward: &mut [f32],
    backward: &mut [f32],
) {
    for p in 0..padded {
        forward[p] = if p % window == 0 {
            value(p)
        } else {
            forward[p - 1].min(value(p))
        };
    }
    for p in (0..padded).rev() {
        let ends_block = p % window == window - 1 || p == padded - 1;
        backward[p] = if ends_block {
            value(p)
        } else {
            backward[p + 1].min(value(p))
        };
    }
}

fn min_line(source: &[f32], out: &mut [f32], radius: usize, scratch: &mut Vec<f32>) {
    let len = source.len();
    let (window, padded) = (2 * radius + 1, len + 2 * radius);
    scratch.clear();
    scratch.resize(2 * padded, f32::INFINITY);
    let (forward, backward) = scratch.split_at_mut(padded);
    let value = |p: usize| {
        if p >= radius && p < radius + len {
            source[p - radius]
        } else {
            f32::INFINITY
        }
    };
    block_minima(padded, window, value, forward, backward);
    for (x, sample) in out.iter_mut().enumerate() {
        *sample = backward[x].min(forward[x + 2 * radius]);
    }
}

fn min_columns(source: &Plane, out: &mut Plane, radius: usize) {
    let height = source.height();
    let (window, padded) = (2 * radius + 1, height + 2 * radius);
    par_column_strips(out, STRIP_WIDTH, |x0, rows| {
        let width = rows[0].len();
        let line = |p: usize| {
            (p >= radius && p < radius + height).then(|| &source.row(p - radius)[x0..x0 + width])
        };
        // Block minima for `width` columns at once, one padded row after
        // another, so the inner loops run along memory.
        let mut forward = vec![f32::INFINITY; padded * width];
        let mut backward = vec![f32::INFINITY; padded * width];
        for p in 0..padded {
            let (before, current) = forward.split_at_mut(p * width);
            let current = &mut current[..width];
            let carried = (p % window != 0).then(|| &before[(p - 1) * width..]);
            merge(current, carried, line(p));
        }
        for p in (0..padded).rev() {
            let (current, after) = backward.split_at_mut((p + 1) * width);
            let current = &mut current[p * width..];
            let carried = (p % window != window - 1 && p != padded - 1).then(|| &after[..width]);
            merge(current, carried, line(p));
        }
        for (y, row) in rows.iter_mut().enumerate() {
            let below = &backward[y * width..(y + 1) * width];
            let above = &forward[(y + 2 * radius) * width..(y + 2 * radius + 1) * width];
            for ((sample, &a), &b) in row.iter_mut().zip(below).zip(above) {
                *sample = a.min(b);
            }
        }
    });
}

/// `current = min(carried, values)`, where a missing operand counts as
/// `+inf` and `current` starts out as `+inf`.
fn merge(current: &mut [f32], carried: Option<&[f32]>, values: Option<&[f32]>) {
    match (carried, values) {
        (Some(carried), Some(values)) => {
            for ((out, &c), &v) in current.iter_mut().zip(carried).zip(values) {
                *out = c.min(v);
            }
        }
        (Some(only), None) | (None, Some(only)) => current.copy_from_slice(only),
        (None, None) => {}
    }
}

/// The minimum over the `(2 * radius + 1)` square around every pixel: the
/// spatial half of a dark channel.
///
/// The window is cut off at the plane's edges, which is the same as
/// replicating them. A radius of 0 returns the input unchanged. Results are
/// exact minima of input samples, so a padded tile matches the whole plane
/// bit for bit. Cost is constant per pixel for any radius. Support: `radius`
/// pixels.
pub fn min_filter(input: &Plane, radius: usize) -> Plane {
    let (width, height) = (input.width(), input.height());
    if radius == 0 || input.is_empty() {
        return input.clone();
    }
    // A window wider than the plane sees all of it either way.
    let (radius_x, radius_y) = (radius.min(width - 1), radius.min(height - 1));
    let mut rows = Plane::new(width, height);
    rows.par_rows_mut_with(Vec::new, |scratch, y, row| {
        min_line(input.row(y), row, radius_x, scratch)
    });
    let mut out = Plane::new(width, height);
    min_columns(&rows, &mut out, radius_y);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_minimum_covers_exactly_the_window() {
        let source = [5.0, 3.0, 8.0, 1.0, 9.0, 7.0, 6.0, 2.0];
        let mut out = [0.0f32; 8];
        min_line(&source, &mut out, 1, &mut Vec::new());
        assert_eq!(out, [3.0, 3.0, 1.0, 1.0, 1.0, 6.0, 2.0, 2.0]);
        min_line(&source, &mut out, 2, &mut Vec::new());
        assert_eq!(out, [3.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0]);
    }

    #[test]
    fn a_radius_beyond_the_plane_gives_the_global_minimum() {
        let plane = Plane::from_fn(5, 4, |x, y| ((x * 7 + y * 3) % 11) as f32 + 1.0);
        let lowest = plane
            .as_slice()
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        assert_eq!(min_filter(&plane, 1_000_000), Plane::filled(5, 4, lowest));
    }
}
