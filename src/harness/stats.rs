//! Pixel statistics: what an agent reads instead of judging tones by eye.
//!
//! Pure functions over decoded 8-bit RGBA images, with no I/O. Alpha is
//! ignored: every statistic is about the colour a viewer sees. Percentiles
//! are nearest-rank: `p5` is the smallest value at or below which at least
//! 5% of the pixels lie.

use image::RgbaImage;
use serde_json::{Value, json};

/// Bins of the luma histogram; each covers four 8-bit levels.
pub const LUMA_BINS: usize = 64;

/// A pixel differs between two images when some channel differs by more
/// than this many 8-bit levels.
pub const DIFF_THRESHOLD: u8 = 2;

/// Rec. 709 luma weights, in ten-thousandths, applied to the encoded
/// (gamma) values as Y' is.
const LUMA_WEIGHTS: [u64; 3] = [2126, 7152, 722];
/// Luma of white, in the same ten-thousandths of a level.
const LUMA_WHITE: u64 = 255 * 10_000;
/// Luma is binned to hundredths of a level for its percentiles.
const LUMA_FINE_BINS: usize = 255 * 100 + 1;

/// One channel's (or luma's) distribution over every pixel, on the 0 to 255
/// scale.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelStats {
    /// Mean value.
    pub mean: f64,
    /// Smallest value.
    pub min: f64,
    /// 1st percentile.
    pub p1: f64,
    /// 5th percentile.
    pub p5: f64,
    /// Median.
    pub p50: f64,
    /// 95th percentile.
    pub p95: f64,
    /// 99th percentile.
    pub p99: f64,
    /// Largest value.
    pub max: f64,
    /// Fraction of pixels at exactly 0 (for luma: black in every channel).
    pub clip_low_fraction: f64,
    /// Fraction of pixels at exactly 255 (for luma: white in every channel).
    pub clip_high_fraction: f64,
}

/// Everything [`image_stats`] measures.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageStats {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Red, green and blue, in that order.
    pub channels: [ChannelStats; 3],
    /// Rec. 709 luma of the encoded values.
    pub luma: ChannelStats,
    /// Fraction of pixels whose luma is in each bin; bin `i` covers luma
    /// from `4i` up to (not including) `4i + 4`, and the last includes 255.
    pub luma_histogram: [f64; LUMA_BINS],
    /// Mean HSV saturation, `(max - min) / max` per pixel (0 for black).
    pub mean_saturation: f64,
}

/// Measure every pixel of `image`. Fails only for an image with no pixels.
pub fn image_stats(image: &RgbaImage) -> Result<ImageStats, String> {
    let total = u64::from(image.width()) * u64::from(image.height());
    if total == 0 {
        return Err("the image has no pixels".to_owned());
    }
    let mut levels = [[0_u64; 256]; 3];
    let mut luma_fine = vec![0_u64; LUMA_FINE_BINS];
    let mut luma_coarse = [0_u64; LUMA_BINS];
    let mut luma_sum = 0_u64;
    // Luma is exactly 0 or 255 only for black or white in every channel,
    // which the hundredth-of-a-level bins alone cannot tell from nearly so.
    let (mut black, mut white) = (0_u64, 0_u64);
    let mut saturation_sum = 0.0_f64;
    for pixel in image.pixels() {
        let [r, g, b, _] = pixel.0;
        for (channel, value) in [r, g, b].into_iter().enumerate() {
            levels[channel][usize::from(value)] += 1;
        }
        let luma = LUMA_WEIGHTS[0] * u64::from(r)
            + LUMA_WEIGHTS[1] * u64::from(g)
            + LUMA_WEIGHTS[2] * u64::from(b);
        luma_sum += luma;
        black += u64::from(luma == 0);
        white += u64::from(luma == LUMA_WHITE);
        luma_fine[((luma + 50) / 100) as usize] += 1;
        luma_coarse[((luma / 40_000) as usize).min(LUMA_BINS - 1)] += 1;
        let (high, low) = (r.max(g).max(b), r.min(g).min(b));
        if high > 0 {
            saturation_sum += f64::from(high - low) / f64::from(high);
        }
    }
    let channels = [0, 1, 2].map(|channel| {
        let counts = &levels[channel];
        let sum: u64 = counts
            .iter()
            .enumerate()
            .map(|(value, count)| value as u64 * count)
            .sum();
        distribution(counts, total, sum as f64 / total as f64, |bin| bin as f64)
    });
    let mut luma = distribution(
        &luma_fine,
        total,
        luma_sum as f64 / total as f64 / 10_000.0,
        |bin| bin as f64 / 100.0,
    );
    luma.clip_low_fraction = black as f64 / total as f64;
    luma.clip_high_fraction = white as f64 / total as f64;
    Ok(ImageStats {
        width: image.width(),
        height: image.height(),
        channels,
        luma,
        luma_histogram: luma_coarse.map(|count| count as f64 / total as f64),
        mean_saturation: saturation_sum / total as f64,
    })
}

