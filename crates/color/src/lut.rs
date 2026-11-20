//! Lookup tables for the per-pixel fast paths. Tables are filled once from
//! the exact `f64` curve functions and then read with linear interpolation,
//! so a pixel outside every mask never pays for `powf`.

/// A uniformly sampled 1-D table over `[0, domain_max]` with clamped linear
/// interpolation. Because it interpolates linearly between exact samples, a
/// table built from a monotonic function is itself monotonic.
#[derive(Clone)]
pub(crate) struct Lut {
    values: Box<[f32]>,
    /// Cells per unit of the input.
    scale: f32,
    /// Index of the last cell boundary, as a float.
    limit: f32,
}

/// Prints the table's shape, not its thousand-odd samples.
impl std::fmt::Debug for Lut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lut")
            .field("cells", &(self.values.len() - 1))
            .field("domain_max", &(self.limit / self.scale))
            .finish()
    }
}

impl Lut {
    /// Sample `f` at `cells + 1` evenly spaced points of `[0, domain_max]`.
    pub(crate) fn build(cells: usize, domain_max: f64, f: impl Fn(f64) -> f64) -> Self {
        let cells = cells.max(1);
        let values = (0..=cells)
            .map(|i| f(domain_max * i as f64 / cells as f64) as f32)
            .collect();
        Self {
            values,
            scale: (cells as f64 / domain_max) as f32,
            limit: cells as f32,
        }
    }

    /// The table value at `x`, clamped to the table's domain. NaN reads as
    /// the first entry.
    #[inline(always)]
    pub(crate) fn eval(&self, x: f32) -> f32 {
        let p = (x * self.scale).max(0.0).min(self.limit);
        let i = (p as usize).min(self.values.len() - 2);
        let (lo, hi) = (self.values[i], self.values[i + 1]);
        let value = lo + (hi - lo) * (p - i as f32);
        // Rounding can carry the interpolant one step past the far sample;
        // holding it there keeps the table monotonic across cell boundaries
        // to the last bit.
        if hi >= lo {
            value.min(hi)
        } else {
            value.max(hi)
        }
    }

    /// The last two samples, for deriving an end slope.
    pub(crate) fn last_pair(&self) -> (f32, f32) {
        let n = self.values.len();
        (self.values[n - 2], self.values[n - 1])
    }
}

/// A tone curve defined on perceptual values but applied to linear ones:
/// `linear_out = decode(curve(encode(linear_in)))`. The table is indexed by
/// `sqrt(linear)`, which spaces the samples almost evenly in perceptual terms
/// and costs one square root per lookup. Above 1.0 the curve continues along
/// its end slope, so over-range values stay ordered instead of clipping.
#[derive(Clone, Debug)]
pub(crate) struct LinearCurveLut {
    table: Lut,
    top: f32,
    top_slope: f32,
}

impl LinearCurveLut {
    /// Build from `linear_out = f(linear_in)` sampled over `[0, 1]`.
    pub(crate) fn build(cells: usize, f: impl Fn(f64) -> f64) -> Self {
        let table = Lut::build(cells, 1.0, |t| f(t * t));
        let (below, top) = table.last_pair();
        let top_slope = (f64::from(top - below) / Self::end_step(cells)).max(0.0) as f32;
        Self {
            table,
            top,
            top_slope,
        }
    }

    /// The linear distance from the last sample below 1.0 to 1.0. The slope
    /// used above 1.0 is the secant over this step, so code that evaluates
    /// the same curve directly can reproduce the extension exactly with
    /// [`extend_above_one`].
    pub(crate) fn end_step(cells: usize) -> f64 {
        let below = 1.0 - 1.0 / cells.max(1) as f64;
        1.0 - below * below
    }

    #[inline(always)]
    pub(crate) fn eval(&self, linear: f32) -> f32 {
        if linear >= 1.0 {
            return self.top + self.top_slope * (linear - 1.0).min(1.0e6);
        }
        self.table.eval(linear.max(0.0).sqrt())
    }
}

/// Evaluate `f` (linear in, linear out, defined on `[0, 1]`) at any
/// non-negative `linear` with the same rule a [`LinearCurveLut`] of `cells`
/// samples uses above 1.0: continue along the secant of the last cell,
/// never downward.
pub(crate) fn extend_above_one(cells: usize, linear: f64, f: impl Fn(f64) -> f64) -> f64 {
    if linear < 1.0 {
        return f(linear.max(0.0));
    }
    let step = LinearCurveLut::end_step(cells);
    let top = f(1.0);
    let slope = ((top - f(1.0 - step)) / step).max(0.0);
    top + slope * (linear - 1.0).min(1.0e6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_reproduces_its_function_and_clamps_outside_the_domain() {
        let lut = Lut::build(256, 2.0, |x| x * x);
        for step in 0..=200 {
            let x = step as f32 / 100.0;
            assert!((lut.eval(x) - x * x).abs() < 2e-5, "{x}");
        }
        assert_eq!(lut.eval(-1.0), 0.0);
        assert_eq!(lut.eval(9.0), 4.0);
        assert_eq!(lut.eval(f32::NAN), 0.0);
    }

    #[test]
    fn a_linear_curve_table_extends_along_its_end_slope() {
        let lut = LinearCurveLut::build(1024, |x| 0.5 * x);
        assert!((lut.eval(0.25) - 0.125).abs() < 1e-5);
        assert!((lut.eval(1.0) - 0.5).abs() < 1e-6);
        assert!((lut.eval(3.0) - 1.5).abs() < 1e-2);
        assert_eq!(lut.eval(-0.5), 0.0);
        let mut previous = -1.0;
        for step in 0..=4000 {
            let value = lut.eval(step as f32 / 2000.0);
            assert!(value >= previous);
            previous = value;
        }
    }
}
