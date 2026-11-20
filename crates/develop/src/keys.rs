//! What each stage cache depends on. Every key is built from the real
//! inputs of its stage and compared by value, so a cache is right by
//! construction rather than by remembering to mark it dirty.

use std::sync::Arc;

use photo_core::develop::{DevelopSettings, NoiseReduction, Spot, WhiteBalance};
use photo_imaging::{Geometry, View};

use crate::front::Front;
use crate::guides::{Guides, Needs};
use crate::masks::{Alphas, ShapeKey, active_masks, shape_keys};
use crate::plan::Plan;
use crate::source::Source;

/// A source compared by identity.
#[derive(Clone)]
pub(crate) struct SameSource(pub Source);

impl PartialEq for SameSource {
    fn eq(&self, other: &Self) -> bool {
        self.0.same(&other.0)
    }
}

/// What cache A depends on: the real inputs of stages 1 to 4.
#[derive(Clone, PartialEq)]
pub(crate) struct FrontKey {
    source: SameSource,
    view: View,
    geometry: Geometry,
    padding: u32,
    spots: Vec<Spot>,
    white_balance: WhiteBalance,
    vignetting: (f32, f32),
    noise: NoiseReduction,
    /// For each active mask that adjusts stage 3 or 4: its shape, and its
    /// amount, temperature, tint and noise.
    masks: Vec<(ShapeKey, [f32; 4])>,
}

impl FrontKey {
    pub(crate) fn new(source: &Source, settings: &DevelopSettings, plan: &Plan) -> Self {
        let masks = active_masks(settings)
            .zip(shape_keys(settings))
            .filter(|(mask, _)| {
                let a = &mask.adjust;
                a.temp != 0.0 || a.tint != 0.0 || a.noise != 0.0
            })
            .map(|(mask, shape)| {
                let a = &mask.adjust;
                (shape, [mask.amount, a.temp, a.tint, a.noise])
            })
            .collect();
        Self {
            source: SameSource(source.clone()),
            view: plan.view,
            geometry: plan.geometry.clone(),
            padding: plan.front_padding,
            spots: settings.spots.clone(),
            white_balance: settings.white_balance,
            vignetting: (settings.lens.vignetting, settings.lens.vignetting_midpoint),
            noise: settings.detail.noise_reduction,
            masks,
        }
    }
}

/// What the mask alpha planes depend on.
#[derive(Clone, PartialEq)]
pub(crate) struct AlphaKey {
    source: SameSource,
    view: View,
    geometry: Geometry,
    padding: u32,
    shapes: Vec<ShapeKey>,
}

impl AlphaKey {
    pub(crate) fn new(source: &Source, settings: &DevelopSettings, plan: &Plan) -> Self {
        Self {
            source: SameSource(source.clone()),
            view: plan.view,
            geometry: plan.geometry.clone(),
            padding: plan.front_padding,
            shapes: shape_keys(settings),
        }
    }
}

/// What the output of stages 5 to 8 (cache B) depends on: the cached
/// products it was made from, by identity, and every setting those stages
/// read; texture, sharpening and effects, which come after, are put back to
/// their defaults.
pub(crate) struct PointKey {
    front: Arc<Front>,
    guides: Arc<Guides>,
    alphas: Arc<Alphas>,
    settings: DevelopSettings,
    padding: u32,
}

impl PointKey {
    pub(crate) fn new(
        front: &Arc<Front>,
        guides: &Arc<Guides>,
        alphas: &Arc<Alphas>,
        settings: &DevelopSettings,
        plan: &Plan,
    ) -> Self {
        let mut relevant = settings.clone();
        relevant.detail = Default::default();
        relevant.effects = Default::default();
        relevant.presence.texture = 0.0;
        for mask in &mut relevant.masks {
            mask.adjust.texture = 0.0;
            mask.adjust.sharpness = 0.0;
            mask.adjust.noise = 0.0;
        }
        Self {
            front: front.clone(),
            guides: guides.clone(),
            alphas: alphas.clone(),
            settings: relevant,
            padding: plan.downstream,
        }
    }
}

impl PartialEq for PointKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.front, &other.front)
            && Arc::ptr_eq(&self.guides, &other.guides)
            && Arc::ptr_eq(&self.alphas, &other.alphas)
            && self.settings == other.settings
            && self.padding == other.padding
    }
}

/// What the whole-image guides depend on: every setting of the stages they
/// render (3 and 5 to 8, masks and spots included), with the ones that do
/// not reach them (geometry other than lens vignetting, detail, effects,
/// texture and the clarity amount) put back to their defaults.
#[derive(Clone, PartialEq)]
pub(crate) struct GuidesKey {
    source: SameSource,
    settings: DevelopSettings,
    needs: Needs,
}

impl GuidesKey {
    pub(crate) fn new(source: &Source, settings: &DevelopSettings, needs: Needs) -> Self {
        let mut relevant = settings.clone();
        relevant.lens.distortion = 0.0;
        relevant.orientation = Default::default();
        relevant.transform = Default::default();
        relevant.crop = Default::default();
        relevant.detail = Default::default();
        relevant.effects = Default::default();
        relevant.presence.texture = 0.0;
        relevant.presence.clarity = 0.0;
        for mask in &mut relevant.masks {
            mask.adjust.texture = 0.0;
            mask.adjust.clarity = 0.0;
            mask.adjust.sharpness = 0.0;
            mask.adjust.noise = 0.0;
        }
        Self {
            source: SameSource(source.clone()),
            settings: relevant,
            needs,
        }
    }
}
