//! Decoded raster pixels to the linear, upright, 16-bit RGB a
//! `SourceImage` holds.

use image::DynamicImage;
use photo_core::colorimetry::RgbSpace;
use rayon::prelude::*;

use crate::error::DecodeError;
use crate::orient::Orientation;

/// One stored channel value.
trait Sample: Copy + Send + Sync {
    /// Number of distinct code values, which is also the size of a lookup
    /// table indexed by them. Zero for floating point, which needs none.
    const LEVELS: usize;

    fn index(self) -> usize;

    /// The value as a fraction of full scale, in 0..=1.
    fn unit(self) -> f32;
}

impl Sample for u8 {
    const LEVELS: usize = 1 << 8;

    #[inline]
    fn index(self) -> usize {
        usize::from(self)
    }

    #[inline]
    fn unit(self) -> f32 {
        f32::from(self) / 255.0
    }
}

impl Sample for u16 {
    const LEVELS: usize = 1 << 16;

    #[inline]
    fn index(self) -> usize {
        usize::from(self)
    }

    #[inline]
    fn unit(self) -> f32 {
        f32::from(self) / 65535.0
    }
}

impl Sample for f32 {
    const LEVELS: usize = 0;

    #[inline]
    fn index(self) -> usize {
        0
    }

    /// Clamped to 0..=1; a NaN counts as zero.
    #[inline]
    fn unit(self) -> f32 {
        if self > 0.0 { self.min(1.0) } else { 0.0 }
    }
}

/// How the channels of one stored pixel are laid out.
#[derive(Clone, Copy)]
struct Layout {
    /// Samples per pixel.
    channels: usize,
    /// One grey channel rather than three colour channels.
    grey: bool,
    /// The last sample is (straight, not premultiplied) alpha.
    alpha: bool,
}

const GREY: Layout = Layout::new(1, true, false);
const GREY_ALPHA: Layout = Layout::new(2, true, true);
const RGB: Layout = Layout::new(3, false, false);
const RGBA: Layout = Layout::new(4, false, true);

impl Layout {
    const fn new(channels: usize, grey: bool, alpha: bool) -> Self {
        Self {
            channels,
            grey,
            alpha,
        }
    }
}

/// Converts a decoded picture to interleaved linear RGB, 65535 = 1.0,
/// turned upright.
///
/// Integer samples are taken as encoded with `space`'s transfer function and
/// linearised with [`RgbSpace::decode`]; floating-point samples are taken as
/// linear light already and clamped to 0..=1. Grey is copied to all three
/// channels and alpha is composited over white in linear light.
pub(crate) fn to_linear_rgb(
    image: &DynamicImage,
    orientation: Orientation,
    space: RgbSpace,
) -> Result<Vec<u16>, DecodeError> {
    let (width, height) = (image.width() as usize, image.height() as usize);
    let convert = Conversion {
        width,
        height,
        orientation,
        space,
    };
    Ok(match image {
        DynamicImage::ImageLuma8(buffer) => convert.run(buffer.as_raw(), GREY),
        DynamicImage::ImageLumaA8(buffer) => convert.run(buffer.as_raw(), GREY_ALPHA),
        DynamicImage::ImageRgb8(buffer) => convert.run(buffer.as_raw(), RGB),
        DynamicImage::ImageRgba8(buffer) => convert.run(buffer.as_raw(), RGBA),
        DynamicImage::ImageLuma16(buffer) => convert.run(buffer.as_raw(), GREY),
        DynamicImage::ImageLumaA16(buffer) => convert.run(buffer.as_raw(), GREY_ALPHA),
        DynamicImage::ImageRgb16(buffer) => convert.run(buffer.as_raw(), RGB),
        DynamicImage::ImageRgba16(buffer) => convert.run(buffer.as_raw(), RGBA),
        DynamicImage::ImageRgb32F(buffer) => convert.run(buffer.as_raw(), RGB),
        DynamicImage::ImageRgba32F(buffer) => convert.run(buffer.as_raw(), RGBA),
        other => {
            let kind = other.color();
            return Err(DecodeError::Unsupported(format!("{kind:?} pixel data")));
        }
    })
}

struct Conversion {
    /// Stored width, before the orientation is applied.
    width: usize,
    /// Stored height, before the orientation is applied.
    height: usize,
    orientation: Orientation,
    space: RgbSpace,
}

impl Conversion {
    fn run<T: Sample>(&self, samples: &[T], layout: Layout) -> Vec<u16> {
        let (out_width, out_height) = self.orientation.upright_size(self.width, self.height);
        let mut out = vec![0u16; out_width * out_height * 3];
        if out.is_empty() {
            return out;
        }
        // Linear value of every code, as a float for blending and already
        // quantized for the common case where no blending is needed.
        let top_code = T::LEVELS.saturating_sub(1) as f32;
        let linear: Vec<f32> = (0..T::LEVELS)
            .map(|code| self.space.decode(code as f32 / top_code))
            .collect();
        let quantized: Vec<u16> = linear.iter().map(|&value| quantize(value)).collect();
        let to_linear = |sample: T| {
            if T::LEVELS == 0 {
                sample.unit()
            } else {
                linear[sample.index()]
            }
        };
        let to_quantized = |sample: T| {
            if T::LEVELS == 0 {
                quantize(sample.unit())
            } else {
                quantized[sample.index()]
            }
        };

        let colour_channels = if layout.grey { 1 } else { 3 };
        out.par_chunks_mut(out_width * 3)
            .enumerate()
            .for_each(|(y, row)| {
                for (x, pixel) in row.chunks_exact_mut(3).enumerate() {
                    let (stored_x, stored_y) =
                        self.orientation
                            .stored_position(x, y, self.width, self.height);
                    let at = (stored_y * self.width + stored_x) * layout.channels;
                    let stored = &samples[at..at + layout.channels];
                    let coverage = if layout.alpha {
                        stored[layout.channels - 1].unit()
                    } else {
                        1.0
                    };
                    for (channel, value) in pixel.iter_mut().enumerate() {
                        let sample = stored[channel.min(colour_channels - 1)];
                        *value = if coverage >= 1.0 {
                            to_quantized(sample)
                        } else {
                            // Over white, in linear light.
                            quantize(to_linear(sample) * coverage + (1.0 - coverage))
                        };
                    }
                }
            });
        out
    }
}

