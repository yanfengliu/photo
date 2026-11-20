//! Timing of the per-pixel chain. Not a contract: run it by hand with
//! `cargo test -p photo-color --test timing -- --ignored --nocapture`.
//!
//! Two kinds of figure are printed. "ns/pixel" lines run every stage on one
//! pixel before moving to the next; a pixel's stages form one long chain of
//! dependent operations, so the processor cannot overlap neighbouring
//! pixels. The "by stage" tables run each stage over the whole buffer, which
//! is how a pipeline with spatial stages in between calls them, and is
//! roughly twice as fast in total.

use std::hint::black_box;
use std::time::Instant;

use photo_color::tone::tone_base_value;
use photo_color::{LocalDelta, PixelPipeline};
use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{CurvePoint, DevelopSettings, PointCurve, Tone, WbPreset, WhiteBalance};
use photo_core::image::{CameraColor, SourceColor};

const COUNT: usize = 1_000_000;
const SRGB: SourceColor = SourceColor::Rgb(RgbSpace::Srgb);

fn camera() -> SourceColor {
    SourceColor::Camera(CameraColor {
        xyz_to_camera: [
            [0.6640, -0.1847, -0.0503],
            [-0.5238, 1.3010, 0.2474],
            [-0.0993, 0.1673, 0.6527],
        ],
        as_shot_gains: [2.31, 1.0, 1.58],
        baseline_exposure: 0.0,
    })
}

/// A plausible edit: exposure, the four tone zones, contrast, a little
/// vibrance and saturation.
fn typical() -> DevelopSettings {
    let mut settings = DevelopSettings {
        tone: Tone {
            exposure: 0.35,
            contrast: 20.0,
            highlights: -40.0,
            shadows: 30.0,
            whites: 10.0,
            blacks: -5.0,
        },
        ..Default::default()
    };
    settings.presence.vibrance = 20.0;
    settings.presence.saturation = 5.0;
    settings
}

/// Every operator of every stage switched on.
fn everything() -> DevelopSettings {
    let mut settings = typical();
    settings.white_balance = WhiteBalance {
        preset: WbPreset::Custom,
        temp: 5200.0,
        tint: 8.0,
    };
    settings.tone_curve.parametric.lights = 20.0;
    settings.tone_curve.rgb = PointCurve::new([
        CurvePoint::new(0.0, 0.0),
        CurvePoint::new(0.4, 0.35),
        CurvePoint::new(1.0, 1.0),
    ]);
    settings.tone_curve.red = settings.tone_curve.rgb.clone();
    settings.tone_curve.blue = settings.tone_curve.rgb.clone();
    settings.color_mixer.saturation[5] = 30.0;
    settings.color_mixer.hue[3] = -20.0;
    settings.color_mixer.luminance[5] = -25.0;
    settings.color_grading.shadows.saturation = 20.0;
    settings.color_grading.shadows.hue = 220.0;
    settings.color_grading.highlights.saturation = 15.0;
    settings.color_grading.highlights.hue = 45.0;
    settings.calibration.blue_saturation = 20.0;
    settings.calibration.shadows_tint = 5.0;
    settings
}

/// Source-native pixels with a spread of brightness and colour, modest
/// enough to be plausible for both a raster file and a raw file.
fn pixels() -> Vec<[f32; 3]> {
    (0..COUNT)
        .map(|i| {
            let t = i as f32 / COUNT as f32;
            let wobble = (t * 37.0).fract();
            [
                0.02 + 0.25 * t,
                0.03 + 0.4 * (1.0 - t) * wobble + 0.1 * t,
                0.02 + 0.3 * wobble * t,
            ]
        })
        .collect()
}

/// Best of five passes, in nanoseconds per item.
fn best_of<T: Copy>(items: &[T], mut f: impl FnMut(T) -> f32) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..5 {
        let start = Instant::now();
        let mut sum = 0.0f32;
        for item in items {
            sum += f(black_box(*item));
        }
        black_box(sum);
        best = best.min(start.elapsed().as_secs_f64() * 1.0e9 / items.len() as f64);
    }
    best
}

fn sum(rgb: [f32; 3]) -> f32 {
    rgb[0] + rgb[1] + rgb[2]
}

