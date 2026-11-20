//! Auto tone, and the tone curve the curve editor plots.

use photo_color::PixelPipeline;
use photo_color::base::SceneToDisplay;
use photo_color::tone::{ToneCurves, encode_perceptual};
use photo_core::colorimetry::luminance;
use photo_core::develop::{DevelopSettings, Tone};
use photo_core::meta::FileKind;

use crate::lens::LensVignetting;
use crate::source::{Source, unit_pixel};

// ---- Tunable constants ----------------------------------------------------

/// Where auto tone puts the median: display-linear middle grey.
const MIDDLE_GREY: f32 = 0.18;
/// The percentiles Whites and Blacks are set from, and the perceptual
/// values they are moved to.
const HIGH_PERCENTILE: f32 = 0.995;
const LOW_PERCENTILE: f32 = 0.005;
const WHITE_TARGET: f32 = 0.97;
const BLACK_TARGET: f32 = 0.03;
/// The tails Highlights and Shadows answer: the share of pixels brighter
/// (darker) than these perceptual values, times the gain, up to the limit.
const BRIGHT_TAIL: f32 = 0.75;
const DARK_TAIL: f32 = 0.25;
const TAIL_GAIN: f32 = 200.0;
const TAIL_LIMIT: f32 = 60.0;
/// The exposure search range, in stops (the slider's).
const EXPOSURE_RANGE: (f32, f32) = (-5.0, 5.0);

// ---------------------------------------------------------------------------

/// Auto tone, from the source's analysis reduction rendered with the
/// current white balance, lens vignetting and profile:
///
/// 1. **Exposure** puts the median luminance at middle grey (display-linear
///    0.18) through the profile's base curve, found by bisection over the
///    slider's range.
/// 2. **Highlights** and **Shadows** answer the tails at that exposure: the
///    share of pixels brighter than perceptual 0.75 (darker than 0.25),
///    times 200, up to 60, pulls the highlights down (lifts the shadows).
/// 3. **Whites** and **Blacks** move the 99.5th and 0.5th percentiles to
///    perceptual 0.97 and 0.03 through the composite tone curve (with the
///    Highlights and Shadows just chosen and the current Contrast and tone
///    curve), each found by bisection, twice in turn since they interact
///    slightly.
///
/// Contrast is left as it is. Values are rounded to the sliders' steps.
/// Masks are not considered: auto tone is a global adjustment.
pub fn auto_tone(source: &Source, settings: &DevelopSettings) -> Tone {
    let reduction = source.analysis().reduction();
    let color = reduction.color;
    let input = DevelopSettings {
        white_balance: settings.white_balance,
        ..Default::default()
    };
    let pipeline = PixelPipeline::new(&input, &color);
    let lens = LensVignetting::new(&settings.lens, source.size());
    let (width, height) = (reduction.width as usize, reduction.height as usize);
    let scene: Vec<[f32; 3]> = (0..width * height)
        .map(|index| {
            let rgb = pipeline.input(unit_pixel(reduction, index), None);
            match &lens {
                Some(lens) => {
                    let u = ((index % width) as f32 + 0.5) / width as f32;
                    let v = ((index / width) as f32 + 0.5) / height as f32;
                    lens.apply(rgb, u, v)
                }
                None => rgb,
            }
        })
        .collect();
    let stage = SceneToDisplay::new(settings.profile, &Tone::default(), &color);

    let mut luminances: Vec<f32> = scene.iter().map(|rgb| luminance(*rgb).max(0.0)).collect();
    luminances.sort_by(f32::total_cmp);
    let median = percentile(&luminances, 0.5);
    let grey = encode_perceptual(MIDDLE_GREY);
    let exposure = bisect(EXPOSURE_RANGE, grey, |ev| stage.base_value([median; 3], ev));

    let mut values: Vec<f32> = scene
        .iter()
        .map(|rgb| stage.base_value(*rgb, exposure))
        .collect();
    values.sort_by(f32::total_cmp);
    let share = |inside: &dyn Fn(f32) -> bool| {
        values.iter().filter(|&&v| inside(v)).count() as f32 / values.len().max(1) as f32
    };
    let tail = |fraction: f32| (fraction * TAIL_GAIN).min(TAIL_LIMIT).round();
    let highlights = -tail(share(&|v| v > BRIGHT_TAIL));
    let shadows = tail(share(&|v| v < DARK_TAIL));
    let (high, low) = (
        percentile(&values, HIGH_PERCENTILE),
        percentile(&values, LOW_PERCENTILE),
    );

    let kind = source.kind();
    let composite = |whites: f32, blacks: f32, v: f32| {
        let tone = Tone {
            exposure: 0.0,
            contrast: settings.tone.contrast,
            highlights,
            shadows,
            whites,
            blacks,
        };
        ToneCurves::new(&tone, &settings.tone_curve, kind).eval(v)
    };
    let (mut whites, mut blacks) = (0.0, 0.0);
    for _ in 0..2 {
        whites = bisect((-100.0, 100.0), WHITE_TARGET, |w| {
            composite(w, blacks, high)
        })
        .round();
        blacks = bisect((-100.0, 100.0), BLACK_TARGET, |b| composite(whites, b, low)).round();
    }
    Tone {
        exposure: (exposure * 100.0).round() / 100.0,
        contrast: settings.tone.contrast,
        highlights,
        shadows,
        whites,
        blacks,
    }
}

/// The value at quantile `q` of sorted `values` (0 when empty).
fn percentile(values: &[f32], q: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let index = (q * (values.len() - 1) as f32).round() as usize;
    values[index.min(values.len() - 1)]
}

/// The `x` in `range` at which increasing `f` reaches `target`, by
/// bisection; an end of the range when `target` lies beyond it.
fn bisect(range: (f32, f32), target: f32, f: impl Fn(f32) -> f32) -> f32 {
    let (mut low, mut high) = range;
    if f(low) >= target {
        return low;
    }
    if f(high) <= target {
        return high;
    }
    for _ in 0..40 {
        let middle = 0.5 * (low + high);
        if f(middle) < target {
            low = middle;
        } else {
            high = middle;
        }
    }
    0.5 * (low + high)
}

/// The curve the Tone Curve panel plots, as a function of perceptual value
/// (0 to 1 in, 0 to 1 out for in-range input): Contrast, then the
/// parametric curve, then the RGB point curve, composed exactly as the
/// renderer composes them. Highlights, Shadows, Whites and Blacks are not
/// part of it, nor are the per-channel curves (see
/// `photo_color::tone::ToneCurves::channel`).
pub fn tone_curve_sampler(settings: &DevelopSettings) -> impl Fn(f32) -> f32 + Send + Sync + use<> {
    let tone = Tone {
        contrast: settings.tone.contrast,
        ..Default::default()
    };
    let curves = ToneCurves::new(&tone, &settings.tone_curve, FileKind::Raster);
    move |v| curves.eval(v)
}
