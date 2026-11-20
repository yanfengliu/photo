//! Masks as per-pixel effective parameters (DESIGN §5.2).
//!
//! Masks are not a stage. Each active mask is rasterized once per view into
//! an alpha plane, and every stage that a mask can adjust reads the pixel's
//! effective value `global + Σ alphaᵢ · amountᵢ · localᵢ`. Two overlapping
//! +1 EV masks therefore add up to +2 EV, and an inert mask (disabled, no
//! amount, no shape or no adjustment) is never rasterized and costs nothing.

use photo_color::LocalDelta;
use photo_core::develop::{DevelopSettings, LocalAdjustments, Mask, MaskComponent};
use photo_imaging::{Geometry, Plane, View, rasterize_mask};
use rayon::prelude::*;

/// The coverage of mask `mask_index` of `settings` over `view`, `0..=1`
/// with no padding: what the red mask overlay shows. It is the shape alone
/// (components and invert), whether or not the mask is enabled or has any
/// adjustment, exactly as the renderer rasterizes it for that view. `None`
/// when there is no such mask.
pub fn mask_alpha(
    source: &crate::Source,
    settings: &DevelopSettings,
    mask_index: usize,
    view: View,
) -> Option<Plane> {
    let mask = settings.masks.get(mask_index)?;
    let geometry = crate::geometry(source, settings);
    Some(rasterize_mask(mask, &geometry, &view, 0))
}

/// The masks that take part in a render, in order.
pub(crate) fn active_masks(settings: &DevelopSettings) -> impl Iterator<Item = &Mask> {
    settings.masks.iter().filter(|mask| !mask.is_inert())
}

/// A mask's amount as a weight, `0..=1`.
fn weight(mask: &Mask) -> f32 {
    if mask.amount.is_finite() {
        (mask.amount / 100.0).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Which local adjustments the active masks use, by the stage that reads
/// them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MaskUse {
    /// Temperature or tint (stage 3).
    pub input: bool,
    /// Noise (stage 4).
    pub noise: bool,
    /// Exposure, highlights or shadows (stage 5).
    pub scene: bool,
    /// Highlights or shadows (the stage-5 base).
    pub zones: bool,
    /// Contrast, whites or blacks (stage 6).
    pub tone: bool,
    /// Saturation (stage 7).
    pub color: bool,
    /// Dehaze (stage 8).
    pub dehaze: bool,
    /// Clarity (stage 8).
    pub clarity: bool,
    /// Texture (stage 8).
    pub texture: bool,
    /// Sharpness (stage 9).
    pub sharpness: bool,
}

impl MaskUse {
    pub(crate) fn of(settings: &DevelopSettings) -> Self {
        let mut used = Self::default();
        for mask in active_masks(settings) {
            let a = &mask.adjust;
            used.input |= a.temp != 0.0 || a.tint != 0.0;
            used.noise |= a.noise != 0.0;
            used.scene |= a.exposure != 0.0 || a.highlights != 0.0 || a.shadows != 0.0;
            used.zones |= a.highlights != 0.0 || a.shadows != 0.0;
            used.tone |= a.contrast != 0.0 || a.whites != 0.0 || a.blacks != 0.0;
            used.color |= a.saturation != 0.0;
            used.dehaze |= a.dehaze != 0.0;
            used.clarity |= a.clarity != 0.0;
            used.texture |= a.texture != 0.0;
            used.sharpness |= a.sharpness != 0.0;
        }
        used
    }
}

/// What an alpha plane depends on: a mask's shape.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ShapeKey {
    components: Vec<MaskComponent>,
    invert: bool,
}

/// The shapes of the active masks, in order.
pub(crate) fn shape_keys(settings: &DevelopSettings) -> Vec<ShapeKey> {
    active_masks(settings)
        .map(|mask| ShapeKey {
            components: mask.components.clone(),
            invert: mask.invert,
        })
        .collect()
}

/// The alpha planes of the active masks for one view, all with the same
/// size and padding.
pub(crate) struct Alphas {
    planes: Vec<Plane>,
    width: usize,
    padding: u32,
}

