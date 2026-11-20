//! Stages 5 to 7 and the per-pixel half of stage 8, row by row.
//!
//! Each stage runs as its own pass over a row (the colour crate measured
//! that chaining every stage per pixel is about twice as slow), reading the
//! row's mask deltas and guide positions from per-thread scratch that is
//! filled once per row.

use photo_color::LocalDelta;
use photo_imaging::RgbImage;
use rayon::prelude::*;

use crate::cancel::CancelToken;
use crate::front::Front;
use crate::guides::{Guides, stops};
use crate::masks::{Extras, Layers};
use crate::plan::Plan;
use crate::presence::{clarity_pixel, dehaze_pixel, perceptual};

/// Per-thread buffers for one row.
struct Scratch {
    deltas: Vec<Option<LocalDelta>>,
    extras: Vec<Extras>,
    positions: Vec<(f32, f32)>,
}

impl Scratch {
    fn new(width: usize) -> Self {
        Self {
            deltas: vec![None; width],
            extras: vec![Extras::default(); width],
            positions: vec![(0.0, 0.0); width],
        }
    }
}

/// Run stages 5 to 8 (dehaze and clarity) on cache A, producing the
/// view's display-linear working RGB padded by `plan.downstream`.
pub(crate) fn develop_point(
    front: &Front,
    plan: &Plan,
    layers: &Layers,
    guides: &Guides,
    cancel: &CancelToken,
) -> Option<RgbImage> {
    let padding = plan.downstream;
    let offset = (front.padding - padding) as usize;
    let width = plan.view.width() as usize + 2 * padding as usize;
    let height = plan.view.height() as usize + 2 * padding as usize;
    let pipeline = &plan.pipeline;
    let (scene, tone, color) = (plan.scene_active(), plan.tone_active(), plan.color_active());
    let needs = plan.needs;
    let masks = plan.masks;
    let want_deltas = !layers.is_empty() && (masks.scene || masks.tone || masks.color);
    let want_extras = !layers.is_empty() && (masks.dehaze || masks.clarity);
    let want_positions = needs.base || needs.dehaze || needs.clarity;
    let light = guides.light();

    let mut out = RgbImage::new(width, height);
    out.pixels_mut()
        .par_chunks_mut(width.max(1))
        .enumerate()
        .for_each_init(
            || Scratch::new(width),
            |scratch, (y, row)| {
                if cancel.is_cancelled() {
                    return;
                }
                row.copy_from_slice(&front.image.row(y + offset)[offset..offset + width]);
                if want_deltas || want_extras {
                    for x in 0..width {
                        let index = layers.index(x, y, padding);
                        if want_deltas {
                            scratch.deltas[x] = layers.delta(index);
                        }
                        if want_extras {
                            scratch.extras[x] = layers.extras(index);
                        }
                    }
                }
                if want_positions {
                    plan.locator.row(y, padding, &mut scratch.positions);
                    for position in scratch.positions.iter_mut() {
                        *position = guides.locate(position.0, position.1);
                    }
                }
                let deltas = &scratch.deltas;
                if scene {
                    for (x, pixel) in row.iter_mut().enumerate() {
                        let delta = deltas[x].as_ref();
                        let base = if needs.base {
                            guides.base(pipeline, scratch.positions[x], stops(*pixel, delta))
                        } else {
                            0.0
                        };
                        *pixel = pipeline.scene_to_display(*pixel, base, delta);
                    }
                }
                if tone {
                    for (x, pixel) in row.iter_mut().enumerate() {
                        *pixel = pipeline.tone(*pixel, deltas[x].as_ref());
                    }
                }
                if color {
                    for (x, pixel) in row.iter_mut().enumerate() {
                        *pixel = pipeline.color(*pixel, deltas[x].as_ref());
                    }
                }
                if needs.dehaze {
                    for (x, pixel) in row.iter_mut().enumerate() {
                        let haze = guides.haze(scratch.positions[x], perceptual(*pixel).1);
                        let amount = plan.dehaze + scratch.extras[x].dehaze;
                        *pixel = dehaze_pixel(*pixel, light, haze, amount);
                    }
                }
                if needs.clarity {
                    for (x, pixel) in row.iter_mut().enumerate() {
                        let mean = guides.clarity_mean(scratch.positions[x], perceptual(*pixel).1);
                        let amount = plan.clarity + scratch.extras[x].clarity;
                        *pixel = clarity_pixel(*pixel, mean, amount);
                    }
                }
            },
        );
    cancel.proceed()?;
    Some(out)
}