/// A linear value in 0..=1 as a 16-bit code, rounded to nearest.
#[inline]
fn quantize(linear: f32) -> u16 {
    (linear * 65535.0).round() as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, ImageBuffer, Luma, Rgb, RgbImage, Rgba, RgbaImage};

    fn convert(image: impl Into<DynamicImage>, orientation: u32, space: RgbSpace) -> Vec<u16> {
        to_linear_rgb(&image.into(), Orientation::from_exif(orientation), space).unwrap()
    }

    #[test]
    fn every_eight_bit_code_is_linearised_with_the_space_transfer_function() {
        let ramp = GrayImage::from_fn(256, 1, |x, _| Luma([x as u8]));
        for space in RgbSpace::ALL {
            let linear = convert(ramp.clone(), 1, space);
            for code in 0..256usize {
                let expected = (space.decode(code as f32 / 255.0) * 65535.0).round() as u16;
                assert_eq!(
                    linear[code * 3..code * 3 + 3],
                    [expected; 3],
                    "{space:?} code {code}"
                );
            }
            assert_eq!(linear[0], 0);
            assert_eq!(linear[255 * 3], 65535);
        }
    }

    #[test]
    fn sixteen_bit_codes_use_all_sixteen_bits() {
        let image: ImageBuffer<Rgb<u16>, Vec<u16>> = ImageBuffer::from_fn(4, 1, |x, _| {
            Rgb([x as u16 * 3, 32768 + x as u16, 65535 - x as u16])
        });
        let linear = convert(image.clone(), 1, RgbSpace::Srgb);
        for (got, code) in linear.iter().zip(image.as_raw()) {
            let want = (RgbSpace::Srgb.decode(f32::from(*code) / 65535.0) * 65535.0).round() as u16;
            assert_eq!(*got, want);
        }
        // Neighbouring 16-bit codes in the midtones stay distinguishable.
        assert_ne!(linear[1], linear[4]);
    }

    #[test]
    fn orientation_moves_pixels_not_just_dimensions() {
        // Stored 2 x 1: red then blue. Turned a quarter clockwise the picture
        // is 1 x 2 with red on top.
        let stored = RgbImage::from_fn(2, 1, |x, _| {
            Rgb(if x == 0 { [255, 0, 0] } else { [0, 0, 255] })
        });
        let red = [65535u16, 0, 0];
        let blue = [0u16, 0, 65535];
        assert_eq!(
            convert(stored.clone(), 1, RgbSpace::Srgb),
            [red, blue].concat()
        );
        assert_eq!(
            convert(stored.clone(), 2, RgbSpace::Srgb),
            [blue, red].concat()
        );
        assert_eq!(
            convert(stored.clone(), 6, RgbSpace::Srgb),
            [red, blue].concat()
        );
        assert_eq!(convert(stored, 8, RgbSpace::Srgb), [blue, red].concat());
    }

    #[test]
    fn transparency_blends_toward_white_in_linear_light() {
        let image = RgbaImage::from_fn(3, 1, |x, _| Rgba([0, 0, 0, [255, 128, 0][x as usize]]));
        let linear = convert(image, 1, RgbSpace::Srgb);
        assert_eq!(linear[0..3], [0; 3]);
        // Half-covered black over white is half of linear white, which is
        // much brighter than the sRGB code 128 it would be if blended encoded.
        let half = (1.0 - 128.0 / 255.0) * 65535.0;
        assert!((f32::from(linear[3]) - half).abs() <= 1.0, "{}", linear[3]);
        assert_eq!(linear[6..9], [65535; 3]);
    }

    #[test]
    fn floats_are_clamped_and_nan_is_black() {
        let values = [f32::NAN, -1.0, 0.25, 7.0, f32::INFINITY, f32::NEG_INFINITY];
        let image: ImageBuffer<Rgb<f32>, Vec<f32>> =
            ImageBuffer::from_fn(values.len() as u32, 1, |x, _| Rgb([values[x as usize]; 3]));
        let linear = convert(DynamicImage::ImageRgb32F(image), 1, RgbSpace::AdobeRgb);
        let expected = [0u16, 0, 16384, 65535, 65535, 0];
        for (index, want) in expected.into_iter().enumerate() {
            assert_eq!(linear[index * 3], want, "sample {}", values[index]);
        }
    }

    #[test]
    fn an_empty_image_gives_an_empty_buffer() {
        assert!(convert(RgbImage::new(0, 0), 1, RgbSpace::Srgb).is_empty());
    }
}
