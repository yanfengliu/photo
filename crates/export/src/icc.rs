//! ICC profiles for the export colour spaces, built from photo-core's own
//! primaries and transfer functions so the profile always describes exactly
//! the numbers the pipeline wrote.
//!
//! Each is a minimal ICC.1:2010 (version 4.3) RGB display profile of the
//! matrix/TRC kind:
//!
//! - `desc` and `cprt`: `multiLocalizedUnicodeType` text.
//! - `wtpt`: the PCS illuminant D50, as version 4 requires of display
//!   profiles (§9.2.36).
//! - `chad`: the Bradford adaptation from the space's own white to D50, so
//!   that white (D65 for most spaces) stays recoverable (§9.2.15 requires
//!   the tag whenever the two differ).
//! - `rXYZ`, `gXYZ`, `bXYZ`: the primaries adapted to D50 with that same
//!   matrix, which is how every matrix profile stores them.
//! - `rTRC`, `gTRC`, `bTRC`: one shared `parametricCurveType`: a pure power
//!   (function 0) for Adobe RGB, and the power with a linear toe
//!   (function 3) for the others.

use photo_core::colorimetry::{Chromaticity, Mat3, RgbSpace, bradford, mat3_mul};

/// The PCS illuminant of ICC.1 (§7.2.16), as the header stores it.
const PCS_D50: [f64; 3] = [0.9642, 1.0, 0.8249];

const HEADER_LEN: usize = 128;

/// The profile for `space`.
pub(crate) fn profile(space: RgbSpace) -> Vec<u8> {
    let d50 = Chromaticity::new(
        PCS_D50[0] / PCS_D50.iter().sum::<f64>(),
        PCS_D50[1] / PCS_D50.iter().sum::<f64>(),
    );
    let adapt = bradford(space.primaries().white, d50);
    let colorants = mat3_mul(&adapt, &space.to_xyz());
    let column = |c: usize| xyz([colorants[0][c], colorants[1][c], colorants[2][c]].map(f64::from));
    let curve = curve(space);
    let tags: [(&[u8; 4], Vec<u8>); 10] = [
        (b"desc", text(description(space))),
        (b"cprt", text("No copyright, use freely")),
        (b"wtpt", xyz(PCS_D50)),
        (b"chad", matrix(&adapt)),
        (b"rXYZ", column(0)),
        (b"gXYZ", column(1)),
        (b"bXYZ", column(2)),
        (b"rTRC", curve.clone()),
        (b"gTRC", curve.clone()),
        (b"bTRC", curve),
    ];

    // The three curves are identical, so they share one element.
    let table_len = 4 + tags.len() * 12;
    let mut table = (tags.len() as u32).to_be_bytes().to_vec();
    let mut data: Vec<u8> = Vec::new();
    let mut stored: Vec<(&[u8], usize)> = Vec::new();
    for (signature, element) in &tags {
        let offset = match stored
            .iter()
            .find(|(bytes, _)| *bytes == element.as_slice())
        {
            Some(&(_, offset)) => offset,
            None => {
                let offset = HEADER_LEN + table_len + data.len();
                data.extend_from_slice(element);
                data.resize(data.len().next_multiple_of(4), 0);
                stored.push((element.as_slice(), offset));
                offset
            }
        };
        table.extend_from_slice(*signature);
        table.extend_from_slice(&(offset as u32).to_be_bytes());
        table.extend_from_slice(&(element.len() as u32).to_be_bytes());
    }

    let size = HEADER_LEN + table.len() + data.len();
    let mut out = header(size as u32);
    out.extend_from_slice(&table);
    out.extend_from_slice(&data);
    out
}

/// The text a profile gives as its name.
fn description(space: RgbSpace) -> &'static str {
    match space {
        RgbSpace::AdobeRgb => "Compatible with Adobe RGB (1998)",
        other => other.name(),
    }
}

