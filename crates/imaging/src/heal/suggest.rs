//! Choosing where a healing spot should take its patch from.
//!
//! A good source looks, around its rim, like the surroundings of the target:
//! the same tone, the same large-scale structure crossing it (so an edge
//! continues through the repair), and the same amount of fine texture. It
//! also has nothing of its own in the middle. Each candidate position is
//! scored on those four counts from a few dozen bilinear samples, at a
//! pyramid level where the spot is about eight pixels in radius, so the cost
//! does not depend on the image or spot size.

use std::f32::consts::TAU;

use photo_core::geom::Point;

use crate::pyramid::SourcePyramid;

/// Candidate centres lie in this many directions around the target...
const DIRECTIONS: usize = 16;
/// ...at these distances, in spot radii. Nearer ones win ties.
const DISTANCES: [f32; 2] = [2.5, 3.5];
/// The rim is sampled this far from a centre, in spot radii: clear of the
/// spot itself, where the blemish is.
const RIM_RADIUS: f32 = 1.25;
const RIM_SAMPLES: usize = 48;
/// The rim profile is smoothed over this many samples either side to
/// separate structure from texture.
const SMOOTHING: usize = 3;
/// The middle of a candidate is sampled on these radii, in spot radii.
const CORE_RADII: [f32; 2] = [0.4, 0.8];
const CORE_SAMPLES: usize = 16;
/// Spot radius, in pixels, of the pyramid level the samples are taken from.
const WORKING_RADIUS: f32 = 8.0;
/// How much a mismatch in fine texture counts against one in structure.
const TEXTURE_WEIGHT: f32 = 4.0;

/// What a position looks like to the scoring: tone, structure and texture
/// around its rim, and the brightness of its middle against its rim.
struct Look {
    tone: [f32; 3],
    structure: [f32; RIM_SAMPLES],
    texture: [f32; RIM_SAMPLES],
    /// Mean brightness of the core minus that of the rim.
    core_offset: f32,
}

/// Circular moving average over `2 * SMOOTHING + 1` samples.
fn smoothed(values: &[f32; RIM_SAMPLES]) -> [f32; RIM_SAMPLES] {
    std::array::from_fn(|index| {
        let window = (0..=2 * SMOOTHING)
            .map(|k| values[(index + RIM_SAMPLES + k - SMOOTHING) % RIM_SAMPLES]);
        window.sum::<f32>() / (2 * SMOOTHING + 1) as f32
    })
}

/// Samples the pyramid in source-pixel coordinates.
struct Sampler<'a> {
    pyramid: &'a SourcePyramid,
    level: usize,
    to_level: f32,
}

impl Sampler<'_> {
    fn ring(
        &self,
        centre: (f32, f32),
        radius: f32,
        count: usize,
    ) -> impl Iterator<Item = [f32; 3]> + '_ {
        (0..count).map(move |index| {
            let (sin, cos) = (TAU * index as f32 / count as f32).sin_cos();
            let (x, y) = (centre.0 + radius * cos, centre.1 + radius * sin);
            self.pyramid
                .sample(self.level, x * self.to_level, y * self.to_level)
        })
    }

    fn look(&self, centre: (f32, f32), radius: f32) -> Look {
        let mut tone = [0.0f32; 3];
        let mut brightness = [0.0f32; RIM_SAMPLES];
        for (slot, pixel) in
            brightness
                .iter_mut()
                .zip(self.ring(centre, RIM_RADIUS * radius, RIM_SAMPLES))
        {
            (0..3).for_each(|channel| tone[channel] += pixel[channel] / RIM_SAMPLES as f32);
            *slot = (pixel[0] + pixel[1] + pixel[2]) / 3.0;
        }
        let rim_mean = brightness.iter().sum::<f32>() / RIM_SAMPLES as f32;
        let low = smoothed(&brightness);
        let detail: [f32; RIM_SAMPLES] = std::array::from_fn(|i| (brightness[i] - low[i]).abs());
        let core: f32 = CORE_RADII
            .iter()
            .flat_map(|&fraction| self.ring(centre, fraction * radius, CORE_SAMPLES))
            .map(|pixel| (pixel[0] + pixel[1] + pixel[2]) / 3.0)
            .sum::<f32>()
            / (CORE_RADII.len() * CORE_SAMPLES) as f32;
        Look {
            tone,
            structure: low.map(|value| value - rim_mean),
            texture: smoothed(&detail),
            core_offset: core - rim_mean,
        }
    }
}

