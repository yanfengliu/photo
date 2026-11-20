//! Randomised checks of the invariants the rest of the pipeline relies on.
//! The other test files pin each contract at hand-picked settings; these
//! sweep a few thousand random combinations of settings, sources, pixels and
//! local deltas (from a fixed seed, so a failure reproduces).

use photo_color::{LocalDelta, PixelPipeline};
use photo_core::colorimetry::{RgbSpace, luminance};
use photo_core::develop::{
    CurvePoint, DevelopSettings, GradeWheel, PointCurve, Profile, Tone, Treatment, WbPreset,
    WhiteBalance,
};
use photo_core::image::{CameraColor, SourceColor};

/// xorshift64*: small, deterministic, good enough to scatter test inputs.
struct Rng(u64);

impl Rng {
    fn unit(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let bits = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40;
        bits as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    fn chance(&mut self, probability: f32) -> bool {
        self.unit() < probability
    }

    /// A slider value: often zero, often at an extreme, otherwise anywhere.
    fn slider(&mut self, limit: f32) -> f32 {
        match self.unit() {
            u if u < 0.2 => 0.0,
            u if u < 0.3 => limit,
            u if u < 0.4 => -limit,
            _ => self.range(-limit, limit),
        }
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[((self.unit() * items.len() as f32) as usize).min(items.len() - 1)]
    }
}

fn camera() -> SourceColor {
    SourceColor::Camera(CameraColor {
        xyz_to_camera: [
            [0.6640, -0.1847, -0.0503],
            [-0.5238, 1.3010, 0.2474],
            [-0.0993, 0.1673, 0.6527],
        ],
        as_shot_gains: [2.31, 1.0, 1.58],
        baseline_exposure: 0.3,
    })
}

fn random_source(rng: &mut Rng) -> SourceColor {
    if rng.chance(0.4) {
        camera()
    } else {
        SourceColor::Rgb(rng.pick(&RgbSpace::ALL))
    }
}

fn random_tone(rng: &mut Rng) -> Tone {
    Tone {
        exposure: rng.slider(5.0),
        contrast: rng.slider(100.0),
        highlights: rng.slider(100.0),
        shadows: rng.slider(100.0),
        whites: rng.slider(100.0),
        blacks: rng.slider(100.0),
    }
}

/// A point curve whose points rise, so the curve does too.
fn rising_curve(rng: &mut Rng) -> PointCurve {
    let count = 2 + (rng.unit() * 4.0) as usize;
    let mut xs: Vec<f32> = (0..count).map(|_| rng.unit()).collect();
    let mut ys: Vec<f32> = (0..count).map(|_| rng.unit()).collect();
    xs.sort_by(f32::total_cmp);
    ys.sort_by(f32::total_cmp);
    PointCurve::new(xs.into_iter().zip(ys).map(|(x, y)| CurvePoint::new(x, y)))
}

fn any_curve(rng: &mut Rng) -> PointCurve {
    let count = 2 + (rng.unit() * 4.0) as usize;
    PointCurve::new((0..count).map(|_| CurvePoint::new(rng.unit(), rng.unit())))
}

fn randomise_tone_panel(rng: &mut Rng, settings: &mut DevelopSettings) {
    settings.tone = random_tone(rng);
    let parametric = &mut settings.tone_curve.parametric;
    parametric.shadows = rng.slider(100.0);
    parametric.darks = rng.slider(100.0);
    parametric.lights = rng.slider(100.0);
    parametric.highlights = rng.slider(100.0);
    if rng.chance(0.5) {
        parametric.split_shadows = rng.range(0.0, 100.0);
        parametric.split_midtones = rng.range(0.0, 100.0);
        parametric.split_highlights = rng.range(0.0, 100.0);
    }
}

fn randomise_colour(rng: &mut Rng, settings: &mut DevelopSettings) {
    settings.presence.vibrance = rng.slider(100.0);
    settings.presence.saturation = rng.slider(100.0);
    if rng.chance(0.6) {
        for band in 0..8 {
            settings.color_mixer.hue[band] = rng.slider(100.0);
            settings.color_mixer.saturation[band] = rng.slider(100.0);
            settings.color_mixer.luminance[band] = rng.slider(100.0);
            settings.color_mixer.bw_mix[band] = rng.slider(100.0);
        }
    }
    let calibration = &mut settings.calibration;
    calibration.red_hue = rng.slider(100.0);
    calibration.red_saturation = rng.slider(100.0);
    calibration.green_hue = rng.slider(100.0);
    calibration.green_saturation = rng.slider(100.0);
    calibration.blue_hue = rng.slider(100.0);
    calibration.blue_saturation = rng.slider(100.0);
}

/// Everything this crate reads, at random.
fn random_settings(rng: &mut Rng, source: &SourceColor) -> DevelopSettings {
    let mut settings = DevelopSettings {
        profile: rng.pick(&Profile::ALL),
        ..Default::default()
    };
    if rng.chance(0.2) {
        settings.treatment = Treatment::BlackAndWhite;
    }
    if rng.chance(0.7) {
        settings.white_balance = if source.is_camera() {
            WhiteBalance {
                preset: WbPreset::Custom,
                temp: 2000.0 * 25.0f32.powf(rng.unit()),
                tint: rng.slider(150.0),
            }
        } else {
            WhiteBalance {
                preset: WbPreset::Custom,
                temp: rng.slider(100.0),
                tint: rng.slider(100.0),
            }
        };
    }
    randomise_tone_panel(rng, &mut settings);
    if rng.chance(0.5) {
        settings.tone_curve.rgb = any_curve(rng);
    }
    if rng.chance(0.3) {
        settings.tone_curve.red = any_curve(rng);
        settings.tone_curve.blue = rising_curve(rng);
    }
    randomise_colour(rng, &mut settings);
    settings.calibration.shadows_tint = rng.slider(100.0);
    let grading = &mut settings.color_grading;
    for wheel in [
        &mut grading.shadows,
        &mut grading.midtones,
        &mut grading.highlights,
        &mut grading.global,
    ] {
        if rng.chance(0.5) {
            *wheel = GradeWheel {
                hue: rng.range(0.0, 360.0),
                saturation: rng.range(0.0, 100.0),
                luminance: rng.slider(100.0),
            };
        }
    }
    grading.blending = rng.range(0.0, 100.0);
    grading.balance = rng.slider(100.0);
    settings
}

fn random_delta(rng: &mut Rng, scale: f32) -> LocalDelta {
    LocalDelta {
        exposure: rng.slider(4.0) * scale,
        contrast: rng.slider(100.0) * scale,
        highlights: rng.slider(100.0) * scale,
        shadows: rng.slider(100.0) * scale,
        whites: rng.slider(100.0) * scale,
        blacks: rng.slider(100.0) * scale,
        temp: rng.slider(100.0) * scale,
        tint: rng.slider(100.0) * scale,
        saturation: rng.slider(100.0) * scale,
    }
}

fn random_pixel(rng: &mut Rng) -> [f32; 3] {
    match rng.unit() {
        u if u < 0.05 => [0.0; 3],
        u if u < 0.15 => [rng.unit(); 3],
        u if u < 0.25 => [
            rng.range(0.0, 16.0),
            rng.range(0.0, 16.0),
            rng.range(0.0, 16.0),
        ],
        u if u < 0.30 => [rng.range(-0.01, 0.01), rng.unit(), rng.range(-0.01, 1.0)],
        u if u < 0.40 => [rng.unit() * 1e-4, rng.unit() * 1e-4, rng.unit() * 1e-4],
        _ => [rng.unit(), rng.unit(), rng.unit()],
    }
}

fn develop(pipeline: &PixelPipeline, source: [f32; 3], delta: Option<&LocalDelta>) -> [f32; 3] {
    let balanced = pipeline.input(source, delta);
    let base = pipeline.tone_base(balanced, delta);
    let display = pipeline.scene_to_display(balanced, base, delta);
    pipeline.color(pipeline.tone(display, delta), delta)
}

#[test]
fn every_stage_output_is_finite_and_non_negative_and_output_stays_in_range() {
    let mut rng = Rng(0x5EED_0001);
    for _ in 0..600 {
        let source = random_source(&mut rng);
        let settings = random_settings(&mut rng, &source);
        let pipeline =
            PixelPipeline::new(&settings, &source).with_output_space(rng.pick(&RgbSpace::ALL));
        for _ in 0..40 {
            let pixel = random_pixel(&mut rng);
            let delta = random_delta(&mut rng, 1.5);
            let local = rng.chance(0.5).then_some(&delta);
            let balanced = pipeline.input(pixel, local);
            let own = pipeline.tone_base(balanced, local);
            assert!(own.is_finite() && own >= 0.0, "base {own} for {pixel:?}");
            // The base comes from a filter, so it need not match the pixel.
            let base = if rng.chance(0.5) {
                own
            } else {
                rng.range(-0.2, 1.5)
            };
            let display = pipeline.scene_to_display(balanced, base, local);
            let toned = pipeline.tone(display, local);
            let coloured = pipeline.color(toned, local);
            for (stage, value) in [
                ("input", balanced),
                ("scene_to_display", display),
                ("tone", toned),
                ("color", coloured),
            ] {
                assert!(
                    value.iter().all(|v| v.is_finite() && *v >= 0.0),
                    "{stage} gave {value:?} for {pixel:?} from {source:?}\n{settings:?}\n{local:?}"
                );
            }
            let encoded = pipeline.output_f32(coloured);
            assert!(
                encoded.iter().all(|v| (0.0..=1.0).contains(v)),
                "{encoded:?}"
            );
            let words = pipeline.output_u16(coloured);
            let bytes = pipeline.output_u8(coloured);
            for channel in 0..3 {
                let exact = f32::from(words[channel]) / 257.0;
                assert!(
                    (f32::from(bytes[channel]) - exact).abs() <= 0.58,
                    "8-bit {bytes:?} and 16-bit {words:?} disagree"
                );
            }
        }
    }
}

#[test]
fn the_tone_response_is_strictly_increasing_for_any_slider_values() {
    let mut rng = Rng(0x5EED_0002);
    for _ in 0..400 {
        let mut settings = DevelopSettings::default();
        randomise_tone_panel(&mut rng, &mut settings);
        if rng.chance(0.5) {
            settings.tone.exposure = 0.0;
        }
        let pipeline = PixelPipeline::new(&settings, &SourceColor::Rgb(RgbSpace::Rec2020));
        let mut previous = -1.0f32;
        for step in 0..=256 {
            let out = luminance(develop(&pipeline, [step as f32 / 256.0; 3], None));
            assert!(
                out > previous,
                "not increasing at level {step} ({previous} -> {out})\n{:?}\n{:?}",
                settings.tone,
                settings.tone_curve.parametric
            );
            previous = out;
        }
    }
}

#[test]
fn tone_keeps_neutrals_neutral_and_the_order_of_the_channels() {
    let mut rng = Rng(0x5EED_0003);
    for _ in 0..300 {
        let mut settings = DevelopSettings::default();
        randomise_tone_panel(&mut rng, &mut settings);
        if rng.chance(0.6) {
            settings.tone_curve.rgb = rising_curve(&mut rng);
        }
        let source = random_source(&mut rng);
        settings.profile = rng.pick(&Profile::ALL);
        let pipeline = PixelPipeline::new(&settings, &source);
        for _ in 0..40 {
            let level = rng.unit() * 1.5;
            let delta = random_delta(&mut rng, 1.0);
            let local = rng.chance(0.3).then_some(&delta);
            // Stages 5 and 6 on a neutral pixel: exactly neutral out.
            let display = pipeline.scene_to_display([level; 3], rng.unit(), local);
            let grey = pipeline.tone(display, local);
            assert!(
                grey[0] == grey[1] && grey[1] == grey[2],
                "{level} -> {grey:?}\n{settings:?}"
            );

            let pixel = [rng.unit() * 1.2, rng.unit() * 1.2, rng.unit() * 1.2];
            let base = pipeline.tone_base(pixel, local);
            let toned = pipeline.tone(pipeline.scene_to_display(pixel, base, local), local);
            for i in 0..3 {
                for j in 0..3 {
                    if pixel[i] >= pixel[j] {
                        assert!(
                            toned[i] >= toned[j] - 2e-6 * toned[i].max(1.0),
                            "{pixel:?} -> {toned:?}\n{settings:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn colour_keeps_neutrals_neutral_when_nothing_is_meant_to_tint_them() {
    let mut rng = Rng(0x5EED_0004);
    for _ in 0..400 {
        let mut settings = DevelopSettings::default();
        randomise_colour(&mut rng, &mut settings);
        if rng.chance(0.3) {
            settings.treatment = Treatment::BlackAndWhite;
        }
        let pipeline = PixelPipeline::new(&settings, &SourceColor::Rgb(RgbSpace::Rec2020));
        for _ in 0..20 {
            let level = rng.unit().powi(3) * 2.0;
            let delta = LocalDelta {
                saturation: rng.slider(100.0),
                ..Default::default()
            };
            let out = pipeline.color([level; 3], rng.chance(0.4).then_some(&delta));
            for channel in out {
                assert!(
                    (channel - level).abs() <= 3e-5 * level.max(0.02),
                    "grey {level} became {out:?}\n{settings:?}"
                );
            }
            // Black and white is neutral for any input, to the bit.
            if settings.treatment == Treatment::BlackAndWhite {
                let grey = pipeline.color(random_pixel(&mut rng), None);
                assert!(grey[0] == grey[1] && grey[1] == grey[2], "{grey:?}");
            }
        }
    }
}

#[test]
fn a_local_delta_agrees_with_the_same_change_made_globally() {
    let mut rng = Rng(0x5EED_0005);
    for _ in 0..400 {
        // Half-range global values and half-range deltas, so that their sum
        // is a legal global value to compare against.
        let mut base = DevelopSettings::default();
        randomise_tone_panel(&mut rng, &mut base);
        base.tone = Tone {
            exposure: base.tone.exposure * 0.4,
            contrast: base.tone.contrast * 0.5,
            highlights: base.tone.highlights * 0.5,
            shadows: base.tone.shadows * 0.5,
            whites: base.tone.whites * 0.5,
            blacks: base.tone.blacks * 0.5,
        };
        if rng.chance(0.5) {
            base.tone_curve.rgb = rising_curve(&mut rng);
        }
        base.presence.saturation = rng.slider(50.0);
        base.white_balance = WhiteBalance {
            preset: WbPreset::Custom,
            temp: rng.slider(50.0),
            tint: rng.slider(50.0),
        };
        let delta = random_delta(&mut rng, 0.5);
        let mut summed = base.clone();
        summed.tone.exposure += delta.exposure;
        summed.tone.contrast += delta.contrast;
        summed.tone.highlights += delta.highlights;
        summed.tone.shadows += delta.shadows;
        summed.tone.whites += delta.whites;
        summed.tone.blacks += delta.blacks;
        summed.white_balance.temp += delta.temp;
        summed.white_balance.tint += delta.tint;
        summed.presence.saturation += delta.saturation;

        let source = SourceColor::Rgb(rng.pick(&[RgbSpace::Srgb, RgbSpace::Rec2020]));
        let local = PixelPipeline::new(&base, &source);
        let global = PixelPipeline::new(&summed, &source);
        let zero = LocalDelta::default();
        for _ in 0..30 {
            let pixel = [rng.unit(), rng.unit(), rng.unit()];
            let (a, b) = (
                develop(&local, pixel, Some(&delta)),
                develop(&global, pixel, None),
            );
            // Compared as perceptual values. The masked path reads its own
            // tables, which agree with the global ones to about 1e-4 except
            // near a corner of a point curve (a curve is flat beyond its end
            // points), where each 1024-cell table can be off by about 1e-3:
            // the bound is two of those, still under an 8-bit code.
            for channel in 0..3 {
                let (x, y) = (a[channel].powf(1.0 / 2.2), b[channel].powf(1.0 / 2.2));
                assert!(
                    (x - y).abs() <= 2.5e-3,
                    "{pixel:?}: local {a:?} vs global {b:?}\n{base:?}\n{delta:?}"
                );
            }
            // A zero delta is the unmasked path, to the bit.
            assert_eq!(
                develop(&local, pixel, Some(&zero)),
                develop(&local, pixel, None)
            );
        }
    }
}

#[test]
fn settings_that_are_not_numbers_neither_panic_nor_poison_pixels() {
    // Settings are sanitised before they are stored, but a renderer must not
    // depend on that.
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0e30, -1.0e30] {
        let mut settings = DevelopSettings::default();
        for adjustment in photo_core::develop::Adj::all() {
            adjustment.set(&mut settings, bad);
        }
        settings.white_balance.preset = WbPreset::Custom;
        let bad_camera = SourceColor::Camera(CameraColor {
            xyz_to_camera: [[bad, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            as_shot_gains: [bad, 1.0, -1.0],
            baseline_exposure: bad,
        });
        let delta = LocalDelta {
            exposure: bad,
            contrast: bad,
            highlights: bad,
            shadows: bad,
            whites: bad,
            blacks: bad,
            temp: bad,
            tint: bad,
            saturation: bad,
        };
        for source in [SourceColor::Rgb(RgbSpace::Srgb), camera(), bad_camera] {
            for treatment in [Treatment::Color, Treatment::BlackAndWhite] {
                settings.treatment = treatment;
                let pipeline = PixelPipeline::new(&settings, &source);
                for pixel in [[0.0; 3], [0.2, 0.5, 0.8], [3.0, 0.0, 1.0e-6], [1.0; 3]] {
                    for local in [None, Some(&delta)] {
                        let balanced = pipeline.input(pixel, local);
                        let base = pipeline.tone_base(balanced, local);
                        assert!(base.is_finite() && base >= 0.0, "{bad}: base {base}");
                        let display = pipeline.scene_to_display(balanced, bad, local);
                        let toned = pipeline.tone(display, local);
                        let coloured = pipeline.color(toned, local);
                        for value in [balanced, display, toned, coloured] {
                            assert!(
                                value.iter().all(|v| v.is_finite() && *v >= 0.0),
                                "{bad}: {value:?} for {pixel:?} from {source:?}"
                            );
                        }
                        let bytes = pipeline.output_u8(coloured);
                        let _ = (bytes, pipeline.output_u16(coloured));
                    }
                }
            }
        }
    }
}
