//! The hue-selective part of the colour stage: the eight-band colour mixer
//! and the black-and-white mix, compiled into a table indexed by Oklab hue.

use std::f64::consts::TAU;
use std::sync::LazyLock;

use photo_core::colorimetry::{RgbSpace, WORKING_SPACE, mat3_apply, rgb_to_rgb};
use photo_core::develop::{ColorBand, ColorMixer};

use crate::math::{slider_unit as unit, smoothstep_d};
use crate::oklab::{HueLut, working_to_oklab};

// ---- Tunable constants ----------------------------------------------------

/// The sRGB-encoded reference colour each band is centred on.
const BAND_COLOURS: [[f32; 3]; ColorBand::COUNT] = [
    [1.0, 0.0, 0.0], // red
    [1.0, 0.5, 0.0], // orange
    [1.0, 1.0, 0.0], // yellow
    [0.0, 1.0, 0.0], // green
    [0.0, 1.0, 1.0], // aqua
    [0.0, 0.0, 1.0], // blue
    [0.5, 0.0, 1.0], // purple
    [1.0, 0.0, 1.0], // magenta
];
/// Hue ±100 turns a band's hue by this fraction of the gap to the
/// neighbouring band's centre. With smoothstep weights (maximum slope 1.5)
/// the hue map `h + shift(h)` keeps a slope of at least `1 - 3 * 0.3 = 0.1`
/// even when two neighbours are pulled toward each other, so hues are never
/// reordered.
const HUE_RANGE: f64 = 0.3;
/// Saturation ±100 scales the band's chroma by `1 ± SATURATION_RANGE`.
const SATURATION_RANGE: f64 = 1.0;
/// Luminance ±100 scales the lightness of a fully saturated colour of the
/// band by `1 ± LUMINANCE_RANGE`...
const LUMINANCE_RANGE: f64 = 0.3;
/// ...and the black-and-white mix by `1 ± BW_RANGE`.
const BW_RANGE: f64 = 0.4;
/// Lightness changes fade out toward grey as `C² / (C² + CHROMA_KNEE²)` (half
/// strength at an Oklab chroma of `CHROMA_KNEE`), so neutrals are untouched
/// and noise in near-neutrals is not amplified.
pub(super) const CHROMA_KNEE: f32 = 0.04;

// ---------------------------------------------------------------------------

/// Oklab hue of each band's reference colour, in radians, ascending.
static BAND_CENTRES: LazyLock<[f64; ColorBand::COUNT]> = LazyLock::new(|| {
    let to_working = rgb_to_rgb(RgbSpace::Srgb, WORKING_SPACE);
    BAND_COLOURS.map(|encoded| {
        let lab = working_to_oklab(mat3_apply(
            &to_working,
            encoded.map(|c| RgbSpace::Srgb.decode(c)),
        ));
        f64::from(lab[2]).atan2(f64::from(lab[1])).rem_euclid(TAU)
    })
});

/// The Oklab hue, in degrees, that a band is centred on: the hue of its
/// sRGB reference colour (red, orange, yellow, green, aqua, blue, purple,
/// magenta). The centres ascend in band order.
pub fn band_centre_degrees(band: ColorBand) -> f32 {
    BAND_CENTRES[band.index()].to_degrees() as f32
}

/// Angular distance from band `index` to the next band's centre, radians.
fn gap_after(index: usize) -> f64 {
    let centres = &*BAND_CENTRES;
    (centres[(index + 1) % ColorBand::COUNT] - centres[index]).rem_euclid(TAU)
}

fn weights_at(hue: f64) -> [f64; ColorBand::COUNT] {
    let centres = &*BAND_CENTRES;
    let hue = if hue.is_finite() {
        hue.rem_euclid(TAU)
    } else {
        0.0
    };
    let mut weights = [0.0; ColorBand::COUNT];
    // The band whose centre is at or before this hue, going around the circle.
    let index = (0..ColorBand::COUNT)
        .rev()
        .find(|index| centres[*index] <= hue)
        .unwrap_or(ColorBand::COUNT - 1);
    let next = (index + 1) % ColorBand::COUNT;
    let toward_next = smoothstep_d((hue - centres[index]).rem_euclid(TAU) / gap_after(index));
    weights[index] = 1.0 - toward_next;
    weights[next] = toward_next;
    weights
}

