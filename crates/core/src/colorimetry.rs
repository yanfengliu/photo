//! Colorimetric constants and the small set of conversions every crate needs:
//! RGB colour spaces, their transfer functions, and the matrices between them.
//!
//! Sources: IEC 61966-2-1 (sRGB), SMPTE EG 432-1 (Display P3), Adobe RGB (1998)
//! Color Image Encoding, ISO 22028-2 (ROMM / ProPhoto), ITU-R BT.2020, and the
//! Bradford chromatic adaptation transform (Lam 1985, as used by ICC v4).
//!
//! Matrices are derived in `f64` from chromaticities rather than typed in, so
//! a round trip between any two spaces is exact to `f32` precision.

use serde::{Deserialize, Serialize};

/// Row-major 3x3 matrix applied to column vectors.
pub type Mat3 = [[f32; 3]; 3];

type Mat3d = [[f64; 3]; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// The linear RGB space the develop pipeline computes in.
pub const WORKING_SPACE: RgbSpace = RgbSpace::Rec2020;

/// Relative luminance weights of [`WORKING_SPACE`] (the Y row of its RGB to
/// XYZ matrix; a test pins them to the derived matrix).
pub const WORKING_LUMA: [f32; 3] = [0.262_700_2, 0.677_998_1, 0.059_301_7];

/// Relative luminance of a linear working-space colour.
#[inline]
pub fn luminance(rgb: [f32; 3]) -> f32 {
    rgb[0] * WORKING_LUMA[0] + rgb[1] * WORKING_LUMA[1] + rgb[2] * WORKING_LUMA[2]
}

#[inline]
pub fn mat3_apply(m: &Mat3, v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

pub fn mat3_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    narrow(&mul_d(&widen(a), &widen(b)))
}

/// Inverse of `m`, or `None` when it is singular (to working precision).
pub fn mat3_invert(m: &Mat3) -> Option<Mat3> {
    invert_d(&widen(m)).map(|inv| narrow(&inv))
}

/// A CIE 1931 xy chromaticity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chromaticity {
    pub x: f64,
    pub y: f64,
}

impl Chromaticity {
    pub const D65: Self = Self { x: 0.3127, y: 0.3290 };
    pub const D50: Self = Self { x: 0.3457, y: 0.3585 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// XYZ tristimulus of this chromaticity at unit luminance (Y = 1).
    pub fn to_xyz(self) -> [f64; 3] {
        [self.x / self.y, 1.0, (1.0 - self.x - self.y) / self.y]
    }
}

/// The chromaticities that define an RGB colour space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primaries {
    pub red: Chromaticity,
    pub green: Chromaticity,
    pub blue: Chromaticity,
    pub white: Chromaticity,
}

/// An RGB colour space: primaries, white point and transfer function.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RgbSpace {
    #[default]
    Srgb,
    DisplayP3,
    AdobeRgb,
    ProPhoto,
    Rec2020,
}

impl RgbSpace {
    pub const ALL: [Self; 5] = [
        Self::Srgb,
        Self::DisplayP3,
        Self::AdobeRgb,
        Self::ProPhoto,
        Self::Rec2020,
    ];

