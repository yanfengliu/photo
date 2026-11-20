//! The renderer: stage order, stage caches and the two output formats.

use std::sync::Arc;
use std::time::{Duration, Instant};

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{DevelopSettings, Spot};
use photo_core::image::{Rgb16Image, Rgba8Image};
use photo_imaging::{Plane, RgbImage, SourcePyramid, View, heal_level, resample_level};
use rayon::prelude::*;

use crate::cancel::CancelToken;
use crate::effects::{FilmGrain, GrainKey};
use crate::front::{Front, develop_front};
use crate::guides::Guides;
use crate::keys::{AlphaKey, FrontKey, GuidesKey, PointKey, SameSource};
use crate::masks::{Alphas, Layers};
use crate::plan::Plan;
use crate::point::develop_point;
use crate::presence::apply_texture;
use crate::sharpen::apply_sharpen;
use crate::source::Source;

/// How hard a render may try. Both satisfy contract T1 and every other
/// contract; they differ only where the difference cannot be seen at the
/// view's scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quality {
    /// For previews while a slider moves: stages whose radius at the view's
    /// scale is under half a pixel (texture under one) are skipped.
    Draft,
    /// For a preview at rest, thumbnails and export: nothing visible is
    /// skipped.
    Full,
}

/// One render: which source, with which settings, through which view, at
/// which quality, encoded into which colour space.
#[derive(Clone, Copy, Debug)]
pub struct RenderRequest<'a> {
    /// The photo.
    pub source: &'a Source,
    /// The edit.
    pub settings: &'a DevelopSettings,
    /// The output rectangle to render and the pixel count to render it at.
    pub view: View,
    /// Draft or full quality.
    pub quality: Quality,
    /// The colour space the result is encoded in.
    pub space: RgbSpace,
}

/// A healed pyramid level, kept until the spots change.
struct Healed {
    source: SameSource,
    level: usize,
    spots: Vec<Spot>,
    pyramid: SourcePyramid,
}

/// Healed levels kept at once: enough to go back and forth between a fit
/// view and a 1:1 view without healing again.
const HEALED_LEVELS: usize = 2;

/// Renders requests and keeps the stage caches between them.
///
/// Caches are keyed by the real inputs of each stage, compared by value:
/// cache A (stages 1 to 4) by the source, view, geometry, spots, white
/// balance, lens vignetting, noise reduction and the masks' temperature,
/// tint and noise; the mask alpha planes by the view, geometry and mask
/// shapes; the whole-image guides by the settings of the stages they
/// render; cache B (stages 5 to 8 per pixel) by the cached products it
/// was made from and the settings of those stages; the grain pattern by
/// the view, grain size and roughness; healed pyramid levels by level and
/// spots. Moving a tone or colour slider reuses cache A; moving a texture,
/// sharpening, vignette or grain slider also reuses cache B. Any warm
/// render gives exactly the bytes a fresh renderer would.
#[derive(Default)]
pub struct Renderer {
    healed: Vec<Healed>,
    alphas: Option<(AlphaKey, Arc<Alphas>)>,
    front: Option<(FrontKey, Arc<Front>)>,
    guides: Option<(GuidesKey, Arc<Guides>)>,
    point: Option<(PointKey, Arc<RgbImage>)>,
    grain: Option<(GrainKey, Arc<Plane>)>,
    times: Vec<(&'static str, Duration)>,
}

impl Renderer {
    /// A renderer with empty caches.
    pub fn new() -> Self {
        Self::default()
    }

    /// Render for the screen: 8-bit RGBA (opaque) in `request.space`, the
    /// size of `request.view`. `None` when `cancel` was cancelled before
    /// the render finished; the renderer stays usable either way.
    pub fn render_rgba8(
        &mut self,
        request: &RenderRequest,
        cancel: &CancelToken,
    ) -> Option<Rgba8Image> {
        let (width, height) = (request.view.width(), request.view.height());
        let pixels = self.render(request, cancel, 4, |plan, rgb, out: &mut [u8]| {
            let [r, g, b] = plan.pipeline.output_u8(rgb);
            out.copy_from_slice(&[r, g, b, u8::MAX]);
        })?;
        Some(Rgba8Image {
            width,
            height,
            pixels,
            space: request.space,
        })
    }