/// The weight of each colour-mixer band at an Oklab hue (degrees), indexed
/// by [`ColorBand::index`]. The weights are smooth in hue, sum to one, are
/// one at their own band's centre, and at most two are non-zero.
pub fn band_weights(hue_degrees: f32) -> [f32; ColorBand::COUNT] {
    weights_at(f64::from(hue_degrees).to_radians()).map(|w| w as f32)
}

fn weighted(weights: &[f64; ColorBand::COUNT], sliders: &[f32; ColorBand::COUNT]) -> f64 {
    weights
        .iter()
        .zip(sliders)
        .map(|(w, slider)| w * unit(*slider))
        .sum()
}

/// The hue table for the colour treatment. Each entry is
/// `[c, s, lightness]`: `(c, s)` is the rotation-and-scale applied to
/// `(a, b)` (the mixer's hue shift and saturation) and `lightness` the
/// relative change in `L` for a fully saturated colour (the mixer's
/// luminance).
pub(super) fn colour_table(mixer: &ColorMixer) -> HueLut<3> {
    // A positive hue value turns toward the next band, a negative one toward
    // the previous band; each by a fraction of that particular gap.
    let turns: [f64; ColorBand::COUNT] = std::array::from_fn(|index| {
        let amount = unit(mixer.hue[index]);
        let gap = if amount >= 0.0 {
            gap_after(index)
        } else {
            gap_after((index + ColorBand::COUNT - 1) % ColorBand::COUNT)
        };
        amount * HUE_RANGE * gap
    });
    HueLut::build(|hue| {
        let weights = weights_at(hue);
        let turn: f64 = weights.iter().zip(turns).map(|(w, turn)| w * turn).sum();
        let scale = (1.0 + SATURATION_RANGE * weighted(&weights, &mixer.saturation)).max(0.0);
        [
            (scale * turn.cos()) as f32,
            (scale * turn.sin()) as f32,
            (LUMINANCE_RANGE * weighted(&weights, &mixer.luminance)) as f32,
        ]
    })
}

/// The hue table for the black-and-white treatment, in the layout of
/// [`colour_table`]: only the lightness column (the black-and-white mix)
/// does anything.
pub(super) fn black_and_white_table(mixer: &ColorMixer) -> HueLut<3> {
    HueLut::build(|hue| {
        let weights = weights_at(hue);
        [
            1.0,
            0.0,
            (BW_RANGE * weighted(&weights, &mixer.bw_mix)) as f32,
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_centres_ascend_and_leave_room_between_neighbours() {
        let centres = &*BAND_CENTRES;
        for index in 0..ColorBand::COUNT - 1 {
            assert!(centres[index + 1] > centres[index]);
        }
        for index in 0..ColorBand::COUNT {
            let gap = gap_after(index).to_degrees();
            assert!(
                (15.0..120.0).contains(&gap),
                "gap after band {index} is {gap} degrees"
            );
        }
    }

    #[test]
    fn the_worst_case_hue_map_keeps_a_positive_slope() {
        let mixer = ColorMixer {
            hue: [100.0, -100.0, 100.0, -100.0, 100.0, -100.0, 100.0, -100.0],
            ..Default::default()
        };
        let turns: Vec<f64> = (0..ColorBand::COUNT)
            .map(|index| {
                let amount = unit(mixer.hue[index]);
                let gap = if amount >= 0.0 {
                    gap_after(index)
                } else {
                    gap_after((index + 7) % 8)
                };
                amount * HUE_RANGE * gap
            })
            .collect();
        let mapped = |hue: f64| {
            let weights = weights_at(hue);
            hue + weights
                .iter()
                .zip(&turns)
                .map(|(w, turn)| w * turn)
                .sum::<f64>()
        };
        let step = TAU / 7200.0;
        for i in 0..7200 {
            let slope = (mapped((i + 1) as f64 * step) - mapped(i as f64 * step)) / step;
            assert!(
                slope > 0.09,
                "slope {slope} at {} degrees",
                (i as f64 * step).to_degrees()
            );
        }
    }
}
