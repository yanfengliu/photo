//! A tiny ICC profile reader: just enough to recognise which of the colour
//! spaces we know a file is tagged with.
//!
//! Only the `rXYZ`, `gXYZ` and `bXYZ` colorant tags of matrix profiles are
//! read (ICC.1:2010, 9.2.44/27/14). They hold the primaries adapted to the
//! D50 profile connection space, so they are compared with our own
//! primaries adapted the same way.

use photo_core::colorimetry::{Chromaticity, RgbSpace, bradford, mat3_mul};

const HEADER_LEN: usize = 128;
const TAG_ENTRY_LEN: usize = 12;
/// A profile claiming more tags than this is not one worth parsing.
const MAX_TAGS: usize = 256;
/// Largest difference, in XYZ units, between a profile's colorants and a
/// space's for the two to be taken as the same. Published profiles of one
/// space agree to about 0.002; the spaces we know differ by 0.06 or more.
const TOLERANCE: f32 = 0.01;

/// The colour space whose primaries match the profile's, if it is a matrix
/// RGB profile for one of [`RgbSpace::ALL`]. Anything else, including bytes
/// that are not a profile at all, is `None`.
pub(crate) fn detect_space(profile: &[u8]) -> Option<RgbSpace> {
    let colorants = read_colorants(profile)?;
    RgbSpace::ALL
        .into_iter()
        .find(|&space| matches(&colorants, space))
}

/// The red, green and blue colorants as XYZ triples.
fn read_colorants(profile: &[u8]) -> Option<[[f32; 3]; 3]> {
    if profile.get(36..40)? != b"acsp" || profile.get(16..20)? != b"RGB " {
        return None;
    }
    let tag_count = usize::try_from(be_u32(profile, HEADER_LEN)?).ok()?;
    if tag_count > MAX_TAGS {
        return None;
    }
    let find = |signature: &[u8; 4]| {
        (0..tag_count)
            .map(|index| HEADER_LEN + 4 + index * TAG_ENTRY_LEN)
            .find(|&entry| profile.get(entry..entry + 4) == Some(signature.as_slice()))
            .and_then(|entry| read_xyz(profile, usize::try_from(be_u32(profile, entry + 4)?).ok()?))
    };
    Some([find(b"rXYZ")?, find(b"gXYZ")?, find(b"bXYZ")?])
}

/// An `XYZType` element: signature, four reserved bytes, three
/// `s15Fixed16Number`s.
fn read_xyz(profile: &[u8], offset: usize) -> Option<[f32; 3]> {
    if profile.get(offset..offset.checked_add(4)?)? != b"XYZ " {
        return None;
    }
    let mut xyz = [0.0; 3];
    for (index, value) in xyz.iter_mut().enumerate() {
        let fixed = be_u32(profile, offset.checked_add(8 + index * 4)?)? as i32;
        *value = fixed as f32 / 65536.0;
    }
    Some(xyz)
}

fn be_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_be_bytes(slice.try_into().ok()?))
}

