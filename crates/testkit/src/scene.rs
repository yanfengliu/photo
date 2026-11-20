//! The synthetic photograph every fixture is rendered from.
//!
//! One procedural scene, defined in linear light with sRGB primaries, with
//! something for every kind of operator to act on:
//!
//! - a sky gradient with mild noise (tone, noise reduction);
//! - a sun disk far brighter than white, with a glow that clips channel by
//!   channel (highlight handling);
//! - a dark, finely textured foreground (shadows, sharpening, detail);
//! - a row of flat, saturated colour patches including two skin tones
//!   (colour operators), and a neutral grey ramp (tone curves, precision).
//!
//! The patches and the ramp are flat and noise-free so tests can measure them
//! exactly; [`patches`] and [`grey_ramp`] say where they are.

use image::{ImageBuffer, Rgb, RgbImage};
use photo_core::colorimetry::RgbSpace;

/// Fraction of the frame height at which the sky meets the ground.
const HORIZON: f32 = 0.56;
/// Sun centre in normalized frame coordinates, and its radius as a fraction
/// of the short edge.
const SUN_CENTRE: (f32, f32) = (0.76, 0.2);
const SUN_RADIUS: f32 = 0.085;
/// Radiance of the sun disk: sixteen times white, so it clips in any
/// encoding and under any plausible white balance.
const SUN_RADIANCE: [f32; 3] = [16.0, 15.0, 12.0];

/// Horizontal extent shared by the colour chart and the grey ramp.
const STRIP_LEFT: f32 = 0.08;
const STRIP_RIGHT: f32 = 0.92;
const CHART_TOP: f32 = 0.64;
const CHART_BOTTOM: f32 = 0.80;
const RAMP_TOP: f32 = 0.86;
const RAMP_BOTTOM: f32 = 0.94;
/// Fraction of each patch slot left as a gap showing the ground.
const PATCH_GAP: f32 = 0.14;

/// Name and linear sRGB colour of each chart patch, left to right. The skin
/// tones are the ColorChecker "light skin" and "dark skin" patches.
const PATCH_COLOURS: [(&str, [f32; 3]); 8] = [
    ("red", [0.60, 0.04, 0.03]),
    ("green", [0.05, 0.45, 0.06]),
    ("blue", [0.03, 0.06, 0.50]),
    ("yellow", [0.70, 0.60, 0.04]),
    ("cyan", [0.04, 0.45, 0.55]),
    ("magenta", [0.50, 0.05, 0.40]),
    ("light skin", [0.539, 0.305, 0.223]),
    ("dark skin", [0.171, 0.084, 0.058]),
];

/// Colour of the corner marker drawn by [`scene_with_marker`].
pub const MARKER_RGB: [u8; 3] = [255, 0, 255];

/// The scene in linear light: row-major pixels with sRGB primaries, where 1.0
/// is diffuse white. Values are never negative and exceed 1.0 inside and
/// around the sun.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearScene {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height` pixels in row order, each red, green, blue.
    pub pixels: Vec<[f32; 3]>,
}

impl LinearScene {
    /// The pixel at `(x, y)`. Panics when out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 3] {
        self.pixels[y as usize * self.width as usize + x as usize]
    }

    /// Mean colour of the rectangle with top-left `(x, y)`. Panics when the
    /// rectangle is empty or leaves the frame.
    pub fn mean(&self, x: u32, y: u32, width: u32, height: u32) -> [f32; 3] {
        assert!(width > 0 && height > 0, "empty rectangle");
        let mut sum = [0.0f64; 3];
        for row in y..y + height {
            for column in x..x + width {
                for (total, value) in sum.iter_mut().zip(self.pixel(column, row)) {
                    *total += f64::from(value);
                }
            }
        }
        let count = f64::from(width) * f64::from(height);
        sum.map(|total| (total / count) as f32)
    }
}

/// A flat region of the scene with a known colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
    /// What the patch is called in failure messages, such as "red" or
    /// "light skin".
    pub name: &'static str,
    /// Left edge of the interior, in pixels.
    pub x: u32,
    /// Top edge of the interior, in pixels.
    pub y: u32,
    /// Interior width; zero when the frame is too small to hold the patch.
    pub width: u32,
    /// Interior height; zero when the frame is too small to hold the patch.
    pub height: u32,
    /// The patch colour in linear light with sRGB primaries.
    pub linear_srgb: [f32; 3],
}

