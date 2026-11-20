//! The guided filter: edge-preserving smoothing in a handful of box blurs.
//!
//! He, Sun and Tang, "Guided Image Filtering" (ECCV 2010; IEEE PAMI 2013).
//! In every window the output is modelled as a linear function `a * I + b`
//! of the guide `I`; `a` and `b` come from the window's means, variance and
//! covariance, and each pixel takes the average of the models of all windows
//! that cover it. Where the guide is flat (variance far below `eps`) that is
//! a plain mean; where it has an edge (variance far above `eps`) `a` is
//! close to 1 and the edge passes through.
//!
//! The fast variant follows He and Sun, "Fast Guided Filter"
//! (arXiv:1505.00996, 2015): `a` and `b` are computed on a reduced grid and
//! interpolated back up, and only the final `a * I + b` runs at full size.
//! It departs from the paper in how the grid is reduced. The paper
//! subsamples the image; here each cell keeps the **means of the moments**
//! of its pixels (`I`, `I^2`, and for a separate input `p` and `I p`). The
//! window statistics are then those of every pixel in the window, exactly,
//! for a window made of whole cells. Averaging the samples first would lose
//! all the variance inside a cell and smooth fine texture far more than the
//! exact filter does.

use crate::buffer::{Plane, par_rows2_mut};

use super::box_blur::{BoxBlurrer, BoxKernel, box_support};

/// `eps` is never taken to be smaller than this. A variance is a small
/// difference of two means of squares; for data around 1 its `f32` rounding
/// noise is about `1e-7`, and dividing that by a smaller `eps` would turn
/// rounding into gain.
const MIN_EPS: f32 = 1e-6;
/// The fast variant reduces by `floor(radius / 4)`, the ratio He and Sun
/// recommend...
const RADIUS_PER_SUBSAMPLE: f32 = 4.0;
/// ...up to this factor.
const MAX_SUBSAMPLE: usize = 16;

/// The window-averaged coefficients `(mean_a, mean_b)` of the guided
/// filter's local linear model.
///
/// `guide` and `guide_squared` hold the guide and its square, or their means
/// over the cells of a reduced grid. `input`, when the guide does not filter
/// itself, is the input and its product with the guide, likewise.
fn coefficients(
    guide: &Plane,
    mut guide_squared: Plane,
    input: Option<(&Plane, Plane)>,
    kernel: BoxKernel,
    eps: f32,
) -> (Plane, Plane) {
    let eps = eps.max(MIN_EPS);
    let mut blur = BoxBlurrer::new(kernel, guide.width(), guide.height());
    let mean_guide = blur.blurred(guide);
    blur.apply(&mut guide_squared);
    let mean_square = guide_squared;

    let (mut a, mut b) = match input {
        None => {
            // a = var / (var + eps), b = mean - a * mean.
            let (mut a, mut b) = (mean_square, mean_guide);
            par_rows2_mut(&mut a, &mut b, |_, a_row, b_row| {
                for (a, b) in a_row.iter_mut().zip(b_row) {
                    let variance = (*a - *b * *b).max(0.0);
                    let gain = variance / (variance + eps);
                    *a = gain;
                    *b *= 1.0 - gain;
                }
            });
            (a, b)
        }
        Some((input, mut product)) => {
            // a = cov / (var + eps), b = mean_input - a * mean_guide.
            let mut b = blur.blurred(input);
            blur.apply(&mut product);
            let mut a = product;
            par_rows2_mut(&mut a, &mut b, |y, a_row, b_row| {
                let means = mean_guide.row(y).iter().zip(mean_square.row(y));
                for ((a, b), (&mean, &square)) in a_row.iter_mut().zip(b_row).zip(means) {
                    let variance = (square - mean * mean).max(0.0);
                    let gain = (*a - mean * *b) / (variance + eps);
                    *a = gain;
                    *b -= gain * mean;
                }
            });
            (a, b)
        }
    };
    blur.apply(&mut a);
    blur.apply(&mut b);
    (a, b)
}

fn exact(guide: &Plane, input: Option<&Plane>, radius: f32, eps: f32) -> Plane {
    let Some(kernel) = BoxKernel::from_radius(radius) else {
        return input.unwrap_or(guide).clone();
    };
    let squared = guide.map(|v| v * v);
    let with_product = input.map(|input| (input, guide.zip_map(input, |g, p| g * p)));
    let (a, mut out) = coefficients(guide, squared, with_product, kernel, eps);
    out.par_rows_mut(|y, row| {
        for ((value, &gain), &g) in row.iter_mut().zip(a.row(y)).zip(guide.row(y)) {
            *value += gain * g;
        }
    });
    out
}