    /// Human-readable name, as shown in the export dialog.
    pub fn name(self) -> &'static str {
        match self {
            Self::Srgb => "sRGB",
            Self::DisplayP3 => "Display P3",
            Self::AdobeRgb => "Adobe RGB (1998)",
            Self::ProPhoto => "ProPhoto RGB",
            Self::Rec2020 => "Rec. 2020",
        }
    }

    pub fn primaries(self) -> Primaries {
        let c = Chromaticity::new;
        match self {
            Self::Srgb => Primaries {
                red: c(0.64, 0.33),
                green: c(0.30, 0.60),
                blue: c(0.15, 0.06),
                white: Chromaticity::D65,
            },
            Self::DisplayP3 => Primaries {
                red: c(0.680, 0.320),
                green: c(0.265, 0.690),
                blue: c(0.150, 0.060),
                white: Chromaticity::D65,
            },
            Self::AdobeRgb => Primaries {
                red: c(0.64, 0.33),
                green: c(0.21, 0.71),
                blue: c(0.15, 0.06),
                white: Chromaticity::D65,
            },
            Self::ProPhoto => Primaries {
                red: c(0.7347, 0.2653),
                green: c(0.1596, 0.8404),
                blue: c(0.0366, 0.0001),
                white: Chromaticity::D50,
            },
            Self::Rec2020 => Primaries {
                red: c(0.708, 0.292),
                green: c(0.170, 0.797),
                blue: c(0.131, 0.046),
                white: Chromaticity::D65,
            },
        }
    }

    /// Linear RGB to XYZ, relative to this space's own white point.
    pub fn to_xyz(self) -> Mat3 {
        narrow(&rgb_to_xyz_d(self.primaries()))
    }

    /// XYZ (relative to this space's own white point) to linear RGB.
    pub fn from_xyz(self) -> Mat3 {
        narrow(&invert_d(&rgb_to_xyz_d(self.primaries())).expect("primaries are not collinear"))
    }

    /// Encoded value to linear light. Odd-symmetric, so out-of-range negative
    /// values survive a round trip instead of turning into NaN.
    #[inline]
    pub fn decode(self, encoded: f32) -> f32 {
        let v = encoded.abs();
        let linear = match self {
            Self::Srgb | Self::DisplayP3 => {
                if v <= 0.040_45 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            }
            Self::AdobeRgb => v.powf(ADOBE_GAMMA),
            Self::ProPhoto => {
                if v < 16.0 * PROPHOTO_LINEAR_LIMIT {
                    v / 16.0
                } else {
                    v.powf(1.8)
                }
            }
            Self::Rec2020 => {
                if v < 4.5 * BT2020_BETA {
                    v / 4.5
                } else {
                    ((v + (BT2020_ALPHA - 1.0)) / BT2020_ALPHA).powf(1.0 / 0.45)
                }
            }
        };
        linear.copysign(encoded)
    }

    /// Linear light to encoded value. Inverse of [`Self::decode`].
    #[inline]
    pub fn encode(self, linear: f32) -> f32 {
        let l = linear.abs();
        let encoded = match self {
            Self::Srgb | Self::DisplayP3 => {
                if l <= 0.003_130_8 {
                    l * 12.92
                } else {
                    1.055 * l.powf(1.0 / 2.4) - 0.055
                }
            }
            Self::AdobeRgb => l.powf(1.0 / ADOBE_GAMMA),
            Self::ProPhoto => {
                if l < PROPHOTO_LINEAR_LIMIT {
                    l * 16.0
                } else {
                    l.powf(1.0 / 1.8)
                }
            }
            Self::Rec2020 => {
                if l < BT2020_BETA {
                    l * 4.5
                } else {
                    BT2020_ALPHA * l.powf(0.45) - (BT2020_ALPHA - 1.0)
                }
            }
        };
        encoded.copysign(linear)
    }
}

const ADOBE_GAMMA: f32 = 563.0 / 256.0;
const PROPHOTO_LINEAR_LIMIT: f32 = 1.0 / 512.0;
const BT2020_ALPHA: f32 = 1.099_296_8;
const BT2020_BETA: f32 = 0.018_053_97;

/// Matrix taking linear RGB in `from` to linear RGB in `to`, including a
/// Bradford chromatic adaptation when the two white points differ. White maps
/// to white for every pair of spaces.
pub fn rgb_to_rgb(from: RgbSpace, to: RgbSpace) -> Mat3 {
    if from == to {
        return IDENTITY;
    }
    let (src, dst) = (from.primaries(), to.primaries());
    let to_xyz = rgb_to_xyz_d(src);
    let adapt = bradford_d(src.white, dst.white);
    let from_xyz = invert_d(&rgb_to_xyz_d(dst)).expect("primaries are not collinear");
    narrow(&mul_d(&from_xyz, &mul_d(&adapt, &to_xyz)))
}

/// Matrix taking D65-relative XYZ to linear working-space RGB.
pub fn xyz_d65_to_working() -> Mat3 {
    WORKING_SPACE.from_xyz()
}

/// Bradford chromatic adaptation matrix (XYZ to XYZ) between two white points.
pub fn bradford(from_white: Chromaticity, to_white: Chromaticity) -> Mat3 {
    narrow(&bradford_d(from_white, to_white))
}

fn rgb_to_xyz_d(p: Primaries) -> Mat3d {
    let (r, g, b) = (p.red.to_xyz(), p.green.to_xyz(), p.blue.to_xyz());
    let columns = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let scale = apply_d(
        &invert_d(&columns).expect("primaries are not collinear"),
        p.white.to_xyz(),
    );
    let mut m = columns;
    for row in &mut m {
        for (value, s) in row.iter_mut().zip(scale) {
            *value *= s;
        }
    }
    m
}

