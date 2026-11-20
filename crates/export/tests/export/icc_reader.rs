//! A minimal ICC reader, independent of the writer: enough of ICC.1:2010
//! to check a matrix/TRC display profile.

use photo_core::colorimetry::{Chromaticity, Mat3, mat3_apply, mat3_invert};

/// What a matrix/TRC profile says.
pub struct Profile {
    pub version: [u8; 2],
    pub class: [u8; 4],
    pub description: String,
    /// The media white point as stored (D50 for a version 4 display).
    pub white_point: [f32; 3],
    /// The chromatic adaptation the profile records.
    pub adaptation: Mat3,
    /// The colorants as stored: columns of the RGB to D50 XYZ matrix.
    pub colorants: Mat3,
    /// The red, green and blue tone curves.
    pub curves: [Curve; 3],
}

/// A parametric curve: function type and parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub function: u16,
    pub parameters: Vec<f64>,
}

impl Curve {
    /// Encoded value to linear light.
    pub fn eval(&self, x: f64) -> f64 {
        let p = &self.parameters;
        match self.function {
            0 => x.powf(p[0]),
            3 if x >= p[4] => (p[1] * x + p[2]).powf(p[0]),
            3 => p[3] * x,
            other => panic!("parametric function {other} not expected"),
        }
    }
}

impl Profile {
    pub fn parse(bytes: &[u8]) -> Self {
        assert!(bytes.len() >= 132, "too short for a profile");
        assert_eq!(be32(bytes, 0) as usize, bytes.len(), "size field");
        assert_eq!(bytes.len() % 4, 0, "profiles are padded to four bytes");
        assert_eq!(&bytes[36..40], b"acsp");
        assert_eq!(&bytes[16..20], b"RGB ");
        assert_eq!(&bytes[20..24], b"XYZ ");
        let tag = |signature: &[u8; 4]| -> &[u8] {
            let count = be32(bytes, 128) as usize;
            (0..count)
                .map(|index| 132 + index * 12)
                .find(|&entry| &bytes[entry..entry + 4] == signature)
                .map(|entry| {
                    let (offset, size) = (
                        be32(bytes, entry + 4) as usize,
                        be32(bytes, entry + 8) as usize,
                    );
                    assert_eq!(offset % 4, 0, "{signature:?} is word aligned");
                    &bytes[offset..offset + size]
                })
                .unwrap_or_else(|| panic!("tag {:?} missing", String::from_utf8_lossy(signature)))
        };
        let colorant = |signature: &[u8; 4]| xyz(tag(signature));
        let (r, g, b) = (colorant(b"rXYZ"), colorant(b"gXYZ"), colorant(b"bXYZ"));
        Self {
            version: [bytes[8], bytes[9]],
            class: bytes[12..16].try_into().unwrap(),
            description: mluc(tag(b"desc")),
            white_point: xyz(tag(b"wtpt")),
            adaptation: sf32(tag(b"chad")),
            colorants: [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]],
            curves: [para(tag(b"rTRC")), para(tag(b"gTRC")), para(tag(b"bTRC"))],
        }
    }

    /// The colour space's own primaries and white: the colorants and the
    /// white point taken back through the recorded adaptation.
    pub fn chromaticities(&self) -> ([Chromaticity; 3], Chromaticity) {
        let undo = mat3_invert(&self.adaptation).expect("chad is invertible");
        let column = |c: usize| {
            mat3_apply(
                &undo,
                [
                    self.colorants[0][c],
                    self.colorants[1][c],
                    self.colorants[2][c],
                ],
            )
        };
        (
            [xy(column(0)), xy(column(1)), xy(column(2))],
            xy(mat3_apply(&undo, self.white_point)),
        )
    }
}

fn xy(xyz: [f32; 3]) -> Chromaticity {
    let sum = f64::from(xyz[0] + xyz[1] + xyz[2]);
    Chromaticity::new(f64::from(xyz[0]) / sum, f64::from(xyz[1]) / sum)
}

fn be32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn fixed(bytes: &[u8], at: usize) -> f64 {
    f64::from(be32(bytes, at) as i32) / 65536.0
}

fn xyz(element: &[u8]) -> [f32; 3] {
    assert_eq!(&element[..4], b"XYZ ");
    std::array::from_fn(|i| fixed(element, 8 + 4 * i) as f32)
}

fn sf32(element: &[u8]) -> Mat3 {
    assert_eq!(&element[..4], b"sf32");
    assert_eq!(element.len(), 8 + 36);
    std::array::from_fn(|row| {
        std::array::from_fn(|column| fixed(element, 8 + 4 * (row * 3 + column)) as f32)
    })
}

fn para(element: &[u8]) -> Curve {
    assert_eq!(&element[..4], b"para");
    let function = u16::from_be_bytes([element[8], element[9]]);
    let count = match function {
        0 => 1,
        1 => 3,
        2 => 4,
        3 => 5,
        4 => 7,
        other => panic!("unknown parametric function {other}"),
    };
    assert_eq!(element.len(), 12 + 4 * count);
    Curve {
        function,
        parameters: (0..count).map(|i| fixed(element, 12 + 4 * i)).collect(),
    }
}

fn mluc(element: &[u8]) -> String {
    assert_eq!(&element[..4], b"mluc");
    assert!(be32(element, 8) >= 1, "at least one record");
    let (length, offset) = (be32(element, 20) as usize, be32(element, 24) as usize);
    let units: Vec<u16> = element[offset..offset + length]
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units).unwrap()
}
