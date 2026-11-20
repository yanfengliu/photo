//! The colour description of a raw file: which colour matrix to use, what
//! the as-shot white balance is, and how a four-colour sensor is folded
//! into three channels.

use std::collections::HashMap;

use photo_core::colorimetry::{Chromaticity, Mat3, RgbSpace, mat3_apply, mat3_invert};
use rawler::imgop::xyz::Illuminant;

/// Correlated colour temperature the pipeline wants its matrix for.
const D65_KELVIN: f32 = 6500.0;

/// The Bradford cone response matrix (Lam 1985), XYZ to sharpened cone
/// signals. Scaling its three outputs independently is a Bradford chromatic
/// adaptation.
const BRADFORD_CONES: Mat3 = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

/// Nominal colour temperature of an EXIF light source, as the DNG SDK
/// assigns them (`dng_camera_profile::IlluminantToTemperature`).
fn kelvin(illuminant: Illuminant) -> Option<f32> {
    use Illuminant::*;
    Some(match illuminant {
        A | Tungsten => 2850.0,
        IsoStudioTungsten => 3200.0,
        D50 => 5000.0,
        D55 | Daylight | FineWeather | Flash | B => 5500.0,
        D65 | C | CloudyWeather => 6500.0,
        D75 | Shade => 7500.0,
        DaylightFluorescent => (5700.0 + 7100.0) / 2.0,
        DaylightWhiteFluorescent => (4600.0 + 5500.0) / 2.0,
        CoolWhiteFluorescent | Fluorescent => (3800.0 + 4500.0) / 2.0,
        WhiteFluorescent => (3250.0 + 3800.0) / 2.0,
        Unknown => return None,
    })
}

/// The colour matrix calibrated closest to D65 among those with `rows` rows
/// of three, as a flat row-major slice.
///
/// A file calibrated for D65 gets exactly that matrix. Otherwise the DNG
/// rule for a white balance outside the calibrated range applies: use the
/// nearest calibration as it is, where "nearest" is measured in reciprocal
/// colour temperature (DNG 1.4 specification, chapter 6). Matrices are never
/// adapted or blended. Among equally near calibrations D65 itself wins, then
/// the lower illuminant code, so the choice never depends on map order.
pub(crate) fn nearest_d65_matrix(
    matrices: &HashMap<Illuminant, Vec<f32>>,
    rows: usize,
) -> Option<&[f32]> {
    let mired = |kelvin: f32| 1.0e6 / kelvin;
    let rank = |illuminant: Illuminant| {
        let distance = match kelvin(illuminant) {
            Some(kelvin) => (mired(kelvin) - mired(D65_KELVIN)).abs(),
            None => f32::INFINITY,
        };
        (distance, illuminant != Illuminant::D65, illuminant as u16)
    };
    matrices
        .iter()
        .filter(|(_, matrix)| matrix.len() == rows * 3)
        .min_by(|(a, _), (b, _)| {
            let (a, b) = (rank(**a), rank(**b));
            a.0.total_cmp(&b.0).then((a.1, a.2).cmp(&(b.1, b.2)))
        })
        .map(|(_, matrix)| matrix.as_slice())
}

/// The XYZ (D65) to camera matrix for a three-colour sensor.
///
/// Falls back to treating the camera as if its native colours were linear
/// sRGB when the file offers no usable matrix (none at all, non-finite
/// entries, or one that cannot be inverted): the colours will be off, but
/// the picture can be shown and adjusted.
pub(crate) fn camera_matrix(matrices: &HashMap<Illuminant, Vec<f32>>) -> Mat3 {
    let found = nearest_d65_matrix(matrices, 3).and_then(|flat| {
        let matrix: Mat3 =
            std::array::from_fn(|row| std::array::from_fn(|column| flat[row * 3 + column]));
        let finite = flat.iter().all(|value| value.is_finite());
        (finite && mat3_invert(&matrix).is_some()).then_some(matrix)
    });
    found.unwrap_or_else(|| {
        log::warn!("raw file has no usable colour matrix; assuming sRGB primaries");
        RgbSpace::Srgb.from_xyz()
    })
}

/// White-balance multipliers with green at 1.0, from the coefficients a raw
/// file reports (in whatever scale it likes).
///
/// When the red, green or blue coefficient is missing, zero, negative or not
/// a number, the camera's daylight balance is used instead: the multipliers
/// that make D65 neutral according to `xyz_to_camera`. If even that cannot
/// be computed the balance is left alone at (1, 1, 1).
pub(crate) fn as_shot_gains(coefficients: [f32; 3], xyz_to_camera: &Mat3) -> [f32; 3] {
    normalized_gains(coefficients).unwrap_or_else(|| daylight_gains(xyz_to_camera))
}

