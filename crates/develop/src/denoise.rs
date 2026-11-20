//! Stage 4, noise reduction (DESIGN §5.7), on scene-linear working RGB
//! before any tone change, so its thresholds do not depend on exposure.
//!
//! The pixel is variance-stabilized by a square root per channel (photon
//! noise, whose variance grows with the signal, becomes roughly constant;
//! the Anscombe transform without its offset), split into a luminance and
//! two colour-difference planes, and each plane is denoised by soft
//! thresholding its à trous wavelet bands (Starck and Murtagh; Donoho
//! 1995). Luminance and colour have their own thresholds.
//!
//! Shrinking colour bands is not edge-aware: weak colour differences at
//! the scales it smooths spread a little across edges (a neutral strip a
//! few dozen pixels wide between coloured areas takes a few percent of
//! their colour at the default strength). Color Detail lowers the fine
//! bands' thresholds to hold thin colour edges; the coarse bands' are kept
//! small.

use photo_core::colorimetry::WORKING_LUMA;
use photo_core::develop::NoiseReduction;
use photo_imaging::filter::{soft_threshold, wavelet_decompose, wavelet_denoise, wavelet_support};
use photo_imaging::{Plane, RgbImage};

use crate::cancel::CancelToken;
use crate::masks::{Layers, MaskUse};
use crate::presence::finite_or_zero;

// ---- Tunable constants ----------------------------------------------------

/// Standard deviation of white noise in each à trous band per unit of
/// noise (the first four from Starck and Murtagh's tables, then halving).
const BAND_SIGMA: [f32; 7] = [0.889, 0.200, 0.086, 0.041, 0.020, 0.010, 0.005];
/// The noise level, in square-root units, that Luminance 100 and Color 100
/// assume, and how many of those standard deviations a coefficient may
/// reach before it is kept.
const LUMINANCE_SIGMA: f32 = 0.02;
const COLOR_SIGMA: f32 = 0.01;
const SIGMAS: f32 = 3.0;
/// Bands denoised at full resolution: luminance noise is fine-grained,
/// colour noise blotchy and coarser.
const LUMINANCE_BANDS: usize = 4;
const COLOR_BANDS: usize = 4;
/// Colour noise is not white after demosaicing; its bands fall off more
/// slowly than white noise's (0.889, 0.200, 0.086, ...).
const COLOR_PROFILE: [f32; COLOR_BANDS] = [1.0, 0.5, 0.25, 0.12];
/// A view this many octaves magnified still denoises at most this many
/// bands, which bounds its padding.
const MAX_LEVELS: usize = 7;

// ---------------------------------------------------------------------------

/// Stage 4 compiled for one view.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Denoise {
    /// Luminance thresholds per view band at Luminance 100.
    luminance: Vec<f32>,
    /// The global Luminance slider as a fraction.
    strength: f32,
    /// Masks change the luminance strength per pixel.
    local: bool,
    /// Colour thresholds per view band.
    color: Vec<f32>,
    /// Padding the wavelets need.
    pub(crate) support: u32,
}

/// Thresholds per view band from thresholds per full-resolution band. View
/// band `j` holds structure of full-resolution band `j + d`, with `d` the
/// octaves of reduction (`log2` of source pixels per view pixel): bands
/// finer than a view pixel are gone already, and a band that was not
/// denoised at full resolution is not denoised here either.
fn view_bands(full: &[f32], footprint: f32) -> Vec<f32> {
    let shift = footprint.max(1.0e-6).log2();
    let at = |position: f32| -> f32 {
        let position = position.max(0.0);
        let below = position.floor() as usize;
        let t = position - below as f32;
        match (full.get(below), full.get(below + 1)) {
            (Some(&a), Some(&b)) => a * (b / a).powf(t),
            (Some(&a), None) => a * (1.0 - t),
            _ => 0.0,
        }
    };
    (0..MAX_LEVELS)
        .map(|band| at(band as f32 + shift))
        .take_while(|threshold| *threshold > 0.0)
        .collect()
}

