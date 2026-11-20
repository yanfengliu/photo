//! Wall-clock cost of a 2.5 MP view of a 24 MP photo (DESIGN §5.10). Not a
//! contract: run it by hand and read the table with
//! `cargo test -p photo-develop --test pipeline timing -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{DevelopSettings, Tone};
use photo_core::meta::FileKind;
use photo_develop::{CancelToken, Quality, RenderRequest, Renderer, Source, fit_view};
use photo_imaging::View;

use crate::fixtures::{camera_scene, raster};
use crate::region::everything;

/// A plausible edit: the six tone sliders, a little vibrance and
/// saturation, on top of the kind's defaults.
fn typical(kind: FileKind) -> DevelopSettings {
    let mut settings = DevelopSettings::default_for(kind);
    settings.tone = Tone {
        exposure: 0.35,
        contrast: 20.0,
        highlights: -40.0,
        shadows: 30.0,
        whites: 10.0,
        blacks: -5.0,
    };
    settings.presence.vibrance = 20.0;
    settings.presence.saturation = 5.0;
    settings
}

fn render(
    renderer: &mut Renderer,
    source: &Source,
    settings: &DevelopSettings,
    view: View,
    quality: Quality,
) -> Duration {
    let request = RenderRequest {
        source,
        settings,
        view,
        quality,
        space: RgbSpace::Srgb,
    };
    let start = Instant::now();
    renderer
        .render_rgba8(&request, &CancelToken::new())
        .unwrap();
    start.elapsed()
}

fn report(name: &str, best: Duration, renderer: &Renderer) {
    println!("{name:<44} {:8.1} ms", best.as_secs_f64() * 1000.0);
    for (stage, time) in renderer.last_stage_times() {
        println!("    {stage:<40} {:8.1} ms", time.as_secs_f64() * 1000.0);
    }
}

/// Best of `runs` renders, each made after `step` moved one slider on, as
/// a drag does.
fn best_warm(
    renderer: &mut Renderer,
    source: &Source,
    base: &DevelopSettings,
    view: View,
    quality: Quality,
    step: impl Fn(&mut DevelopSettings, f32),
) -> Duration {
    render(renderer, source, base, view, quality);
    (0..7)
        .map(|run| {
            let mut settings = base.clone();
            step(&mut settings, (run + 1) as f32);
            render(renderer, source, &settings, view, quality)
        })
        .min()
        .unwrap()
}

fn exposure(settings: &mut DevelopSettings, run: f32) {
    settings.tone.exposure += 0.01 * run;
}

fn vignette(settings: &mut DevelopSettings, run: f32) {
    settings.effects.vignette.amount -= run;
}

#[test]
#[ignore = "prints timings; run on demand with --ignored --nocapture"]
fn timings_of_a_2_5_mp_view_of_a_24_mp_photo() {
    let (width, height) = (6000, 4000);
    println!(
        "\n{width} x {height} source, {} threads",
        rayon::current_num_threads()
    );
    let start = Instant::now();
    let (_, raster) = raster(width, height);
    let camera = camera_scene(width, height);
    println!(
        "{:<44} {:8.1} ms (two sources)",
        "Source::new",
        start.elapsed().as_secs_f64() * 500.0
    );
    for (name, source, kind) in [
        ("raster", &raster, FileKind::Raster),
        ("camera", &camera, FileKind::Raw),
    ] {
        let typical = typical(kind);
        let all = DevelopSettings {
            detail: typical.detail,
            ..everything()
        };
        let view = fit_view(source, &typical, 1936, 1291);
        let half = View::new(view.roi(), view.width() / 2, view.height() / 2);
        println!("\n{name}: view {} x {}", view.width(), view.height());

        let mut renderer = Renderer::new();
        let cold = (0..3)
            .map(|_| {
                renderer = Renderer::new();
                render(&mut renderer, source, &typical, view, Quality::Full)
            })
            .min()
            .unwrap();
        report("cold, typical settings", cold, &renderer);

        let warm = best_warm(
            &mut renderer,
            source,
            &typical,
            view,
            Quality::Full,
            exposure,
        );
        report("warm, typical, one tone slider moved", warm, &renderer);

        let mut renderer = Renderer::new();
        let everything = best_warm(&mut renderer, source, &all, view, Quality::Full, exposure);
        report(
            "warm, every stage, one tone slider moved",
            everything,
            &renderer,
        );

        let effects = best_warm(&mut renderer, source, &all, view, Quality::Full, vignette);
        report("warm, every stage, vignette moved", effects, &renderer);

        let mut renderer = Renderer::new();
        let draft = best_warm(&mut renderer, source, &all, half, Quality::Draft, exposure);
        report("draft at half scale, every stage", draft, &renderer);
    }
}