/// Scene radiance at the centre of pixel `(x, y)` of a `width` x `height`
/// frame, in linear light with sRGB primaries.
///
/// Coordinates outside the frame are allowed and continue the scene past its
/// edges, which is what lets a fixture add sensor margins around the picture.
pub fn radiance(x: i64, y: i64, width: u32, height: u32) -> [f32; 3] {
    let u = ((x as f32 + 0.5) / width as f32).clamp(0.0, 1.0);
    let v = ((y as f32 + 0.5) / height as f32).clamp(0.0, 1.0);
    if (CHART_TOP..CHART_BOTTOM).contains(&v)
        && let Some(index) = patch_index(u)
    {
        return PATCH_COLOURS[index].1;
    }
    if (RAMP_TOP..RAMP_BOTTOM).contains(&v) && (STRIP_LEFT..STRIP_RIGHT).contains(&u) {
        let level = RgbSpace::Srgb.decode((u - STRIP_LEFT) / (STRIP_RIGHT - STRIP_LEFT));
        return [level; 3];
    }
    if v < HORIZON {
        sky(x, y, v, width, height)
    } else {
        ground(x, y, v)
    }
}

/// Which chart patch the normalized horizontal position `u` falls in.
fn patch_index(u: f32) -> Option<usize> {
    if !(STRIP_LEFT..STRIP_RIGHT).contains(&u) {
        return None;
    }
    let slot = (u - STRIP_LEFT) / (STRIP_RIGHT - STRIP_LEFT) * PATCH_COLOURS.len() as f32;
    let within = slot.fract();
    let inside = (PATCH_GAP / 2.0..1.0 - PATCH_GAP / 2.0).contains(&within);
    inside.then_some((slot as usize).min(PATCH_COLOURS.len() - 1))
}

fn sky(x: i64, y: i64, v: f32, width: u32, height: u32) -> [f32; 3] {
    const ZENITH: [f32; 3] = [0.08, 0.20, 0.55];
    const HAZE: [f32; 3] = [0.55, 0.68, 0.85];
    const GLOW: [f32; 3] = [1.8, 1.5, 1.0];

    let short_edge = width.min(height) as f32;
    let dx = (x as f32 + 0.5 - SUN_CENTRE.0 * width as f32) / short_edge;
    let dy = (y as f32 + 0.5 - SUN_CENTRE.1 * height as f32) / short_edge;
    let distance = (dx * dx + dy * dy).sqrt();
    if distance <= SUN_RADIUS {
        return SUN_RADIANCE;
    }

    let descent = v / HORIZON;
    let blend = descent * descent.sqrt();
    let glow = (-(distance - SUN_RADIUS) / (0.6 * SUN_RADIUS)).exp();
    let grain = 1.0 + 0.02 * signed_noise(x, y);
    std::array::from_fn(|c| ((ZENITH[c] + (HAZE[c] - ZENITH[c]) * blend) + GLOW[c] * glow) * grain)
}

fn ground(x: i64, y: i64, v: f32) -> [f32; 3] {
    const SOIL: [f32; 3] = [0.034, 0.038, 0.024];

    let depth = (v - HORIZON) / (1.0 - HORIZON);
    let falloff = 1.15 - 0.5 * depth;
    let grain = 0.6 + 0.4 * (signed_noise(x, y) + 1.0);
    let weave = 1.0 + 0.3 * (x as f32 * 2.1 + y as f32 * 0.9).sin();
    SOIL.map(|c| c * falloff * grain * weave)
}

/// Deterministic per-pixel noise in `[-1, 1)`.
fn signed_noise(x: i64, y: i64) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9e37_79b1) ^ (y as u32).wrapping_mul(0x85eb_ca77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h >> 8) as f32 / 8_388_608.0 - 1.0
}

/// The scene in linear light. Deterministic: the same size always gives the
/// same pixels.
pub fn scene_linear(width: u32, height: u32) -> LinearScene {
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.push(radiance(i64::from(x), i64::from(y), width, height));
        }
    }
    LinearScene {
        width,
        height,
        pixels,
    }
}