/// The 128-byte header of a profile `size` bytes long.
fn header(size: u32) -> Vec<u8> {
    let mut header = vec![0u8; HEADER_LEN];
    header[0..4].copy_from_slice(&size.to_be_bytes());
    // Version 4.3.0.0.
    header[8..12].copy_from_slice(&[4, 0x30, 0, 0]);
    header[12..16].copy_from_slice(b"mntr");
    header[16..20].copy_from_slice(b"RGB ");
    header[20..24].copy_from_slice(b"XYZ ");
    // Creation date and time, fixed so that a profile's bytes depend only
    // on its colour space: 2026-11-19 00:00:00.
    for (slot, value) in [2026u16, 11, 19, 0, 0, 0].into_iter().enumerate() {
        header[24 + slot * 2..26 + slot * 2].copy_from_slice(&value.to_be_bytes());
    }
    header[36..40].copy_from_slice(b"acsp");
    for (slot, value) in PCS_D50.into_iter().enumerate() {
        header[68 + slot * 4..72 + slot * 4].copy_from_slice(&fixed(value));
    }
    // Everything else (CMM, platform, flags, device, intent, creator and
    // the optional profile ID) is zero: perceptual intent, not computed.
    header
}

/// `s15Fixed16Number`, big-endian.
fn fixed(value: f64) -> [u8; 4] {
    ((value * 65536.0).round() as i32).to_be_bytes()
}

/// `XYZType` holding one colour.
fn xyz(value: [f64; 3]) -> Vec<u8> {
    let mut element = b"XYZ \0\0\0\0".to_vec();
    for component in value {
        element.extend_from_slice(&fixed(component));
    }
    element
}

/// `s15Fixed16ArrayType` holding a 3 x 3 matrix in row order, as `chad`
/// does.
fn matrix(m: &Mat3) -> Vec<u8> {
    let mut element = b"sf32\0\0\0\0".to_vec();
    for value in m.iter().flatten() {
        element.extend_from_slice(&fixed(f64::from(*value)));
    }
    element
}

/// `multiLocalizedUnicodeType` with one US English record.
fn text(text: &str) -> Vec<u8> {
    let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut element = b"mluc\0\0\0\0".to_vec();
    element.extend_from_slice(&1u32.to_be_bytes());
    element.extend_from_slice(&12u32.to_be_bytes());
    element.extend_from_slice(b"enUS");
    element.extend_from_slice(&(utf16.len() as u32).to_be_bytes());
    element.extend_from_slice(&28u32.to_be_bytes());
    element.extend_from_slice(&utf16);
    element
}

/// The space's decoding function (encoded value to linear light) as a
/// `parametricCurveType`. Function 3 is `Y = (aX + b)^g` for `X >= d` and
/// `Y = cX` below; function 0 is `Y = X^g`.
fn curve(space: RgbSpace) -> Vec<u8> {
    // ITU-R BT.2020 constants, as photo-core uses them.
    const ALPHA: f64 = 1.099_296_826_809_44;
    const BETA: f64 = 0.018_053_968_510_807;
    let (function, parameters): (u16, Vec<f64>) = match space {
        RgbSpace::Srgb | RgbSpace::DisplayP3 => (
            3,
            vec![2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.040_45],
        ),
        RgbSpace::AdobeRgb => (0, vec![563.0 / 256.0]),
        RgbSpace::ProPhoto => (3, vec![1.8, 1.0, 0.0, 1.0 / 16.0, 1.0 / 32.0]),
        RgbSpace::Rec2020 => (
            3,
            vec![
                1.0 / 0.45,
                1.0 / ALPHA,
                (ALPHA - 1.0) / ALPHA,
                1.0 / 4.5,
                4.5 * BETA,
            ],
        ),
    };
    let mut element = b"para\0\0\0\0".to_vec();
    element.extend_from_slice(&function.to_be_bytes());
    element.extend_from_slice(&[0, 0]);
    for parameter in parameters {
        element.extend_from_slice(&fixed(parameter));
    }
    element
}
