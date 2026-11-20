//! Small display-ready pictures: scale down, flatten, convert to sRGB and
//! turn upright.

use image::{DynamicImage, Rgba, RgbaImage};
use photo_core::colorimetry::{RgbSpace, mat3_apply, rgb_to_rgb};
use photo_core::image::Rgba8Image;
use rayon::prelude::*;

use crate::orient::Orientation;

/// Turns a decoded picture into an upright, opaque, sRGB-encoded RGBA8 image
/// whose longer edge is at most `max_edge` pixels (at least one pixel; a
/// picture already that small is not enlarged).
///
/// `space` says what the stored values mean: integer samples are encoded in
/// it and floating-point samples are linear light with its primaries.
/// Transparency is flattened onto white in linear light.
pub(crate) fn to_preview(
    image: DynamicImage,
    orientation: Orientation,
    space: RgbSpace,
    max_edge: u32,
) -> Rgba8Image {
    let (width, height) = fit(image.width(), image.height(), max_edge);
    // Scale first: everything after this touches only the few pixels kept.
    let small = if (width, height) == (image.width(), image.height()) {
        image
    } else {
        image.thumbnail_exact(width, height)
    };
    let stored = encoded_rgba8(small, space);
    let (stored_width, stored_height) = (width as usize, height as usize);
    let (out_width, out_height) = orientation.upright_size(stored_width, stored_height);

    let flatten = Flatten::new(space);
    let mut pixels = vec![0u8; out_width * out_height * 4];
    if !pixels.is_empty() {
        pixels
            .par_chunks_mut(out_width * 4)
            .enumerate()
            .for_each(|(y, row)| {
                for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
                    let (stored_x, stored_y) =
                        orientation.stored_position(x, y, stored_width, stored_height);
                    let at = (stored_y * stored_width + stored_x) * 4;
                    let source = &stored.as_raw()[at..at + 4];
                    pixel.copy_from_slice(
                        &flatten.apply([source[0], source[1], source[2], source[3]]),
                    );
                }
            });
    }
    Rgba8Image {
        width: out_width as u32,
        height: out_height as u32,
        pixels,
        space: RgbSpace::Srgb,
    }
}

/// The size `width` x `height` scales to so that its longer edge is at most
/// `max_edge`, keeping the aspect ratio. Never larger than the original and
/// never smaller than one pixel on either side.
fn fit(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let long_edge = width.max(height);
    let target = max_edge.max(1);
    if long_edge <= target {
        return (width, height);
    }
    let scaled = |side: u32| {
        let exact =
            (u64::from(side) * u64::from(target) + u64::from(long_edge) / 2) / u64::from(long_edge);
        (exact as u32).max(1)
    };
    (scaled(width), scaled(height))
}

/// The picture as 8-bit RGBA whose colour channels are encoded in `space`.
fn encoded_rgba8(image: DynamicImage, space: RgbSpace) -> RgbaImage {
    match image {
        DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_) => {
            let linear = image.into_rgba32f();
            let to_code = |unit: f32| (unit.clamp(0.0, 1.0) * 255.0).round() as u8;
            RgbaImage::from_fn(linear.width(), linear.height(), |x, y| {
                let [r, g, b, a] = linear.get_pixel(x, y).0;
                let encode = |value: f32| to_code(space.encode(value.clamp(0.0, 1.0)));
                Rgba([encode(r), encode(g), encode(b), to_code(a)])
            })
        }
        other => other.into_rgba8(),
    }
}

/// Per-pixel conversion from 8-bit RGBA encoded in some space to opaque
/// sRGB.
struct Flatten {
    /// Linear value of each 8-bit code in the source space.
    linear: [f32; 256],
    /// Source primaries to sRGB primaries; `None` when they are the same.
    to_srgb: Option<[[f32; 3]; 3]>,
}

impl Flatten {
    fn new(space: RgbSpace) -> Self {
        Self {
            linear: std::array::from_fn(|code| space.decode(code as f32 / 255.0)),
            to_srgb: (space != RgbSpace::Srgb).then(|| rgb_to_rgb(space, RgbSpace::Srgb)),
        }
    }

    #[inline]
    fn apply(&self, [r, g, b, a]: [u8; 4]) -> [u8; 4] {
        if a == u8::MAX && self.to_srgb.is_none() {
            return [r, g, b, u8::MAX];
        }
        let coverage = f32::from(a) / 255.0;
        // Over white, in linear light.
        let mut colour =
            [r, g, b].map(|code| self.linear[usize::from(code)] * coverage + (1.0 - coverage));
        if let Some(matrix) = &self.to_srgb {
            colour = mat3_apply(matrix, colour);
        }
        let [r, g, b] = colour
            .map(|value| (RgbSpace::Srgb.encode(value.clamp(0.0, 1.0)) * 255.0).round() as u8);
        [r, g, b, u8::MAX]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb, RgbImage};

