//! The histogram of a rendered image.

use photo_core::colorimetry::RgbSpace;
use photo_core::image::Rgba8Image;
use rayon::prelude::*;

/// Pixels handed to one thread at a time.
const CHUNK_PIXELS: usize = 1 << 15;
/// Luminance weights are held as fractions of this, so that they sum to
/// exactly one and a grey pixel's luminance is exactly its own value.
const WEIGHT_ONE: u32 = 1 << 16;

/// How many pixels of an image fall on each of the 256 code values, per
/// channel and for luminance, and how much of the image is clipped.
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    /// `red[v]` is the number of pixels whose red code value is `v`.
    pub red: [u32; 256],
    /// `green[v]` is the number of pixels whose green code value is `v`.
    pub green: [u32; 256],
    /// `blue[v]` is the number of pixels whose blue code value is `v`.
    pub blue: [u32; 256],
    /// Counts of `round(wr * R + wg * G + wb * B)` on the encoded values,
    /// with the luminance weights of the image's own colour space (for sRGB:
    /// 0.2126, 0.7152, 0.0722). This is what a photographer's histogram
    /// shows, not a count of linear luminance.
    pub luma: [u32; 256],
    /// Fraction of pixels, 0 to 1, with at least one of R, G, B at 0.
    pub clipped_shadows: f32,
    /// Fraction of pixels, 0 to 1, with at least one of R, G, B at 255.
    pub clipped_highlights: f32,
}

impl Histogram {
    /// Number of bins per channel.
    pub const BINS: usize = 256;

    /// The number of pixels counted: the sum of any one of the four
    /// histograms.
    pub fn pixel_count(&self) -> u64 {
        self.luma.iter().map(|&count| u64::from(count)).sum()
    }
}

/// Counts for part of an image; merged by addition.
struct Tally {
    bins: Box<[[u32; 256]; 4]>,
    shadows: u64,
    highlights: u64,
}

impl Tally {
    fn new() -> Self {
        Self {
            bins: Box::new([[0; 256]; 4]),
            shadows: 0,
            highlights: 0,
        }
    }

    fn count(mut self, pixels: &[u8], weights: [u32; 3]) -> Self {
        for pixel in pixels.chunks_exact(Rgba8Image::CHANNELS) {
            let (r, g, b) = (pixel[0], pixel[1], pixel[2]);
            let weighted =
                weights[0] * u32::from(r) + weights[1] * u32::from(g) + weights[2] * u32::from(b);
            let luma = (weighted + WEIGHT_ONE / 2) / WEIGHT_ONE;
            self.bins[0][usize::from(r)] += 1;
            self.bins[1][usize::from(g)] += 1;
            self.bins[2][usize::from(b)] += 1;
            self.bins[3][luma as usize] += 1;
            self.shadows += u64::from(r == 0 || g == 0 || b == 0);
            self.highlights += u64::from(r == 255 || g == 255 || b == 255);
        }
        self
    }

    fn merge(mut self, other: Self) -> Self {
        for (mine, theirs) in self
            .bins
            .iter_mut()
            .flatten()
            .zip(other.bins.iter().flatten())
        {
            *mine += theirs;
        }
        self.shadows += other.shadows;
        self.highlights += other.highlights;
        self
    }
}

/// The relative luminance weights of `space` as integer fractions of
/// [`WEIGHT_ONE`] that sum to exactly [`WEIGHT_ONE`].
fn luma_weights(space: RgbSpace) -> [u32; 3] {
    let y = space.to_xyz()[1];
    let red = (y[0] * WEIGHT_ONE as f32).round() as u32;
    let green = (y[1] * WEIGHT_ONE as f32).round() as u32;
    [red, green, WEIGHT_ONE - red - green]
}

/// Count the histogram of `image`.
///
/// Every complete RGBA quadruple in `image.pixels` is counted once; alpha is
/// ignored. Each of the four histograms sums to the pixel count. A grey
/// pixel `(v, v, v)` always lands in luminance bin `v`. An empty image gives
/// all-zero counts and clipping fractions of 0.
///
/// The clipping fractions count a pixel when **any** of its channels is at
/// the limit, which is what a clipping warning needs: a saturated colour can
/// have lost detail in one channel long before it reads as white. A pixel
/// can count toward both. Per-channel clipping is bins 0 and 255 of the
/// channel histograms.
///
/// Counting is exact integer work split across threads, so the result is the
/// same on any number of them.
pub fn histogram(image: &Rgba8Image) -> Histogram {
    let weights = luma_weights(image.space);
    let tally = image
        .pixels
        .par_chunks(CHUNK_PIXELS * Rgba8Image::CHANNELS)
        .map(|chunk| Tally::new().count(chunk, weights))
        .reduce(Tally::new, Tally::merge);
    let pixels = (image.pixels.len() / Rgba8Image::CHANNELS) as f64;
    let fraction = |count: u64| {
        if pixels > 0.0 {
            (count as f64 / pixels) as f32
        } else {
            0.0
        }
    };
    let [red, green, blue, luma] = *tally.bins;
    Histogram {
        red,
        green,
        blue,
        luma,
        clipped_shadows: fraction(tally.shadows),
        clipped_highlights: fraction(tally.highlights),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_weights_sum_to_exactly_one_in_every_space() {
        for space in RgbSpace::ALL {
            let weights = luma_weights(space);
            assert_eq!(weights.iter().sum::<u32>(), WEIGHT_ONE, "{space:?}");
            assert!(weights.iter().all(|&w| w < WEIGHT_ONE));
        }
        let srgb = luma_weights(RgbSpace::Srgb).map(|w| w as f32 / WEIGHT_ONE as f32);
        assert!((srgb[0] - 0.2126).abs() < 1e-4 && (srgb[1] - 0.7152).abs() < 1e-4);
    }

    #[test]
    fn counting_in_pieces_equals_counting_at_once() {
        let pixels: Vec<u8> = (0..4000u32).map(|i| (i * 37 % 256) as u8).collect();
        let weights = luma_weights(RgbSpace::Srgb);
        let whole = Tally::new().count(&pixels, weights);
        let (first, second) = pixels.split_at(1600);
        let pieces = Tally::new()
            .count(first, weights)
            .merge(Tally::new().count(second, weights));
        assert_eq!(whole.bins, pieces.bins);
        assert_eq!(
            (whole.shadows, whole.highlights),
            (pieces.shadows, pieces.highlights)
        );
    }
}