/// The multipliers that make D65 white neutral for a camera with this
/// matrix, green at 1.0; (1, 1, 1) when the matrix gives no sane answer.
fn daylight_gains(xyz_to_camera: &Mat3) -> [f32; 3] {
    let d65 = Chromaticity::D65.to_xyz().map(|value| value as f32);
    let neutral = mat3_apply(xyz_to_camera, d65);
    normalized_gains(neutral.map(|response| 1.0 / response)).unwrap_or([1.0; 3])
}

/// Scales multipliers so green is 1.0. `None` unless all three are finite,
/// positive and within a factor of 1000 of green.
fn normalized_gains([red, green, blue]: [f32; 3]) -> Option<[f32; 3]> {
    let gains = [red / green, 1.0, blue / green];
    let sane = |gain: f32| gain.is_finite() && (1.0e-3..=1.0e3).contains(&gain);
    (green.is_finite() && green > 0.0 && gains.into_iter().all(sane)).then_some(gains)
}

/// How a four-colour sensor (CYGM, RGBE) is presented as a three-channel
/// camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FourColorFold {
    /// Four sensor channels to three virtual camera channels.
    pub(crate) fold: [[f32; 4]; 3],
    /// XYZ (D65) to the virtual camera's channels.
    pub(crate) xyz_to_camera: Mat3,
    /// As-shot multipliers of the virtual camera, green at 1.0.
    pub(crate) as_shot_gains: [f32; 3],
}

/// Folds a four-colour sensor into a virtual three-channel camera.
///
/// `photo_core`'s camera description has a 3 x 3 matrix and three gains, so
/// four channels have to become three before they leave this crate. The
/// least-squares solution takes the four signals back to XYZ (the
/// pseudo-inverse of the 4 x 3 colour matrix, as dcraw does for these
/// sensors), and XYZ is then expressed in Bradford cone signals. A renderer
/// that white-balances by scaling those three channels is performing a
/// Bradford adaptation, which is a sound thing for it to be doing.
///
/// The fold is scaled so that a saturated sensor reads 1.0 in its brightest
/// virtual channel. Returns `None` when the matrix cannot be inverted or the
/// result is not usable.
pub(crate) fn fold_four_colors(
    xyz_to_sensor: &[[f32; 3]; 4],
    coefficients: [f32; 4],
) -> Option<FourColorFold> {
    // Normal equations: (At A) x = At s.
    let mut normal = [[0.0f32; 3]; 3];
    for (i, row) in normal.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = xyz_to_sensor
                .iter()
                .map(|sensor| sensor[i] * sensor[j])
                .sum();
        }
    }
    let inverse = mat3_invert(&normal)?;
    let sensor_to_cones = |channel: usize| -> [f32; 3] {
        // Column `channel` of B (At A)^-1 At.
        let at_column: [f32; 3] = std::array::from_fn(|i| xyz_to_sensor[channel][i]);
        mat3_apply(&BRADFORD_CONES, mat3_apply(&inverse, at_column))
    };
    let columns: [[f32; 3]; 4] = std::array::from_fn(sensor_to_cones);
    let mut fold: [[f32; 4]; 3] =
        std::array::from_fn(|row| std::array::from_fn(|channel| columns[channel][row]));

    let apply = |fold: &[[f32; 4]; 3], sensor: [f32; 4]| -> [f32; 3] {
        std::array::from_fn(|row| {
            (0..4)
                .map(|channel| fold[row][channel] * sensor[channel])
                .sum()
        })
    };
    let saturated = apply(&fold, [1.0; 4]);
    let peak = saturated.into_iter().fold(f32::NEG_INFINITY, f32::max);
    if !(peak.is_finite() && peak > 0.0) {
        return None;
    }
    for value in fold.iter_mut().flatten() {
        *value /= peak;
    }
    let xyz_to_camera = BRADFORD_CONES.map(|row| row.map(|value| value / peak));
    if fold.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }

    // The sensor's response to a neutral object is the reciprocal of its
    // white-balance coefficients.
    let neutral = apply(&fold, coefficients.map(|coefficient| 1.0 / coefficient));
    let from_file = coefficients
        .iter()
        .all(|coefficient| coefficient.is_finite() && *coefficient > 0.0)
        .then(|| normalized_gains(neutral.map(|response| 1.0 / response)))
        .flatten();
    Some(FourColorFold {
        fold,
        xyz_to_camera,
        as_shot_gains: from_file.unwrap_or_else(|| daylight_gains(&xyz_to_camera)),
    })
}

/// The virtual camera a monochrome sensor is presented as: linear sRGB
/// primaries and a neutral balance, so equal channels render as grey.
pub(crate) fn monochrome_matrix() -> Mat3 {
    RgbSpace::Srgb.from_xyz()
}

#[cfg(test)]
mod tests;
