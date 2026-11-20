//! Raw code values to linear 0..=1: black subtraction and white scaling,
//! with an optional per-site gain folded into the same pass.

use rawler::cfa::CFA;
use rawler::formats::tiff::Rational;
use rawler::{RawImage, RawImageData};
use rayon::prelude::*;

use crate::error::DecodeError;

/// Largest repeating pattern of levels and gains that is tabulated.
const MAX_PATTERN_SITES: usize = 1 << 16;

/// A gain for every site of a repeating mosaic pattern.
pub(crate) struct SiteGains {
    width: usize,
    height: usize,
    /// Row-major, `width * height` entries.
    gains: Vec<f32>,
}

impl SiteGains {
    /// No gain anywhere.
    pub(crate) const NONE: Self = Self {
        width: 1,
        height: 1,
        gains: Vec::new(),
    };

    /// The gain of each site's colour, for a red/green/blue mosaic whose
    /// channel gains are `gains`. The pattern is anchored at the sensor's
    /// top-left pixel, like `cfa` itself.
    pub(crate) fn for_mosaic(cfa: &CFA, gains: [f32; 3]) -> Self {
        let sites = (0..cfa.height).flat_map(|row| (0..cfa.width).map(move |column| (row, column)));
        Self {
            width: cfa.width.max(1),
            height: cfa.height.max(1),
            gains: sites
                .map(|(row, column)| gains.get(cfa.color_at(row, column)).copied().unwrap_or(1.0))
                .collect(),
        }
    }

    fn at(&self, row: usize, column: usize) -> f32 {
        let index = (row % self.height) * self.width + column % self.width;
        self.gains.get(index).copied().unwrap_or(1.0)
    }
}

/// What to do with one sample: subtract, scale, limit.
#[derive(Clone, Copy)]
struct Site {
    black: f32,
    /// Gain divided by the white-to-black range.
    scale: f32,
    /// The gain itself: what a saturated sample comes out as.
    limit: f32,
}

/// Scales the samples of `raw` (taken out of it as `data`) to linear values.
///
/// Each sample has its black level subtracted and is divided by the range up
/// to its white level, giving 0.0 at black and 1.0 at saturation; values
/// outside that are clamped and a NaN becomes 0. The result is then
/// multiplied by the gain of the sample's site, so with [`SiteGains::NONE`]
/// the output is within 0..=1.
///
/// Level patterns repeat from the sensor's top-left pixel, as rawler's own
/// scaling has them. Samples of odd-sized frames are all scaled, which
/// rawler's does not do.
pub(crate) fn normalized(
    raw: &RawImage,
    data: RawImageData,
    gains: &SiteGains,
) -> Result<Vec<f32>, DecodeError> {
    let row_len = raw.width.checked_mul(raw.cpp).filter(|&len| len > 0);
    let expected = row_len.and_then(|len| len.checked_mul(raw.height));
    let actual = match &data {
        RawImageData::Integer(samples) => samples.len(),
        RawImageData::Float(samples) => samples.len(),
    };
    let (Some(row_len), true) = (row_len, expected == Some(actual)) else {
        return Err(DecodeError::Corrupt(format!(
            "the raw data holds {actual} samples, not the {} x {} x {} its header promises",
            raw.width, raw.height, raw.cpp
        )));
    };
    let table = SiteTable::new(raw, gains)?;
    Ok(match data {
        RawImageData::Integer(samples) => table.apply(&samples, row_len),
        RawImageData::Float(samples) => table.apply(&samples, row_len),
    })
}

/// One [`Site`] per sample of the smallest block of pixels after which
/// black levels, white levels and gains all repeat.
struct SiteTable {
    /// Samples per table row: pattern width times samples per pixel.
    row_len: usize,
    rows: usize,
    sites: Vec<Site>,
}

impl SiteTable {
    fn new(raw: &RawImage, gains: &SiteGains) -> Result<Self, DecodeError> {
        let cpp = raw.cpp;
        let black = Pattern::black(raw);
        let white = Pattern::white(raw)?;
        let width = lcm(lcm(black.width, white.width), gains.width);
        let height = lcm(lcm(black.height, white.height), gains.height);
        let total = width
            .checked_mul(height)
            .and_then(|sites| sites.checked_mul(cpp));
        let Some(total) = total.filter(|&total| total <= MAX_PATTERN_SITES) else {
            return Err(DecodeError::Unsupported(
                "a black level pattern this large".into(),
            ));
        };

        let mut sites = Vec::with_capacity(total);
        for row in 0..height {
            for column in 0..width {
                for channel in 0..cpp {
                    let black = black.at(row, column, channel);
                    let range = white.at(row, column, channel) - black;
                    if !(range.is_finite() && range > 0.0 && black.is_finite()) {
                        return Err(DecodeError::Corrupt(
                            "the white level is not above the black level".into(),
                        ));
                    }
                    let gain = gains.at(row, column);
                    sites.push(Site {
                        black,
                        scale: gain / range,
                        limit: gain,
                    });
                }
            }
        }
        Ok(Self {
            row_len: width * cpp,
            rows: height,
            sites,
        })
    }

