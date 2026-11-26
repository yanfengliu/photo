//! Frames of the crop, healing and masking tools, rendered on the GPU at
//! 1600 x 1000 points, at 1x and 2x, to
//! `target/ui-snapshots/develop-tools-*.png`, for a person to look at. On a
//! machine with no GPU nothing is rendered and the tests only drive the
//! tools.

use egui::{Key, Modifiers, Pos2, Role, vec2};
use photo_core::geom::Point;
use photo_ui::PhotoApp;
use photo_ui::harness::Harness;

use crate::snapshots::{snap, snap_now};
use crate::{Studio, TestResult, develop, scene_files, settle};

/// Where a point of the photo is on the canvas.
fn on_canvas(harness: &Harness<PhotoApp>, u: f32, v: f32) -> Pos2 {
    harness
        .app()
        .canvas_position(Point::new(u, v))
        .expect("the photo is on the canvas")
}

/// Every tool, at `scale` pixels per point.
fn every_tool(scale: u32) -> TestResult {
    let (_files_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with_at(files, scale as f32);
    develop(&mut harness, "scene.png")?;

    // The crop tool: a corner mid-drag, with the thirds.
    harness.press_key(Key::R, Modifiers::NONE)?;
    settle(&mut harness)?;
    snap(&mut harness, "tools-crop-open", scale)?;
    harness.click(Role::ComboBox, "Crop aspect")?;
    harness.click(Role::Button, "16 : 9")?;
    harness.set_value(Role::Slider, "Crop Angle", 4.0)?;
    settle(&mut harness)?;
    let rect = harness.app().crop_rect().ok_or("the rectangle is drawn")?;
    let corner = rect.right_bottom();
    harness.pointer_down(corner)?;
    harness.pointer_move(corner - vec2(60.0, 30.0))?;
    harness.pointer_move(corner - vec2(140.0, 70.0))?;
    settle(&mut harness)?;
    snap_now(&mut harness, "tools-crop-dragging", scale)?;
    harness.pointer_up(corner - vec2(140.0, 70.0))?;

    // The straighten tool: the line being drawn over the grid.
    harness.click(Role::Button, "Straighten")?;
    let photo = harness.app().canvas_photo_rect().ok_or("a photo")?;
    let from = photo.center() + vec2(-260.0, 40.0);
    let to = photo.center() + vec2(240.0, -10.0);
    harness.pointer_down(from)?;
    harness.pointer_move(from.lerp(to, 0.5))?;
    harness.pointer_move(to)?;
    settle(&mut harness)?;
    snap_now(&mut harness, "tools-crop-straightening", scale)?;
    harness.pointer_up(to)?;
    snap(&mut harness, "tools-crop-straightened", scale)?;
    harness.click(Role::Button, "Rotate right")?;
    snap(&mut harness, "tools-crop-turned", scale)?;
    harness.click(Role::Button, "Rotate left")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    snap(&mut harness, "tools-crop-done", scale)?;

    // Healing: a spot placed by a click, one sized by a drag.
    harness.press_key(Key::Q, Modifiers::NONE)?;
    settle(&mut harness)?;
    harness.click_at(on_canvas(&harness, 0.45, 0.72))?;
    settle(&mut harness)?;
    harness.drag(
        on_canvas(&harness, 0.62, 0.45),
        on_canvas(&harness, 0.66, 0.45),
    )?;
    snap(&mut harness, "tools-heal-spots", scale)?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;

    // A linear gradient, with its overlay and without.
    harness.press_key(Key::M, Modifiers::NONE)?;
    settle(&mut harness)?;
    harness.drag(
        on_canvas(&harness, 0.5, 0.25),
        on_canvas(&harness, 0.55, 0.6),
    )?;
    harness.set_value(Role::Slider, "Mask Exposure", -1.0)?;
    snap(&mut harness, "tools-mask-linear-overlay", scale)?;
    harness.press_key(Key::O, Modifiers::NONE)?;
    snap(&mut harness, "tools-mask-linear", scale)?;

    // A radial gradient.
    harness.press_key(Key::M, Modifiers::SHIFT)?;
    harness.drag(
        on_canvas(&harness, 0.5, 0.5),
        on_canvas(&harness, 0.62, 0.62),
    )?;
    harness.set_value(Role::Slider, "Mask Exposure", 1.2)?;
    snap(&mut harness, "tools-mask-radial-overlay", scale)?;
    harness.press_key(Key::O, Modifiers::NONE)?;
    snap(&mut harness, "tools-mask-radial", scale)?;

    // The brush: a stroke, its overlay, and the brush's outline under the
    // pointer.
    harness.press_key(Key::K, Modifiers::NONE)?;
    harness.set_value(Role::Slider, "Brush Size", 5.0)?;
    harness.drag(
        on_canvas(&harness, 0.42, 0.35),
        on_canvas(&harness, 0.62, 0.42),
    )?;
    settle(&mut harness)?;
    harness.hover_at(on_canvas(&harness, 0.7, 0.4))?;
    settle(&mut harness)?;
    snap_now(&mut harness, "tools-mask-brush", scale)?;

    // The same at 1:1: the marks stay on their content.
    harness.pointer_leave()?;
    harness.click(Role::Button, "1:1")?;
    settle(&mut harness)?;
    harness.hover_at(egui::pos2(760.0, 430.0))?;
    settle(&mut harness)?;
    snap_now(&mut harness, "tools-mask-one-to-one", scale)?;
    harness.pointer_leave()?;
    harness.click(Role::Button, "Fit")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    snap(&mut harness, "tools-done", scale)?;
    Ok(())
}

#[test]
fn every_tool_renders_at_1x() -> TestResult {
    every_tool(1)
}

#[test]
fn every_tool_renders_at_2x() -> TestResult {
    every_tool(2)
}
