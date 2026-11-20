//! Correlated colour temperature and tint to and from CIE 1931 xy.
//!
//! This is the model of the DNG SDK's `dng_temperature` (`dng_temperature.cpp`):
//! Robertson's method (A. R. Robertson, "Computation of Correlated Color
//! Temperature and Distribution Temperature", JOSA 58, 1968) over the 31
//! isotherms tabulated in Wyszecki & Stiles, "Color Science", 2nd edition,
//! p. 228, with tint defined as `-3000 x` the CIE 1960 uv distance from the
//! Planckian locus along the isotherm. Positive tint therefore describes a
//! greener light source, whose correction is magenta.
//!
//! Copies of this table in circulation disagree on one entry: the 325-mired
//! row appears with `u = 0.24702` and with `u = 0.24792`. This file carries
//! `0.24792`, the value that is consistent with its neighbours and with the
//! Planckian locus; the other is a transposed digit. Rather than trust any
//! one copy, `tests/temperature.rs` pins every row against Krystek's and Kim
//! et al.'s independent approximations of the locus and checks the table
//! for smoothness.

use photo_core::colorimetry::Chromaticity;

/// Tint units per unit of CIE 1960 uv distance (DNG SDK `kTintScale`).
pub const TINT_SCALE: f64 = -3000.0;

/// One isotherm: a point on the Planckian locus and the slope of the line of
/// constant correlated colour temperature through it, in CIE 1960 uv.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Isotherm {
    /// Reciprocal temperature in mired (`1e6 / kelvin`).
    pub mired: f64,
    /// CIE 1960 u of the Planckian locus at this temperature.
    pub u: f64,
    /// CIE 1960 v of the Planckian locus at this temperature.
    pub v: f64,
    /// dv/du along the isotherm.
    pub slope: f64,
}

const fn row(mired: f64, u: f64, v: f64, slope: f64) -> Isotherm {
    Isotherm { mired, u, v, slope }
}

/// The isotherm table, ordered by increasing mired (decreasing temperature),
/// from infinite temperature down to 1667 K.
pub const ISOTHERMS: [Isotherm; 31] = [
    row(0.0, 0.18006, 0.26352, -0.24341),
    row(10.0, 0.18066, 0.26589, -0.25479),
    row(20.0, 0.18133, 0.26846, -0.26876),
    row(30.0, 0.18208, 0.27119, -0.28539),
    row(40.0, 0.18293, 0.27407, -0.30470),
    row(50.0, 0.18388, 0.27709, -0.32675),
    row(60.0, 0.18494, 0.28021, -0.35156),
    row(70.0, 0.18611, 0.28342, -0.37915),
    row(80.0, 0.18740, 0.28668, -0.40955),
    row(90.0, 0.18880, 0.28997, -0.44278),
    row(100.0, 0.19032, 0.29326, -0.47888),
    row(125.0, 0.19462, 0.30141, -0.58204),
    row(150.0, 0.19962, 0.30921, -0.70471),
    row(175.0, 0.20525, 0.31647, -0.84901),
    row(200.0, 0.21142, 0.32312, -1.0182),
    row(225.0, 0.21807, 0.32909, -1.2168),
    row(250.0, 0.22511, 0.33439, -1.4512),
    row(275.0, 0.23247, 0.33904, -1.7298),
    row(300.0, 0.24010, 0.34308, -2.0637),
    row(325.0, 0.24792, 0.34655, -2.4681),
    row(350.0, 0.25591, 0.34951, -2.9641),
    row(375.0, 0.26400, 0.35200, -3.5814),
    row(400.0, 0.27218, 0.35407, -4.3633),
    row(425.0, 0.28039, 0.35577, -5.3762),
    row(450.0, 0.28863, 0.35714, -6.7262),
    row(475.0, 0.29685, 0.35823, -8.5955),
    row(500.0, 0.30505, 0.35907, -11.324),
    row(525.0, 0.31320, 0.35968, -15.628),
    row(550.0, 0.32129, 0.36011, -23.325),
    row(575.0, 0.32931, 0.36038, -40.770),
    row(600.0, 0.33724, 0.36051, -116.45),
];

const LAST: usize = ISOTHERMS.len() - 1;

/// Unit vector along an isotherm.
fn direction(isotherm: &Isotherm) -> (f64, f64) {
    let length = (1.0 + isotherm.slope * isotherm.slope).sqrt();
    (1.0 / length, isotherm.slope / length)
}