impl Denoise {
    /// `None` when neither luminance nor colour noise is reduced anywhere,
    /// or when every band to reduce lies below what the view can show.
    pub(crate) fn new(noise: &NoiseReduction, masks: MaskUse, footprint: f32) -> Option<Self> {
        let unit = |value: f32| (finite_or_zero(value) / 100.0).clamp(0.0, 1.0);
        let strength = unit(noise.luminance);
        let luminance = if strength > 0.0 || masks.noise {
            let detail = 1.5 - unit(noise.luminance_detail);
            let contrast = 1.0 - 0.7 * unit(noise.luminance_contrast);
            let full: Vec<f32> = (0..LUMINANCE_BANDS)
                .map(|band| {
                    let shape = if band < 2 { detail } else { contrast };
                    LUMINANCE_SIGMA * SIGMAS * BAND_SIGMA[band] * shape
                })
                .collect();
            view_bands(&full, footprint)
        } else {
            Vec::new()
        };
        let amount = unit(noise.color);
        let color = if amount > 0.0 {
            let detail = 1.5 - unit(noise.color_detail);
            let smoothness = 0.5 + unit(noise.color_smoothness);
            let full: Vec<f32> = COLOR_PROFILE
                .iter()
                .enumerate()
                .map(|(band, profile)| {
                    let shape = if band < 2 { detail } else { smoothness };
                    COLOR_SIGMA * SIGMAS * amount * profile * shape
                })
                .collect();
            view_bands(&full, footprint)
        } else {
            Vec::new()
        };
        let levels = luminance.len().max(color.len());
        (levels > 0).then(|| Self {
            luminance,
            strength,
            local: masks.noise,
            color,
            support: wavelet_support(levels) as u32,
        })
    }
}

/// Denoise `image` (padded by `padding`, at least the support) and return
/// it cropped by the support on every side.
pub(crate) fn apply_denoise(
    image: &RgbImage,
    denoise: &Denoise,
    layers: &Layers,
    padding: u32,
    cancel: &CancelToken,
) -> Option<RgbImage> {
    let (width, height) = (image.width(), image.height());
    let [wr, wg, wb] = WORKING_LUMA;
    let roots = |rgb: [f32; 3]| rgb.map(|c| c.max(0.0).sqrt());
    let mut luma = image.map_to_plane(|rgb| {
        let e = roots(rgb);
        wr * e[0] + wg * e[1] + wb * e[2]
    });
    let chroma = |channel: usize| {
        image.map_to_plane(|rgb| {
            let e = roots(rgb);
            e[channel] - (wr * e[0] + wg * e[1] + wb * e[2])
        })
    };
    let (mut red, mut blue) = (chroma(0), chroma(2));
    cancel.proceed()?;
    if !denoise.luminance.is_empty() {
        luma = if denoise.local {
            let strengths = layers.effective_plane(
                width,
                height,
                padding,
                denoise.strength * 100.0,
                (0.0, 100.0),
                |e| e.noise,
            );
            local_denoise(&luma, &denoise.luminance, &strengths)
        } else {
            let thresholds: Vec<f32> = denoise
                .luminance
                .iter()
                .map(|t| t * denoise.strength)
                .collect();
            wavelet_denoise(&luma, &thresholds)
        };
        cancel.proceed()?;
    }
    if !denoise.color.is_empty() {
        red = wavelet_denoise(&red, &denoise.color);
        cancel.proceed()?;
        blue = wavelet_denoise(&blue, &denoise.color);
        cancel.proceed()?;
    }
    let support = denoise.support as usize;
    let (out_width, out_height) = (width - 2 * support, height - 2 * support);
    let mut out = RgbImage::new(out_width, out_height);
    out.par_rows_mut(|y, row| {
        for (x, pixel) in row.iter_mut().enumerate() {
            let index = (y + support) * width + x + support;
            let l = luma.as_slice()[index];
            let r = (l + red.as_slice()[index]).max(0.0);
            let b = (l + blue.as_slice()[index]).max(0.0);
            let g = ((l - wr * r - wb * b) / wg).max(0.0);
            *pixel = [r * r, g * g, b * b];
        }
    });
    cancel.proceed()?;
    Some(out)
}

/// Luminance denoising whose strength varies per pixel: every band is
/// decomposed once and each coefficient is shrunk by its band's threshold
/// times the pixel's own strength (`0..=100`), so masks cost no extra pass.
fn local_denoise(luma: &Plane, thresholds: &[f32], strengths: &Plane) -> Plane {
    let mut bands = wavelet_decompose(luma, thresholds.len());
    for (detail, &threshold) in bands.details_mut().iter_mut().zip(thresholds) {
        detail.par_rows_mut(|y, row| {
            let strength = strengths.row(y);
            for (value, &s) in row.iter_mut().zip(strength) {
                *value = soft_threshold(*value, threshold * s / 100.0);
            }
        });
    }
    bands.reconstruct()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_views_drop_the_bands_they_cannot_show() {
        let full = [4.0, 2.0, 1.0, 0.5];
        assert_eq!(view_bands(&full, 1.0), full);
        // Two octaves down, view band 0 holds full band 2.
        assert_eq!(view_bands(&full, 4.0), [1.0, 0.5]);
        assert!(view_bands(&full, 32.0).is_empty());
        // Magnified, the finest view bands take full band 0's threshold.
        assert_eq!(view_bands(&full, 0.5)[..2], [4.0, 4.0]);
    }
}
