//! Stage 8, presence (DESIGN §5.7): dehaze, clarity and texture.
//!
//! Clarity and texture act on perceptual luminance `v = Y^(1/2.2)` and
//! hand the change back to the pixel as a gain on all three channels, so
//! hue and the channel ratios are kept. Dehaze acts on display-linear RGB.
//!
//! Dehaze and clarity have radii of whole percent of the long edge; their
//! smooth parts (the refined transmission and the edge-aware local mean)
//! come from the whole-image guides (see `guides`) and only the per-pixel
//! formulas below run at render resolution. Texture's radii are small, so
//! it filters the view's own padded buffer.

use std::sync::LazyLock;

use photo_color::tone::{decode_perceptual, encode_perceptual};
use photo_core::colorimetry::luminance;
use photo_core::develop::DevelopSettings;
use photo_imaging::filter::{guided_support, self_guided_filter};
use photo_imaging::{Plane, RgbImage};
use rayon::prelude::*;

use crate::cancel::CancelToken;
use crate::masks::{Layers, MaskUse};
use crate::render::Quality;

// ---- Tunable constants ----------------------------------------------------

/// Dehaze: the share of the haze removed at +100 (He, Sun and Tang's ω).
pub(crate) const DEHAZE_OMEGA: f32 = 0.95;
/// Dehaze: the lowest transmission divided by (their `t0`). They use 0.1;
/// dividing by it multiplies the noise of the densest haze tenfold, which
/// the tuning grid showed as a grainy fringe round a light source's glow,
/// so it is held at 0.2.
const DEHAZE_MIN_TRANSMISSION: f32 = 0.2;
/// Dehaze below zero: the transmission at -100 where the image is hazy
/// already, and where it is clear.
const ADDED_HAZE_HAZY: f32 = 0.55;
const ADDED_HAZE_CLEAR: f32 = 0.2;
/// The atmospheric light is held at least this bright per channel, so a
/// dark picture does not divide by almost nothing.
pub(crate) const DEHAZE_MIN_LIGHT: f32 = 0.05;
/// Clarity: the share of the detail `v - mean` added at +100 and removed at
/// -100.
const CLARITY_BOOST: f32 = 1.0;
const CLARITY_CUT: f32 = 0.75;
/// Texture: the share of the band added at +100 and removed at -100.
const TEXTURE_BOOST: f32 = 1.5;
const TEXTURE_CUT: f32 = 0.8;
/// Texture's band, as fractions of the source's long edge: structure
/// between these two radii (about 1.2 and 12 pixels on a 24 MP photo).
const TEXTURE_FINE: f32 = 0.0002;
const TEXTURE_COARSE: f32 = 0.002;
/// Texture's edge threshold (a variance of perceptual values): edges far
/// stronger than `sqrt` of this stay out of the band, so they get no halo.
const TEXTURE_EPS: f32 = 0.002;
/// Radii are held to this many view pixels (a deep zoom would otherwise
/// ask for unbounded padding).
pub(crate) const MAX_VIEW_RADIUS: f32 = 256.0;
/// Below this luminance a pixel's colour is not trusted to carry a gain.
const MIN_LUMINANCE: f32 = 1.0e-7;

// ---------------------------------------------------------------------------

/// A pixel's working-space luminance (never negative) and its perceptual
/// value.
#[inline]
pub(crate) fn perceptual(rgb: [f32; 3]) -> (f32, f32) {
    let y = luminance(rgb).max(0.0);
    (y, encode_perceptual(y))
}

/// Samples of the perceptual decode `v^2.2` over `0..=1`.
const DECODE_CELLS: usize = 4096;

/// `v^2.2` sampled on `0..=1`. Linear interpolation between these exact
/// samples is within `3e-8` of the curve (its second derivative is under
/// 2.7) and, between increasing samples, increasing.
static DECODE: LazyLock<Box<[f32]>> = LazyLock::new(|| {
    (0..=DECODE_CELLS)
        .map(|i| decode_perceptual(i as f32 / DECODE_CELLS as f32))
        .collect()
});

/// [`decode_perceptual`] from a table on `0..1`, exact above.
#[inline]
fn decode_fast(v: f32) -> f32 {
    if v >= 1.0 {
        return decode_perceptual(v);
    }
    if v.is_nan() || v <= 0.0 {
        return 0.0;
    }
    let position = v * DECODE_CELLS as f32;
    let index = (position as usize).min(DECODE_CELLS - 1);
    let (low, high) = (DECODE[index], DECODE[index + 1]);
    low + (high - low) * (position - index as f32)
}

