//! Area-average downscaling of display-ready images, for thumbnails.

use photo_core::colorimetry::RgbSpace;
use photo_core::image::Rgba8Image;
use rayon::prelude::*;

const CHANNELS: usize = Rgba8Image::CHANNELS;

/// For every output position along one axis: the first input position it
/// overlaps and the weights of the input positions from there on. Weights
/// are the fraction of the output pixel's footprint each input pixel
/// covers, and sum to 1.
struct Coverage {
    starts: Vec<usize>,
    /// `weights[offsets[i]..offsets[i + 1]]` belongs to output position `i`.
    offsets: Vec<usize>,
    weights: Vec<f32>,
}

impl Coverage {
    fn new(input: usize, output: usize) -> Self {
        let scale = input as f64 / output as f64;
        let mut coverage = Self {
            starts: Vec::with_capacity(output),
            offsets: Vec::with_capacity(output + 1),
            weights: Vec::new(),
        };
        coverage.offsets.push(0);
        for index in 0..output {
            let (low, high) = (
                index as f64 * scale,
                ((index + 1) as f64 * scale).min(input as f64),
            );
            let first = (low.floor() as usize).min(input - 1);
            let last = ((high.ceil() as usize).max(first + 1)).min(input);
            let begin = coverage.weights.len();
            for position in first..last {
                let overlap = (high.min(position as f64 + 1.0) - low.max(position as f64)).max(0.0);
                coverage.weights.push(overlap as f32);
            }
            let total: f32 = coverage.weights[begin..].iter().sum();
            if total > 0.0 {
                coverage.weights[begin..]
                    .iter_mut()
                    .for_each(|w| *w /= total);
            } else {
                // Rounding left nothing: fall back to the pixel underneath.
                coverage.weights.truncate(begin);
                coverage.weights.push(1.0);
            }
            coverage.starts.push(first);
            coverage.offsets.push(coverage.weights.len());
        }
        coverage
    }

    fn span(&self, index: usize) -> (usize, &[f32]) {
        (
            self.starts[index],
            &self.weights[self.offsets[index]..self.offsets[index + 1]],
        )
    }
}

/// Code value to linear light, per the space's transfer function. Alpha is
/// linear already.
struct Codec {
    decode: [f32; 256],
    /// `thresholds[c]` is the linear value halfway, in encoded terms,
    /// between codes `c` and `c + 1`.
    thresholds: [f32; 255],
}

impl Codec {
    fn new(space: RgbSpace) -> Self {
        Self {
            decode: std::array::from_fn(|code| space.decode(code as f32 / 255.0)),
            thresholds: std::array::from_fn(|code| space.decode((code as f32 + 0.5) / 255.0)),
        }
    }

    /// The code whose encoded value is nearest to that of `linear`.
    fn encode(&self, linear: f32) -> u8 {
        self.thresholds
            .partition_point(|&threshold| threshold <= linear) as u8
    }
}