/// The scene as an 8-bit sRGB picture: [`scene_linear`] clipped at white and
/// sRGB-encoded.
pub fn scene(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| {
        Rgb(radiance(i64::from(x), i64::from(y), width, height).map(|c| encode(c, 255.0) as u8))
    })
}

/// The scene as a 16-bit sRGB picture: the same rendering as [`scene`] with
/// 16 bits of real precision, not 8-bit values widened.
pub fn scene16(width: u32, height: u32) -> ImageBuffer<Rgb<u16>, Vec<u16>> {
    ImageBuffer::from_fn(width, height, |x, y| {
        Rgb(radiance(i64::from(x), i64::from(y), width, height).map(|c| encode(c, 65535.0) as u16))
    })
}

fn encode(linear: f32, white: f32) -> f32 {
    (RgbSpace::Srgb.encode(linear.clamp(0.0, 1.0)) * white).round()
}

/// Side of the square [`scene_with_marker`] draws, in pixels.
pub fn marker_size(width: u32, height: u32) -> u32 {
    (width.min(height) / 6).max(8).min(width).min(height)
}

/// [`scene`] with a solid [`MARKER_RGB`] square of side [`marker_size`] in the
/// top-left corner, so a test can tell which way up a decoded picture is.
pub fn scene_with_marker(width: u32, height: u32) -> RgbImage {
    let mut image = scene(width, height);
    let side = marker_size(width, height);
    for y in 0..side {
        for x in 0..side {
            image.put_pixel(x, y, Rgb(MARKER_RGB));
        }
    }
    image
}

/// The chart patches of a `width` x `height` scene, left to right: red,
/// green, blue, yellow, cyan, magenta, light skin, dark skin.
///
/// Each rectangle is the patch interior, inset from the patch edge so that
/// resampling, chroma subsampling and demosaicing at the border do not leak
/// into a measurement. Frames smaller than about 160 x 120 leave no interior,
/// which shows as a zero width or height.
pub fn patches(width: u32, height: u32) -> Vec<Patch> {
    let rows = span(height, |y| {
        let v = (y as f32 + 0.5) / height as f32;
        (CHART_TOP..CHART_BOTTOM).contains(&v)
    });
    PATCH_COLOURS
        .iter()
        .enumerate()
        .map(|(index, &(name, linear_srgb))| {
            let columns = span(width, |x| {
                patch_index((x as f32 + 0.5) / width as f32) == Some(index)
            });
            let (x, patch_width) = inset(columns);
            let (y, patch_height) = inset(rows);
            Patch {
                name,
                x,
                y,
                width: patch_width,
                height: patch_height,
                linear_srgb,
            }
        })
        .collect()
}

/// The grey ramp of a `width` x `height` scene as `(x, y, width, height)`.
///
/// Across that rectangle the scene is neutral and constant down each column;
/// from left to right its sRGB-encoded value climbs evenly from black to
/// white. Empty when the frame is too small.
pub fn grey_ramp(width: u32, height: u32) -> (u32, u32, u32, u32) {
    let columns = span(width, |x| {
        let u = (x as f32 + 0.5) / width as f32;
        (STRIP_LEFT..STRIP_RIGHT).contains(&u)
    });
    let rows = span(height, |y| {
        let v = (y as f32 + 0.5) / height as f32;
        (RAMP_TOP..RAMP_BOTTOM).contains(&v)
    });
    (columns.0, rows.0, columns.1, rows.1)
}

/// First index and length of the run of indices in `0..extent` for which
/// `inside` holds. The scene's regions are contiguous, so one run covers them.
fn span(extent: u32, inside: impl Fn(u32) -> bool) -> (u32, u32) {
    let start = (0..extent).find(|&i| inside(i));
    match start {
        Some(start) => {
            let length = (start..extent).take_while(|&i| inside(i)).count() as u32;
            (start, length)
        }
        None => (0, 0),
    }
}

/// Shrinks a run by a margin on both sides: a sixth of its length, at least
/// three pixels.
fn inset((start, length): (u32, u32)) -> (u32, u32) {
    let margin = (length / 6).max(3);
    (
        start + margin.min(length),
        length.saturating_sub(2 * margin),
    )
}

#[cfg(test)]
mod tests;
