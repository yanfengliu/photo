//! Frames of every Develop state, rendered on the GPU at 1600 x 1000
//! points, at 1x and 2x, to `target/ui-snapshots/develop-*.png`, for a
//! person to look at. On a machine with no GPU nothing is rendered and the
//! tests only drive the states.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers, PointerButton, Role, pos2};
use photo_ui::harness::{Harness, RenderError};
use photo_ui::{CanvasImage, PhotoApp};

use crate::{
    PATIENCE, Studio, TestResult, drag_slider, open_panel, protect, scene_files, session, settle,
    slider,
};

/// Where a frame goes: `target/ui-snapshots` of the workspace.
fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/ui-snapshots")
        .join(name)
}

/// Render the resting frame as `develop-{name}.png` (with `@2x` at two
/// pixels per point).
pub fn snap(harness: &mut Harness<PhotoApp>, name: &str, scale: u32) -> TestResult {
    settle(harness)?;
    harness.pointer_leave()?;
    harness.advance(3.0);
    harness.run()?;
    snap_now(harness, name, scale)
}

/// Render the frame as it is, while work may still be running.
pub fn snap_now(harness: &mut Harness<PhotoApp>, name: &str, scale: u32) -> TestResult {
    let suffix = if scale == 1 {
        String::new()
    } else {
        format!("@{scale}x")
    };
    let file = format!("develop-{name}{suffix}.png");
    let window = harness.ctx().content_rect().size() * scale as f32;
    match harness.save_png(snapshot_path(&file)) {
        Ok(()) => {
            let image = image::open(snapshot_path(&file))?;
            assert_eq!(
                (image.width(), image.height()),
                (window.x as u32, window.y as u32)
            );
            Ok(())
        }
        Err(RenderError::NoAdapter(reason)) => {
            eprintln!("{file} not rendered, no GPU adapter: {reason}");
            Ok(())
        }
        Err(other) => Err(other.into()),
    }
}

/// Set a slider through the accessibility tree: one complete edit.
fn set(harness: &mut Harness<PhotoApp>, name: &str, value: f64) -> TestResult {
    harness.set_value(Role::Slider, name, value)?;
    Ok(())
}