fn matches(colorants: &[[f32; 3]; 3], space: RgbSpace) -> bool {
    // Columns of this matrix are the space's primaries as a D50 profile
    // stores them.
    let to_d50 = bradford(space.primaries().white, Chromaticity::D50);
    let expected = mat3_mul(&to_d50, &space.to_xyz());
    colorants.iter().enumerate().all(|(column, colorant)| {
        colorant
            .iter()
            .enumerate()
            .all(|(row, value)| (value - expected[row][column]).abs() <= TOLERANCE)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest thing the reader accepts: a header and three colorants.
    fn profile(colorants: [[f32; 3]; 3]) -> Vec<u8> {
        let mut bytes = vec![0u8; HEADER_LEN];
        bytes[16..20].copy_from_slice(b"RGB ");
        bytes[36..40].copy_from_slice(b"acsp");
        bytes.extend_from_slice(&3u32.to_be_bytes());
        let data_start = HEADER_LEN + 4 + 3 * TAG_ENTRY_LEN;
        for (index, signature) in [b"rXYZ", b"gXYZ", b"bXYZ"].into_iter().enumerate() {
            bytes.extend_from_slice(signature);
            bytes.extend_from_slice(&((data_start + index * 20) as u32).to_be_bytes());
            bytes.extend_from_slice(&20u32.to_be_bytes());
        }
        for colorant in colorants {
            bytes.extend_from_slice(b"XYZ \0\0\0\0");
            for value in colorant {
                bytes.extend_from_slice(&((value * 65536.0).round() as i32).to_be_bytes());
            }
        }
        bytes
    }

    /// Colorants as published in the profiles people actually embed: sRGB
    /// IEC61966-2.1, Adobe RGB (1998), Apple's Display P3 and ProPhoto (ROMM).
    const PUBLISHED: [(RgbSpace, [[f32; 3]; 3]); 4] = [
        (
            RgbSpace::Srgb,
            [
                [0.43607, 0.22249, 0.01392],
                [0.38515, 0.71687, 0.09708],
                [0.14307, 0.06061, 0.71410],
            ],
        ),
        (
            RgbSpace::AdobeRgb,
            [
                [0.60974, 0.31111, 0.01947],
                [0.20528, 0.62567, 0.06087],
                [0.14919, 0.06322, 0.74457],
            ],
        ),
        (
            RgbSpace::DisplayP3,
            [
                [0.51512, 0.24120, -0.00105],
                [0.29198, 0.69225, 0.04189],
                [0.15710, 0.06657, 0.78407],
            ],
        ),
        (
            RgbSpace::ProPhoto,
            [
                [0.79767, 0.28804, 0.00000],
                [0.13519, 0.71188, 0.00000],
                [0.03134, 0.00009, 0.82491],
            ],
        ),
    ];

    #[test]
    fn published_profiles_are_recognised() {
        for (space, colorants) in PUBLISHED {
            assert_eq!(detect_space(&profile(colorants)), Some(space), "{space:?}");
        }
    }

    #[test]
    fn our_own_primaries_are_recognised_for_every_space() {
        for space in RgbSpace::ALL {
            let to_d50 = bradford(space.primaries().white, Chromaticity::D50);
            let matrix = mat3_mul(&to_d50, &space.to_xyz());
            let colorants = std::array::from_fn(|c| [matrix[0][c], matrix[1][c], matrix[2][c]]);
            assert_eq!(detect_space(&profile(colorants)), Some(space), "{space:?}");
        }
    }

    #[test]
    fn no_two_spaces_are_close_enough_to_be_confused() {
        for (space, colorants) in PUBLISHED {
            let hits = RgbSpace::ALL
                .into_iter()
                .filter(|&s| matches(&colorants, s));
            assert_eq!(hits.collect::<Vec<_>>(), [space]);
        }
    }

    #[test]
    fn unknown_primaries_match_nothing() {
        // The Adobe RGB colorants with red and blue exchanged.
        let [r, g, b] = PUBLISHED[1].1;
        assert_eq!(detect_space(&profile([b, g, r])), None);
        // A profile nudged just past the tolerance.
        let mut nudged = PUBLISHED[0].1;
        nudged[0][0] += 0.02;
        assert_eq!(detect_space(&profile(nudged)), None);
    }

    #[test]
    fn things_that_are_not_rgb_matrix_profiles_are_ignored() {
        let good = profile(PUBLISHED[0].1);
        let mut grey = good.clone();
        grey[16..20].copy_from_slice(b"GRAY");
        assert_eq!(detect_space(&grey), None);
        let mut no_magic = good.clone();
        no_magic[36..40].copy_from_slice(b"nope");
        assert_eq!(detect_space(&no_magic), None);
        // A lookup-table profile has no colorant tags.
        let mut lut = good.clone();
        lut[HEADER_LEN + 4..HEADER_LEN + 8].copy_from_slice(b"A2B0");
        assert_eq!(detect_space(&lut), None);
        // The element is not an XYZ type.
        let mut wrong_type = good.clone();
        let element = HEADER_LEN + 4 + 3 * TAG_ENTRY_LEN;
        wrong_type[element..element + 4].copy_from_slice(b"curv");
        assert_eq!(detect_space(&wrong_type), None);
    }

    #[test]
    fn truncated_and_hostile_profiles_are_ignored_without_panicking() {
        let good = profile(PUBLISHED[2].1);
        for length in 0..good.len() {
            assert_eq!(detect_space(&good[..length]), None, "length {length}");
        }
        let mut huge_count = good.clone();
        huge_count[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(detect_space(&huge_count), None);
        let mut wild_offset = good.clone();
        let offset_field = HEADER_LEN + 4 + 4;
        wild_offset[offset_field..offset_field + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(detect_space(&wild_offset), None);
        assert_eq!(detect_space(&[0xff; 4096]), None);
    }
}