    /// Render for export: 16-bit RGB in `request.space`, the size of
    /// `request.view`, encoded exactly (no lookup table). `None` when
    /// cancelled. Agrees with [`Self::render_rgba8`] to 8-bit precision.
    pub fn render_rgb16(
        &mut self,
        request: &RenderRequest,
        cancel: &CancelToken,
    ) -> Option<Rgb16Image> {
        let (width, height) = (request.view.width(), request.view.height());
        let pixels = self.render(request, cancel, 3, |plan, rgb, out: &mut [u16]| {
            out.copy_from_slice(&plan.pipeline.output_u16(rgb));
        })?;
        Some(Rgb16Image {
            width,
            height,
            pixels,
            space: request.space,
        })
    }

    /// How long each stage that ran in the most recent render took, in the
    /// order they ran, for profiling. A stage served from its cache does
    /// not appear.
    pub fn last_stage_times(&self) -> &[(&'static str, Duration)] {
        &self.times
    }

    /// Stages 1 to 10, then `encode` (stage 11) into `channels` samples per
    /// pixel.
    fn render<T: Copy + Default + Send>(
        &mut self,
        request: &RenderRequest,
        cancel: &CancelToken,
        channels: usize,
        encode: impl Fn(&Plan, [f32; 3], &mut [T]) + Sync,
    ) -> Option<Vec<T>> {
        self.times.clear();
        cancel.proceed()?;
        let (width, height) = (request.view.width(), request.view.height());
        if width == 0 || height == 0 {
            return Some(Vec::new());
        }
        let plan = Plan::new(request);
        let alphas = self.alphas(request, &plan);
        let layers = Layers::new(&alphas, request.settings);
        cancel.proceed()?;
        let guides = self.guides(request, &plan, cancel)?;
        let front = self.front(request, &plan, &layers, cancel)?;
        let key = PointKey::new(&front, &guides, &alphas, request.settings, &plan);
        let point = match &self.point {
            Some((cached, point)) if *cached == key => point.clone(),
            _ => {
                let start = Instant::now();
                let point = Arc::new(develop_point(&front, &plan, &layers, &guides, cancel)?);
                self.times.push(("stages 5-8 per pixel", start.elapsed()));
                self.point = Some((key, point.clone()));
                point
            }
        };
        let textured = match &plan.texture {
            Some(texture) => {
                let start = Instant::now();
                let image = apply_texture(&point, texture, &layers, plan.downstream, cancel)?;
                self.times.push(("texture", start.elapsed()));
                Some(image)
            }
            None => None,
        };
        let sharpened = match &plan.sharpen {
            Some(sharpen) => {
                let start = Instant::now();
                let input = textured.as_ref().unwrap_or(&point);
                let image = apply_sharpen(input, sharpen, &layers, plan.downstream, cancel)?;
                self.times.push(("sharpen", start.elapsed()));
                Some(image)
            }
            None => None,
        };
        let image = sharpened.as_ref().or(textured.as_ref()).unwrap_or(&point);
        let grain_settings = &request.settings.effects.grain;
        let pattern = GrainKey::new(grain_settings, &plan.view).map(|key| (key, self.grain(key)));
        let grain = pattern
            .as_ref()
            .map(|(key, pattern)| FilmGrain::new(grain_settings, key, pattern));
        let start = Instant::now();
        let out = finish(image, &plan, grain.as_ref(), cancel, channels, encode);
        self.times.push(("effects and output", start.elapsed()));
        out
    }

    /// The grain pattern for `key`, from the cache or made.
    fn grain(&mut self, key: GrainKey) -> Arc<Plane> {
        if let Some((cached, pattern)) = &self.grain
            && *cached == key
        {
            return pattern.clone();
        }
        let start = Instant::now();
        let pattern = Arc::new(key.pattern());
        self.times.push(("grain pattern", start.elapsed()));
        self.grain = Some((key, pattern.clone()));
        pattern
    }

    fn alphas(&mut self, request: &RenderRequest, plan: &Plan) -> Arc<Alphas> {
        let key = AlphaKey::new(request.source, request.settings, plan);
        if let Some((cached, alphas)) = &self.alphas
            && *cached == key
        {
            return alphas.clone();
        }
        let start = Instant::now();
        let alphas = Arc::new(Alphas::rasterize(
            request.settings,
            &plan.geometry,
            &plan.view,
            plan.front_padding,
        ));
        self.times.push(("mask alphas", start.elapsed()));
        self.alphas = Some((key, alphas.clone()));
        alphas
    }

    fn guides(
        &mut self,
        request: &RenderRequest,
        plan: &Plan,
        cancel: &CancelToken,
    ) -> Option<Arc<Guides>> {
        let key = GuidesKey::new(request.source, request.settings, plan.needs);
        if let Some((cached, guides)) = &self.guides
            && *cached == key
        {
            return Some(guides.clone());
        }
        let start = Instant::now();
        let guides = Arc::new(Guides::build(
            request.source,
            request.settings,
            &plan.pipeline,
            plan.needs,
            cancel,
        )?);
        self.times.push(("whole-image guides", start.elapsed()));
        self.guides = Some((key, guides.clone()));
        Some(guides)
    }

    fn front(
        &mut self,
        request: &RenderRequest,
        plan: &Plan,
        layers: &Layers,
        cancel: &CancelToken,
    ) -> Option<Arc<Front>> {
        let key = FrontKey::new(request.source, request.settings, plan);
        if let Some((cached, front)) = &self.front
            && *cached == key
        {
            return Some(front.clone());
        }
        let start = Instant::now();
        let pyramid = self.healed_pyramid(request, plan);
        self.times.push(("heal", start.elapsed()));
        cancel.proceed()?;
        let front = Arc::new(develop_front(
            request.source,
            &pyramid,
            plan,
            layers,
            cancel,
            &mut self.times,
        )?);
        self.front = Some((key, front.clone()));
        Some(front)
    }

    /// Stage 1: the pyramid with the level this view samples healed, or
    /// the source's own pyramid (nothing copied) when there are no spots.
    fn healed_pyramid(&mut self, request: &RenderRequest, plan: &Plan) -> SourcePyramid {
        let source = request.source;
        let spots = &request.settings.spots;
        if spots.is_empty() {
            return source.pyramid().clone();
        }
        let level = resample_level(source.pyramid(), &plan.geometry, &plan.view);
        let wanted = SameSource(source.clone());
        if let Some(healed) = self
            .healed
            .iter()
            .find(|h| h.source == wanted && h.level == level && h.spots == *spots)
        {
            return healed.pyramid.clone();
        }
        let level_image = Arc::new(heal_level(source.pyramid(), level, spots));
        let pyramid = source
            .pyramid()
            .with_level(level, level_image)
            .unwrap_or_else(|| source.pyramid().clone());
        self.healed
            .retain(|h| h.source == wanted && h.spots == *spots);
        if self.healed.len() >= HEALED_LEVELS {
            self.healed.remove(0);
        }
        self.healed.push(Healed {
            source: wanted,
            level,
            spots: spots.clone(),
            pyramid: pyramid.clone(),
        });
        pyramid
    }
}

/// Stage 10 (vignette and grain, in output coordinates) and stage 11 (the
/// encoding `encode` does) for the view's own pixels.
fn finish<T: Copy + Default + Send>(
    image: &RgbImage,
    plan: &Plan,
    grain: Option<&FilmGrain>,
    cancel: &CancelToken,
    channels: usize,
    encode: impl Fn(&Plan, [f32; 3], &mut [T]) + Sync,
) -> Option<Vec<T>> {
    let view = &plan.view;
    let (width, height) = (view.width() as usize, view.height() as usize);
    let padding = plan.downstream as usize;
    cancel.proceed()?;
    let mut out = vec![T::default(); width * height * channels];
    // Each effect is its own pass over a row, as in stages 5 to 8.
    out.par_chunks_mut(width * channels)
        .enumerate()
        .for_each_init(
            || Vec::with_capacity(width),
            |scratch: &mut Vec<[f32; 3]>, (y, row)| {
                if cancel.is_cancelled() {
                    return;
                }
                scratch.clear();
                scratch.extend_from_slice(&image.row(y + padding)[padding..padding + width]);
                if let Some(vignette) = &plan.vignette {
                    let at_y = view.to_output(0.0, y as f32 + 0.5).y;
                    for (x, rgb) in scratch.iter_mut().enumerate() {
                        let at_x = view.to_output(x as f32 + 0.5, 0.0).x;
                        *rgb = vignette.apply(*rgb, at_x, at_y);
                    }
                }
                if let Some(grain) = grain {
                    for (x, rgb) in scratch.iter_mut().enumerate() {
                        *rgb = grain.apply(*rgb, y * width + x);
                    }
                }
                for (rgb, out) in scratch.iter().zip(row.chunks_exact_mut(channels)) {
                    encode(plan, *rgb, out);
                }
            },
        );
    cancel.proceed()?;
    Some(out)
}