/// `rgb`, whose luminance is `y`, carried to perceptual value `v` by one
/// gain on all channels. A pixel with no luminance becomes a neutral of
/// that value.
#[inline]
pub(crate) fn relight(rgb: [f32; 3], y: f32, v: f32) -> [f32; 3] {
    let target = decode_fast(v);
    if y > MIN_LUMINANCE {
        let gain = target / y;
        [rgb[0] * gain, rgb[1] * gain, rgb[2] * gain]
    } else {
        [target; 3]
    }
}

/// How much of a local contrast change a pixel of perceptual value `v`
/// takes: 1 in the midtones, easing to 0 at black and white so that
/// clarity and texture neither clip nor crush.
#[inline]
fn midtones(v: f32) -> f32 {
    let u = (2.0 * v - 1.0).clamp(-1.0, 1.0);
    let u2 = u * u;
    1.0 - u2 * u2
}

/// A presence slider (global plus local, in slider units) as a signed gain.
#[inline]
fn signed_gain(amount: f32, boost: f32, cut: f32) -> f32 {
    let unit = (amount / 100.0).clamp(-1.0, 1.0);
    if unit >= 0.0 {
        unit * boost
    } else {
        unit * cut
    }
}

/// Dehaze one display-linear pixel (He, Sun and Tang 2009, the haze model
/// `I = J t + A (1 - t)`). `haze` is `1 - t` at full strength, `0..ω`,
/// from the refined transmission; `amount` is the slider value, global plus
/// local. Positive amounts recover `J = (I - A) / t + A` with the haze
/// scaled by the amount; negative amounts add haze, more where there is
/// some already. Zero returns the pixel unchanged.
#[inline]
pub(crate) fn dehaze_pixel(rgb: [f32; 3], light: [f32; 3], haze: f32, amount: f32) -> [f32; 3] {
    let strength = (amount / 100.0).clamp(-1.0, 1.0);
    if strength == 0.0 || !strength.is_finite() {
        return rgb;
    }
    if strength > 0.0 {
        let t = (1.0 - strength * haze).max(DEHAZE_MIN_TRANSMISSION);
        let inverse = 1.0 / t;
        std::array::from_fn(|c| ((rgb[c] - light[c]) * inverse + light[c]).max(0.0))
    } else {
        let hazy = (haze / DEHAZE_OMEGA).clamp(0.0, 1.0);
        let added = -strength * (ADDED_HAZE_CLEAR + (ADDED_HAZE_HAZY - ADDED_HAZE_CLEAR) * hazy);
        std::array::from_fn(|c| rgb[c] + (light[c] - rgb[c]) * added)
    }
}

/// Clarity on one pixel: `v + c · (v - mean) · midtones(v)` where `mean`
/// is the edge-aware local mean of `v` and `c` follows the slider value
/// (global plus local). Zero returns the pixel unchanged.
#[inline]
pub(crate) fn clarity_pixel(rgb: [f32; 3], mean: f32, amount: f32) -> [f32; 3] {
    let gain = signed_gain(amount, CLARITY_BOOST, CLARITY_CUT);
    if gain == 0.0 || !gain.is_finite() {
        return rgb;
    }
    let (y, v) = perceptual(rgb);
    relight(rgb, y, v + gain * (v - mean) * midtones(v))
}

/// Stage 8's texture, compiled for one view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Texture {
    /// The band's radii in view pixels; `fine` below half a pixel means the
    /// pixel itself.
    fine: f32,
    coarse: f32,
    /// The global slider value.
    amount: f32,
    /// Masks change the amount per pixel.
    local: bool,
    /// Padding the filter needs.
    pub(crate) support: u32,
}

/// Texture's band radii in view pixels for a source of `long_edge` pixels
/// seen at `footprint` source pixels per view pixel.
fn texture_radii(long_edge: f32, footprint: f32) -> (f32, f32) {
    let radius = |fraction: f32| (fraction * long_edge / footprint).min(MAX_VIEW_RADIUS);
    (radius(TEXTURE_FINE), radius(TEXTURE_COARSE))
}

/// The most padding texture can need for this source and footprint.
pub(crate) fn texture_support_max(long_edge: f32, footprint: f32) -> u32 {
    guided_support(texture_radii(long_edge, footprint).1) as u32
}

