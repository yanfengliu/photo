//! Stage 10, effects (DESIGN §5.7): the post-crop vignette and film grain,
//! both defined in output coordinates so that they sit still on the
//! picture in every view and tile.

use photo_core::develop::{Grain, Vignette};
use photo_imaging::{Plane, View, grain_plane};

use crate::presence::{finite_or_zero, perceptual, relight};

// ---- Tunable constants ----------------------------------------------------

/// Where the vignette is half way, as a distance (1 at the middle of each
/// edge, about 1.41 at the corners for a round frame) at Midpoint 0 and
/// 100.
const MIDPOINT_NEAR: f32 = 0.35;
const MIDPOINT_FAR: f32 = 1.35;
/// Width of the vignette's transition at Feather 0 and 100.
const FEATHER_HARD: f32 = 0.04;
const FEATHER_SOFT: f32 = 1.4;
/// Exponent of the superellipse at Roundness -100 (2 is an ellipse).
const SQUARE_POWER: f32 = 8.0;
/// Grain size in output pixels at Size 0 and 100.
const GRAIN_SMALL: f32 = 1.0;
const GRAIN_LARGE: f32 = 7.0;
/// Standard deviation of the grain at Amount 100, in perceptual units.
const GRAIN_STRENGTH: f32 = 0.06;

// ---------------------------------------------------------------------------

/// The post-crop vignette compiled for one output size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PostCropVignette {
    amount: f32,
    /// One over the half-axes of the shape, in output pixels.
    scale: (f32, f32),
    centre: (f32, f32),
    power: f32,
    /// Where the transition starts, and one over its width.
    low: f32,
    steep: f32,
    highlights: f32,
}

impl PostCropVignette {
    /// `None` when the amount is zero. The shape follows the output frame:
    /// an ellipse touching the middle of each edge at Roundness 0, turning
    /// into a circle (of the frame's area) toward +100 and toward a
    /// rounded rectangle (a superellipse) toward -100.
    pub(crate) fn new(vignette: &Vignette, output_size: (u32, u32)) -> Option<Self> {
        let amount = (finite_or_zero(vignette.amount) / 100.0).clamp(-1.0, 1.0);
        if amount == 0.0 {
            return None;
        }
        let unit = |value: f32, fallback: f32| {
            if value.is_finite() {
                (value / 100.0).clamp(0.0, 1.0)
            } else {
                fallback
            }
        };
        let roundness = (finite_or_zero(vignette.roundness) / 100.0).clamp(-1.0, 1.0);
        let (half_w, half_h) = (output_size.0 as f32 / 2.0, output_size.1 as f32 / 2.0);
        let round = (half_w * half_h).sqrt();
        let toward = roundness.max(0.0);
        let midpoint =
            MIDPOINT_NEAR + (MIDPOINT_FAR - MIDPOINT_NEAR) * unit(vignette.midpoint, 0.5);
        let width = FEATHER_HARD + (FEATHER_SOFT - FEATHER_HARD) * unit(vignette.feather, 0.5);
        Some(Self {
            amount,
            scale: (
                1.0 / (half_w + (round - half_w) * toward),
                1.0 / (half_h + (round - half_h) * toward),
            ),
            centre: (half_w, half_h),
            power: 2.0 * (SQUARE_POWER / 2.0).powf((-roundness).max(0.0)),
            low: midpoint - width / 2.0,
            steep: 1.0 / width,
            highlights: unit(vignette.highlights, 0.0),
        })
    }