impl Alphas {
    /// Rasterize every active mask of `settings` for `view`, padded by
    /// `padding` pixels (the layout of `resample`).
    pub(crate) fn rasterize(
        settings: &DevelopSettings,
        geometry: &Geometry,
        view: &View,
        padding: u32,
    ) -> Self {
        let planes = active_masks(settings)
            .map(|mask| rasterize_mask(mask, geometry, view, padding))
            .collect();
        Self {
            planes,
            width: view.width() as usize + 2 * padding as usize,
            padding,
        }
    }
}

/// The local values the colour crate does not take: presence, sharpness
/// and noise, each the alpha-weighted sum over the masks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Extras {
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub sharpness: f32,
    pub noise: f32,
}

/// The masks of one render: their alpha planes with this render's amounts
/// and adjustments.
pub(crate) struct Layers<'a> {
    alphas: &'a Alphas,
    adjustments: Vec<(f32, LocalAdjustments)>,
}

impl<'a> Layers<'a> {
    /// Pair the alpha planes (rasterized for the same `settings`' active
    /// masks) with the masks' current amounts and adjustments.
    pub(crate) fn new(alphas: &'a Alphas, settings: &DevelopSettings) -> Self {
        let adjustments = active_masks(settings)
            .map(|mask| (weight(mask), mask.adjust))
            .collect();
        Self {
            alphas,
            adjustments,
        }
    }

    /// True when no mask takes part.
    pub(crate) fn is_empty(&self) -> bool {
        self.adjustments.is_empty()
    }

    /// The index into the alpha planes of pixel `(x, y)` of a buffer of the
    /// same view padded by `padding` (at most the planes' own).
    #[inline]
    pub(crate) fn index(&self, x: usize, y: usize, padding: u32) -> usize {
        let offset = (self.alphas.padding - padding) as usize;
        (y + offset) * self.alphas.width + x + offset
    }

    /// The weights `alphaᵢ · amountᵢ` of the masks covering pixel `index`.
    #[inline]
    fn covering(&self, index: usize) -> impl Iterator<Item = (f32, &LocalAdjustments)> {
        self.alphas.planes.iter().zip(&self.adjustments).filter_map(
            move |(plane, (weight, adjust))| {
                let alpha = plane.as_slice()[index];
                (alpha > 0.0).then_some((alpha * weight, adjust))
            },
        )
    }

    /// The pixel's [`LocalDelta`], or `None` outside every mask.
    #[inline]
    pub(crate) fn delta(&self, index: usize) -> Option<LocalDelta> {
        let mut delta = None;
        for (weight, adjust) in self.covering(index) {
            delta
                .get_or_insert_with(LocalDelta::default)
                .add(adjust, weight);
        }
        delta
    }

    /// The pixel's presence, sharpness and noise sums.
    #[inline]
    pub(crate) fn extras(&self, index: usize) -> Extras {
        let mut extras = Extras::default();
        for (weight, adjust) in self.covering(index) {
            extras.texture += adjust.texture * weight;
            extras.clarity += adjust.clarity * weight;
            extras.dehaze += adjust.dehaze * weight;
            extras.sharpness += adjust.sharpness * weight;
            extras.noise += adjust.noise * weight;
        }
        extras
    }

    /// A plane, `width` x `height` with `padding`, of `global` plus each
    /// pixel's sum of `field` over the masks, clamped to `range`: the
    /// effective value of one mask-adjustable setting everywhere.
    pub(crate) fn effective_plane(
        &self,
        width: usize,
        height: usize,
        padding: u32,
        global: f32,
        range: (f32, f32),
        field: impl Fn(&Extras) -> f32 + Sync,
    ) -> Plane {
        let mut plane = Plane::new(width, height);
        plane
            .as_mut_slice()
            .par_chunks_mut(width.max(1))
            .enumerate()
            .for_each(|(y, row)| {
                for (x, value) in row.iter_mut().enumerate() {
                    let local = field(&self.extras(self.index(x, y, padding)));
                    *value = (global + local).clamp(range.0, range.1);
                }
            });
        plane
    }
}