/// The statistics of a histogram whose bin `i` holds the value `value(i)`.
fn distribution(
    counts: &[u64],
    total: u64,
    mean: f64,
    value: impl Fn(usize) -> f64,
) -> ChannelStats {
    let first = counts.iter().position(|count| *count > 0).unwrap_or(0);
    let last = counts.iter().rposition(|count| *count > 0).unwrap_or(0);
    let percentile = |p: f64| value(nearest_rank(counts, total, p));
    let last_bin = counts.len() - 1;
    ChannelStats {
        mean,
        min: value(first),
        p1: percentile(1.0),
        p5: percentile(5.0),
        p50: percentile(50.0),
        p95: percentile(95.0),
        p99: percentile(99.0),
        max: value(last),
        clip_low_fraction: counts[0] as f64 / total as f64,
        clip_high_fraction: counts[last_bin] as f64 / total as f64,
    }
}

/// The bin of the `p`th percentile, nearest-rank: the first bin by which at
/// least `p`% of the `total` values have been counted.
fn nearest_rank(counts: &[u64], total: u64, p: f64) -> usize {
    // Multiplying first keeps whole-number percentiles of whole counts exact.
    let rank = (p * total as f64 / 100.0).ceil().max(1.0) as u64;
    let mut seen = 0;
    for (bin, count) in counts.iter().enumerate() {
        seen += count;
        if seen >= rank {
            return bin;
        }
    }
    counts.len() - 1
}

/// How one channel differs between two images.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelDiff {
    /// Mean absolute difference, in 8-bit levels.
    pub mean_abs_diff: f64,
    /// Largest absolute difference.
    pub max_abs_diff: u8,
}

/// What [`compare`] finds.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    /// Width of both images.
    pub width: u32,
    /// Height of both images.
    pub height: u32,
    /// Red, green and blue, in that order.
    pub channels: [ChannelDiff; 3],
    /// Pixels where some channel differs by more than [`DIFF_THRESHOLD`].
    pub differing_pixels: u64,
    /// Those pixels as a fraction of all.
    pub differing_fraction: f64,
}

/// Compare two images of one size, pixel by pixel.
pub fn compare(a: &RgbaImage, b: &RgbaImage) -> Result<Comparison, String> {
    if a.dimensions() != b.dimensions() {
        return Err(format!(
            "the images differ in size: {} x {} and {} x {}",
            a.width(),
            a.height(),
            b.width(),
            b.height()
        ));
    }
    let total = u64::from(a.width()) * u64::from(a.height());
    if total == 0 {
        return Err("the images have no pixels".to_owned());
    }
    let mut sums = [0_u64; 3];
    let mut maxima = [0_u8; 3];
    let mut differing = 0_u64;
    for (left, right) in a.pixels().zip(b.pixels()) {
        let mut differs = false;
        for channel in 0..3 {
            let diff = left.0[channel].abs_diff(right.0[channel]);
            sums[channel] += u64::from(diff);
            maxima[channel] = maxima[channel].max(diff);
            differs |= diff > DIFF_THRESHOLD;
        }
        differing += u64::from(differs);
    }
    Ok(Comparison {
        width: a.width(),
        height: a.height(),
        channels: [0, 1, 2].map(|channel| ChannelDiff {
            mean_abs_diff: sums[channel] as f64 / total as f64,
            max_abs_diff: maxima[channel],
        }),
        differing_pixels: differing,
        differing_fraction: differing as f64 / total as f64,
    })
}