/// How badly `candidate` would serve as the source for `target`; 0 is a
/// perfect match.
fn mismatch(target: &Look, candidate: &Look) -> f32 {
    let squared =
        |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>();
    squared(&target.tone, &candidate.tone)
        + squared(&target.structure, &candidate.structure) / RIM_SAMPLES as f32
        + TEXTURE_WEIGHT * squared(&target.texture, &candidate.texture) / RIM_SAMPLES as f32
        + candidate.core_offset * candidate.core_offset
}

/// A good place to take the patch from for a spot at `target` with the given
/// `radius`: the [`Spot::source`](photo_core::develop::Spot::source) to
/// start with when the user clicks on a blemish.
///
/// `target` is in normalized source coordinates and `radius` a fraction of
/// the long edge, as in a `Spot`; so is the result.
///
/// Thirty-two candidates are tried, 2.5 and 3.5 radii from the target in
/// sixteen directions. Those whose disc would leave the image are discarded,
/// and of the rest the one wins whose rim best matches the target's rim in
/// tone, in the structure crossing it and in fine texture, and whose middle
/// does not stand out from its own rim. Ties go to the nearer candidate.
/// The answer is a pure function of the arguments.
///
/// When no candidate fits (a spot larger than the image), the result is the
/// point 2.5 radii from the target toward the image centre, clamped into
/// the image. A non-finite `target` is treated as the image centre, and
/// with a non-positive or non-finite `radius` the target itself comes back.
pub fn suggest_heal_source(pyramid: &SourcePyramid, target: Point, radius: f32) -> Point {
    let (width, height) = pyramid.source_size();
    let (width, height) = (width as f32, height as f32);
    let finite_or_half = |value: f32| if value.is_finite() { value } else { 0.5 };
    let target = Point::new(finite_or_half(target.x), finite_or_half(target.y));
    if !radius.is_finite() || radius <= 0.0 {
        return target;
    }
    let centre = (target.x * width, target.y * height);
    let radius = radius * width.max(height);

    let level =
        ((radius / WORKING_RADIUS).log2().floor().max(0.0) as usize).min(pyramid.level_count() - 1);
    let sampler = Sampler {
        pyramid,
        level,
        to_level: 0.5f32.powi(level as i32),
    };
    let wanted = sampler.look(centre, radius);

    let mut best: Option<(f32, (f32, f32))> = None;
    for distance in DISTANCES {
        for direction in 0..DIRECTIONS {
            let (sin, cos) = (TAU * direction as f32 / DIRECTIONS as f32).sin_cos();
            let (x, y) = (
                centre.0 + distance * radius * cos,
                centre.1 + distance * radius * sin,
            );
            let fits = x >= radius && y >= radius && x <= width - radius && y <= height - radius;
            if !fits {
                continue;
            }
            let score = mismatch(&wanted, &sampler.look((x, y), radius));
            if best.is_none_or(|(lowest, _)| score < lowest) {
                best = Some((score, (x, y)));
            }
        }
    }
    let (x, y) = best.map_or_else(
        || {
            let (dx, dy) = (width / 2.0 - centre.0, height / 2.0 - centre.1);
            let length = dx.hypot(dy);
            let (dx, dy) = if length > 1e-3 {
                (dx / length, dy / length)
            } else {
                (1.0, 0.0)
            };
            let reach = DISTANCES[0] * radius;
            (
                (centre.0 + reach * dx).clamp(0.0, width),
                (centre.1 + reach * dy).clamp(0.0, height),
            )
        },
        |(_, position)| position,
    );
    Point::new(x / width, y / height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_wraps_around_and_keeps_the_mean() {
        let mut values = [0.0f32; RIM_SAMPLES];
        values[0] = 7.0;
        let low = smoothed(&values);
        assert_eq!(low[0], 1.0);
        assert_eq!(low[SMOOTHING], 1.0);
        assert_eq!(low[RIM_SAMPLES - SMOOTHING], 1.0);
        assert_eq!(low[SMOOTHING + 1], 0.0);
        assert!((low.iter().sum::<f32>() - 7.0).abs() < 1e-5);
    }
}