/// Farthest pixel that can influence an output pixel of [`guided_filter`]
/// and [`self_guided_filter`]: two box blurs deep, `2 * ceil(radius)`.
pub fn guided_support(radius: f32) -> usize {
    2 * box_support(radius)
}

/// Edge-preserving smoothing of `input` steered by its own edges.
///
/// `radius` is the half-width of the square window in pixels (fractional
/// values are honoured, see [`box_blur`](super::box_blur())). `eps` is the
/// variance that separates "flat, smooth it" from "edge, keep it": for data
/// in `0..1`, features with a local contrast well under `sqrt(eps)` are
/// averaged away and ones well over it survive at close to full height.
/// Values of `eps` under `1e-6` are treated as `1e-6`, the resolution of a
/// variance in `f32`.
///
/// A radius of 0 or less, or NaN, returns the input unchanged. Cost is
/// constant per pixel for any radius. Edges are replicated. Support:
/// [`guided_support`].
pub fn self_guided_filter(input: &Plane, radius: f32, eps: f32) -> Plane {
    exact(input, None, radius, eps)
}

/// Smooth `input` while following the edges of `guide`: where the guide has
/// structure the output reproduces `input`'s relation to it, elsewhere it is
/// `input`'s local mean. Used to refine a coarse map (a transmission
/// estimate, a mask) against the image it belongs to.
///
/// Parameters, cost, edge handling and support are those of
/// [`self_guided_filter`], with `eps` compared against the guide's variance.
/// Panics when the two planes differ in size.
pub fn guided_filter(guide: &Plane, input: &Plane, radius: f32, eps: f32) -> Plane {
    guide.assert_same_size(input);
    exact(guide, Some(input), radius, eps)
}

/// The factor by which [`fast_guided_filter`] reduces its grid for this
/// radius: `floor(radius / 4)`, between 1 (no reduction: the exact filter)
/// and 16.
pub fn fast_guided_subsample(radius: f32) -> usize {
    if radius.is_nan() || radius <= 0.0 {
        1
    } else {
        ((radius / RADIUS_PER_SUBSAMPLE) as usize).clamp(1, MAX_SUBSAMPLE)
    }
}

/// The radius, in cells `step` pixels wide, of the box on the reduced grid.
/// Chosen so that a window of whole cells weighs `2 * radius + 1` pixels
/// per axis, as the exact filter's window does: `(2 * reduced + 1) * step`
/// is exactly that.
fn reduced_radius(radius: f32, step: usize) -> f32 {
    let step = step as f32;
    (radius - (step - 1.0) / 2.0) / step
}

/// How far to pad a region for [`fast_guided_filter`] and
/// [`fast_self_guided_filter`]: no pixel farther away than this can
/// influence an output pixel. With subsampling factor `s` and the reduced
/// grid's radius `r' = (radius - (s - 1) / 2) / s`, it is
/// `(2 * ceil(r') + 2) * s`: the two box blurs on the reduced grid, one cell
/// for the interpolation and one for the cell the pixel sits in. That is a
/// bound, up to one cell more than the true reach. Equals
/// [`guided_support`] when `s` is 1.
pub fn fast_guided_support(radius: f32) -> usize {
    let step = fast_guided_subsample(radius);
    if step == 1 {
        guided_support(radius)
    } else {
        (2 * box_support(reduced_radius(radius, step)) + 2) * step
    }
}

/// How a plane is divided into `step` x `step` cells that line up with a
/// grid shared by every tile of a view.
struct CellGrid {
    step: usize,
    /// Where the plane's first sample sits inside its first cell.
    offset: (usize, usize),
    cells: (usize, usize),
}

impl CellGrid {
    fn new(width: usize, height: usize, step: usize, origin: (i64, i64)) -> Self {
        let offset = (
            origin.0.rem_euclid(step as i64) as usize,
            origin.1.rem_euclid(step as i64) as usize,
        );
        let count = |offset: usize, len: usize| {
            if len == 0 {
                0
            } else {
                (offset + len).div_ceil(step)
            }
        };
        Self {
            step,
            offset,
            cells: (count(offset.0, width), count(offset.1, height)),
        }
    }

