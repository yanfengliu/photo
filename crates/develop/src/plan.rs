//! One render request compiled: which stages run, with what parameters at
//! this view's scale, and how much padding each spatial stage needs
//! (contract R1).

use photo_color::PixelPipeline;
use photo_core::develop::DevelopSettings;
use photo_imaging::{Geometry, View};

use crate::denoise::Denoise;
use crate::effects::PostCropVignette;
use crate::geometry::geometry;
use crate::guides::Needs;
use crate::lens::LensVignetting;
use crate::locate::Locator;
use crate::masks::MaskUse;
use crate::presence::{Texture, finite_or_zero, texture_support_max};
use crate::render::RenderRequest;
use crate::sharpen::{Sharpen, sharpen_support_max};

/// Everything a render decides before touching a pixel.
pub(crate) struct Plan {
    pub view: View,
    pub geometry: Geometry,
    pub pipeline: PixelPipeline,
    pub masks: MaskUse,
    /// Source positions of the view's pixels.
    pub locator: Locator,
    pub lens: Option<LensVignetting>,
    pub denoise: Option<Denoise>,
    pub needs: Needs,
    /// The global Dehaze and Clarity sliders.
    pub dehaze: f32,
    pub clarity: f32,
    pub texture: Option<Texture>,
    pub sharpen: Option<Sharpen>,
    pub vignette: Option<PostCropVignette>,
    /// Padding the stages after cache A need: texture and sharpening.
    pub downstream: u32,
    /// Padding of the resampled buffer: denoising's support plus the most
    /// the later stages could need at this scale, so that cache A does not
    /// depend on their settings.
    pub front_padding: u32,
}

impl Plan {
    pub(crate) fn new(request: &RenderRequest) -> Self {
        let settings: &DevelopSettings = request.settings;
        let source = request.source;
        let geometry = geometry(source, settings);
        let view = request.view;
        // Source pixels per view pixel at the centre of the output: what a
        // radius given in source pixels is divided by.
        let footprint = {
            let footprint = geometry.source_pixels_per_output_pixel() / view.scale();
            if footprint.is_finite() && footprint > 0.0 {
                footprint
            } else {
                1.0
            }
        };
        let masks = MaskUse::of(settings);
        let pipeline =
            PixelPipeline::new(settings, &source.color()).with_output_space(request.space);
        let (width, height) = source.size();
        let long_edge = width.max(height) as f32;
        let denoise = Denoise::new(&settings.detail.noise_reduction, masks, footprint);
        let dehaze = finite_or_zero(settings.presence.dehaze);
        let clarity = finite_or_zero(settings.presence.clarity);
        let needs = Needs {
            base: pipeline.tone_needs_base(),
            dehaze: dehaze != 0.0 || masks.dehaze,
            clarity: clarity != 0.0 || masks.clarity,
        };
        let texture = Texture::new(settings, masks, long_edge, footprint, request.quality);
        let sharpen = Sharpen::new(settings, masks, footprint, request.quality);
        let downstream = texture.map_or(0, |t| t.support) + sharpen.map_or(0, |s| s.support);
        let downstream_max =
            texture_support_max(long_edge, footprint) + sharpen_support_max(footprint);
        let front_padding = denoise.as_ref().map_or(0, |d| d.support) + downstream_max;
        Self {
            vignette: PostCropVignette::new(&settings.effects.vignette, geometry.output_size()),
            lens: LensVignetting::new(&settings.lens, source.size()),
            locator: Locator::new(&view, &geometry, settings),
            view,
            geometry,
            pipeline,
            masks,
            denoise,
            needs,
            dehaze,
            clarity,
            texture,
            sharpen,
            downstream,
            front_padding,
        }
    }

    /// True when stage 5 changes anything.
    pub(crate) fn scene_active(&self) -> bool {
        !self.pipeline.scene_to_display_is_identity() || self.masks.scene
    }

    /// True when stage 6 changes anything.
    pub(crate) fn tone_active(&self) -> bool {
        !self.pipeline.tone_is_identity() || self.masks.tone
    }

    /// True when stage 7 changes anything.
    pub(crate) fn color_active(&self) -> bool {
        !self.pipeline.color_is_identity() || self.masks.color
    }
}
