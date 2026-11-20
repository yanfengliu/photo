//! Stage 9, sharpening (DESIGN §5.7): an unsharp mask on perceptual
//! luminance with a detail threshold that holds back halos and a
//! gradient-magnitude mask that keeps flat areas smooth.

use photo_core::develop::DevelopSettings;
use photo_imaging::RgbImage;
use photo_imaging::filter::{gaussian_support, gradient_magnitude, high_pass, soft_threshold};
use rayon::prelude::*;

use crate::cancel::CancelToken;
use crate::masks::{Layers, MaskUse};
use crate::presence::{MAX_VIEW_RADIUS, finite_or_zero, perceptual, relight};
use crate::render::Quality;

// ---- Tunable constants ----------------------------------------------------

/// Coring at Detail 0, in perceptual units: smaller differences are left
/// alone, so sharpening does not turn noise into grit. It falls to zero at
/// Detail 100 as `(1 - detail)²`.
const CORE: f32 = 0.006;
/// Halo limit at Detail 0 and at Detail 100, in perceptual units: the
/// detail added at a pixel is compressed as `d · h / (h + |d|)`, so an edge
/// gains at most `h`. Low Detail keeps strong edges from ringing; high
/// Detail lets every edge through.
const HALO_LOW: f32 = 0.02;
const HALO_HIGH: f32 = 0.5;
/// Masking 100 sharpens fully only where the perceptual gradient reaches
/// this many units per source pixel, fading to nothing on flat ground.
const MASK_EDGE: f32 = 0.03;
/// The local Sharpness sum, with the global Amount, is held to this range;
/// below zero the unsharp mask turns into a blur.
const AMOUNT_RANGE: (f32, f32) = (-100.0, 150.0);

// ---------------------------------------------------------------------------

/// Stage 9 compiled for one view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sharpen {
    /// The blur's sigma in view pixels.
    sigma: f32,
    /// The global Amount.
    amount: f32,
    /// Masks change the amount per pixel.
    local: bool,
    threshold: f32,
    halo: f32,
    /// Gradient (perceptual units per view pixel) that gets the full
    /// effect; `None` without masking.
    edge: Option<f32>,
    /// Padding the filters need.
    pub(crate) support: u32,
}

/// Sigma in view pixels for a radius in source pixels.
fn view_sigma(radius: f32, footprint: f32) -> f32 {
    let radius = if radius.is_finite() {
        radius.clamp(0.5, 3.0)
    } else {
        1.0
    };
    (radius / footprint).min(MAX_VIEW_RADIUS)
}

/// The most padding sharpening can need at this footprint (the largest
/// radius, with masking).
pub(crate) fn sharpen_support_max(footprint: f32) -> u32 {
    gaussian_support(view_sigma(3.0, footprint)) as u32 + 1
}

impl Sharpen {
    /// `None` when nothing is sharpened anywhere; Draft quality also skips
    /// a radius under half a view pixel.
    pub(crate) fn new(
        settings: &DevelopSettings,
        masks: MaskUse,
        footprint: f32,
        quality: Quality,
    ) -> Option<Self> {
        let sharpening = &settings.detail.sharpening;
        let amount = finite_or_zero(sharpening.amount).clamp(0.0, AMOUNT_RANGE.1);
        if amount == 0.0 && !masks.sharpness {
            return None;
        }
        let sigma = view_sigma(sharpening.radius, footprint);
        if quality == Quality::Draft && sigma < 0.5 {
            return None;
        }
        let detail = (finite_or_zero(sharpening.detail) / 100.0).clamp(0.0, 1.0);
        let masking = (finite_or_zero(sharpening.masking) / 100.0).clamp(0.0, 1.0);
        let edge = (masking > 0.0).then_some(MASK_EDGE * masking * footprint);
        Some(Self {
            sigma,
            amount,
            local: masks.sharpness,
            threshold: CORE * (1.0 - detail) * (1.0 - detail),
            halo: HALO_LOW + (HALO_HIGH - HALO_LOW) * detail * detail,
            edge,
            support: gaussian_support(sigma) as u32 + u32::from(edge.is_some()),
        })
    }
}

/// Smoothstep from 0 at 0 to 1 at `edge`.
#[inline]
fn ramp(value: f32, edge: f32) -> f32 {
    let t = (value / edge).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `image` (padded by `padding`) sharpened.
pub(crate) fn apply_sharpen(
    image: &RgbImage,
    sharpen: &Sharpen,
    layers: &Layers,
    padding: u32,
    cancel: &CancelToken,
) -> Option<RgbImage> {
    let (width, height) = (image.width(), image.height());
    let values = image.map_to_plane(|rgb| perceptual(rgb).1);
    cancel.proceed()?;
    let detail = high_pass(&values, sharpen.sigma);
    let gradient = sharpen.edge.map(|_| gradient_magnitude(&values));
    cancel.proceed()?;
    let amounts = sharpen.local.then(|| {
        layers.effective_plane(width, height, padding, sharpen.amount, AMOUNT_RANGE, |e| {
            e.sharpness
        })
    });
    let mut out = RgbImage::new(width, height);
    out.pixels_mut()
        .par_chunks_mut(width.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            if cancel.is_cancelled() {
                return;
            }
            for (x, (pixel, &rgb)) in row.iter_mut().zip(image.row(y)).enumerate() {
                let index = y * width + x;
                let amount = match &amounts {
                    Some(plane) => plane.as_slice()[index],
                    None => sharpen.amount,
                } / 100.0;
                if amount == 0.0 {
                    *pixel = rgb;
                    continue;
                }
                let d = detail.as_slice()[index];
                let boost = if amount > 0.0 {
                    let cored = soft_threshold(d, sharpen.threshold);
                    let limited = cored * sharpen.halo / (sharpen.halo + cored.abs());
                    let weight = match (&gradient, sharpen.edge) {
                        (Some(gradient), Some(edge)) => ramp(gradient.as_slice()[index], edge),
                        _ => 1.0,
                    };
                    amount * limited * weight
                } else {
                    amount * d
                };
                let (luma, v) = perceptual(rgb);
                *pixel = relight(rgb, luma, v + boost);
            }
        });
    cancel.proceed()?;
    Some(out)
}
