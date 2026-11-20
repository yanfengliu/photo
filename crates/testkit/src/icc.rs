//! A minimal ICC display profile for each colour space the app knows, so
//! fixtures can be tagged the way real files are.

use photo_core::colorimetry::{Chromaticity, RgbSpace, bradford, mat3_mul};

/// Builds a small but structurally valid ICC v2 matrix/TRC display profile
/// for `space`.
///
/// The `rXYZ`, `gXYZ` and `bXYZ` colorants are the space's primaries adapted
/// to the D50 profile connection space with Bradford, which is how real
/// profiles store them. The tone curves are a plain gamma close to the
/// space's own, so do not rely on them for exact transfer functions.
pub fn icc_profile(space: RgbSpace) -> Vec<u8> {
    let to_d50 = mat3_mul(
        &bradford(space.primaries().white, Chromaticity::D50),
        &space.to_xyz(),
    );
    let colorant =
        |column: usize| xyz_element([to_d50[0][column], to_d50[1][column], to_d50[2][column]]);
    let d50 = Chromaticity::D50.to_xyz().map(|v| v as f32);

    let gamma: f32 = match space {
        RgbSpace::ProPhoto => 1.8,
        RgbSpace::Srgb | RgbSpace::DisplayP3 | RgbSpace::AdobeRgb | RgbSpace::Rec2020 => {
            563.0 / 256.0
        }
    };
    let curve = curve_element(gamma);
    let elements: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"desc", description_element(space.name())),
        (b"cprt", text_element("No copyright, use freely")),
        (b"wtpt", xyz_element(d50)),
        (b"rXYZ", colorant(0)),
        (b"gXYZ", colorant(1)),
        (b"bXYZ", colorant(2)),
        (b"rTRC", curve.clone()),
        (b"gTRC", curve.clone()),
        (b"bTRC", curve),
    ];

    const HEADER_LEN: usize = 128;
    let table_len = 4 + elements.len() * 12;
    let mut table = Vec::with_capacity(table_len);
    let mut data = Vec::new();
    table.extend_from_slice(&(elements.len() as u32).to_be_bytes());
    for (signature, element) in &elements {
        let offset = HEADER_LEN + table_len + data.len();
        table.extend_from_slice(*signature);
        table.extend_from_slice(&(offset as u32).to_be_bytes());
        table.extend_from_slice(&(element.len() as u32).to_be_bytes());
        data.extend_from_slice(element);
        // Every element starts on a four-byte boundary.
        data.resize(data.len().next_multiple_of(4), 0);
    }

    let mut profile = vec![0u8; HEADER_LEN];
    let size = (HEADER_LEN + table.len() + data.len()) as u32;
    profile[0..4].copy_from_slice(&size.to_be_bytes());
    profile[8..12].copy_from_slice(&[2, 0x10, 0, 0]);
    profile[12..16].copy_from_slice(b"mntr");
    profile[16..20].copy_from_slice(b"RGB ");
    profile[20..24].copy_from_slice(b"XYZ ");
    // Creation date: 2024-01-01 00:00:00, as six big-endian u16.
    profile[24..36].copy_from_slice(&[0x07, 0xe8, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
    profile[36..40].copy_from_slice(b"acsp");
    for (slot, value) in d50.into_iter().enumerate() {
        profile[68 + slot * 4..72 + slot * 4].copy_from_slice(&fixed(value));
    }
    profile.extend_from_slice(&table);
    profile.extend_from_slice(&data);
    profile
}

/// `s15Fixed16Number`, big-endian.
fn fixed(value: f32) -> [u8; 4] {
    ((f64::from(value) * 65536.0).round() as i32).to_be_bytes()
}

fn xyz_element(xyz: [f32; 3]) -> Vec<u8> {
    let mut element = b"XYZ \0\0\0\0".to_vec();
    for value in xyz {
        element.extend_from_slice(&fixed(value));
    }
    element
}

/// `curveType` holding a single gamma value as `u8Fixed8Number`.
fn curve_element(gamma: f32) -> Vec<u8> {
    let mut element = b"curv\0\0\0\0".to_vec();
    element.extend_from_slice(&1u32.to_be_bytes());
    element.extend_from_slice(&((gamma * 256.0).round() as u16).to_be_bytes());
    element
}

fn text_element(text: &str) -> Vec<u8> {
    let mut element = b"text\0\0\0\0".to_vec();
    element.extend_from_slice(text.as_bytes());
    element.push(0);
    element
}

/// ICC v2 `textDescriptionType` with only the ASCII name filled in.
fn description_element(name: &str) -> Vec<u8> {
    let mut element = b"desc\0\0\0\0".to_vec();
    element.extend_from_slice(&(name.len() as u32 + 1).to_be_bytes());
    element.extend_from_slice(name.as_bytes());
    element.push(0);
    // Unicode language code and count, then ScriptCode code, count and its
    // fixed 67-byte buffer: all empty.
    element.extend_from_slice(&[0; 4 + 4 + 2 + 1 + 67]);
    element
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag<'a>(profile: &'a [u8], signature: &[u8; 4]) -> &'a [u8] {
        let count = u32::from_be_bytes(profile[128..132].try_into().unwrap()) as usize;
        (0..count)
            .map(|index| &profile[132 + index * 12..144 + index * 12])
            .find(|entry| &entry[..4] == signature)
            .map(|entry| {
                let offset = u32::from_be_bytes(entry[4..8].try_into().unwrap()) as usize;
                let size = u32::from_be_bytes(entry[8..12].try_into().unwrap()) as usize;
                assert_eq!(offset % 4, 0, "elements are word aligned");
                &profile[offset..offset + size]
            })
            .expect("tag present")
    }

    fn xyz(element: &[u8]) -> [f32; 3] {
        assert_eq!(&element[..4], b"XYZ ");
        std::array::from_fn(|i| {
            i32::from_be_bytes(element[8 + i * 4..12 + i * 4].try_into().unwrap()) as f32 / 65536.0
        })
    }

    #[test]
    fn the_header_declares_an_rgb_display_profile_of_the_right_size() {
        let profile = icc_profile(RgbSpace::Srgb);
        assert_eq!(
            u32::from_be_bytes(profile[0..4].try_into().unwrap()) as usize,
            profile.len()
        );
        assert_eq!(&profile[12..16], b"mntr");
        assert_eq!(&profile[16..20], b"RGB ");
        assert_eq!(&profile[36..40], b"acsp");
        assert_eq!(profile.len() % 4, 0);
    }

    #[test]
    fn colorants_match_the_published_profiles() {
        // Colorants of the widely distributed sRGB IEC61966-2.1, Adobe RGB
        // (1998), Apple Display P3 and ProPhoto (ROMM) profiles.
        let published = [
            (
                RgbSpace::Srgb,
                [
                    [0.4361, 0.2225, 0.0139],
                    [0.3851, 0.7169, 0.0971],
                    [0.1431, 0.0606, 0.7141],
                ],
            ),
            (
                RgbSpace::AdobeRgb,
                [
                    [0.6097, 0.3111, 0.0195],
                    [0.2053, 0.6257, 0.0609],
                    [0.1492, 0.0632, 0.7446],
                ],
            ),
            (
                RgbSpace::DisplayP3,
                [
                    [0.5151, 0.2412, -0.0011],
                    [0.2920, 0.6922, 0.0419],
                    [0.1571, 0.0666, 0.7841],
                ],
            ),
            (
                RgbSpace::ProPhoto,
                [
                    [0.7977, 0.2880, 0.0000],
                    [0.1352, 0.7119, 0.0000],
                    [0.0313, 0.0001, 0.8249],
                ],
            ),
        ];
        for (space, expected) in published {
            let profile = icc_profile(space);
            for (signature, expected) in [b"rXYZ", b"gXYZ", b"bXYZ"].into_iter().zip(expected) {
                let actual = xyz(tag(&profile, signature));
                for (a, e) in actual.iter().zip(expected) {
                    assert!(
                        (a - e).abs() < 1.5e-3,
                        "{space:?} {signature:?}: {actual:?} vs {expected:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn colorants_sum_to_the_d50_white_point() {
        for space in RgbSpace::ALL {
            let profile = icc_profile(space);
            let (r, g, b) = (
                xyz(tag(&profile, b"rXYZ")),
                xyz(tag(&profile, b"gXYZ")),
                xyz(tag(&profile, b"bXYZ")),
            );
            let white = xyz(tag(&profile, b"wtpt"));
            for i in 0..3 {
                assert!((r[i] + g[i] + b[i] - white[i]).abs() < 1e-3, "{space:?}");
            }
        }
    }

    #[test]
    fn the_description_names_the_space() {
        let profile = icc_profile(RgbSpace::AdobeRgb);
        let description = tag(&profile, b"desc");
        let length = u32::from_be_bytes(description[8..12].try_into().unwrap()) as usize;
        assert_eq!(&description[12..12 + length - 1], b"Adobe RGB (1998)");
    }
}
