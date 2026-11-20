//! Pixel containers passed between crates.
//!
//! A [`SourceImage`] is what a decoder produces and the develop pipeline
//! consumes: linear light, never tone-mapped, never 8-bit. A [`Rgba8Image`] or
//! [`Rgb16Image`] is what the pipeline hands to the screen or an encoder.

use crate::colorimetry::{Mat3, RgbSpace};

/// Decoded source pixels: interleaved RGB, linear light, `u16` where
/// [`SourceImage::WHITE`] is 1.0. Already rotated upright per the file's EXIF
/// orientation, so `width`/`height` are what a viewer would call them.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u16>,
    pub color: SourceColor,
}

impl SourceImage {
    /// The `u16` code value that represents linear 1.0.
    pub const WHITE: u16 = u16::MAX;
    pub const CHANNELS: usize = 3;

    /// Build an image, checking the buffer length against the dimensions.
    pub fn new(width: u32, height: u32, pixels: Vec<u16>, color: SourceColor) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(Self::CHANNELS)?;
        (width > 0 && height > 0 && pixels.len() == expected).then_some(Self {
            width,
            height,
            pixels,
            color,
        })
    }

    /// The pixel at `(x, y)`. Panics when out of bounds.
    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> [u16; 3] {
        let i = (y as usize * self.width as usize + x as usize) * Self::CHANNELS;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]]
    }

    pub fn long_edge(&self) -> u32 {
        self.width.max(self.height)
    }
}

/// What the numbers in a [`SourceImage`] mean.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SourceColor {
    /// Linear RGB in a known colour space. Raster files (JPEG, PNG, TIFF...).
    /// The camera's or editor's rendering is already baked in.
    Rgb(RgbSpace),
    /// Demosaiced camera-native RGB: black-subtracted, scaled so the sensor's
    /// white level is 1.0, and deliberately not white balanced. Camera RAW.
    Camera(CameraColor),
}

impl SourceColor {
    pub fn is_camera(&self) -> bool {
        matches!(self, Self::Camera(_))
    }
}

/// Colour description of a camera-native [`SourceImage`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraColor {
    /// XYZ (D65) to camera-native RGB, as in a DNG `ColorMatrix` for D65.
    pub xyz_to_camera: Mat3,
    /// The camera's as-shot white balance as channel multipliers with green
    /// normalized to 1.0.
    pub as_shot_gains: [f32; 3],
    /// Exposure offset in stops the file asks a renderer to apply (DNG
    /// `BaselineExposure`), 0.0 when the file does not say.
    pub baseline_exposure: f32,
}

/// Display-ready pixels: interleaved RGBA, 8 bits, encoded in `space`,
/// opaque alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct Rgba8Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub space: RgbSpace,
}

impl Rgba8Image {
    pub const CHANNELS: usize = 4;

    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * Self::CHANNELS;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }
}

/// Export-ready pixels: interleaved RGB, 16 bits, encoded in `space`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rgb16Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u16>,
    pub space: RgbSpace,
}

impl Rgb16Image {
    pub const CHANNELS: usize = 3;

    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> [u16; 3] {
        let i = (y as usize * self.width as usize + x as usize) * Self::CHANNELS;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_a_buffer_that_does_not_match_the_dimensions() {
        let color = SourceColor::Rgb(RgbSpace::Srgb);
        assert!(SourceImage::new(2, 2, vec![0; 12], color).is_some());
        assert!(SourceImage::new(2, 2, vec![0; 11], color).is_none());
        assert!(SourceImage::new(0, 2, vec![], color).is_none());
    }

    #[test]
    fn pixel_reads_interleaved_rgb_in_row_major_order() {
        let pixels = (0..12).collect();
        let image = SourceImage::new(2, 2, pixels, SourceColor::Rgb(RgbSpace::Srgb)).unwrap();
        assert_eq!(image.pixel(1, 0), [3, 4, 5]);
        assert_eq!(image.pixel(0, 1), [6, 7, 8]);
    }
}