    /// The vignette applied to a display-linear pixel at output position
    /// `(x, y)`. A negative amount darkens toward the edges as an exposure
    /// falloff (holding bright areas back by Highlights); a positive one
    /// lightens toward white.
    #[inline]
    pub(crate) fn apply(&self, rgb: [f32; 3], x: f32, y: f32) -> [f32; 3] {
        let u = ((x - self.centre.0) * self.scale.0).abs();
        let v = ((y - self.centre.1) * self.scale.1).abs();
        let distance = if self.power == 2.0 {
            // `u` and `v` are bounded by the output's size over its own
            // half-axes, so the plain root cannot overflow.
            (u * u + v * v).sqrt()
        } else {
            (u.powf(self.power) + v.powf(self.power)).powf(1.0 / self.power)
        };
        let t = ((distance - self.low) * self.steep).clamp(0.0, 1.0);
        let weight = t * t * (3.0 - 2.0 * t);
        if weight == 0.0 {
            return rgb;
        }
        if self.amount < 0.0 {
            let hold = if self.highlights > 0.0 {
                let (_, value) = perceptual(rgb);
                let bright = ((value - 0.6) / 0.4).clamp(0.0, 1.0);
                1.0 - self.highlights * bright * bright * (3.0 - 2.0 * bright)
            } else {
                1.0
            };
            let gain = 1.0 + self.amount * weight * hold;
            [rgb[0] * gain, rgb[1] * gain, rgb[2] * gain]
        } else {
            let lift = self.amount * weight;
            rgb.map(|c| c + (1.0 - c).max(0.0) * lift)
        }
    }
}

/// What a view's grain pattern depends on (not the amount, which only
/// scales it), so that moving any other slider reuses the pattern.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GrainKey {
    view: View,
    size: f32,
    rough: f32,
}

impl GrainKey {
    /// `None` when the amount is zero. Grains are `size` output pixels
    /// across; Roughness blends in a second layer at half the size.
    pub(crate) fn new(grain: &Grain, view: &View) -> Option<Self> {
        let amount = (finite_or_zero(grain.amount) / 100.0).clamp(0.0, 1.0);
        let size_unit = (finite_or_zero(grain.size) / 100.0).clamp(0.0, 1.0);
        (amount > 0.0).then(|| Self {
            view: *view,
            size: GRAIN_SMALL + (GRAIN_LARGE - GRAIN_SMALL) * size_unit,
            rough: (finite_or_zero(grain.roughness) / 100.0).clamp(0.0, 1.0),
        })
    }

    /// The pattern: unit-variance noise, the two sizes mixed.
    pub(crate) fn pattern(&self) -> Plane {
        let coarse = grain_plane(&self.view, self.size, 0);
        if self.rough == 0.0 {
            return coarse;
        }
        let (smooth, rough) = (1.0 - self.rough, self.rough);
        let norm = (smooth * smooth + rough * rough).sqrt();
        let fine = grain_plane(&self.view, self.size / 2.0, 0);
        coarse.zip_map(&fine, |c, f| (smooth * c + rough * f) / norm)
    }
}

/// Film grain compiled for one view: monochrome noise added to perceptual
/// luminance.
pub(crate) struct FilmGrain<'a> {
    pattern: &'a Plane,
    strength: f32,
}

impl<'a> FilmGrain<'a> {
    /// A view whose pixels are larger than the grains averages them away,
    /// as downsampling the full-size render would, so the strength is
    /// scaled by the grain size in view pixels when that is under one.
    pub(crate) fn new(grain: &Grain, key: &GrainKey, pattern: &'a Plane) -> Self {
        let amount = (finite_or_zero(grain.amount) / 100.0).clamp(0.0, 1.0);
        let visible = (key.size * key.view.scale()).min(1.0);
        Self {
            pattern,
            strength: amount * GRAIN_STRENGTH * visible,
        }
    }

    /// Grain added to the view pixel `index` (row-major in the view).
    #[inline]
    pub(crate) fn apply(&self, rgb: [f32; 3], index: usize) -> [f32; 3] {
        let noise = self.pattern.as_slice()[index];
        let (y, v) = perceptual(rgb);
        // Grain shows most in the midtones and fades toward black and
        // white, as it does on film.
        let presence = (4.0 * v * (1.0 - v)).clamp(0.0, 1.0).sqrt();
        relight(rgb, y, v + self.strength * noise * presence)
    }
}
