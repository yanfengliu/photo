//! Stages 1 to 4: heal, resample, input and denoise. Their result is cache
//! A (DESIGN §5.2), which moving a tone or colour slider reuses.

use std::time::{Duration, Instant};

use photo_imaging::{RgbImage, SourcePyramid, resample};
use rayon::prelude::*;

use crate::cancel::CancelToken;
use crate::denoise::apply_denoise;
use crate::masks::Layers;
use crate::plan::Plan;
use crate::source::Source;

/// Working-space scene-linear RGB of a view after stage 4, surrounded by
/// `padding` pixels.
pub(crate) struct Front {
    pub image: RgbImage,
    pub padding: u32,
}

/// Run stages 2 to 4 on `pyramid`, whose level for this view has already
/// been healed (stage 1) when there are spots, recording how long each
/// took in `times`.
pub(crate) fn develop_front(
    source: &Source,
    pyramid: &SourcePyramid,
    plan: &Plan,
    layers: &Layers,
    cancel: &CancelToken,
    times: &mut Vec<(&'static str, Duration)>,
) -> Option<Front> {
    let padding = plan.front_padding;
    let start = Instant::now();
    let mut image = resample(pyramid, &plan.geometry, &plan.view, padding);
    times.push(("resample", start.elapsed()));
    cancel.proceed()?;
    let start = Instant::now();
    input(&mut image, source, plan, layers, cancel)?;
    times.push(("input", start.elapsed()));
    match &plan.denoise {
        Some(denoise) => {
            let start = Instant::now();
            let image = apply_denoise(&image, denoise, layers, padding, cancel)?;
            times.push(("denoise", start.elapsed()));
            Some(Front {
                image,
                padding: padding - denoise.support,
            })
        }
        None => Some(Front { image, padding }),
    }
}

/// Stage 3 in place: the colour crate's input transform (white balance,
/// highlight reconstruction, the matrix to working space) with each
/// pixel's mask temperature and tint, then the lens vignetting gain at the
/// pixel's source position. Skipped when it would change nothing.
fn input(
    image: &mut RgbImage,
    source: &Source,
    plan: &Plan,
    layers: &Layers,
    cancel: &CancelToken,
) -> Option<()> {
    let pipeline = &plan.pipeline;
    if pipeline.input_is_identity() && plan.lens.is_none() && !plan.masks.input {
        return Some(());
    }
    let width = image.width();
    let padding = plan.front_padding;
    let (source_width, source_height) = source.size();
    let (inverse_width, inverse_height) = (1.0 / source_width as f32, 1.0 / source_height as f32);
    image
        .pixels_mut()
        .par_chunks_mut(width.max(1))
        .enumerate()
        .for_each_init(
            || vec![(0.0f32, 0.0f32); width],
            |positions, (y, row)| {
                if cancel.is_cancelled() {
                    return;
                }
                if plan.lens.is_some() {
                    plan.locator.row(y, padding, positions);
                }
                for (x, pixel) in row.iter_mut().enumerate() {
                    let delta = if plan.masks.input {
                        layers.delta(layers.index(x, y, padding))
                    } else {
                        None
                    };
                    let rgb = pipeline.input(*pixel, delta.as_ref());
                    *pixel = match &plan.lens {
                        Some(lens) => {
                            let (sx, sy) = positions[x];
                            lens.apply(rgb, sx * inverse_width, sy * inverse_height)
                        }
                        None => rgb,
                    };
                }
            },
        );
    cancel.proceed()
}
