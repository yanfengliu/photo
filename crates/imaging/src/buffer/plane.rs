//! The single-channel buffer and its parallel helpers.

use rayon::prelude::*;

/// A single-channel `f32` image in row-major order.
///
/// `width * height` always equals the number of samples. Either dimension may
/// be zero, in which case the plane is empty.
#[derive(Clone, Debug, PartialEq)]
pub struct Plane {
    width: usize,
    height: usize,
    data: Vec<f32>,
}

impl Plane {
    /// A plane of zeros.
    pub fn new(width: usize, height: usize) -> Self {
        Self::filled(width, height, 0.0)
    }

    /// A plane with every sample set to `value`.
    pub fn filled(width: usize, height: usize, value: f32) -> Self {
        Self {
            width,
            height,
            data: vec![value; width * height],
        }
    }

    /// Wrap existing samples without copying, or `None` when the buffer
    /// length is not `width * height`.
    pub fn from_vec(width: usize, height: usize, data: Vec<f32>) -> Option<Self> {
        (width.checked_mul(height) == Some(data.len())).then_some(Self {
            width,
            height,
            data,
        })
    }

    /// A plane whose sample at `(x, y)` is `f(x, y)`. Rows are computed in
    /// parallel; the result is the same on any number of threads.
    pub fn from_fn(width: usize, height: usize, f: impl Fn(usize, usize) -> f32 + Sync) -> Self {
        let mut plane = Self::new(width, height);
        plane.par_rows_mut(|y, row| {
            for (x, value) in row.iter_mut().enumerate() {
                *value = f(x, y);
            }
        });
        plane
    }

    /// Samples per row.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Number of rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// True when the plane holds no samples.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// All samples, row after row.
    pub fn as_slice(&self) -> &[f32] {
        &self.data
    }

    /// All samples, row after row. The length cannot be changed through this.
    pub fn as_mut_slice(&mut self) -> &mut [f32] {
        &mut self.data
    }

    /// The sample buffer, without copying.
    pub fn into_vec(self) -> Vec<f32> {
        self.data
    }

    /// The sample at `(x, y)`. Panics when out of bounds.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> f32 {
        assert!(x < self.width, "x {x} is outside the plane's width");
        self.data[y * self.width + x]
    }

    /// Set the sample at `(x, y)`. Panics when out of bounds.
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, value: f32) {
        assert!(x < self.width, "x {x} is outside the plane's width");
        self.data[y * self.width + x] = value;
    }

    /// Row `y`. Panics when out of bounds.
    #[inline]
    pub fn row(&self, y: usize) -> &[f32] {
        &self.data[y * self.width..(y + 1) * self.width]
    }

    /// Row `y`, mutable. Panics when out of bounds.
    #[inline]
    pub fn row_mut(&mut self, y: usize) -> &mut [f32] {
        &mut self.data[y * self.width..(y + 1) * self.width]
    }

    /// Run `f(y, row)` for every row, in parallel. Each row is visited exactly
    /// once and rows cannot see each other, so the result does not depend on
    /// the thread count.
    pub fn par_rows_mut(&mut self, f: impl Fn(usize, &mut [f32]) + Sync) {
        if self.width == 0 {
            return;
        }
        self.data
            .par_chunks_exact_mut(self.width)
            .enumerate()
            .for_each(|(y, row)| f(y, row));
    }

    /// Like [`Self::par_rows_mut`], with a scratch value made by `init` and
    /// reused between rows on the same thread. Which rows share a scratch
    /// value is unspecified, so `f` must not let a result depend on what an
    /// earlier row left in it.
    pub fn par_rows_mut_with<S>(
        &mut self,
        init: impl Fn() -> S + Sync + Send,
        f: impl Fn(&mut S, usize, &mut [f32]) + Sync + Send,
    ) {
        if self.width == 0 {
            return;
        }
        self.data
            .par_chunks_exact_mut(self.width)
            .enumerate()
            .for_each_init(init, |scratch, (y, row)| f(scratch, y, row));
    }

    /// A new plane with `f` applied to every sample.
    pub fn map(&self, f: impl Fn(f32) -> f32 + Sync) -> Self {
        let mut out = Self::new(self.width, self.height);
        out.par_rows_mut(|y, row| {
            for (value, &source) in row.iter_mut().zip(self.row(y)) {
                *value = f(source);
            }
        });
        out
    }

    /// A new plane combining this one with `other` sample by sample. Panics
    /// when the two planes differ in size.
    pub fn zip_map(&self, other: &Self, f: impl Fn(f32, f32) -> f32 + Sync) -> Self {
        self.assert_same_size(other);
        let mut out = Self::new(self.width, self.height);
        out.par_rows_mut(|y, row| {
            for ((value, &a), &b) in row.iter_mut().zip(self.row(y)).zip(other.row(y)) {
                *value = f(a, b);
            }
        });
        out
    }

    /// A copy of the `width` x `height` rectangle whose top-left sample is
    /// `(x, y)`. Panics when the rectangle does not fit inside the plane.
    pub fn crop(&self, x: usize, y: usize, width: usize, height: usize) -> Self {
        assert!(
            x + width <= self.width && y + height <= self.height,
            "crop {width}x{height} at ({x}, {y}) does not fit a {}x{} plane",
            self.width,
            self.height
        );
        let mut data = Vec::with_capacity(width * height);
        for row in y..y + height {
            data.extend_from_slice(&self.row(row)[x..x + width]);
        }
        Self {
            width,
            height,
            data,
        }
    }

    /// Panics, naming both sizes, unless `other` has this plane's size.
    pub(crate) fn assert_same_size(&self, other: &Self) {
        assert!(
            self.width == other.width && self.height == other.height,
            "planes differ in size: {}x{} and {}x{}",
            self.width,
            self.height,
            other.width,
            other.height
        );
    }
}