    /// The samples of a `len`-long axis that fall in cell `cell`.
    fn span(&self, cell: usize, offset: usize, len: usize) -> std::ops::Range<usize> {
        (cell * self.step).saturating_sub(offset)..((cell + 1) * self.step - offset).min(len)
    }

    /// The mean over every cell of `f(a, b)`, for two planes of the same
    /// size. Cells cut short by the planes' edge average the samples they
    /// have. Samples are summed in the same order whatever part of the grid
    /// the planes cover, so a whole cell has the same mean in every tile.
    fn reduce(&self, a: &Plane, b: &Plane, f: impl Fn(f32, f32) -> f32 + Sync) -> Plane {
        let mut out = Plane::new(self.cells.0, self.cells.1);
        out.par_rows_mut(|cell_y, row| {
            let rows = self.span(cell_y, self.offset.1, a.height());
            for y in rows.clone() {
                let (line_a, line_b) = (a.row(y), b.row(y));
                for (cell_x, sum) in row.iter_mut().enumerate() {
                    let span = self.span(cell_x, self.offset.0, a.width());
                    for (&va, &vb) in line_a[span.clone()].iter().zip(&line_b[span]) {
                        *sum += f(va, vb);
                    }
                }
            }
            for (cell_x, sum) in row.iter_mut().enumerate() {
                *sum /= (rows.len() * self.span(cell_x, self.offset.0, a.width()).len()) as f32;
            }
        });
        out
    }

    /// For each sample along an axis: the two cells it lies between and the
    /// weight of the second. Derived from the sample's integer position
    /// inside its cell, so it is identical for every tile.
    fn taps(&self, offset: usize, len: usize, cells: usize) -> Vec<(usize, usize, f32)> {
        (0..len)
            .map(|index| {
                let position = offset + index;
                let cell = position / self.step;
                let shift = ((position % self.step) as f32 + 0.5) / self.step as f32 - 0.5;
                if shift >= 0.0 {
                    (cell, (cell + 1).min(cells - 1), shift)
                } else {
                    (cell.saturating_sub(1), cell, shift + 1.0)
                }
            })
            .collect()
    }

    /// `a * guide + b` with the reduced coefficient planes interpolated
    /// bilinearly back to the guide's size.
    fn apply(&self, a: &Plane, b: &Plane, guide: &Plane) -> Plane {
        let columns = self.taps(self.offset.0, guide.width(), self.cells.0);
        let rows = self.taps(self.offset.1, guide.height(), self.cells.1);
        let mut out = Plane::new(guide.width(), guide.height());
        out.par_rows_mut(|y, row| {
            let (top, bottom, weight_y) = rows[y];
            let (a_top, a_bottom, b_top, b_bottom) =
                (a.row(top), a.row(bottom), b.row(top), b.row(bottom));
            let samples = row.iter_mut().zip(guide.row(y)).zip(&columns);
            for ((value, &g), &(left, right, weight_x)) in samples {
                let between = |upper: &[f32], lower: &[f32]| {
                    let first = upper[left] + (upper[right] - upper[left]) * weight_x;
                    let second = lower[left] + (lower[right] - lower[left]) * weight_x;
                    first + (second - first) * weight_y
                };
                *value = between(a_top, a_bottom) * g + between(b_top, b_bottom);
            }
        });
        out
    }
}

fn fast(guide: &Plane, input: Option<&Plane>, radius: f32, eps: f32, origin: (i64, i64)) -> Plane {
    let step = fast_guided_subsample(radius);
    let reduced_kernel = BoxKernel::from_radius(reduced_radius(radius, step));
    let (Some(kernel), false, false) = (reduced_kernel, step == 1, guide.is_empty()) else {
        return exact(guide, input, radius, eps);
    };
    let grid = CellGrid::new(guide.width(), guide.height(), step, origin);
    let mean_guide = grid.reduce(guide, guide, |g, _| g);
    let mean_square = grid.reduce(guide, guide, |g, _| g * g);
    let mean_input = input.map(|input| grid.reduce(input, input, |p, _| p));
    let with_product = input
        .zip(mean_input.as_ref())
        .map(|(input, mean)| (mean, grid.reduce(guide, input, |g, p| g * p)));
    let (a, b) = coefficients(&mean_guide, mean_square, with_product, kernel, eps);
    grid.apply(&a, &b, guide)
}