/// The chromaticity of a light source of the given correlated colour
/// temperature (kelvin) and tint (DNG `Get_xy_coord`).
///
/// The temperature is clamped to the table's range (1667 K and up) and the
/// tint to ±500, so the result is finite for every input, NaN included.
pub fn temp_tint_to_xy(kelvin: f64, tint: f64) -> Chromaticity {
    let kelvin = if kelvin.is_nan() { 6500.0 } else { kelvin };
    let tint = if tint.is_nan() {
        0.0
    } else {
        tint.clamp(-500.0, 500.0)
    };
    let mired = if kelvin > 0.0 {
        (1.0e6 / kelvin).clamp(0.0, ISOTHERMS[LAST].mired)
    } else {
        ISOTHERMS[LAST].mired
    };
    let offset = tint / TINT_SCALE;

    let index = (0..LAST)
        .find(|index| mired < ISOTHERMS[index + 1].mired)
        .unwrap_or(LAST - 1);
    let (lower, upper) = (&ISOTHERMS[index], &ISOTHERMS[index + 1]);
    // Weight of the lower-mired line.
    let f = (upper.mired - mired) / (upper.mired - lower.mired);

    let mut u = lower.u * f + upper.u * (1.0 - f);
    let mut v = lower.v * f + upper.v * (1.0 - f);
    let (du1, dv1) = direction(lower);
    let (du2, dv2) = direction(upper);
    let (du, dv) = (du1 * f + du2 * (1.0 - f), dv1 * f + dv2 * (1.0 - f));
    let length = (du * du + dv * dv).sqrt();
    u += du / length * offset;
    v += dv / length * offset;

    let denominator = u - 4.0 * v + 2.0;
    Chromaticity::new(1.5 * u / denominator, v / denominator)
}

/// The correlated colour temperature (kelvin) and tint of a chromaticity
/// (DNG `Set_xy_coord`). Inverse of [`temp_tint_to_xy`] to within the
/// interpolation error of Robertson's method (well under one kelvin per
/// thousand and a fraction of a tint unit over the slider ranges).
///
/// Chromaticities hotter than the table's first row or cooler than its last
/// are read against the nearest pair of isotherms; the result is always
/// finite, with the temperature capped at 1e6 K.
pub fn xy_to_temp_tint(xy: Chromaticity) -> (f64, f64) {
    let denominator = 1.5 - xy.x + 6.0 * xy.y;
    let u = 2.0 * xy.x / denominator;
    let v = 3.0 * xy.y / denominator;

    let (mut last_dt, mut last_du, mut last_dv) = (0.0, 0.0, 0.0);
    for index in 1..=LAST {
        let isotherm = &ISOTHERMS[index];
        let (mut du, mut dv) = direction(isotherm);
        // Signed distance of the point above this isotherm.
        let dt = -(u - isotherm.u) * dv + (v - isotherm.v) * du;
        if dt <= 0.0 || index == LAST {
            let below = -dt.min(0.0);
            let f = if index == 1 || last_dt + below <= 0.0 {
                0.0
            } else {
                below / (last_dt + below)
            };
            let previous = &ISOTHERMS[index - 1];
            let mired = previous.mired * f + isotherm.mired * (1.0 - f);
            let kelvin = 1.0e6 / mired.max(1.0);

            let uu = u - (previous.u * f + isotherm.u * (1.0 - f));
            let vv = v - (previous.v * f + isotherm.v * (1.0 - f));
            du = du * (1.0 - f) + last_du * f;
            dv = dv * (1.0 - f) + last_dv * f;
            let length = (du * du + dv * dv).sqrt();
            let tint = (uu * du + vv * dv) / length * TINT_SCALE;
            return if kelvin.is_finite() && tint.is_finite() {
                (kelvin, tint)
            } else {
                (6500.0, 0.0)
            };
        }
        (last_dt, last_du, last_dv) = (dt, du, dv);
    }
    unreachable!("the loop returns at the last isotherm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_ordered_and_spans_infinity_to_1667_kelvin() {
        assert_eq!(ISOTHERMS[0].mired, 0.0);
        assert_eq!(ISOTHERMS[LAST].mired, 600.0);
        for pair in ISOTHERMS.windows(2) {
            assert!(pair[1].mired > pair[0].mired);
            assert!(pair[1].u > pair[0].u && pair[1].v > pair[0].v);
            assert!(pair[1].slope < pair[0].slope);
        }
    }

    #[test]
    fn a_table_temperature_reproduces_its_row() {
        for isotherm in &ISOTHERMS[1..] {
            let xy = temp_tint_to_xy(1.0e6 / isotherm.mired, 0.0);
            let denominator = 1.5 - xy.x + 6.0 * xy.y;
            assert!((2.0 * xy.x / denominator - isotherm.u).abs() < 1e-12);
            assert!((3.0 * xy.y / denominator - isotherm.v).abs() < 1e-12);
        }
    }
}