/// Run `f(y, row_a, row_b)` for every row of two planes of the same size, in
/// parallel: for point operations that update two planes at once.
pub(crate) fn par_rows2_mut(
    a: &mut Plane,
    b: &mut Plane,
    f: impl Fn(usize, &mut [f32], &mut [f32]) + Sync,
) {
    a.assert_same_size(b);
    if a.width == 0 {
        return;
    }
    a.data
        .par_chunks_exact_mut(a.width)
        .zip(b.data.par_chunks_exact_mut(b.width))
        .enumerate()
        .for_each(|(y, (row_a, row_b))| f(y, row_a, row_b));
}

/// Run `f(x0, rows)` for vertical strips of `plane`, in parallel, where
/// `rows[y]` is the part of row `y` that starts at column `x0`.
///
/// This is how the column passes of the running-sum filters parallelize: a
/// column is computed top to bottom by one thread with no help from its
/// neighbours, so the result is independent of the strip width and of the
/// thread count.
pub(crate) fn par_column_strips(
    plane: &mut Plane,
    strip_width: usize,
    f: impl Fn(usize, &mut [&mut [f32]]) + Sync,
) {
    let (width, height) = (plane.width, plane.height);
    if width == 0 || height == 0 {
        return;
    }
    let strip_width = strip_width.max(1);
    let mut strips: Vec<Vec<&mut [f32]>> = (0..width.div_ceil(strip_width))
        .map(|_| Vec::with_capacity(height))
        .collect();
    for row in plane.data.chunks_exact_mut(width) {
        for (strip, segment) in strips.iter_mut().zip(row.chunks_mut(strip_width)) {
            strip.push(segment);
        }
    }
    strips
        .into_par_iter()
        .enumerate()
        .for_each(|(index, mut rows)| f(index * strip_width, &mut rows));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_vec_checks_the_length() {
        assert!(Plane::from_vec(3, 2, vec![0.0; 6]).is_some());
        assert!(Plane::from_vec(3, 2, vec![0.0; 5]).is_none());
    }

    #[test]
    fn from_fn_fills_in_row_major_order() {
        let plane = Plane::from_fn(3, 2, |x, y| (y * 10 + x) as f32);
        assert_eq!(plane.as_slice(), [0.0, 1.0, 2.0, 10.0, 11.0, 12.0]);
        assert_eq!(plane.get(2, 1), 12.0);
        assert_eq!(plane.row(1), [10.0, 11.0, 12.0]);
    }

    #[test]
    fn crop_copies_the_requested_rectangle() {
        let plane = Plane::from_fn(4, 3, |x, y| (y * 10 + x) as f32);
        let cropped = plane.crop(1, 1, 2, 2);
        assert_eq!(cropped.as_slice(), [11.0, 12.0, 21.0, 22.0]);
    }

    #[test]
    fn point_operations_visit_every_sample() {
        let plane = Plane::from_fn(4, 3, |x, y| (y * 10 + x) as f32);
        assert_eq!(plane.map(|v| v + 1.0).get(3, 2), 24.0);
        assert_eq!(plane.zip_map(&plane, |a, b| a * b).get(2, 1), 144.0);
        let (mut a, mut b) = (plane.clone(), plane.clone());
        par_rows2_mut(&mut a, &mut b, |y, row_a, row_b| {
            for (x, (va, vb)) in row_a.iter_mut().zip(row_b).enumerate() {
                *va += 1.0;
                *vb = (y * 10 + x) as f32 * 2.0;
            }
        });
        assert_eq!(a, plane.map(|v| v + 1.0));
        assert_eq!(b, plane.map(|v| v * 2.0));
    }

    #[test]
    fn an_empty_plane_is_allowed_everywhere() {
        let mut plane = Plane::new(0, 5);
        plane.par_rows_mut(|_, _| panic!("no rows to visit"));
        assert!(plane.is_empty());
        assert!(plane.map(|v| v + 1.0).is_empty());
        par_column_strips(&mut plane, 8, |_, _| panic!("no strips to visit"));
        let mut flat = Plane::new(5, 0);
        par_column_strips(&mut flat, 8, |_, _| panic!("no strips to visit"));
    }

    #[test]
    fn column_strips_cover_every_sample_once() {
        let mut plane = Plane::new(10, 4);
        par_column_strips(&mut plane, 4, |x0, rows| {
            for (y, row) in rows.iter_mut().enumerate() {
                for (i, value) in row.iter_mut().enumerate() {
                    *value += (y * 100 + x0 + i) as f32;
                }
            }
        });
        assert_eq!(plane, Plane::from_fn(10, 4, |x, y| (y * 100 + x) as f32));
    }

    #[test]
    fn scratch_rows_see_their_own_index() {
        let mut plane = Plane::new(5, 9);
        plane.par_rows_mut_with(
            || vec![0.0f32; 5],
            |scratch, y, row| {
                scratch.fill(y as f32);
                row.copy_from_slice(scratch);
            },
        );
        assert_eq!(plane, Plane::from_fn(5, 9, |_, y| y as f32));
    }
}