/// Every Develop state, at `scale` pixels per point.
fn every_state(scale: u32) -> TestResult {
    let (_files_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with_at(files, scale as f32);
    settle(&mut harness)?;
    let dng = harness
        .control(Role::ListBoxOption, "scene.dng")?
        .rect
        .center();
    harness.click_at(dng)?;

    // The camera's preview while the raw file is read.
    harness.app_mut().hold_results(true);
    harness.press_key(Key::D, Modifiers::NONE)?;
    let started = Instant::now();
    while started.elapsed() < PATIENCE {
        std::thread::sleep(Duration::from_millis(1));
        harness.app_mut().hold_results(false);
        harness.step(1);
        harness.app_mut().hold_results(true);
        harness.step(1);
        if harness.app().canvas_shows().image == CanvasImage::EmbeddedPreview {
            break;
        }
    }
    if harness.app().canvas_shows().image == CanvasImage::EmbeddedPreview {
        snap_now(&mut harness, "embedded-preview", scale)?;
    }
    harness.app_mut().hold_results(false);
    snap(&mut harness, "raw-opened", scale)?;

    // A first edit of the raw file.
    for (name, value) in [
        ("Exposure", 0.45),
        ("Highlights", -55.0),
        ("Shadows", 35.0),
        ("Whites", 12.0),
        ("Blacks", -8.0),
        ("Vibrance", 18.0),
        ("Clarity", 10.0),
    ] {
        set(&mut harness, name, value)?;
    }
    snap(&mut harness, "basic", scale)?;

    open_panel(&mut harness, "Tone Curve")?;
    set(&mut harness, "Curve Lights", 25.0)?;
    set(&mut harness, "Curve Darks", -20.0)?;
    snap(&mut harness, "tone-curve-parametric", scale)?;
    harness.click(Role::Button, "RGB curve")?;
    let plot = harness.control(Role::Canvas, "Tone curve RGB")?.rect;
    for (x, y) in [(0.25, 0.20), (0.75, 0.82)] {
        let at = pos2(
            plot.left() + x * plot.width(),
            plot.bottom() - x * plot.height(),
        );
        let to = pos2(at.x, plot.bottom() - y * plot.height());
        harness.drag(at, to)?;
    }
    snap(&mut harness, "tone-curve-point", scale)?;

    open_panel(&mut harness, "Color Mixer")?;
    harness.click(Role::Button, "Mixer All")?;
    set(&mut harness, "Hue Orange", -12.0)?;
    set(&mut harness, "Saturation Blue", 25.0)?;
    set(&mut harness, "Luminance Blue", -20.0)?;
    snap(&mut harness, "color-mixer", scale)?;
    open_panel(&mut harness, "Basic")?;
    harness.click(Role::Button, "Black & White")?;
    open_panel(&mut harness, "Color Mixer")?;
    set(&mut harness, "B&W Blue", -40.0)?;
    snap(&mut harness, "color-mixer-black-and-white", scale)?;
    open_panel(&mut harness, "Basic")?;
    harness.click(Role::Button, "Color")?;

    open_panel(&mut harness, "Color Grading")?;
    for (wheel, x, y) in [
        ("Shadows wheel", 0.36, 0.6),
        ("Highlights wheel", 0.6, 0.42),
    ] {
        let disc = harness.control(Role::ColorWell, wheel)?.rect;
        let at = pos2(
            disc.left() + x * disc.width(),
            disc.top() + y * disc.height(),
        );
        harness.click_at(at)?;
    }
    snap(&mut harness, "color-grading", scale)?;

    for panel in [
        "Detail",
        "Lens Corrections",
        "Transform",
        "Effects",
        "Calibration",
    ] {
        open_panel(&mut harness, panel)?;
        let name = panel.to_lowercase().replace(' ', "-");
        snap(&mut harness, &name, scale)?;
    }
    open_panel(&mut harness, "Basic")?;

    harness.press_key(Key::Y, Modifiers::NONE)?;
    snap(&mut harness, "before-after", scale)?;
    harness.press_key(Key::Y, Modifiers::SHIFT)?;
    snap(&mut harness, "split", scale)?;
    harness.press_key(Key::Backslash, Modifiers::NONE)?;
    snap(&mut harness, "before", scale)?;
    harness.press_key(Key::Backslash, Modifiers::NONE)?;

    set(&mut harness, "Exposure", 1.6)?;
    harness.press_key(Key::J, Modifiers::NONE)?;
    snap(&mut harness, "clipping", scale)?;
    harness.press_key(Key::J, Modifiers::NONE)?;
    set(&mut harness, "Exposure", 0.45)?;

    harness.click(Role::Button, "1:1")?;
    snap(&mut harness, "one-to-one", scale)?;
    harness.click(Role::Button, "4:1")?;
    snap(&mut harness, "four-to-one", scale)?;
    harness.click(Role::Button, "Fit")?;

    harness.press_key(Key::N, Modifiers::COMMAND)?;
    harness.type_text("Warm edit")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    drag_slider(&mut harness, "Exposure", -0.3)?;
    harness.press_key(Key::N, Modifiers::COMMAND)?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    let preset = harness
        .control(Role::TreeItem, "Preset Mono Contrast")?
        .rect
        .center();
    harness.hover_at(preset)?;
    settle(&mut harness)?;
    harness.hover_at(preset)?;
    snap_now(&mut harness, "presets", scale)?;
    harness.pointer_leave()?;

    harness.press_key(Key::C, Modifiers::COMMAND | Modifiers::SHIFT)?;
    snap(&mut harness, "copy-settings", scale)?;
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    harness.click(Role::Button, "New preset")?;
    harness.type_text("Harbour evening")?;
    snap(&mut harness, "new-preset", scale)?;
    harness.press_key(Key::Escape, Modifiers::NONE)?;

    let step = harness
        .control(Role::TreeItem, "Snapshot Warm edit")?
        .rect
        .center();
    harness.click_at_with(step, PointerButton::Secondary, Modifiers::NONE)?;
    snap(&mut harness, "snapshot-menu", scale)?;
    harness.press_key(Key::Escape, Modifiers::NONE)?;

    // The rendered file beside it, and the slider row being dragged.
    let png = harness
        .control(Role::ListBoxOption, "Filmstrip scene.png")?
        .rect
        .center();
    harness.click_at(png)?;
    settle(&mut harness)?;
    assert_eq!(session(&harness).kind(), photo_core::meta::FileKind::Raster);
    let exposure = slider(&harness, "Exposure")?;
    harness.pointer_down(exposure.rect.center())?;
    harness.pointer_move(pos2(
        exposure.rect.center().x + 30.0,
        exposure.rect.center().y,
    ))?;
    settle(&mut harness)?;
    snap_now(&mut harness, "dragging", scale)?;
    harness.pointer_up(pos2(
        exposure.rect.center().x + 30.0,
        exposure.rect.center().y,
    ))?;

    harness.press_key(Key::G, Modifiers::NONE)?;
    harness.click(Role::DisclosureTriangle, "Quick Develop")?;
    snap(&mut harness, "quick-develop", scale)?;
    Ok(())
}

/// The read-only state, at `scale` pixels per point.
fn read_only(scale: u32) -> TestResult {
    let (_files_dir, files) = scene_files();
    let studio = Studio::new();
    {
        let mut engine = studio.options.open_engine()?;
        crate::import(&mut engine, files);
    }
    if !protect(&studio.options.catalog_path) {
        eprintln!("skipped: this user can write a read-only file");
        return Ok(());
    }
    let mut harness = studio.app_at(scale as f32);
    crate::develop(&mut harness, "scene.dng")?;
    snap(&mut harness, "read-only", scale)
}

#[test]
fn every_develop_state_renders_at_1x() -> TestResult {
    every_state(1)?;
    read_only(1)
}

#[test]
fn every_develop_state_renders_at_2x() -> TestResult {
    every_state(2)?;
    read_only(2)
}
