//! Film-grain noise as a function of position.
//!
//! Grain must sit still on the picture: the same output position has to get
//! the same value in a fit-to-window preview, in a 1:1 tile and in an export
//! strip, or it would crawl as the user zooms and seams would show between
//! tiles. So the noise is not generated per buffer; it is a pure function of
//! the full-resolution output position, evaluated wherever a view has a
//! pixel.
//!
//! The function is gradient noise (Perlin, "An Image Synthesizer", SIGGRAPH
//! 1985, with the quintic fade of Perlin, "Improving Noise", SIGGRAPH 2002):
//! a pseudo-random unit gradient at every point of a square lattice,
//! interpolated smoothly inside each cell. Two such lattices are summed,
//! turned by different angles so that neither lines up with the pixel grid
//! or with the other, which hides the lattice and evens out the variance.

use crate::buffer::Plane;
use crate::view::View;

/// Sixteen unit vectors, 22.5 degrees apart, starting at 11.25 degrees.
/// Written out as constants so the noise is the same bits on every platform.
const GRADIENTS: [(f32, f32); 16] = {
    const A: f32 = 0.980_785_3;
    const B: f32 = 0.195_090_32;
    const C: f32 = 0.831_469_6;
    const D: f32 = 0.555_570_24;
    [
        (A, B),
        (C, D),
        (D, C),
        (B, A),
        (-B, A),
        (-D, C),
        (-C, D),
        (-A, B),
        (-A, -B),
        (-C, -D),
        (-D, -C),
        (-B, -A),
        (B, -A),
        (D, -C),
        (C, -D),
        (A, -B),
    ]
};

/// A lattice: its rotation and scale relative to the picture (as the two
/// rows of a matrix applied to `position / size`), an offset, and a seed.
struct Lattice {
    row_u: (f32, f32),
    row_v: (f32, f32),
    offset: (f32, f32),
    seed: u32,
}

/// Turned by `atan(1 / golden ratio)`, about 31.7 degrees, one cell per
/// grain size.
const FIRST: Lattice = Lattice {
    row_u: (0.850_650_8, -0.525_731_1),
    row_v: (0.525_731_1, 0.850_650_8),
    offset: (0.0, 0.0),
    seed: 0x9E37_79B9,
};

/// Turned by -17 degrees, with cells 1.17 times smaller.
const SECOND: Lattice = Lattice {
    row_u: (1.118_876_6, 0.342_074_9),
    row_v: (-0.342_074_9, 1.118_876_6),
    offset: (0.731, 0.419),
    seed: 0x85EB_CA6B,
};

/// Scales the sum of the two lattices to unit variance. One lattice has a
/// variance of `4 A B` averaged over a cell, with `A = 0.029692` and
/// `B = 0.391775` the integrals of `fade(t)^2 (1 - t)^2` and `fade(t)^2`
/// over `0..1`; the two are independent, so the sum has twice that.
const UNIT_VARIANCE: f32 = 3.278_1;

/// A well-mixed 32-bit hash (the "lowbias32" finalizer of Wellons,
/// "Prospecting for Hash Functions", 2018).
#[inline]
fn mix(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846C_A68B);
    value ^ (value >> 16)
}

/// The gradient at lattice point `(x, y)`: the two coordinates are spread
/// over the word by odd multipliers, combined, and mixed once.
#[inline]
fn gradient(x: i32, y: i32, seed: u32) -> (f32, f32) {
    let spread = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    GRADIENTS[(mix(spread ^ seed) >> 28) as usize]
}

impl Lattice {
    /// Gradient noise at `(x, y)` in units of the grain size. Zero mean,
    /// within +-0.71, and zero at the lattice points themselves.
    #[inline]
    fn noise(&self, x: f32, y: f32) -> f32 {
        let u = x * self.row_u.0 + y * self.row_u.1 + self.offset.0;
        let v = x * self.row_v.0 + y * self.row_v.1 + self.offset.1;
        let (cell_u, cell_v) = (u.floor(), v.floor());
        let (fu, fv) = (u - cell_u, v - cell_v);
        let (iu, iv) = (cell_u as i32, cell_v as i32);
        let corner = |du: i32, dv: i32| {
            let (gx, gy) = gradient(iu.wrapping_add(du), iv.wrapping_add(dv), self.seed);
            gx * (fu - du as f32) + gy * (fv - dv as f32)
        };
        let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        let (su, sv) = (fade(fu), fade(fv));
        let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * su;
        let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * su;
        top + (bottom - top) * sv
    }
}

