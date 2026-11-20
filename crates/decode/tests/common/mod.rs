//! Helpers shared by the integration tests.
#![allow(dead_code)]

use photo_core::colorimetry::RgbSpace;
use photo_core::image::SourceImage;

/// What a correct decoder stores for an 8-bit code value in `space`.
pub fn linear_from_8(space: RgbSpace, code: u8) -> u16 {
    (space.decode(f32::from(code) / 255.0) * 65535.0).round() as u16
}

/// What a correct decoder stores for a 16-bit code value in `space`.
pub fn linear_from_16(space: RgbSpace, code: u16) -> u16 {
    (space.decode(f32::from(code) / 65535.0) * 65535.0).round() as u16
}

/// An 8-bit picture linearised the way the decoder must, as interleaved RGB.
pub fn linearised_8(image: &image::RgbImage, space: RgbSpace) -> Vec<u16> {
    image
        .as_raw()
        .iter()
        .map(|&code| linear_from_8(space, code))
        .collect()
}

/// A 16-bit picture linearised the way the decoder must, as interleaved RGB.
pub fn linearised_16(
    image: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    space: RgbSpace,
) -> Vec<u16> {
    image
        .as_raw()
        .iter()
        .map(|&code| linear_from_16(space, code))
        .collect()
}

/// The decoded pixel as linear floats in 0..1.
pub fn linear_pixel(image: &SourceImage, x: u32, y: u32) -> [f32; 3] {
    image.pixel(x, y).map(|v| f32::from(v) / 65535.0)
}

/// Mean of a rectangle of the decoded image, as linear floats in 0..1.
pub fn mean(image: &SourceImage, x: u32, y: u32, width: u32, height: u32) -> [f32; 3] {
    assert!(width > 0 && height > 0, "empty rectangle");
    let mut sum = [0.0f64; 3];
    for row in y..y + height {
        for column in x..x + width {
            for (total, value) in sum.iter_mut().zip(image.pixel(column, row)) {
                *total += f64::from(value);
            }
        }
    }
    let count = f64::from(width) * f64::from(height) * 65535.0;
    sum.map(|total| (total / count) as f32)
}

/// Mean absolute difference between two equally long sample buffers, in
/// 16-bit code values.
pub fn mean_abs_diff(a: &[u16], b: &[u16]) -> f64 {
    assert_eq!(a.len(), b.len());
    let total: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    total as f64 / a.len() as f64
}

/// Largest absolute difference between two equally long sample buffers.
pub fn max_abs_diff(a: &[u16], b: &[u16]) -> u16 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}