/// Nearest code for a linear `0..=1` alpha.
fn encode_alpha(alpha: f32) -> u8 {
    (alpha * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// Resize `image` to `width` x `height` by averaging, in linear light, the
/// area of the input each output pixel covers.
///
/// Meant for making thumbnails and small previews from a rendered image.
/// Each output pixel is the mean of the input pixels under it, weighted by
/// how much of each it covers (so any ratio is handled, not only whole
/// ones), computed on linear-light values decoded with the image's own
/// transfer function and encoded back. Fine bright detail therefore keeps
/// its brightness instead of darkening as it would if code values were
/// averaged. Alpha is averaged as it is, without premultiplication, which
/// is exact for the opaque images this type carries.
///
/// Guarantees: the result is exactly `width` x `height` in the input's
/// colour space; a flat image stays the same flat colour; asking for the
/// input's own size returns an identical copy; the result is the same on
/// any number of threads. Asking for a larger size than the input repeats
/// pixels, blending two where an output pixel straddles them (a box filter
/// cannot do better); it does not fail.
///
/// A zero `width` or `height`, or an empty input, gives an image with no
/// pixels.
///
/// # Panics
///
/// When `image.pixels` does not match its dimensions.
pub fn downscale_rgba8(image: &Rgba8Image, width: u32, height: u32) -> Rgba8Image {
    let (source_w, source_h) = (image.width as usize, image.height as usize);
    assert_eq!(
        image.pixels.len(),
        source_w * source_h * CHANNELS,
        "image buffer does not match its dimensions"
    );
    let (out_w, out_h) = (width as usize, height as usize);
    let mut out = Rgba8Image {
        width,
        height,
        pixels: Vec::new(),
        space: image.space,
    };
    if source_w == 0 || source_h == 0 || out_w == 0 || out_h == 0 {
        return out;
    }
    let codec = Codec::new(image.space);
    let columns = Coverage::new(source_w, out_w);
    let rows = Coverage::new(source_h, out_h);
    let stride = source_w * CHANNELS;

    out.pixels = vec![0u8; out_w * out_h * CHANNELS];
    out.pixels
        .par_chunks_exact_mut(out_w * CHANNELS)
        .enumerate()
        .for_each_init(
            || vec![[0.0f32; CHANNELS]; out_w],
            |sums, (y, out_row)| {
                sums.fill([0.0; CHANNELS]);
                let (first_row, row_weights) = rows.span(y);
                for (row_offset, &row_weight) in row_weights.iter().enumerate() {
                    let source = &image.pixels[(first_row + row_offset) * stride..][..stride];
                    for (x, sum) in sums.iter_mut().enumerate() {
                        let (first_column, column_weights) = columns.span(x);
                        let pixels = source[first_column * CHANNELS..].chunks_exact(CHANNELS);
                        for (pixel, &column_weight) in pixels.zip(column_weights) {
                            let weight = row_weight * column_weight;
                            sum[0] += weight * codec.decode[usize::from(pixel[0])];
                            sum[1] += weight * codec.decode[usize::from(pixel[1])];
                            sum[2] += weight * codec.decode[usize::from(pixel[2])];
                            sum[3] += weight * f32::from(pixel[3]) / 255.0;
                        }
                    }
                }
                for (pixel, sum) in out_row.chunks_exact_mut(CHANNELS).zip(sums.iter()) {
                    pixel[0] = codec.encode(sum[0]);
                    pixel[1] = codec.encode(sum[1]);
                    pixel[2] = codec.encode(sum[2]);
                    pixel[3] = encode_alpha(sum[3]);
                }
            },
        );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_weights_are_the_overlap_fractions() {
        let coverage = Coverage::new(5, 2);
        assert_eq!(coverage.span(0), (0, &[0.4, 0.4, 0.2][..]));
        assert_eq!(coverage.span(1), (2, &[0.2, 0.4, 0.4][..]));
        let whole = Coverage::new(6, 3);
        assert_eq!(whole.span(2), (4, &[0.5, 0.5][..]));
        let same = Coverage::new(4, 4);
        assert_eq!(same.span(3), (3, &[1.0][..]));
        let enlarging = Coverage::new(2, 5);
        assert_eq!(enlarging.span(2), (0, &[0.5, 0.5][..]));
        assert_eq!(enlarging.span(4), (1, &[1.0][..]));
    }

    #[test]
    fn every_code_survives_a_decode_and_encode_in_every_space() {
        for space in RgbSpace::ALL {
            let codec = Codec::new(space);
            for code in 0..=255u8 {
                assert_eq!(
                    codec.encode(codec.decode[usize::from(code)]),
                    code,
                    "{space:?}"
                );
            }
            assert_eq!(codec.encode(-1.0), 0);
            assert_eq!(codec.encode(7.0), 255);
        }
        assert_eq!(encode_alpha(1.0), 255);
        assert_eq!(encode_alpha(0.0), 0);
        assert_eq!(encode_alpha(127.6 / 255.0), 128);
    }
}