impl Texture {
    /// `None` when texture is off everywhere, or when its band lies below
    /// what the view can show (under half a view pixel; Draft quality
    /// skips it under one).
    pub(crate) fn new(
        settings: &DevelopSettings,
        masks: MaskUse,
        long_edge: f32,
        footprint: f32,
        quality: Quality,
    ) -> Option<Self> {
        let amount = finite_or_zero(settings.presence.texture);
        if amount == 0.0 && !masks.texture {
            return None;
        }
        let (fine, coarse) = texture_radii(long_edge, footprint);
        let smallest = match quality {
            Quality::Full => 0.5,
            Quality::Draft => 1.0,
        };
        (coarse >= smallest).then(|| Self {
            fine,
            coarse,
            amount,
            local: masks.texture,
            support: guided_support(coarse) as u32,
        })
    }
}

pub(crate) fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// `image` (padded by `padding`) with texture applied.
pub(crate) fn apply_texture(
    image: &RgbImage,
    texture: &Texture,
    layers: &Layers,
    padding: u32,
    cancel: &CancelToken,
) -> Option<RgbImage> {
    let (width, height) = (image.width(), image.height());
    let values = image.map_to_plane(|rgb| perceptual(rgb).1);
    cancel.proceed()?;
    let fine = if texture.fine >= 0.5 {
        self_guided_filter(&values, texture.fine, TEXTURE_EPS)
    } else {
        values
    };
    cancel.proceed()?;
    let coarse = self_guided_filter(&fine, texture.coarse, TEXTURE_EPS);
    cancel.proceed()?;
    let amounts = texture.local.then(|| {
        layers.effective_plane(
            width,
            height,
            padding,
            texture.amount,
            (-100.0, 100.0),
            |e| e.texture,
        )
    });
    let amount_at = |index: usize| match &amounts {
        Some(plane) => plane.as_slice()[index],
        None => texture.amount,
    };
    band_apply(image, &fine, &coarse, &amount_at, cancel)
}

/// Each pixel with its share of the band `fine - coarse` added to its
/// perceptual value.
fn band_apply(
    image: &RgbImage,
    fine: &Plane,
    coarse: &Plane,
    amount_at: &(dyn Fn(usize) -> f32 + Sync),
    cancel: &CancelToken,
) -> Option<RgbImage> {
    let width = image.width();
    let mut out = RgbImage::new(width, image.height());
    out.pixels_mut()
        .par_chunks_mut(width.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            if cancel.is_cancelled() {
                return;
            }
            for (x, (pixel, &rgb)) in row.iter_mut().zip(image.row(y)).enumerate() {
                let index = y * width + x;
                let gain = signed_gain(amount_at(index), TEXTURE_BOOST, TEXTURE_CUT);
                *pixel = if gain == 0.0 {
                    rgb
                } else {
                    let band = fine.as_slice()[index] - coarse.as_slice()[index];
                    let (luma, v) = perceptual(rgb);
                    relight(rgb, luma, v + gain * band * midtones(v))
                };
            }
        });
    cancel.proceed()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_amounts_leave_pixels_alone() {
        let rgb = [0.3, 0.2, 0.1];
        assert_eq!(dehaze_pixel(rgb, [0.8; 3], 0.5, 0.0), rgb);
        assert_eq!(clarity_pixel(rgb, 0.1, 0.0), rgb);
    }

    #[test]
    fn dehaze_moves_away_from_the_haze_and_negative_moves_toward_it() {
        let light = [0.9, 0.9, 0.95];
        let rgb = [0.6, 0.62, 0.7];
        let clear = dehaze_pixel(rgb, light, 0.6, 100.0);
        let hazy = dehaze_pixel(rgb, light, 0.6, -100.0);
        for c in 0..3 {
            assert!(clear[c] < rgb[c] && hazy[c] > rgb[c], "{clear:?} {hazy:?}");
        }
    }

    #[test]
    fn the_decode_table_follows_the_curve_and_never_falls() {
        let mut previous = 0.0;
        for step in 0..=20_000 {
            let v = step as f32 / 10_000.0;
            let fast = decode_fast(v);
            assert!((fast - decode_perceptual(v)).abs() < 1e-6, "{v}");
            assert!(fast >= previous, "{v}");
            previous = fast;
        }
        assert_eq!(decode_fast(f32::NAN), 0.0);
    }

    #[test]
    fn the_midtone_weight_is_one_in_the_middle_and_zero_at_the_ends() {
        assert_eq!(midtones(0.5), 1.0);
        assert_eq!(midtones(0.0), 0.0);
        assert_eq!(midtones(1.0), 0.0);
        assert!(midtones(0.25) > 0.9);
    }
}
