//! The mip pyramid of a decoded source.

use std::sync::Arc;

use photo_core::image::SourceImage;
use rayon::prelude::*;

use crate::sample;

/// A source image and its successively halved copies.
///
/// Level 0 is the image itself, shared rather than copied (a 60 MP source is
/// 360 MB). Level `n + 1` is level `n` averaged over 2 x 2 blocks in linear
/// light and stored as `u16` again, so the whole pyramid costs a third more
/// memory than the source. Levels are built until the long edge is at most
/// [`Self::SMALLEST_LONG_EDGE`].
///
/// A level's width and height are those of the level above divided by two and
/// rounded **up**: an odd last column or row is kept (averaged with itself)
/// instead of dropped. Pixel `(x, y)` of level `n` therefore always covers
/// source pixels `x * 2^n .. (x + 1) * 2^n`, and a source coordinate maps to
/// level `n` by dividing by `2^n`.
///
/// Cloning is cheap: levels are reference counted.
#[derive(Clone, Debug)]
pub struct SourcePyramid {
    levels: Vec<Arc<SourceImage>>,
}

impl SourcePyramid {
    /// No level is built from one whose long edge is already this small.
    pub const SMALLEST_LONG_EDGE: u32 = 64;

    /// Build the pyramid, taking ownership of `source` or sharing it when it
    /// is already in an [`Arc`]. Level 0 is never copied.
    ///
    /// # Panics
    ///
    /// When `source.pixels` does not hold exactly `width * height * 3`
    /// samples, or either dimension is zero. [`SourceImage::new`] never
    /// produces such an image.
    pub fn new(source: impl Into<Arc<SourceImage>>) -> Self {
        let source = source.into();
        let samples = source.width as usize * source.height as usize * SourceImage::CHANNELS;
        assert!(
            samples > 0 && source.pixels.len() == samples,
            "source image buffer does not match its {}x{} dimensions",
            source.width,
            source.height
        );
        let mut levels = vec![source];
        loop {
            let last = &levels[levels.len() - 1];
            if last.long_edge() <= Self::SMALLEST_LONG_EDGE {
                break;
            }
            let next = halve(last);
            levels.push(Arc::new(next));
        }
        Self { levels }
    }

    /// Number of levels, at least 1.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// The image at `level` (0 is the source). Panics when `level` is not
    /// below [`Self::level_count`].
    pub fn level(&self, level: usize) -> &Arc<SourceImage> {
        &self.levels[level]
    }

    /// Width and height of level 0, in pixels.
    pub fn source_size(&self) -> (u32, u32) {
        (self.levels[0].width, self.levels[0].height)
    }

    /// A pyramid that shares every level with this one except `level`, which
    /// is replaced by `image`: how a healed level is put back for
    /// resampling. `None` when `level` does not exist or `image` has
    /// different dimensions (or a buffer that does not match them).
    pub fn with_level(&self, level: usize, image: Arc<SourceImage>) -> Option<Self> {
        let current = self.levels.get(level)?;
        let fits = image.width == current.width
            && image.height == current.height
            && image.pixels.len() == current.pixels.len();
        fits.then(|| {
            let mut levels = self.levels.clone();
            levels[level] = image;
            Self { levels }
        })
    }

    /// Bilinear sample of `level` at `(x, y)`, in that level's pixels with
    /// pixel centres at `+0.5`. Returns linear RGB in `0..=1`. Positions
    /// outside the image read the nearest edge pixel; a pixel centre returns
    /// that pixel exactly. Support: the 2 x 2 pixels around the position.
    ///
    /// Panics when `level` is not below [`Self::level_count`].
    #[inline]
    pub fn sample(&self, level: usize, x: f32, y: f32) -> [f32; 3] {
        sample::bilinear(&self.levels[level], f64::from(x), f64::from(y))
    }
}

/// Average 2 x 2 blocks, rounding to nearest. An odd last column or row is
/// averaged with itself.
fn halve(source: &SourceImage) -> SourceImage {
    const CHANNELS: usize = SourceImage::CHANNELS;
    let (source_width, source_height) = (source.width as usize, source.height as usize);
    let (width, height) = (source_width.div_ceil(2), source_height.div_ceil(2));
    let stride = source_width * CHANNELS;
    let mut pixels = vec![0u16; width * height * CHANNELS];
    pixels
        .par_chunks_exact_mut(width * CHANNELS)
        .enumerate()
        .for_each(|(y, out)| {
            let top = &source.pixels[2 * y * stride..][..stride];
            let bottom = &source.pixels[(2 * y + 1).min(source_height - 1) * stride..][..stride];
            for (x, pixel) in out.chunks_exact_mut(CHANNELS).enumerate() {
                let left = 2 * x * CHANNELS;
                let right = (2 * x + 1).min(source_width - 1) * CHANNELS;
                for (channel, value) in pixel.iter_mut().enumerate() {
                    let sum = u32::from(top[left + channel])
                        + u32::from(top[right + channel])
                        + u32::from(bottom[left + channel])
                        + u32::from(bottom[right + channel]);
                    *value = ((sum + 2) / 4) as u16;
                }
            }
        });
    SourceImage {
        width: width as u32,
        height: height as u32,
        pixels,
        color: source.color,
    }
}

#[cfg(test)]
mod tests {
    use photo_core::colorimetry::RgbSpace;
    use photo_core::image::SourceColor;

    use super::*;

    fn flat(width: u32, height: u32, value: u16) -> SourceImage {
        let pixels = vec![value; width as usize * height as usize * 3];
        SourceImage::new(width, height, pixels, SourceColor::Rgb(RgbSpace::Rec2020)).unwrap()
    }

    #[test]
    fn a_flat_image_stays_flat_at_every_level() {
        let pyramid = SourcePyramid::new(flat(300, 77, 12345));
        assert!(pyramid.level_count() > 2);
        for level in 0..pyramid.level_count() {
            let image = pyramid.level(level);
            assert!(image.pixels.iter().all(|&v| v == 12345), "level {level}");
            assert_eq!(image.color, SourceColor::Rgb(RgbSpace::Rec2020));
            assert_eq!(
                image.pixels.len(),
                image.width as usize * image.height as usize * 3
            );
        }
    }

    #[test]
    fn a_very_thin_image_never_reaches_zero_height() {
        let pyramid = SourcePyramid::new(flat(1000, 3, 9));
        let last = pyramid.level(pyramid.level_count() - 1);
        assert_eq!(last.height, 1);
        assert!(last.width <= SourcePyramid::SMALLEST_LONG_EDGE);
    }

    #[test]
    #[should_panic(expected = "does not match")]
    fn a_malformed_source_is_rejected_up_front() {
        let mut image = flat(4, 4, 0);
        image.pixels.pop();
        SourcePyramid::new(image);
    }
}
