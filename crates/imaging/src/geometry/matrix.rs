//! 3 x 3 homogeneous matrices in `f64`.
//!
//! Geometry is composed and inverted in double precision so that a 60 MP
//! frame keeps sub-thousandth-of-a-pixel accuracy, and so that the pixel
//! preserving cases (quarter turns, mirrors, whole-pixel crops) stay exact:
//! their matrices hold only small integers and halves, which `f64` multiplies
//! and inverts without rounding.

/// Row-major 3 x 3 matrix applied to column vectors `(x, y, 1)`.
pub(crate) type Mat3 = [[f64; 3]; 3];

pub(crate) const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub(crate) fn translate(dx: f64, dy: f64) -> Mat3 {
    [[1.0, 0.0, dx], [0.0, 1.0, dy], [0.0, 0.0, 1.0]]
}

pub(crate) fn scale(sx: f64, sy: f64) -> Mat3 {
    [[sx, 0.0, 0.0], [0.0, sy, 0.0], [0.0, 0.0, 1.0]]
}

/// `a * b`: apply `b` first, then `a`.
pub(crate) fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

/// The product of several matrices, the last one applied first.
pub(crate) fn chain(matrices: &[Mat3]) -> Mat3 {
    matrices.iter().fold(IDENTITY, |acc, m| mul(&acc, m))
}

/// Inverse by the adjugate, or `None` when the determinant is zero or
/// anything is not finite.
///
/// There is deliberately no "nearly singular" threshold. These matrices mix
/// units: a translation column in pixels (thousands), a dimensionless
/// linear part, a perspective row in inverse pixels. No comparison of the
/// determinant with the entries' magnitudes survives a change of image
/// size, and a mirror of a wide panorama is as invertible as a mirror of a
/// thumbnail.
pub(crate) fn invert(m: &Mat3) -> Option<Mat3> {
    let minor =
        |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let adjugate = [
        [minor(1, 2, 1, 2), -minor(0, 2, 1, 2), minor(0, 1, 1, 2)],
        [-minor(1, 2, 0, 2), minor(0, 2, 0, 2), -minor(0, 1, 0, 2)],
        [minor(1, 2, 0, 1), -minor(0, 2, 0, 1), minor(0, 1, 0, 1)],
    ];
    let det = m[0][0] * adjugate[0][0] + m[0][1] * adjugate[1][0] + m[0][2] * adjugate[2][0];
    if !det.is_finite() || det == 0.0 {
        return None;
    }
    let inverse = adjugate.map(|row| row.map(|v| v / det));
    inverse
        .iter()
        .flatten()
        .all(|v| v.is_finite())
        .then_some(inverse)
}

/// Multiply every entry by `factor`. A homography is unchanged by this; it
/// only fixes the scale of the homogeneous coordinate.
pub(crate) fn scaled(m: &Mat3, factor: f64) -> Mat3 {
    m.map(|row| row.map(|v| v * factor))
}

/// `m * (x, y, 1)` without the perspective divide.
#[inline]
pub(crate) fn apply(m: &Mat3, x: f64, y: f64) -> [f64; 3] {
    [
        m[0][0] * x + m[0][1] * y + m[0][2],
        m[1][0] * x + m[1][1] * y + m[1][2],
        m[2][0] * x + m[2][1] * y + m[2][2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverting_a_pixel_preserving_matrix_is_exact() {
        // A clockwise quarter turn of a 4000-row image followed by a
        // whole-pixel crop offset.
        let turn = [[0.0, -1.0, 4000.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let m = mul(&translate(-37.0, 12.0), &turn);
        let inverse = invert(&m).unwrap();
        assert_eq!(mul(&m, &inverse), IDENTITY);
        assert_eq!(apply(&inverse, 100.5, 7.5), [-4.5, 3862.5, 1.0]);
    }

    #[test]
    fn a_singular_matrix_has_no_inverse() {
        assert!(invert(&scale(1.0, 0.0)).is_none());
        assert!(invert(&[[f64::NAN; 3]; 3]).is_none());
        assert!(invert(&[[f64::INFINITY, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]).is_none());
        assert!(invert(&[[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]]).is_none());
    }

    #[test]
    fn invertibility_does_not_depend_on_the_size_of_the_picture() {
        // The translation column is in pixels and can dwarf the rest; that
        // says nothing about whether the matrix can be inverted.
        let mirror = [[-1.0, 0.0, 250_000.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(invert(&mirror), Some(mirror));
        // 1% about the centre of a 6000 x 4000 frame.
        let tiny = chain(&[translate(3000.0, 2000.0), scale(0.01, 0.01), translate(-3000.0, -2000.0)]);
        let inverse = invert(&tiny).expect("a hundredth is small, not singular");
        let [x, y, w] = apply(&inverse, 3010.0, 1990.0);
        assert!((x - 4000.0).abs() < 1e-6 && (y - 1000.0).abs() < 1e-6 && w == 1.0);
    }

    #[test]
    fn chain_applies_the_last_matrix_first() {
        let m = chain(&[translate(10.0, 0.0), scale(2.0, 2.0)]);
        assert_eq!(apply(&m, 1.0, 1.0), [12.0, 2.0, 1.0]);
    }
}
