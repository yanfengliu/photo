//! Pure pixel-statistics functions for harness observations.
//!
//! These numbers are the harness's primary "value range" evidence channel:
//! clipping fractions, per-channel percentiles, a luma histogram, and image
//! comparison metrics. All math runs on sRGB-encoded 8-bit RGBA buffers (the
//! app's decode/render format); values are display-referred by design.

pub(crate) const LUMA_HISTOGRAM_BINS: usize = 64;

/// Per-pixel channel deltas at or below this are counted as "same" by
/// [`compare_images`] — absorbs codec/rounding jitter without hiding real
/// tuning differences.
pub(crate) const COMPARE_DIFF_TOLERANCE: u8 = 2;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ChannelStats {
    pub(crate) mean: f32,
    pub(crate) min: u8,
    pub(crate) p1: u8,
    pub(crate) p5: u8,
    pub(crate) p50: u8,
    pub(crate) p95: u8,
    pub(crate) p99: u8,
    pub(crate) max: u8,
    /// Fraction of pixels at exactly 0 (crushed).
    pub(crate) clip_low_fraction: f32,
    /// Fraction of pixels at exactly 255 (blown).
    pub(crate) clip_high_fraction: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ImageStatsReport {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) r: ChannelStats,
    pub(crate) g: ChannelStats,
    pub(crate) b: ChannelStats,
    /// Rec. 709 luma computed on the sRGB-encoded values.
    pub(crate) luma: ChannelStats,
    /// 64 bins over luma 0..=255 (bin = luma / 4), counts in pixels.
    pub(crate) luma_histogram: Vec<u64>,
    /// Mean HSV saturation (0 = grayscale, 1 = fully saturated).
    pub(crate) mean_saturation: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ImageCompareReport {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Mean absolute per-pixel difference, per channel (R, G, B).
    pub(crate) mean_abs_diff: [f32; 3],
    /// Maximum absolute per-pixel difference, per channel (R, G, B).
    pub(crate) max_abs_diff: [u8; 3],
    /// Fraction of pixels where any channel differs by more than
    /// [`COMPARE_DIFF_TOLERANCE`].
    pub(crate) differing_fraction: f32,
}

/// Computes statistics for an RGBA8 buffer. Alpha is ignored.
pub(crate) fn image_stats(
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<ImageStatsReport, String> {
    let expected = width as usize * height as usize * 4;
    if pixels.len() != expected {
        return Err(format!(
            "pixel buffer length {} does not match {width}x{height} RGBA ({expected})",
            pixels.len()
        ));
    }
    if width == 0 || height == 0 {
        return Err("image has zero dimensions".to_string());
    }

    let mut histograms = [[0u64; 256]; 4]; // r, g, b, luma
    let mut luma_histogram = vec![0u64; LUMA_HISTOGRAM_BINS];
    let mut saturation_sum = 0.0f64;

    for pixel in pixels.chunks_exact(4) {
        let (r, g, b) = (pixel[0], pixel[1], pixel[2]);
        histograms[0][r as usize] += 1;
        histograms[1][g as usize] += 1;
        histograms[2][b as usize] += 1;

        let luma = (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b))
            .round()
            .clamp(0.0, 255.0) as u8;
        histograms[3][luma as usize] += 1;
        luma_histogram[luma as usize / (256 / LUMA_HISTOGRAM_BINS)] += 1;

        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        if max > 0 {
            saturation_sum += f64::from(max - min) / f64::from(max);
        }
    }

    let total = u64::from(width) * u64::from(height);
    Ok(ImageStatsReport {
        width,
        height,
        r: channel_stats(&histograms[0], total),
        g: channel_stats(&histograms[1], total),
        b: channel_stats(&histograms[2], total),
        luma: channel_stats(&histograms[3], total),
        luma_histogram,
        mean_saturation: (saturation_sum / total as f64) as f32,
    })
}

/// Compares two same-sized RGBA8 buffers. Alpha is ignored.
pub(crate) fn compare_images(
    pixels_a: &[u8],
    width_a: u32,
    height_a: u32,
    pixels_b: &[u8],
    width_b: u32,
    height_b: u32,
) -> Result<ImageCompareReport, String> {
    if (width_a, height_a) != (width_b, height_b) {
        return Err(format!(
            "image dimensions differ: {width_a}x{height_a} vs {width_b}x{height_b}"
        ));
    }
    let expected = width_a as usize * height_a as usize * 4;
    if pixels_a.len() != expected || pixels_b.len() != expected {
        return Err("pixel buffer length does not match dimensions".to_string());
    }
    if width_a == 0 || height_a == 0 {
        return Err("image has zero dimensions".to_string());
    }

    let mut abs_diff_sum = [0u64; 3];
    let mut max_abs_diff = [0u8; 3];
    let mut differing = 0u64;

    for (a, b) in pixels_a.chunks_exact(4).zip(pixels_b.chunks_exact(4)) {
        let mut pixel_differs = false;
        for channel in 0..3 {
            let diff = a[channel].abs_diff(b[channel]);
            abs_diff_sum[channel] += u64::from(diff);
            max_abs_diff[channel] = max_abs_diff[channel].max(diff);
            if diff > COMPARE_DIFF_TOLERANCE {
                pixel_differs = true;
            }
        }
        if pixel_differs {
            differing += 1;
        }
    }

    let total = u64::from(width_a) * u64::from(height_a);
    Ok(ImageCompareReport {
        width: width_a,
        height: height_a,
        mean_abs_diff: abs_diff_sum.map(|sum| (sum as f64 / total as f64) as f32),
        max_abs_diff,
        differing_fraction: (differing as f64 / total as f64) as f32,
    })
}

fn channel_stats(histogram: &[u64; 256], total: u64) -> ChannelStats {
    let mean_sum: u64 = histogram
        .iter()
        .enumerate()
        .map(|(value, count)| value as u64 * count)
        .sum();
    ChannelStats {
        mean: (mean_sum as f64 / total as f64) as f32,
        min: percentile_from_histogram(histogram, total, 0.0),
        p1: percentile_from_histogram(histogram, total, 1.0),
        p5: percentile_from_histogram(histogram, total, 5.0),
        p50: percentile_from_histogram(histogram, total, 50.0),
        p95: percentile_from_histogram(histogram, total, 95.0),
        p99: percentile_from_histogram(histogram, total, 99.0),
        max: percentile_from_histogram(histogram, total, 100.0),
        clip_low_fraction: (histogram[0] as f64 / total as f64) as f32,
        clip_high_fraction: (histogram[255] as f64 / total as f64) as f32,
    }
}

/// Nearest-rank percentile over a 256-bin histogram: the smallest value whose
/// cumulative count reaches `ceil(p/100 * total)`, with rank floored at 1 so
/// `p = 0` yields the minimum. Exact for 8-bit data.
pub(crate) fn percentile_from_histogram(histogram: &[u64; 256], total: u64, percentile: f64) -> u8 {
    if total == 0 {
        return 0;
    }
    let rank = ((percentile / 100.0) * total as f64).ceil().max(1.0) as u64;
    let mut cumulative = 0u64;
    for (value, count) in histogram.iter().enumerate() {
        cumulative += count;
        if cumulative >= rank {
            return value as u8;
        }
    }
    255
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_rgba(r: u8, g: u8, b: u8, count: usize) -> Vec<u8> {
        [r, g, b, 255].repeat(count)
    }

    #[test]
    fn all_black_clips_low() {
        let stats = image_stats(&solid_rgba(0, 0, 0, 16), 4, 4).unwrap();
        assert_eq!(stats.r.mean, 0.0);
        assert_eq!(stats.r.min, 0);
        assert_eq!(stats.r.max, 0);
        assert_eq!(stats.r.clip_low_fraction, 1.0);
        assert_eq!(stats.r.clip_high_fraction, 0.0);
        assert_eq!(stats.luma.p50, 0);
        assert_eq!(stats.luma_histogram[0], 16);
        assert_eq!(stats.mean_saturation, 0.0);
    }

    #[test]
    fn all_white_clips_high() {
        let stats = image_stats(&solid_rgba(255, 255, 255, 16), 4, 4).unwrap();
        assert_eq!(stats.g.mean, 255.0);
        assert_eq!(stats.g.clip_high_fraction, 1.0);
        assert_eq!(stats.g.clip_low_fraction, 0.0);
        assert_eq!(stats.luma.p50, 255);
        assert_eq!(stats.luma_histogram[LUMA_HISTOGRAM_BINS - 1], 16);
        assert_eq!(stats.mean_saturation, 0.0);
    }

    #[test]
    fn gradient_percentiles_are_exact() {
        // 256 pixels: R takes each value 0..=255 exactly once (16x16).
        let mut pixels = Vec::with_capacity(256 * 4);
        for value in 0..=255u8 {
            pixels.extend_from_slice(&[value, 0, 0, 255]);
        }
        let stats = image_stats(&pixels, 16, 16).unwrap();
        assert_eq!(stats.r.min, 0);
        assert_eq!(stats.r.max, 255);
        // rank(p) = ceil(p/100 * 256); value = rank - 1 on this uniform ramp.
        assert_eq!(stats.r.p1, 2);
        assert_eq!(stats.r.p5, 12);
        assert_eq!(stats.r.p50, 127);
        assert_eq!(stats.r.p95, 243);
        assert_eq!(stats.r.p99, 253);
        assert!((stats.r.mean - 127.5).abs() < 1e-3);
        // 1/256 of pixels sit at 0 and at 255.
        assert!((stats.r.clip_low_fraction - 1.0 / 256.0).abs() < 1e-6);
        assert!((stats.r.clip_high_fraction - 1.0 / 256.0).abs() < 1e-6);
    }

    #[test]
    fn partial_clip_fractions() {
        let mut pixels = solid_rgba(0, 0, 0, 10);
        pixels.extend_from_slice(&solid_rgba(128, 128, 128, 90));
        let stats = image_stats(&pixels, 10, 10).unwrap();
        assert!((stats.b.clip_low_fraction - 0.1).abs() < 1e-6);
        assert_eq!(stats.b.clip_high_fraction, 0.0);
        assert_eq!(stats.b.min, 0);
        assert_eq!(stats.b.max, 128);
    }

    #[test]
    fn luma_uses_rec709_weights() {
        let stats = image_stats(&solid_rgba(255, 0, 0, 4), 2, 2).unwrap();
        assert_eq!(stats.luma.p50, 54); // round(0.2126 * 255)
        let stats = image_stats(&solid_rgba(0, 255, 0, 4), 2, 2).unwrap();
        assert_eq!(stats.luma.p50, 182); // round(0.7152 * 255)
        let stats = image_stats(&solid_rgba(0, 0, 255, 4), 2, 2).unwrap();
        assert_eq!(stats.luma.p50, 18); // round(0.0722 * 255)
    }

    #[test]
    fn saturation_mean_over_mixed_pixels() {
        // Pure red (saturation 1) + neutral gray (saturation 0).
        let mut pixels = solid_rgba(255, 0, 0, 2);
        pixels.extend_from_slice(&solid_rgba(128, 128, 128, 2));
        let stats = image_stats(&pixels, 2, 2).unwrap();
        assert!((stats.mean_saturation - 0.5).abs() < 1e-6);
    }

    #[test]
    fn histogram_bins_cover_range_and_sum_to_total() {
        let mut pixels = solid_rgba(0, 0, 0, 3);
        pixels.extend_from_slice(&solid_rgba(255, 255, 255, 5));
        let stats = image_stats(&pixels, 4, 2).unwrap();
        assert_eq!(stats.luma_histogram[0], 3);
        assert_eq!(stats.luma_histogram[LUMA_HISTOGRAM_BINS - 1], 5);
        assert_eq!(stats.luma_histogram.iter().sum::<u64>(), 8);
    }

    #[test]
    fn stats_rejects_bad_buffers() {
        assert!(image_stats(&[0, 0, 0], 1, 1).is_err());
        assert!(image_stats(&[], 0, 0).is_err());
    }

    #[test]
    fn compare_identical_is_zero() {
        let a = solid_rgba(10, 20, 30, 16);
        let report = compare_images(&a, 4, 4, &a, 4, 4).unwrap();
        assert_eq!(report.mean_abs_diff, [0.0; 3]);
        assert_eq!(report.max_abs_diff, [0; 3]);
        assert_eq!(report.differing_fraction, 0.0);
    }

    #[test]
    fn compare_detects_single_pixel_change() {
        let a = solid_rgba(10, 20, 30, 16);
        let mut b = a.clone();
        b[4 * 7 + 1] = 25; // pixel 7, green +5
        let report = compare_images(&a, 4, 4, &b, 4, 4).unwrap();
        assert_eq!(report.max_abs_diff, [0, 5, 0]);
        assert!((report.mean_abs_diff[1] - 5.0 / 16.0).abs() < 1e-6);
        assert!((report.differing_fraction - 1.0 / 16.0).abs() < 1e-6);
    }

    #[test]
    fn compare_tolerance_hides_codec_jitter() {
        let a = solid_rgba(10, 20, 30, 16);
        let mut b = a.clone();
        for pixel in b.chunks_exact_mut(4) {
            pixel[0] += COMPARE_DIFF_TOLERANCE; // within tolerance everywhere
        }
        let report = compare_images(&a, 4, 4, &b, 4, 4).unwrap();
        assert_eq!(report.differing_fraction, 0.0);
        assert_eq!(report.max_abs_diff[0], COMPARE_DIFF_TOLERANCE);
        assert!(report.mean_abs_diff[0] > 0.0);
    }

    #[test]
    fn compare_rejects_dimension_mismatch() {
        let a = solid_rgba(0, 0, 0, 16);
        let b = solid_rgba(0, 0, 0, 4);
        assert!(compare_images(&a, 4, 4, &b, 2, 2).is_err());
    }
}
