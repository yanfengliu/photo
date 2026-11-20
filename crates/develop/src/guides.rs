//! Whole-image guides for the large-radius stages: the edge-aware base of
//! Highlights and Shadows (stage 5), the dehaze transmission and the local
//! mean of clarity (stage 8).
//!
//! Their radii are whole percent of the long edge, far too large to pad a
//! view or tile by. They are guided filters (He, Sun and Tang, "Guided
//! Image Filtering", ECCV 2010) computed the fast way of He and Sun ("Fast
//! Guided Filter", 2015): the linear coefficients `(a, b)` of the filter
//! are found on a reduced image and interpolated up, and only `a · I + b`
//! runs at full resolution, with the full-resolution guide `I`, so edges
//! stay as sharp as the render's own. The reduced grid here is the source's
//! analysis reduction, in source coordinates: every view, scale and tile of
//! a render reads the same coefficients at the same source positions, so
//! the base, the transmission and the mean are the same function of
//! position everywhere (contracts R1 to R3 hold for these stages by
//! construction), and no render pays for a large filter.
//!
//! The reduction is rendered once per change of the settings these depend
//! on, through the same per-pixel code as the full render (stages 3 and 5
//! to 8 with masks, spots and lens vignetting).
//!
//! The tone base is filtered in log₂ scene luminance (stops), as darktable's
//! tone equalizer filters its mask, and only then turned into the
//! perceptual display value that `PixelPipeline::tone_base` defines. In
//! display terms a base curve squeezes the stops above nominal white into
//! the last fraction of a percent below 1, where no edge threshold can tell
//! an edge from texture, and the zone gain changes steeply with the base:
//! filtering there left a bright band along every edge of recovered
//! highlights. In stops every edge and every texture has the same size
//! wherever it lies on the curve.

use std::borrow::Cow;

use photo_color::{LocalDelta, PixelPipeline};
use photo_core::colorimetry::luminance;
use photo_core::develop::DevelopSettings;
use photo_core::image::SourceImage;
use photo_imaging::filter::{box_blur, min_filter};
use photo_imaging::{Geometry, Plane, View, heal_image};
use rayon::prelude::*;

use crate::analysis::DARK_CHANNEL_RADIUS;
use crate::cancel::CancelToken;
use crate::lens::LensVignetting;
use crate::masks::{Alphas, Layers};
use crate::presence::{DEHAZE_MIN_LIGHT, DEHAZE_OMEGA, dehaze_pixel, finite_or_zero, perceptual};
use crate::source::{Source, unit_pixel};

// ---- Tunable constants ----------------------------------------------------

/// The tone base: radius as a fraction of the long edge, and the variance
/// (in stops squared) above which a feature counts as an edge the base
/// follows rather than a texture it smooths over: texture of a few tenths
/// of a stop stays out of the base, a step of a stop or more stays in it.
/// The radius is that of darktable's tone equalizer at its default
/// smoothing diameter (5%).
const BASE_RADIUS: f32 = 0.025;
const BASE_EPS: f32 = 0.025;
/// Luminance is held at least this (about -20 stops) before its logarithm
/// is taken.
const LUMINANCE_FLOOR: f32 = 1.0e-6;
/// The dehaze transmission's refinement: radius and edge variance.
const HAZE_RADIUS: f32 = 0.03;
const HAZE_EPS: f32 = 1.0e-3;
/// Clarity's local mean: radius (DESIGN §5.7: about 1% of the long edge)
/// and edge variance. A strong edge must stay out of the mean almost
/// entirely, or clarity turns the part that leaks in into a halo along it
/// (at 0.01 a patch on a dark ground took a rim 10 codes brighter than its
/// middle); structure up to about 0.03 in perceptual value is boosted.
const CLARITY_RADIUS: f32 = 0.01;
const CLARITY_EPS: f32 = 0.001;
/// No variance is trusted below this; see the imaging crate's guided
/// filter.
const MIN_EPS: f32 = 1.0e-6;

// ---------------------------------------------------------------------------

/// Which guides a render needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Needs {
    pub base: bool,
    pub dehaze: bool,
    pub clarity: bool,
}

/// Interleaved `(a, b)` coefficient planes on the reduction's grid.
type Coefficients = Vec<[f32; 2]>;

/// The guides of one set of settings for one source.
pub(crate) struct Guides {
    width: usize,
    height: usize,
    /// Reduction pixels per source pixel, along x and y.
    to_reduction: (f32, f32),
    base: Option<Coefficients>,
    haze: Option<Coefficients>,
    clarity: Option<Coefficients>,
    /// The dehaze atmospheric light, display-linear working RGB.
    light: [f32; 3],
}