fn chain(name: &str, settings: &DevelopSettings, source: &SourceColor, local: Option<&LocalDelta>) {
    let pipeline = PixelPipeline::new(settings, source);
    let source_pixels = pixels();
    let total = best_of(&source_pixels, |pixel| {
        let balanced = pipeline.input(pixel, local);
        let display = pipeline.scene_to_display(balanced, local);
        let toned = pipeline.tone(display, tone_base_value(display), local);
        let bytes = pipeline.output_u8(pipeline.color(toned, local));
        f32::from(bytes[0]) + f32::from(bytes[1]) + f32::from(bytes[2])
    });
    println!("{name:<44} {total:7.1} ns/pixel");
}

/// Each stage timed on its own over the whole buffer, the way a pipeline
/// that interleaves spatial stages runs them.
fn breakdown(
    name: &str,
    settings: &DevelopSettings,
    source: &SourceColor,
    local: Option<&LocalDelta>,
) {
    let pipeline = PixelPipeline::new(settings, source);
    let source_pixels = pixels();
    let balanced: Vec<[f32; 3]> = source_pixels
        .iter()
        .map(|p| pipeline.input(*p, local))
        .collect();
    let display: Vec<[f32; 3]> = balanced
        .iter()
        .map(|p| pipeline.scene_to_display(*p, local))
        .collect();
    let with_base: Vec<([f32; 3], f32)> =
        display.iter().map(|p| (*p, tone_base_value(*p))).collect();
    let toned: Vec<[f32; 3]> = with_base
        .iter()
        .map(|(p, base)| pipeline.tone(*p, *base, local))
        .collect();
    let coloured: Vec<[f32; 3]> = toned.iter().map(|p| pipeline.color(*p, local)).collect();
    let stages = [
        (
            "input",
            best_of(&source_pixels, |p| sum(pipeline.input(p, local))),
        ),
        (
            "scene_to_display",
            best_of(&balanced, |p| sum(pipeline.scene_to_display(p, local))),
        ),
        ("tone_base_value", best_of(&display, tone_base_value)),
        (
            "tone",
            best_of(&with_base, |(p, base)| sum(pipeline.tone(p, base, local))),
        ),
        ("color", best_of(&toned, |p| sum(pipeline.color(p, local)))),
        (
            "output_u8",
            best_of(&coloured, |p| f32::from(pipeline.output_u8(p)[0])),
        ),
    ];
    let overhead = best_of(&coloured, sum);
    println!("{name}");
    for (stage, nanoseconds) in stages {
        println!("    {stage:<18} {:6.1} ns", nanoseconds - overhead);
    }
    let total: f64 = stages
        .iter()
        .map(|(_, nanoseconds)| nanoseconds - overhead)
        .sum();
    println!("    {:<18} {total:6.1} ns", "sum of stages");
}

#[test]
#[ignore = "timing, not a contract"]
fn time_the_per_pixel_chain() {
    let (typical, everything) = (typical(), everything());
    let delta = LocalDelta {
        exposure: 0.5,
        contrast: 10.0,
        highlights: -10.0,
        shadows: 10.0,
        whites: 5.0,
        blacks: 5.0,
        temp: 10.0,
        tint: 5.0,
        saturation: 10.0,
    };
    let build = Instant::now();
    for _ in 0..200 {
        black_box(PixelPipeline::new(black_box(&everything), &camera()));
    }
    println!(
        "building a pipeline with every table        {:7.1} us",
        build.elapsed().as_secs_f64() * 5.0e3
    );
    chain(
        "raster, neutral settings",
        &DevelopSettings::default(),
        &SRGB,
        None,
    );
    chain("raster, typical settings", &typical, &SRGB, None);
    chain("camera, typical settings", &typical, &camera(), None);
    chain(
        "camera, every operator active",
        &everything,
        &camera(),
        None,
    );
    chain(
        "camera, every operator active, masked pixel",
        &everything,
        &camera(),
        Some(&delta),
    );
    breakdown("raster, typical settings, by stage", &typical, &SRGB, None);
    breakdown(
        "camera, typical settings, by stage",
        &typical,
        &camera(),
        None,
    );
    breakdown(
        "camera, every operator active, by stage",
        &everything,
        &camera(),
        None,
    );
    breakdown(
        "camera, every operator active, masked, by stage",
        &everything,
        &camera(),
        Some(&delta),
    );
}