/// [`self_guided_filter`] for large radii, at a fraction of the cost.
///
/// The plane is divided into cells [`fast_guided_subsample`] pixels wide.
/// The filter's coefficients are computed per cell, from the statistics of
/// all the pixels in a window of whole cells as wide as the exact filter's
/// (so `eps` means what it means for the exact filter), then interpolated
/// back and applied to the full-size plane. Edges stay as sharp as the
/// plane's own; only the decision of how much to smooth is made at lower
/// resolution. For radii under 8 pixels nothing is reduced and the result
/// is exactly [`self_guided_filter`]'s.
///
/// `origin` is the position of this plane's top-left sample in the pixel
/// grid of the view it belongs to: the view's
/// [`View::grid_position`](crate::View::grid_position) minus the plane's
/// padding, for a whole view and a tile alike. A whole view's plane padded
/// by `p` therefore has origin `(-p, -p)`, and `(0, 0)` only when it is not
/// padded. The cells are anchored to that grid, which is what makes a
/// padded tile come out the same as the whole (contract R1); every plane of
/// one render must be given its origin by the same rule.
///
/// A radius of 0 or less, or NaN, returns the input unchanged. Edges are
/// replicated. Support: [`fast_guided_support`].
pub fn fast_self_guided_filter(input: &Plane, radius: f32, eps: f32, origin: (i64, i64)) -> Plane {
    fast(input, None, radius, eps, origin)
}

/// [`guided_filter`] for large radii, at a fraction of the cost. See
/// [`fast_self_guided_filter`] for how it works and what `origin` means.
/// Panics when the two planes differ in size.
pub fn fast_guided_filter(
    guide: &Plane,
    input: &Plane,
    radius: f32,
    eps: f32,
    origin: (i64, i64),
) -> Plane {
    guide.assert_same_size(input);
    fast(guide, Some(input), radius, eps, origin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subsample_factor_follows_the_radius() {
        assert_eq!(fast_guided_subsample(0.0), 1);
        assert_eq!(fast_guided_subsample(7.9), 1);
        assert_eq!(fast_guided_subsample(8.0), 2);
        assert_eq!(fast_guided_subsample(20.0), 5);
        assert_eq!(fast_guided_subsample(1.0e9), 16);
        assert_eq!(fast_guided_subsample(f32::INFINITY), 16);
        assert_eq!(fast_guided_subsample(f32::NAN), 1);
        assert_eq!(fast_guided_subsample(-5.0), 1);
    }

    #[test]
    fn support_counts_both_blurs_and_the_interpolation() {
        assert_eq!(guided_support(2.5), 6);
        assert_eq!(fast_guided_support(2.5), 6);
        // radius 20 reduces by 5 to a radius of 4: (2 * 4 + 2) * 5.
        assert_eq!(fast_guided_support(20.0), 50);
    }

    #[test]
    fn cells_are_anchored_to_the_shared_grid() {
        let plane = Plane::from_fn(10, 1, |x, _| x as f32);
        // Starting at global x = 3 with 4-wide cells: [3], [4..8), [8..12), [12].
        let grid = CellGrid::new(10, 1, 4, (3, 0));
        assert_eq!(grid.cells, (4, 1));
        let means = grid.reduce(&plane, &plane, |a, _| a);
        assert_eq!(means.as_slice(), [0.0, 2.5, 6.5, 9.0]);
        let squares = grid.reduce(&plane, &plane, |a, b| a * b);
        assert_eq!(squares.as_slice(), [0.0, 7.5, 43.5, 81.0]);
        // A negative origin lands on the same grid.
        let shifted = CellGrid::new(10, 1, 4, (-5, 0));
        assert_eq!(shifted.offset.0, 3);
    }

    #[test]
    fn interpolation_weights_depend_only_on_the_position_inside_a_cell() {
        let grid = CellGrid::new(8, 1, 4, (0, 0));
        let taps = grid.taps(0, 8, 2);
        assert_eq!(taps[0], (0, 0, 0.625));
        assert_eq!(taps[1], (0, 0, 0.875));
        assert_eq!(taps[2], (0, 1, 0.125));
        assert_eq!(taps[3], (0, 1, 0.375));
        assert_eq!(taps[4], (0, 1, 0.625));
        assert_eq!(taps[7], (1, 1, 0.375));
    }
}