impl Guides {
    /// Render the reduction and fit the guides `needs` asks for.
    pub(crate) fn build(
        source: &Source,
        settings: &DevelopSettings,
        pipeline: &PixelPipeline,
        needs: Needs,
        cancel: &CancelToken,
    ) -> Option<Self> {
        let reduction = source.analysis().reduction();
        let image: Cow<SourceImage> = if settings.spots.is_empty() {
            Cow::Borrowed(reduction)
        } else {
            Cow::Owned(heal_image(reduction, &settings.spots))
        };
        let (width, height) = (image.width as usize, image.height as usize);
        let (source_width, source_height) = source.size();
        let mut guides = Self {
            width,
            height,
            to_reduction: (
                width as f32 / source_width as f32,
                height as f32 / source_height as f32,
            ),
            base: None,
            haze: None,
            clarity: None,
            light: [1.0; 3],
        };
        if needs == Needs::default() {
            return Some(guides);
        }
        let long_edge = width.max(height) as f32;
        let grid = Geometry::identity(width as u32, height as u32);
        let alphas = Alphas::rasterize(settings, &grid, &View::full(&grid), 0);
        let layers = Layers::new(&alphas, settings);
        let lens = LensVignetting::new(&settings.lens, source.size());
        let at = |index: usize| {
            (
                ((index % width) as f32 + 0.5) / width as f32,
                ((index / width) as f32 + 0.5) / height as f32,
            )
        };

        // Stage 3, then the tone base.
        let scene: Vec<[f32; 3]> = (0..width * height)
            .into_par_iter()
            .map(|index| {
                let delta = layers.delta(index);
                let rgb = pipeline.input(unit_pixel(&image, index), delta.as_ref());
                match &lens {
                    Some(lens) => {
                        let (u, v) = at(index);
                        lens.apply(rgb, u, v)
                    }
                    None => rgb,
                }
            })
            .collect();
        cancel.proceed()?;
        let mut base = vec![0.0f32; width * height];
        if needs.base {
            let values = plane(width, height, |index| {
                stops(scene[index], layers.delta(index).as_ref())
            });
            let coefficients = coefficients(&values, None, BASE_RADIUS * long_edge, BASE_EPS);
            base.par_iter_mut().enumerate().for_each(|(index, value)| {
                let [a, b] = coefficients[index];
                *value = tone_base(pipeline, a * values.as_slice()[index] + b);
            });
            guides.base = Some(coefficients);
            cancel.proceed()?;
        }
        if !(needs.dehaze || needs.clarity) {
            return Some(guides);
        }

        // Stages 5 to 7, then dehaze and clarity's mean.
        let mut display: Vec<[f32; 3]> = (0..width * height)
            .into_par_iter()
            .map(|index| {
                let delta = layers.delta(index);
                let delta = delta.as_ref();
                let shown = pipeline.scene_to_display(scene[index], base[index], delta);
                pipeline.color(pipeline.tone(shown, delta), delta)
            })
            .collect();
        cancel.proceed()?;
        let dehaze = finite_or_zero(settings.presence.dehaze);
        if needs.dehaze {
            let light = atmospheric_light(&display, source.analysis().haze_pixels());
            let dark = plane(width, height, |index| {
                let d = display[index];
                (d[0] / light[0]).min(d[1] / light[1]).min(d[2] / light[2])
            });
            // The dark channel is the minimum over a window (an erosion),
            // which eats into every bright region from its border; opening
            // it (the same window's maximum after the minimum, Serra 1982)
            // gives each region back its own value up to its edge, so a
            // flat area gets one transmission instead of a darker middle.
            let radius = (DARK_CHANNEL_RADIUS * long_edge).round().max(1.0) as usize;
            let eroded = min_filter(&dark, radius);
            let opened = min_filter(&eroded.map(|d| -d), radius);
            let transmission = opened.map(|d| 1.0 + DEHAZE_OMEGA * d);
            let guide = plane(width, height, |index| perceptual(display[index]).1);
            let haze = coefficients(
                &guide,
                Some(&transmission),
                HAZE_RADIUS * long_edge,
                HAZE_EPS,
            );
            display.par_iter_mut().enumerate().for_each(|(index, rgb)| {
                let [a, b] = haze[index];
                let amount = dehaze + layers.extras(index).dehaze;
                let haze = haze_of(a * guide.as_slice()[index] + b);
                *rgb = dehaze_pixel(*rgb, light, haze, amount);
            });
            guides.haze = Some(haze);
            guides.light = light;
            cancel.proceed()?;
        }
        if needs.clarity {
            let values = plane(width, height, |index| perceptual(display[index]).1);
            guides.clarity = Some(coefficients(
                &values,
                None,
                CLARITY_RADIUS * long_edge,
                CLARITY_EPS,
            ));
        }
        cancel.proceed()?;
        Some(guides)
    }

