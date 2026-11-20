//! The output transform (DESIGN §5.8, pipeline stage 11): display-linear
//! working RGB to an encoded RGB colour space.
//!
//! Colours the output space cannot hold are desaturated toward their own
//! luminance until no channel is negative, then clipped at 1. That keeps
//! hue and lightness, which a per-channel clip would shift.

use std::sync::LazyLock;

use photo_core::colorimetry::{Mat3, RgbSpace, WORKING_SPACE, mat3_apply, rgb_to_rgb};

use crate::math::toward_luminance;

// ---- Tunable constants ----------------------------------------------------

/// Entries of the 8-bit encoding tables. They are indexed by the square
/// root of the linear value, against which every supported transfer
/// function has a slope below 2, so with 4096 entries a table lookup is
/// within 0.07 of a code value of the exact encoding before rounding.
const CODE_ENTRIES: usize = 4096;

// ---------------------------------------------------------------------------

/// One table of 8-bit codes per output space, in [`RgbSpace::ALL`] order.
static CODE_TABLES: LazyLock<[Box<[u8]>; 5]> = LazyLock::new(|| {
    RgbSpace::ALL.map(|space| {
        (0..CODE_ENTRIES)
            .map(|i| {
                let root = i as f32 / (CODE_ENTRIES - 1) as f32;
                (space.encode(root * root).clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect()
    })
});

fn table_index(space: RgbSpace) -> usize {
    RgbSpace::ALL
        .iter()
        .position(|candidate| *candidate == space)
        .unwrap_or(0)
}

/// Stage 11 compiled for one output colour space.
#[derive(Clone, Copy)]
pub struct OutputTransform {
    space: RgbSpace,
    /// Working RGB to linear output RGB; `None` when they are the same space.
    matrix: Option<Mat3>,
    /// Luminance weights of the output space.
    luma: [f32; 3],
    codes: &'static [u8],
}

/// Prints the output space, not the encoding table.
impl std::fmt::Debug for OutputTransform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputTransform")
            .field("space", &self.space)
            .finish_non_exhaustive()
    }
}

impl OutputTransform {
    /// Build the transform into `space` (with Bradford adaptation when its
    /// white point is not D65).
    pub fn new(space: RgbSpace) -> Self {
        Self {
            space,
            matrix: (space != WORKING_SPACE).then(|| rgb_to_rgb(WORKING_SPACE, space)),
            luma: space.to_xyz()[1],
            codes: &CODE_TABLES[table_index(space)],
        }
    }

    /// The colour space this transform encodes into.
    pub fn space(&self) -> RgbSpace {
        self.space
    }

    /// Linear RGB in the output space, gamut-mapped and clipped: every
    /// channel is in 0 to 1 for any input (NaN becomes 0). Equal input
    /// channels give equal output channels.
    // `max` then `min` rather than `clamp`: `f32::max` drops a NaN, `clamp`
    // would pass it through.
    #[allow(clippy::manual_clamp)]
    #[inline]
    pub fn to_linear(&self, working: [f32; 3]) -> [f32; 3] {
        if working[0] == working[1] && working[1] == working[2] {
            return [working[0].max(0.0).min(1.0); 3];
        }
        let rgb = match &self.matrix {
            Some(matrix) => mat3_apply(matrix, working),
            None => working,
        };
        let rgb = toward_luminance(rgb, &self.luma);
        [
            rgb[0].max(0.0).min(1.0),
            rgb[1].max(0.0).min(1.0),
            rgb[2].max(0.0).min(1.0),
        ]
    }

    /// The colour encoded with the output space's transfer function, each
    /// channel in 0 to 1. Exact (it calls the transfer function), so this is
    /// the path for export.
    #[inline]
    pub fn encode(&self, working: [f32; 3]) -> [f32; 3] {
        self.to_linear(working)
            .map(|v| self.space.encode(v).clamp(0.0, 1.0))
    }

    /// The colour as 16-bit code values: [`Self::encode`] rounded to the
    /// nearest of 65536 levels.
    #[inline]
    pub fn encode_u16(&self, working: [f32; 3]) -> [u16; 3] {
        self.encode(working).map(|v| (v * 65535.0 + 0.5) as u16)
    }

    /// The colour as 8-bit code values, from a lookup table: the fast path
    /// for the screen. It agrees with rounding [`Self::encode`] except
    /// within a few hundredths of a code value of a rounding boundary,
    /// where it may pick the neighbouring code. Monotonic in each channel.
    #[inline]
    pub fn encode_u8(&self, working: [f32; 3]) -> [u8; 3] {
        let scale = (CODE_ENTRIES - 1) as f32;
        self.to_linear(working).map(|v| {
            let index = (v.sqrt() * scale + 0.5) as usize;
            self.codes[index.min(CODE_ENTRIES - 1)]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_space_gets_its_own_table() {
        for (index, space) in RgbSpace::ALL.into_iter().enumerate() {
            assert_eq!(table_index(space), index);
            let table = &CODE_TABLES[index];
            assert_eq!((table[0], table[CODE_ENTRIES - 1]), (0, 255));
            assert!(
                table
                    .windows(2)
                    .all(|pair| pair[1] >= pair[0] && pair[1] - pair[0] <= 1)
            );
        }
        // sRGB and Adobe RGB encode middle grey differently.
        let grey = [0.18; 3];
        assert_ne!(
            OutputTransform::new(RgbSpace::Srgb).encode_u8(grey),
            OutputTransform::new(RgbSpace::AdobeRgb).encode_u8(grey)
        );
    }
}
