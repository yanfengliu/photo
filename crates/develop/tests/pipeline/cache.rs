//! Stage caches change nothing, and cancellation is prompt.

use std::time::{Duration, Instant};

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{
    CropRect, DevelopSettings, Mask, MaskShape, Orientation, Spot, SpotMode, WbPreset, WhiteBalance,
};
use photo_core::geom::{Point, Rect};
use photo_develop::{CancelToken, Quality, RenderRequest, Renderer, Source};
use photo_imaging::View;

use crate::fixtures::{camera_scene, full, raster, render};
use crate::region::everything;

/// One step of the script: settings, view and quality.
struct Step {
    name: &'static str,
    settings: DevelopSettings,
    view: View,
    quality: Quality,
}

fn script(source: &Source) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut settings = DevelopSettings::default();
    let whole = full(source, &settings);
    let mut push = |name, settings: &DevelopSettings, view: View, quality| {
        steps.push(Step {
            name,
            settings: settings.clone(),
            view,
            quality,
        });
    };
    push("defaults", &settings, whole, Quality::Full);
    settings.tone.exposure = 0.5;
    push("exposure", &settings, whole, Quality::Full);
    settings.tone.highlights = -40.0;
    settings.tone.shadows = 30.0;
    push("highlights and shadows", &settings, whole, Quality::Full);
    settings.presence.vibrance = 30.0;
    settings.color_mixer.hue[5] = 20.0;
    push("colour", &settings, whole, Quality::Full);
    settings.white_balance = WhiteBalance {
        preset: WbPreset::Custom,
        temp: 20.0,
        tint: -10.0,
    };
    push("white balance", &settings, whole, Quality::Full);
    settings.detail.noise_reduction.luminance = 30.0;
    settings.detail.noise_reduction.color = 20.0;
    push("noise reduction", &settings, whole, Quality::Full);
    settings.spots.push(Spot {
        mode: SpotMode::Clone,
        target: Point::new(0.3, 0.7),
        source: Point::new(0.5, 0.7),
        radius: 0.05,
        feather: 30.0,
        opacity: 100.0,
    });
    push("spot", &settings, whole, Quality::Full);
    let mut mask = Mask::from_shape(
        MaskShape::Linear {
            start: Point::new(0.5, 0.0),
            end: Point::new(0.5, 0.6),
        },
        1,
    );
    mask.adjust.exposure = -0.7;
    settings.masks.push(mask);
    push("mask", &settings, whole, Quality::Full);
    settings.masks[0].adjust.exposure = 0.4;
    settings.masks[0].adjust.temp = 15.0;
    push("mask adjustment", &settings, whole, Quality::Full);
    settings.masks[0].components[0].shape = MaskShape::Linear {
        start: Point::new(0.2, 0.0),
        end: Point::new(0.6, 0.6),
    };
    push("mask shape", &settings, whole, Quality::Full);
    settings.presence.clarity = 30.0;
    settings.presence.texture = 25.0;
    settings.presence.dehaze = 15.0;
    settings.detail.sharpening.amount = 40.0;
    settings.effects.grain.amount = 20.0;
    push("presence and detail", &settings, whole, Quality::Full);
    // Each of these moves only a stage after cache B.
    settings.presence.texture = -40.0;
    push("texture alone", &settings, whole, Quality::Full);
    settings.detail.sharpening.amount = 90.0;
    push("sharpening alone", &settings, whole, Quality::Full);
    settings.effects.vignette.amount = -45.0;
    push("vignette alone", &settings, whole, Quality::Full);
    settings.effects.grain.amount = 60.0;
    push("grain amount alone", &settings, whole, Quality::Full);
    settings.effects.grain.size = 70.0;
    push("grain size", &settings, whole, Quality::Full);
    settings.masks[0].adjust.sharpness = 50.0;
    push("mask sharpness", &settings, whole, Quality::Full);
    let pan = View::new(Rect::new(40.0, 30.0, 240.0, 180.0), 200, 150);
    push("pan", &settings, pan, Quality::Full);
    let zoom = View::new(Rect::new(40.0, 30.0, 140.0, 105.0), 200, 150);
    push("zoom in", &settings, zoom, Quality::Full);
    push("draft", &settings, zoom, Quality::Draft);
    settings.crop.rect = Some(CropRect {
        cx: 0.45,
        cy: 0.5,
        width: 0.7,
        height: 0.8,
    });
    push("crop", &settings, full(source, &settings), Quality::Full);
    settings.orientation = Orientation::new(1, false);
    push("turn", &settings, full(source, &settings), Quality::Full);
    settings.tone.exposure = -0.3;
    push(
        "exposure again",
        &settings,
        full(source, &settings),
        Quality::Full,
    );
    steps
}

#[test]
fn every_warm_render_equals_a_cold_render_of_the_same_request() {
    let (_, raster) = raster(320, 220);
    let camera = camera_scene(320, 220);
    let mut warm = Renderer::new();
    let cancel = CancelToken::new();
    for source in [&raster, &camera, &raster] {
        for step in script(source) {
            let request = RenderRequest {
                source,
                settings: &step.settings,
                view: step.view,
                quality: step.quality,
                space: RgbSpace::Srgb,
            };
            let reused = warm.render_rgba8(&request, &cancel).unwrap();
            let fresh = Renderer::new().render_rgba8(&request, &cancel).unwrap();
            assert!(reused == fresh, "{}: a warm render differs", step.name);
            let reused = warm.render_rgb16(&request, &cancel).unwrap();
            let fresh = Renderer::new().render_rgb16(&request, &cancel).unwrap();
            assert!(
                reused == fresh,
                "{}: a warm 16-bit render differs",
                step.name
            );
        }
    }
}

#[test]
fn a_cancelled_render_returns_none_promptly_and_the_renderer_stays_usable() {
    let (_, source) = raster(2400, 1600);
    let settings = everything();
    let view = full(&source, &settings);
    let request = RenderRequest {
        source: &source,
        settings: &settings,
        view,
        quality: Quality::Full,
        space: RgbSpace::Srgb,
    };
    let mut renderer = Renderer::new();

    let cancelled = CancelToken::new();
    cancelled.cancel();
    let start = Instant::now();
    assert!(renderer.render_rgba8(&request, &cancelled).is_none());
    assert!(start.elapsed() < Duration::from_millis(50));

    // Measure an uncancelled cold render, then cancel one shortly after it
    // starts and time how long it takes to give up.
    let start = Instant::now();
    let complete = Renderer::new()
        .render_rgba8(&request, &CancelToken::new())
        .unwrap();
    let cold = start.elapsed();
    let token = CancelToken::new();
    let canceller = {
        let token = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(cold / 5);
            let at = Instant::now();
            token.cancel();
            at
        })
    };
    let result = renderer.render_rgba8(&request, &token);
    let returned = Instant::now();
    let cancelled_at = canceller.join().unwrap();
    assert!(
        result.is_none(),
        "the render finished before it was cancelled"
    );
    let lag = returned.saturating_duration_since(cancelled_at);
    println!("cold render {cold:?}, returned {lag:?} after the cancel");
    assert!(lag < Duration::from_millis(60).max(cold / 10), "{lag:?}");

    // The renderer still works, and gives what a fresh one gives.
    let after = renderer
        .render_rgba8(&request, &CancelToken::new())
        .unwrap();
    assert!(after == complete);
    assert!(render(&source, &settings, view) == complete);
}