    /// Reduction coordinates of source position `(x, y)` (source pixels).
    #[inline]
    pub(crate) fn locate(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.to_reduction.0, y * self.to_reduction.1)
    }

    /// The tone base (`PixelPipeline::tone_base`'s perceptual display value)
    /// at reduction position `at` for a pixel whose own [`stops`] are
    /// `value`. Only meaningful when the base was asked for.
    #[inline]
    pub(crate) fn base(&self, pipeline: &PixelPipeline, at: (f32, f32), value: f32) -> f32 {
        tone_base(pipeline, self.apply(self.base.as_deref(), at, value))
    }

    /// The haze (`1 - t`, `0..ω`) at `at` for a pixel whose perceptual
    /// luminance before dehazing is `value`.
    #[inline]
    pub(crate) fn haze(&self, at: (f32, f32), value: f32) -> f32 {
        haze_of(self.apply(self.haze.as_deref(), at, value))
    }

    /// Clarity's edge-aware local mean at `at` for perceptual value `value`.
    #[inline]
    pub(crate) fn clarity_mean(&self, at: (f32, f32), value: f32) -> f32 {
        self.apply(self.clarity.as_deref(), at, value)
    }

    /// The dehaze atmospheric light.
    pub(crate) fn light(&self) -> [f32; 3] {
        self.light
    }

    #[inline]
    fn apply(&self, coefficients: Option<&[[f32; 2]]>, at: (f32, f32), value: f32) -> f32 {
        match coefficients {
            Some(coefficients) => {
                let [a, b] = self.sample(coefficients, at);
                a * value + b
            }
            None => value,
        }
    }

    /// Bilinear sample of a coefficient plane at a reduction position,
    /// replicating the edge.
    #[inline]
    fn sample(&self, coefficients: &[[f32; 2]], (x, y): (f32, f32)) -> [f32; 2] {
        let axis = |position: f32, len: usize| {
            let position = if position.is_finite() { position } else { 0.0 };
            let p = (position - 0.5).clamp(0.0, (len - 1) as f32);
            let low = p as usize;
            (low, (low + 1).min(len - 1), p - low as f32)
        };
        let (x0, x1, tx) = axis(x, self.width);
        let (y0, y1, ty) = axis(y, self.height);
        let at = |x: usize, y: usize| coefficients[y * self.width + x];
        let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bottom = c[k] + (d[k] - c[k]) * tx;
            top + (bottom - top) * ty
        })
    }
}

/// What the tone base is filtered in: the pixel's scene luminance in stops,
/// its local exposure (masks) included.
#[inline]
pub(crate) fn stops(rgb: [f32; 3], delta: Option<&LocalDelta>) -> f32 {
    luminance(rgb).max(LUMINANCE_FLOOR).log2() + delta.map_or(0.0, |delta| delta.exposure)
}

/// The perceptual base of a smoothed luminance in stops: the tone base of a
/// neutral pixel of that luminance.
#[inline]
fn tone_base(pipeline: &PixelPipeline, stops: f32) -> f32 {
    pipeline.tone_base([stops.exp2(); 3], None)
}

/// The haze `1 - t` of a refined transmission, held to `0..ω`.
#[inline]
fn haze_of(transmission: f32) -> f32 {
    (1.0 - transmission).clamp(0.0, DEHAZE_OMEGA)
}

/// The mean colour of the haze pixels in the current rendering, held at
/// least [`DEHAZE_MIN_LIGHT`] per channel.
fn atmospheric_light(display: &[[f32; 3]], haze: &[u32]) -> [f32; 3] {
    let mut sum = [0.0f64; 3];
    for &index in haze {
        for (total, value) in sum.iter_mut().zip(display[index as usize]) {
            *total += f64::from(value);
        }
    }
    let count = haze.len().max(1) as f64;
    sum.map(|total| ((total / count) as f32).max(DEHAZE_MIN_LIGHT))
}

fn plane(width: usize, height: usize, f: impl Fn(usize) -> f32 + Sync) -> Plane {
    Plane::from_fn(width, height, |x, y| f(y * width + x))
}

/// The window-averaged coefficients of the guided filter of `input` (or of
/// the guide itself) steered by `guide`: `a = cov(I, p) / (var(I) + eps)`
/// and `b = mean(p) - a · mean(I)` in every `radius` window, each averaged
/// over the windows covering a pixel. The filter's output is then
/// `a · I + b`. Box blurs replicate the edges.
fn coefficients(guide: &Plane, input: Option<&Plane>, radius: f32, eps: f32) -> Coefficients {
    let eps = eps.max(MIN_EPS);
    let mean_guide = box_blur(guide, radius);
    let mean_square = box_blur(&guide.map(|g| g * g), radius);
    let (a, b) = match input {
        None => {
            let a = mean_square.zip_map(&mean_guide, |square, mean| {
                let variance = (square - mean * mean).max(0.0);
                variance / (variance + eps)
            });
            let b = a.zip_map(&mean_guide, |a, mean| (1.0 - a) * mean);
            (a, b)
        }
        Some(input) => {
            let mean_input = box_blur(input, radius);
            let mean_product = box_blur(&guide.zip_map(input, |g, p| g * p), radius);
            let a = Plane::from_fn(guide.width(), guide.height(), |x, y| {
                let mean = mean_guide.get(x, y);
                let variance = (mean_square.get(x, y) - mean * mean).max(0.0);
                (mean_product.get(x, y) - mean * mean_input.get(x, y)) / (variance + eps)
            });
            let b = Plane::from_fn(guide.width(), guide.height(), |x, y| {
                mean_input.get(x, y) - a.get(x, y) * mean_guide.get(x, y)
            });
            (a, b)
        }
    };
    let (a, b) = (box_blur(&a, radius), box_blur(&b, radius));
    a.as_slice()
        .iter()
        .zip(b.as_slice())
        .map(|(&a, &b)| [a, b])
        .collect()
}
