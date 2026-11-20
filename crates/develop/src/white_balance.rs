//! White balance helpers: the as-shot reading, the picker and auto white
//! balance.

use photo_color::PixelPipeline;
use photo_color::camera::camera_as_shot_temp_tint;
use photo_core::colorimetry::luminance;
use photo_core::develop::{DevelopSettings, WbPreset, WhiteBalance};
use photo_core::geom::Point;
use photo_core::image::{SourceColor, SourceImage};

use crate::analysis::CLIPPED;
use crate::geometry::geometry;
use crate::source::{Source, unit_pixel};

// ---- Tunable constants ----------------------------------------------------

/// Pixels darker than this (working-space luminance after balancing) are
/// left out of the grey-world estimate: their colour is mostly noise.
const GREY_WORLD_DARKEST: f32 = 0.005;
/// Pixels whose chroma, `(max - min) / max` after balancing, exceeds this
/// are left out: grey world is an estimate over near-neutral surfaces.
const GREY_WORLD_SATURATION: f32 = 0.3;
/// Below this fraction of usable pixels the saturation limit is dropped.
const GREY_WORLD_MIN_FRACTION: f32 = 0.01;
/// The picker averages a square this many long edges across (at least
/// three pixels), which is enough to average out noise on a grey card.
const PICKER_SIZE: f32 = 0.003;

// ---------------------------------------------------------------------------

/// The camera's as-shot white balance as `(kelvin, tint)` on the DNG scale,
/// for a camera source; `None` for a raster file, whose as-shot state is
/// the file as it is (relative `(0, 0)`).
pub fn as_shot_white_balance(source: &Source) -> Option<(f32, f32)> {
    match source.color() {
        SourceColor::Camera(camera) => Some(camera_as_shot_temp_tint(&camera)),
        SourceColor::Rgb(_) => None,
    }
}

/// The white-balance picker: the custom white balance that renders the
/// source neutral at the output position `output_point` (in full-resolution
/// output pixels, as a view reports them).
///
/// The point is mapped back through the geometry of `settings` and a small
/// square of the source around it is averaged (healing spots are not
/// applied: the picker reads the photo as shot). `None` when the point maps
/// outside the source or the sample has a channel at zero (a pure black or
/// fully saturated sample has no neutral to restore).
pub fn pick_white_balance(
    source: &Source,
    settings: &DevelopSettings,
    output_point: Point,
) -> Option<WhiteBalance> {
    let at = geometry(source, settings).out_to_src(output_point);
    let (width, height) = source.size();
    let inside = at.x.is_finite()
        && at.y.is_finite()
        && (0.0..width as f32).contains(&at.x)
        && (0.0..height as f32).contains(&at.y);
    if !inside {
        return None;
    }
    let image = source.pyramid().level(0);
    let half = ((PICKER_SIZE * width.max(height) as f32) / 2.0).max(1.0) as i64;
    let (cx, cy) = (at.x.floor() as i64, at.y.floor() as i64);
    let mut sum = [0.0f64; 3];
    let mut count = 0.0f64;
    for y in (cy - half).max(0)..=(cy + half).min(i64::from(height) - 1) {
        for x in (cx - half).max(0)..=(cx + half).min(i64::from(width) - 1) {
            let pixel = unit_pixel(image, y as usize * width as usize + x as usize);
            for (total, value) in sum.iter_mut().zip(pixel) {
                *total += f64::from(value);
            }
            count += 1.0;
        }
    }
    let mean = sum.map(|total| (total / count) as f32);
    photo_color::pick_white_balance(&source.color(), mean)
}

/// Auto white balance: a grey-world estimate (Buchsbaum, "A spatial
/// processor model for object colour perception", 1980) refined toward the
/// pixels most likely to be neutral surfaces.
///
/// Over the source's analysis reduction, pixels with a clipped channel and
/// pixels too dark to have a reliable colour are dropped. The first
/// estimate is the white balance that renders the mean of the rest
/// neutral. It is then refined twice: each time only the pixels whose
/// chroma, rendered with the current estimate, is under 30% are averaged,
/// so that strongly coloured surfaces stop pulling the estimate while a
/// strong cast cannot hide the neutral ones (they are judged after the
/// cast has been mostly removed). With fewer than 1% of the pixels under
/// the chroma limit a refinement keeps the previous estimate; with no
/// usable pixel at all the result is as-shot. Computed once per source
/// (R3). Like every grey-world method it is fooled by a scene that really
/// is dominated by one colour.
///
/// The result has [`WbPreset::Auto`] and the estimate in `temp` and
/// `tint`, in the units of the source kind.
pub fn auto_white_balance(source: &Source) -> WhiteBalance {
    source.analysis().auto_white_balance()
}

/// The estimate behind [`auto_white_balance`], from a reduction.
pub(crate) fn grey_world(reduction: &SourceImage) -> WhiteBalance {
    let color = reduction.color;
    let mut estimate = WhiteBalance::default();
    for pass in 0..3 {
        let settings = DevelopSettings {
            white_balance: estimate,
            ..Default::default()
        };
        let pipeline = PixelPipeline::new(&settings, &color);
        let limit = (pass > 0).then_some(GREY_WORLD_SATURATION);
        let neutral = mean_of(reduction, &pipeline, limit);
        let Some(picked) = neutral.and_then(|mean| photo_color::pick_white_balance(&color, mean))
        else {
            return estimate;
        };
        estimate = WhiteBalance {
            preset: WbPreset::Auto,
            ..picked
        };
    }
    estimate
}

/// Mean source-native colour of the usable pixels, or `None` when too few
/// are usable. `saturation` is the chroma limit, if any.
fn mean_of(
    reduction: &SourceImage,
    pipeline: &PixelPipeline,
    saturation: Option<f32>,
) -> Option<[f32; 3]> {
    let count = reduction.width as usize * reduction.height as usize;
    let mut sum = [0.0f64; 3];
    let mut used = 0usize;
    for index in 0..count {
        let source = unit_pixel(reduction, index);
        if source.iter().any(|&channel| channel >= CLIPPED) {
            continue;
        }
        let balanced = pipeline.input(source, None);
        if luminance(balanced) < GREY_WORLD_DARKEST {
            continue;
        }
        let high = balanced[0].max(balanced[1]).max(balanced[2]);
        let low = balanced[0].min(balanced[1]).min(balanced[2]);
        if saturation.is_some_and(|limit| (high - low) > limit * high) {
            continue;
        }
        for (total, value) in sum.iter_mut().zip(source) {
            *total += f64::from(value);
        }
        used += 1;
    }
    let enough = if saturation.is_some() {
        (used as f32) >= GREY_WORLD_MIN_FRACTION * count as f32 && used > 0
    } else {
        used > 0
    };
    enough.then(|| sum.map(|total| (total / used as f64) as f32))
}