/// The channel names, in the order statistics keep them.
const CHANNEL_NAMES: [&str; 3] = ["r", "g", "b"];

/// `value` rounded to `places` decimals, for numbers people read.
fn round(value: f64, places: i32) -> f64 {
    let scale = 10_f64.powi(places);
    (value * scale).round() / scale
}

fn channel_json(stats: &ChannelStats) -> Value {
    json!({
        "mean": round(stats.mean, 3),
        "min": round(stats.min, 2),
        "p1": round(stats.p1, 2),
        "p5": round(stats.p5, 2),
        "p50": round(stats.p50, 2),
        "p95": round(stats.p95, 2),
        "p99": round(stats.p99, 2),
        "max": round(stats.max, 2),
        "clip_low_fraction": round(stats.clip_low_fraction, 6),
        "clip_high_fraction": round(stats.clip_high_fraction, 6),
    })
}

impl ImageStats {
    /// The statistics as the protocol reports them.
    pub fn to_json(&self) -> Value {
        let mut channels = serde_json::Map::new();
        for (name, stats) in CHANNEL_NAMES.iter().zip(&self.channels) {
            channels.insert((*name).to_owned(), channel_json(stats));
        }
        channels.insert("luma".to_owned(), channel_json(&self.luma));
        json!({
            "width": self.width,
            "height": self.height,
            "pixels": u64::from(self.width) * u64::from(self.height),
            "channels": channels,
            "luma_histogram": self
                .luma_histogram
                .iter()
                .map(|fraction| round(*fraction, 6))
                .collect::<Vec<f64>>(),
            "luma_histogram_bin_width": 4,
            "mean_saturation": round(self.mean_saturation, 4),
        })
    }
}

impl Comparison {
    /// The comparison as the protocol reports it.
    pub fn to_json(&self) -> Value {
        let mut channels = serde_json::Map::new();
        for (name, diff) in CHANNEL_NAMES.iter().zip(&self.channels) {
            channels.insert(
                (*name).to_owned(),
                json!({
                    "mean_abs_diff": round(diff.mean_abs_diff, 4),
                    "max_abs_diff": diff.max_abs_diff,
                }),
            );
        }
        json!({
            "width": self.width,
            "height": self.height,
            "channels": channels,
            "differing_pixels": self.differing_pixels,
            "differing_fraction": round(self.differing_fraction, 6),
            "differing_threshold": DIFF_THRESHOLD,
            "identical": self.channels.iter().all(|diff| diff.max_abs_diff == 0),
        })
    }
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    fn image_of(pixels: &[[u8; 3]]) -> RgbaImage {
        let mut image = RgbaImage::new(pixels.len() as u32, 1);
        for (x, [r, g, b]) in pixels.iter().enumerate() {
            image.put_pixel(x as u32, 0, Rgba([*r, *g, *b, 255]));
        }
        image
    }

    #[test]
    fn black_and_white_are_clipped_and_grey_is_not() {
        let stats = image_stats(&image_of(&[
            [0, 0, 0],
            [255, 255, 255],
            [128, 128, 128],
            [128, 128, 128],
        ]))
        .unwrap();
        for channel in &stats.channels {
            assert_eq!((channel.min, channel.max), (0.0, 255.0));
            assert_eq!(channel.clip_low_fraction, 0.25);
            assert_eq!(channel.clip_high_fraction, 0.25);
            assert_eq!(channel.mean, (255.0 + 256.0) / 4.0);
            assert_eq!(channel.p50, 128.0);
        }
        assert_eq!(stats.luma.clip_low_fraction, 0.25);
        assert_eq!(stats.luma.clip_high_fraction, 0.25);
        assert_eq!(stats.luma.max, 255.0);
        assert_eq!(stats.mean_saturation, 0.0, "greys have no saturation");
    }