/// Grain at full-resolution output position `(x, y)`, for grains about
/// `size` output pixels across.
///
/// A pure function of its arguments with a fixed seed: the same position
/// gives the same value in every view, tile and session, on every platform.
/// It depends on position only through `x / size` and `y / size`.
///
/// The value has zero mean and unit variance over any area a few grains
/// wide, and always lies within +-4.7. It is band-limited: smooth over
/// distances well under `size`, uncorrelated beyond a few times `size`, and
/// without energy at much larger scales, so it never shifts the brightness
/// of a region. `size` below 1 (or not finite) is treated as 1, since finer
/// grain than the pixel grid would only alias. Non-finite positions are
/// treated as 0, and positions more than ten million grains from the origin
/// are held there (an `f32` position that far out has no fraction left to
/// vary over anyway).
///
/// A view at reduced scale samples this function more sparsely than its
/// grains; scaling the amplitude for that is the caller's decision.
#[inline]
pub fn grain_noise(x: f32, y: f32, size: f32) -> f32 {
    const LIMIT: f32 = 1.0e7;
    let size = if size.is_finite() { size.max(1.0) } else { 1.0 };
    let in_grains = |value: f32| {
        if value.is_finite() {
            (value / size).clamp(-LIMIT, LIMIT)
        } else {
            0.0
        }
    };
    let (x, y) = (in_grains(x), in_grains(y));
    (FIRST.noise(x, y) + SECOND.noise(x, y)) * UNIT_VARIANCE
}

/// [`grain_noise`] at the centre of every pixel of `view`, surrounded by
/// `padding` extra view pixels on every side (the layout of
/// [`resample`](crate::resample())).
///
/// A tile of a view gets exactly the values the whole view has there (R1),
/// and views at different scales agree wherever their pixel centres
/// coincide. The result is the same on any number of threads (R4).
pub fn grain_plane(view: &View, size: f32, padding: u32) -> Plane {
    let grid = view.grid(padding);
    let mut out = Plane::new(grid.width, grid.height);
    out.par_rows_mut(|row_index, row| {
        let y = grid.y(row_index) as f32;
        for (column, value) in row.iter_mut().enumerate() {
            *value = grain_noise(grid.x(column) as f32, y, size);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradients_are_unit_vectors_spread_evenly() {
        let (mut sum_x, mut sum_y) = (0.0f32, 0.0f32);
        for (x, y) in GRADIENTS {
            assert!((x.hypot(y) - 1.0).abs() < 1e-6);
            sum_x += x;
            sum_y += y;
        }
        assert!(sum_x.abs() < 1e-6 && sum_y.abs() < 1e-6);
    }

    #[test]
    fn the_hash_uses_every_gradient_about_equally() {
        let mut counts = [0u32; 16];
        for y in -64..64 {
            for x in -64..64 {
                let (gx, gy) = gradient(x, y, FIRST.seed);
                let index = GRADIENTS.iter().position(|&g| g == (gx, gy)).unwrap();
                counts[index] += 1;
            }
        }
        // 16 384 draws, 1 024 expected per gradient.
        assert!(
            counts.iter().all(|&count| (900..1150).contains(&count)),
            "{counts:?}"
        );
    }

    #[test]
    fn a_lattice_is_zero_at_its_points_and_continuous_across_cells() {
        let plain = Lattice {
            row_u: (1.0, 0.0),
            row_v: (0.0, 1.0),
            offset: (0.0, 0.0),
            seed: 7,
        };
        assert_eq!(plain.noise(3.0, -2.0), 0.0);
        let before = plain.noise(4.0 - 1e-4, 0.37);
        let after = plain.noise(4.0 + 1e-4, 0.37);
        assert!((before - after).abs() < 1e-3, "{before} vs {after}");
        assert!(plain.noise(3.5, -1.5).abs() <= 0.708);
    }
}