    fn apply<T: Copy + Into<f32> + Sync>(&self, samples: &[T], row_len: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; samples.len()];
        out.par_chunks_mut(row_len)
            .zip(samples.par_chunks(row_len))
            .enumerate()
            .for_each(|(row, (out_row, in_row))| {
                let start = (row % self.rows) * self.row_len;
                let sites = &self.sites[start..start + self.row_len];
                let blocks = out_row
                    .chunks_mut(self.row_len)
                    .zip(in_row.chunks(self.row_len));
                for (out_block, in_block) in blocks {
                    for ((out, &sample), site) in out_block.iter_mut().zip(in_block).zip(sites) {
                        let linear = (sample.into() - site.black) * site.scale;
                        // Written so that a NaN sample comes out as 0.
                        *out = if linear > 0.0 {
                            linear.min(site.limit)
                        } else {
                            0.0
                        };
                    }
                }
            });
        out
    }
}

/// A level for every sample of a repeating block of pixels.
struct Pattern {
    width: usize,
    height: usize,
    cpp: usize,
    /// `height * width * cpp` values, or a single value for everything.
    values: Vec<f32>,
}

impl Pattern {
    fn uniform(value: f32) -> Self {
        Self {
            width: 1,
            height: 1,
            cpp: 1,
            values: vec![value],
        }
    }

    /// The black levels of `raw`. A pattern whose size does not match its
    /// own description falls back to its first value.
    fn black(raw: &RawImage) -> Self {
        let levels = &raw.blacklevel;
        let values: Vec<f32> = levels.levels.iter().map(rational).collect();
        let consistent = levels.width > 0
            && levels.height > 0
            && levels.cpp == raw.cpp
            && levels
                .width
                .checked_mul(levels.height)
                .and_then(|n| n.checked_mul(levels.cpp))
                == Some(values.len());
        if consistent {
            Self {
                width: levels.width,
                height: levels.height,
                cpp: levels.cpp,
                values,
            }
        } else {
            Self::uniform(values.first().copied().unwrap_or(0.0))
        }
    }

    /// The white levels of `raw`: one per channel, or for single-channel
    /// data one per site of a 2 x 2 block (as rawler reads four of them),
    /// or one for everything.
    fn white(raw: &RawImage) -> Result<Self, DecodeError> {
        let values: Vec<f32> = raw.whitelevel.0.iter().map(|&level| level as f32).collect();
        let (width, height, cpp) = match (raw.cpp, values.len()) {
            (_, 0) => {
                return Err(DecodeError::Corrupt(
                    "the raw file has no white level".into(),
                ));
            }
            (1, 4) => (2, 2, 1),
            (cpp, count) if cpp == count => (1, 1, cpp),
            _ => return Ok(Self::uniform(values[0])),
        };
        Ok(Self {
            width,
            height,
            cpp,
            values,
        })
    }

    fn at(&self, row: usize, column: usize, channel: usize) -> f32 {
        let pixel = (row % self.height) * self.width + column % self.width;
        self.values[pixel * self.cpp + channel % self.cpp]
    }
}

/// A TIFF rational as a float; zero denominators read as zero.
fn rational(value: &Rational) -> f32 {
    if value.d == 0 {
        0.0
    } else {
        value.n as f32 / value.d as f32
    }
}

fn lcm(a: usize, b: usize) -> usize {
    fn gcd(a: usize, b: usize) -> usize {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let (a, b) = (a.max(1), b.max(1));
    (a / gcd(a, b)).saturating_mul(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_least_common_multiple_covers_every_pattern() {
        assert_eq!(lcm(1, 1), 1);
        assert_eq!(lcm(2, 6), 6);
        assert_eq!(lcm(2, 3), 6);
        assert_eq!(lcm(4, 6), 12);
        assert_eq!(lcm(0, 5), 5, "an empty pattern counts as one site");
        assert_eq!(
            lcm(usize::MAX, 2),
            usize::MAX,
            "overflow saturates and is then rejected"
        );
    }

    #[test]
    fn mosaic_gains_follow_the_filter_colours() {
        let gains = SiteGains::for_mosaic(&CFA::new("RGGB"), [2.0, 1.0, 1.5]);
        assert_eq!(
            [
                gains.at(0, 0),
                gains.at(0, 1),
                gains.at(1, 0),
                gains.at(1, 1)
            ],
            [2.0, 1.0, 1.0, 1.5]
        );
        assert_eq!(gains.at(6, 9), 1.0, "even row, odd column: green");
        assert_eq!(gains.at(7, 9), 1.5);
        let shifted = SiteGains::for_mosaic(&CFA::new("RGGB").shift(1, 0), [2.0, 1.0, 1.5]);
        assert_eq!([shifted.at(0, 0), shifted.at(0, 1)], [1.0, 2.0]);
        assert_eq!(SiteGains::NONE.at(123, 456), 1.0);
    }

    #[test]
    fn rationals_with_a_zero_denominator_read_as_zero() {
        assert_eq!(rational(&Rational::new(1024, 4)), 256.0);
        assert_eq!(rational(&Rational::new(7, 0)), 0.0);
    }
}