    #[test]
    fn fitting_keeps_the_aspect_and_never_enlarges() {
        assert_eq!(fit(320, 240, 160), (160, 120));
        assert_eq!(fit(240, 320, 160), (120, 160));
        assert_eq!(fit(320, 240, 100), (100, 75));
        assert_eq!(fit(320, 240, 320), (320, 240));
        assert_eq!(fit(320, 240, 5000), (320, 240));
        assert_eq!(fit(320, 240, u32::MAX), (320, 240));
        assert_eq!(fit(u32::MAX, 3, 1000), (1000, 1));
    }

    #[test]
    fn nothing_collapses_to_zero_pixels() {
        assert_eq!(fit(320, 240, 0), (1, 1));
        assert_eq!(fit(1000, 1, 10), (10, 1));
        assert_eq!(fit(1, 1, 0), (1, 1));
        assert_eq!(fit(3, 1000, 2), (1, 2));
    }

    #[test]
    fn opaque_srgb_pixels_pass_through_untouched() {
        let image = RgbImage::from_fn(4, 2, |x, y| Rgb([x as u8 * 60, y as u8 * 200, 17]));
        let preview = to_preview(
            image.clone().into(),
            Orientation::UPRIGHT,
            RgbSpace::Srgb,
            64,
        );
        assert_eq!((preview.width, preview.height), (4, 2));
        assert_eq!(preview.space, RgbSpace::Srgb);
        for (got, want) in preview.pixels.chunks_exact(4).zip(image.pixels()) {
            assert_eq!(got, [want.0[0], want.0[1], want.0[2], 255]);
        }
    }

    #[test]
    fn a_quarter_turn_swaps_the_sides_after_scaling() {
        let image = RgbImage::from_fn(40, 20, |x, _| {
            Rgb(if x < 20 { [255, 0, 0] } else { [0, 0, 255] })
        });
        let preview = to_preview(image.into(), Orientation::from_exif(6), RgbSpace::Srgb, 10);
        assert_eq!((preview.width, preview.height), (5, 10));
        assert_eq!(
            preview.pixel(2, 1),
            [255, 0, 0, 255],
            "the stored left ends up on top"
        );
        assert_eq!(preview.pixel(2, 8), [0, 0, 255, 255]);
    }

    #[test]
    fn wide_gamut_values_are_converted_and_clipped_into_srgb() {
        // Pure Display P3 red is outside sRGB: red clips, green and blue hit zero.
        let image = RgbImage::from_pixel(2, 2, Rgb([255, 0, 0]));
        let preview = to_preview(image.into(), Orientation::UPRIGHT, RgbSpace::DisplayP3, 8);
        assert_eq!(preview.pixel(0, 0), [255, 0, 0, 255]);
        // A grey stays the same grey: both spaces share white and transfer.
        let grey = RgbImage::from_pixel(2, 2, Rgb([128, 128, 128]));
        let preview = to_preview(grey.into(), Orientation::UPRIGHT, RgbSpace::DisplayP3, 8);
        assert_eq!(preview.pixel(1, 1), [128, 128, 128, 255]);
        // A dark Adobe RGB grey is darker than the same code in sRGB, whose
        // curve has a linear toe where Adobe's is a pure power.
        let grey = RgbImage::from_pixel(2, 2, Rgb([20, 20, 20]));
        let preview = to_preview(grey.into(), Orientation::UPRIGHT, RgbSpace::AdobeRgb, 8);
        let [r, g, b, _] = preview.pixel(0, 0);
        assert!(
            r == g && g == b && (10..=14).contains(&r),
            "{:?}",
            [r, g, b]
        );
    }

    #[test]
    fn linear_float_pictures_are_encoded_for_display() {
        let image: ImageBuffer<Rgb<f32>, Vec<f32>> =
            ImageBuffer::from_pixel(2, 2, Rgb([0.18, 0.18, 0.18]));
        let preview = to_preview(
            DynamicImage::ImageRgb32F(image),
            Orientation::UPRIGHT,
            RgbSpace::Srgb,
            8,
        );
        let [r, g, b, a] = preview.pixel(0, 0);
        assert_eq!((r, g, b, a), (118, 118, 118, 255), "18 % grey is sRGB 118");
    }

    #[test]
    fn an_empty_picture_gives_an_empty_preview() {
        let preview = to_preview(
            RgbImage::new(0, 0).into(),
            Orientation::UPRIGHT,
            RgbSpace::Srgb,
            8,
        );
        assert_eq!((preview.width, preview.height), (0, 0));
        assert!(preview.pixels.is_empty());
    }
}