fn bradford_d(from_white: Chromaticity, to_white: Chromaticity) -> Mat3d {
    const CONE: Mat3d = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    if from_white == to_white {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let src = apply_d(&CONE, from_white.to_xyz());
    let dst = apply_d(&CONE, to_white.to_xyz());
    let mut scaled = CONE;
    for (row, (s, d)) in scaled.iter_mut().zip(src.iter().zip(dst)) {
        for value in row.iter_mut() {
            *value *= d / s;
        }
    }
    mul_d(&invert_d(&CONE).expect("Bradford matrix is invertible"), &scaled)
}

fn widen(m: &Mat3) -> Mat3d {
    m.map(|row| row.map(f64::from))
}

fn narrow(m: &Mat3d) -> Mat3 {
    m.map(|row| row.map(|v| v as f32))
}

fn apply_d(m: &Mat3d, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn mul_d(a: &Mat3d, b: &Mat3d) -> Mat3d {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn invert_d(m: &Mat3d) -> Option<Mat3d> {
    let cofactor = |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let adjugate = [
        [cofactor(1, 2, 1, 2), -cofactor(0, 2, 1, 2), cofactor(0, 1, 1, 2)],
        [-cofactor(1, 2, 0, 2), cofactor(0, 2, 0, 2), -cofactor(0, 1, 0, 2)],
        [cofactor(1, 2, 0, 1), -cofactor(0, 2, 0, 1), cofactor(0, 1, 0, 1)],
    ];
    let det = m[0][0] * adjugate[0][0] + m[0][1] * adjugate[1][0] + m[0][2] * adjugate[2][0];
    let scale = m.iter().flatten().fold(0.0f64, |acc, v| acc.max(v.abs()));
    if det.abs() <= scale.powi(3) * 1e-12 {
        return None;
    }
    Some(adjugate.map(|row| row.map(|v| v / det)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: [f32; 3], b: [f32; 3], tolerance: f32) {
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() <= tolerance, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn srgb_to_xyz_matches_the_published_matrix() {
        let m = RgbSpace::Srgb.to_xyz();
        assert_close(m[0], [0.4124, 0.3576, 0.1805], 2e-4);
        assert_close(m[1], [0.2126, 0.7152, 0.0722], 2e-4);
        assert_close(m[2], [0.0193, 0.1192, 0.9505], 2e-4);
    }

    #[test]
    fn srgb_to_rec2020_matches_bt2087() {
        let m = rgb_to_rgb(RgbSpace::Srgb, RgbSpace::Rec2020);
        assert_close(m[0], [0.6274, 0.3293, 0.0433], 2e-4);
        assert_close(m[1], [0.0691, 0.9195, 0.0114], 2e-4);
        assert_close(m[2], [0.0164, 0.0880, 0.8956], 2e-4);
    }

    #[test]
    fn working_luma_is_the_y_row_of_the_working_space() {
        assert_close(WORKING_SPACE.to_xyz()[1], WORKING_LUMA, 1e-6);
        assert!((WORKING_LUMA.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn white_maps_to_white_between_every_pair_of_spaces() {
        for from in RgbSpace::ALL {
            for to in RgbSpace::ALL {
                let white = mat3_apply(&rgb_to_rgb(from, to), [1.0, 1.0, 1.0]);
                assert_close(white, [1.0, 1.0, 1.0], 1e-5);
            }
        }
    }

    #[test]
    fn converting_there_and_back_is_identity() {
        let colour = [0.25, 0.5, 0.75];
        for from in RgbSpace::ALL {
            for to in RgbSpace::ALL {
                let there = mat3_apply(&rgb_to_rgb(from, to), colour);
                let back = mat3_apply(&rgb_to_rgb(to, from), there);
                assert_close(back, colour, 1e-5);
            }
        }
    }

    #[test]
    fn transfer_functions_round_trip_and_fix_the_endpoints() {
        for space in RgbSpace::ALL {
            assert_eq!(space.encode(0.0), 0.0);
            assert!((space.encode(1.0) - 1.0).abs() < 1e-6, "{space:?}");
            for step in 0..=1000 {
                let linear = step as f32 / 1000.0;
                let back = space.decode(space.encode(linear));
                assert!((back - linear).abs() < 2e-6, "{space:?} at {linear}");
            }
        }
    }

    #[test]
    fn transfer_functions_are_monotonic() {
        for space in RgbSpace::ALL {
            let mut previous = -1.0f32;
            for step in 0..=4096 {
                let encoded = space.encode(step as f32 / 4096.0);
                assert!(encoded > previous, "{space:?} at step {step}");
                previous = encoded;
            }
        }
    }

    #[test]
    fn transfer_functions_are_odd_symmetric() {
        for space in RgbSpace::ALL {
            assert_eq!(space.encode(-0.25), -space.encode(0.25));
            assert_eq!(space.decode(-0.25), -space.decode(0.25));
        }
    }

    #[test]
    fn srgb_encodes_middle_grey_to_the_familiar_value() {
        assert!((RgbSpace::Srgb.encode(0.18) - 0.4614).abs() < 1e-3);
    }

    #[test]
    fn invert_rejects_singular_matrices() {
        let singular = [[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 1.0, 0.0]];
        assert!(mat3_invert(&singular).is_none());
        let inverse = mat3_invert(&RgbSpace::Srgb.to_xyz()).unwrap();
        assert_close(mat3_apply(&inverse, [0.9505, 1.0, 1.0891]), [1.0, 1.0, 1.0], 1e-3);
    }

    #[test]
    fn bradford_moves_the_source_white_onto_the_destination_white() {
        let m = bradford(Chromaticity::D65, Chromaticity::D50);
        let d65 = Chromaticity::D65.to_xyz().map(|v| v as f32);
        let d50 = Chromaticity::D50.to_xyz().map(|v| v as f32);
        assert_close(mat3_apply(&m, d65), d50, 1e-5);
    }
}