    #[test]
    fn luma_weighs_green_most_and_a_pure_colour_is_not_clipped_luma() {
        let stats = image_of(&[[255, 0, 0], [0, 255, 0], [0, 0, 255]]);
        let stats = image_stats(&stats).unwrap();
        assert_eq!(stats.luma.min, 18.41, "blue: 0.0722 x 255");
        assert_eq!(stats.luma.max, 182.38, "green: 0.7152 x 255");
        assert_eq!(stats.luma.p50, 54.21, "red: 0.2126 x 255");
        assert_eq!(stats.luma.clip_high_fraction, 0.0);
        assert_eq!(stats.luma.clip_low_fraction, 0.0);
        let red = &stats.channels[0];
        assert!((red.clip_high_fraction - 1.0 / 3.0).abs() < 1e-12);
        assert!((red.clip_low_fraction - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(stats.mean_saturation, 1.0);
    }

    #[test]
    fn percentiles_are_nearest_rank_and_ordered() {
        // 0, 1, ..., 99 in red: the pth percentile of 100 values is p - 1.
        let pixels: Vec<[u8; 3]> = (0..100).map(|v| [v as u8, 0, 0]).collect();
        let red = &image_stats(&image_of(&pixels)).unwrap().channels[0];
        assert_eq!(
            (red.p1, red.p5, red.p50, red.p95, red.p99),
            (0.0, 4.0, 49.0, 94.0, 98.0)
        );
        assert!(red.min <= red.p1 && red.p99 <= red.max);
        assert_eq!(red.mean, 49.5);
    }

    #[test]
    fn the_histogram_has_four_levels_a_bin_and_sums_to_one() {
        let stats = image_stats(&image_of(&[
            [0, 0, 0],
            [3, 3, 3],
            [4, 4, 4],
            [255, 255, 255],
        ]))
        .unwrap();
        assert_eq!(stats.luma_histogram[0], 0.5);
        assert_eq!(stats.luma_histogram[1], 0.25);
        assert_eq!(stats.luma_histogram[LUMA_BINS - 1], 0.25);
        assert!((stats.luma_histogram.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn an_empty_image_has_no_statistics() {
        assert!(image_stats(&RgbaImage::new(0, 0)).is_err());
    }

    #[test]
    fn an_image_compared_with_itself_does_not_differ() {
        let image = image_of(&[[10, 20, 30], [200, 100, 0]]);
        let same = compare(&image, &image).unwrap();
        assert_eq!(same.differing_pixels, 0);
        assert!(
            same.channels
                .iter()
                .all(|c| c.max_abs_diff == 0 && c.mean_abs_diff == 0.0)
        );
    }

    #[test]
    fn differences_are_counted_per_channel_and_beyond_the_threshold() {
        let a = image_of(&[[10, 10, 10], [10, 10, 10], [10, 10, 10], [10, 10, 10]]);
        let b = image_of(&[[12, 10, 10], [10, 13, 10], [10, 10, 0], [10, 10, 10]]);
        let found = compare(&a, &b).unwrap();
        assert_eq!(found.channels[0].max_abs_diff, 2);
        assert_eq!(found.channels[1].max_abs_diff, 3);
        assert_eq!(found.channels[2].max_abs_diff, 10);
        assert_eq!(found.channels[2].mean_abs_diff, 2.5);
        assert_eq!(
            found.differing_pixels, 2,
            "a difference of 2 is within the threshold"
        );
        assert_eq!(found.differing_fraction, 0.5);
    }

    #[test]
    fn images_of_different_sizes_are_not_compared() {
        let error = compare(&RgbaImage::new(2, 1), &RgbaImage::new(1, 2)).unwrap_err();
        assert!(
            error.contains("2 x 1") && error.contains("1 x 2"),
            "{error}"
        );
    }
}
