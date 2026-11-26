//! The masking tool: a radial gradient with exposure brightens inside it
//! and not outside, and the engine's overlay covers the same place; a
//! linear gradient is moved and turned by its handles; the brush paints
//! and, with Alt, erases; the list renames, hides, inverts and deletes;
//! Escape takes everything back.

use egui::{Key, Modifiers, Role, vec2};
use photo_core::develop::{MaskShape, Spot};
use photo_core::geom::Point;
use photo_ui::harness::Harness;
use photo_ui::{PhotoApp, Tool};

use crate::{
    Studio, TestResult, develop, frame_image, history, luma_at, scene_files, session, settings,
    settle,
};

/// Where a point of the photo is on the canvas.
fn on_canvas(harness: &Harness<PhotoApp>, u: f32, v: f32) -> egui::Pos2 {
    harness
        .app()
        .canvas_position(Point::new(u, v))
        .expect("the photo is on the canvas")
}

/// The scene's PNG open in Develop.
fn developing() -> Result<(tempfile::TempDir, Studio, Harness<PhotoApp>), Box<dyn std::error::Error>>
{
    let (dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    Ok((dir, studio, harness))
}

/// The coverage the engine's overlay has at fractions `(u, v)` of the
/// photo.
fn overlay_at(harness: &Harness<PhotoApp>, u: f32, v: f32) -> Option<f32> {
    let overlay = session(harness).mask_overlay()?;
    let alpha = overlay.alpha();
    let x = ((u * alpha.width() as f32) as usize).min(alpha.width() - 1);
    let y = ((v * alpha.height() as f32) as usize).min(alpha.height() - 1);
    Some(alpha.get(x, y))
}

#[test]
fn a_radial_mask_with_exposure_brightens_inside_only_and_the_overlay_matches() -> TestResult {
    let (_dir, _studio, mut harness) = developing()?;
    let steps = history(&harness).len();
    let before = frame_image(&harness);
    harness.press_key(Key::M, Modifiers::SHIFT)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().tool(), Some(Tool::Mask));

    // Pull an ellipse out in the sky: 120 pixels wide each way, 80 high.
    let (inside, outside) = ((0.3, 0.3), (0.62, 0.3));
    let centre = on_canvas(&harness, inside.0, inside.1);
    harness.drag(centre, on_canvas(&harness, 0.4, 0.4))?;
    settle(&mut harness)?;
    let masks = settings(&harness).masks;
    assert_eq!(masks.len(), 1);
    assert_eq!(masks[0].name, "Radial Gradient 1");
    let MaskShape::Radial {
        center,
        radius_x,
        radius_y,
        angle,
        ..
    } = masks[0].components[0].shape
    else {
        panic!("a radial gradient: {:?}", masks[0].components);
    };
    assert!(center.distance(Point::new(0.3, 0.3)) < 0.002, "{center:?}");
    assert!((radius_x - 0.1).abs() < 0.003, "{radius_x}");
    assert!((radius_y - 80.0 / 1200.0).abs() < 0.003, "{radius_y}");
    assert!(angle.abs() < 0.01, "{angle}");
    assert_eq!(harness.app().selected_mask(), Some(0));
    assert_eq!(history(&harness).len(), steps, "nothing recorded yet");

    harness.set_value(Role::Slider, "Mask Exposure", 1.0)?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).masks[0].adjust.exposure, 1.0);
    let after = frame_image(&harness);
    let lift = |at: (f32, f32)| luma_at(&after, at.0, at.1) - luma_at(&before, at.0, at.1);
    assert!(lift(inside) > 25.0, "brighter inside: {}", lift(inside));
    assert!(
        lift(outside).abs() <= 1.0,
        "untouched outside: {}",
        lift(outside)
    );

    // The overlay is the engine's coverage of that mask: on where the
    // mask is, off where it is not, and drawn red over the photo.
    assert!(harness.app().mask_overlay(), "a new mask shows its overlay");
    assert_eq!(
        session(&harness)
            .mask_overlay()
            .map(|overlay| overlay.mask()),
        Some(0)
    );
    assert!(overlay_at(&harness, inside.0, inside.1).ok_or("an overlay")? > 0.95);
    assert_eq!(overlay_at(&harness, outside.0, outside.1), Some(0.0));
    if harness.can_render() {
        harness.pointer_leave()?;
        let image = harness.render()?;
        let redness = |at: (f32, f32)| {
            let on = on_canvas(&harness, at.0, at.1);
            let pixel = image.get_pixel(on.x as u32, on.y as u32);
            i32::from(pixel[0]) - i32::from(pixel[2])
        };
        // Beside the middle, where the mask's pin is drawn.
        let beside = (inside.0 + 0.03, inside.1 + 0.01);
        assert!(
            redness(beside) > redness(outside) + 60,
            "red inside: {} against {}",
            redness(beside),
            redness(outside)
        );
    }

    // O puts the overlay away.
    harness.press_key(Key::O, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert!(!harness.app().mask_overlay());
    assert!(session(&harness).mask_overlay().is_none());
    assert_eq!(
        harness
            .control(Role::CheckBox, "Show mask overlay")?
            .toggled,
        Some(false)
    );

    harness.press_key(Key::Enter, Modifiers::NONE)?;
    settle(&mut harness)?;
    let labels = history(&harness);
    assert_eq!(labels.len(), steps + 1, "one step: {labels:?}");
    assert_eq!(labels[labels.len() - 1], "Add Mask");
    assert_eq!(harness.app().tool(), None);
    Ok(())
}

#[test]
fn a_linear_gradient_is_placed_by_a_drag_and_moved_and_turned_by_its_handles() -> TestResult {
    let (_dir, _studio, mut harness) = developing()?;
    let steps = history(&harness).len();
    harness.press_key(Key::M, Modifiers::NONE)?;
    settle(&mut harness)?;
    let linear = |harness: &Harness<PhotoApp>| match &settings(harness).masks[0].components[0].shape
    {
        MaskShape::Linear { start, end } => (*start, *end),
        other => panic!("a linear gradient: {other:?}"),
    };

    // Full effect from the top, fading out between a fifth and three
    // fifths of the way down.
    harness.drag(on_canvas(&harness, 0.5, 0.2), on_canvas(&harness, 0.5, 0.6))?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).masks.len(), 1);
    let (start, end) = linear(&harness);
    assert!(start.distance(Point::new(0.5, 0.2)) < 0.003, "{start:?}");
    assert!(end.distance(Point::new(0.5, 0.6)) < 0.003, "{end:?}");

    // The pin in the middle moves all of it.
    let pin = on_canvas(&harness, 0.5, 0.4);
    harness.drag(pin, on_canvas(&harness, 0.4, 0.45))?;
    settle(&mut harness)?;
    let (start, end) = linear(&harness);
    assert!(start.distance(Point::new(0.4, 0.25)) < 0.004, "{start:?}");
    assert!(end.distance(Point::new(0.4, 0.65)) < 0.004, "{end:?}");

    // The handle at its end stretches it.
    harness.drag(
        on_canvas(&harness, 0.4, 0.65),
        on_canvas(&harness, 0.4, 0.8),
    )?;
    settle(&mut harness)?;
    let (start, end) = linear(&harness);
    assert!(start.distance(Point::new(0.4, 0.25)) < 0.004, "{start:?}");
    assert!(end.distance(Point::new(0.4, 0.8)) < 0.004, "{end:?}");

    // The round handle beside the pin turns it: taken straight below the
    // pin, the gradient lies on its side, as long as it was.
    let middle = on_canvas(&harness, 0.4, 0.525);
    harness.drag(middle - vec2(64.0, 0.0), middle + vec2(0.0, 64.0))?;
    settle(&mut harness)?;
    let (start, end) = linear(&harness);
    assert!((start.y - 0.525).abs() < 0.004 && (end.y - 0.525).abs() < 0.004);
    let length = (end.x - start.x) * 1200.0;
    assert!((length - 0.55 * 800.0).abs() < 4.0, "{length}");

    // The list: rename, invert, hide.
    harness.set_text(Role::TextInput, "Mask name", "Sky")?;
    harness.click(Role::CheckBox, "Invert mask")?;
    harness.click(Role::Button, "Enable mask 1")?;
    settle(&mut harness)?;
    let mask = settings(&harness).masks[0].clone();
    assert_eq!(
        (mask.name.as_str(), mask.invert, mask.enabled),
        ("Sky", true, false)
    );
    assert_eq!(harness.count(Role::TreeItem, "Mask 1: Sky"), 1);

    // Delete removes the mask; Escape then drops the whole visit.
    harness.press_key(Key::Delete, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert!(settings(&harness).masks.is_empty());
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().tool(), None);
    assert_eq!(history(&harness).len(), steps, "no step");
    assert!(!session(&harness).is_editing());
    Ok(())
}

#[test]
fn a_brush_stroke_paints_and_an_alt_stroke_erases() -> TestResult {
    let (_dir, _studio, mut harness) = developing()?;
    let steps = history(&harness).len();
    let before = frame_image(&harness);
    harness.press_key(Key::K, Modifiers::NONE)?;
    settle(&mut harness)?;
    harness.set_value(Role::Slider, "Brush Size", 4.0)?;
    harness.set_value(Role::Slider, "Brush Feather", 0.0)?;

    // A stroke across the sky.
    let (left, middle, right) = ((0.2, 0.3), (0.35, 0.3), (0.5, 0.3));
    harness.drag(
        on_canvas(&harness, left.0, left.1),
        on_canvas(&harness, right.0, right.1),
    )?;
    settle(&mut harness)?;
    let masks = settings(&harness).masks;
    assert_eq!(masks.len(), 1);
    assert_eq!(masks[0].name, "Brush 1");
    let MaskShape::Brush { strokes } = &masks[0].components[0].shape else {
        panic!("a brush: {:?}", masks[0].components);
    };
    assert_eq!(strokes.len(), 1);
    assert!(!strokes[0].erase && strokes[0].points.len() >= 5);
    assert!((strokes[0].radius - 0.04).abs() < 1.0e-6);
    assert!(strokes[0].radius >= Spot::MIN_RADIUS);
    let covered = overlay_at(&harness, middle.0, middle.1).ok_or("an overlay")?;
    assert!(covered > 0.95, "painted: {covered}");
    assert_eq!(overlay_at(&harness, 0.35, 0.5), Some(0.0), "not painted");

    harness.set_value(Role::Slider, "Mask Exposure", 1.0)?;
    settle(&mut harness)?;
    let painted = frame_image(&harness);
    let lift = |image: &photo_core::image::Rgba8Image, at: (f32, f32)| {
        luma_at(image, at.0, at.1) - luma_at(&before, at.0, at.1)
    };
    assert!(lift(&painted, middle) > 25.0, "{}", lift(&painted, middle));
    assert!(lift(&painted, right) > 25.0, "the stroke's end is painted");
    assert!(lift(&painted, (0.35, 0.5)).abs() <= 1.0, "off the stroke");

    // With Alt held the same brush rubs the right half out.
    harness.drag_with(
        on_canvas(&harness, middle.0 + 0.06, middle.1),
        on_canvas(&harness, right.0 + 0.06, right.1),
        Modifiers::ALT,
    )?;
    settle(&mut harness)?;
    let MaskShape::Brush { strokes } = &settings(&harness).masks[0].components[0].shape else {
        panic!("a brush");
    };
    assert_eq!(strokes.len(), 2);
    assert!(strokes[1].erase);
    let erased = frame_image(&harness);
    assert!(
        lift(&erased, right).abs() <= 1.0,
        "rubbed out: {}",
        lift(&erased, right)
    );
    assert!(lift(&erased, (0.25, 0.3)) > 25.0, "the rest stays painted");
    assert_eq!(overlay_at(&harness, right.0, right.1), Some(0.0));

    harness.press_key(Key::Enter, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(history(&harness).len(), steps + 1, "one step");
    Ok(())
}

#[test]
fn a_shape_is_added_to_a_mask_or_cut_out_of_it() -> TestResult {
    let (_dir, _studio, mut harness) = developing()?;
    harness.press_key(Key::M, Modifiers::SHIFT)?;
    settle(&mut harness)?;
    harness.drag(
        on_canvas(&harness, 0.3, 0.3),
        on_canvas(&harness, 0.42, 0.44),
    )?;
    settle(&mut harness)?;
    assert!(overlay_at(&harness, 0.3, 0.3).ok_or("an overlay")? > 0.95);

    // Cut a brush stroke out of its middle.
    harness.click(Role::Button, "Subtract")?;
    harness.click(Role::Button, "Brush")?;
    harness.set_value(Role::Slider, "Brush Feather", 0.0)?;
    harness.drag(
        on_canvas(&harness, 0.27, 0.3),
        on_canvas(&harness, 0.33, 0.3),
    )?;
    settle(&mut harness)?;
    let mask = settings(&harness).masks[0].clone();
    assert_eq!(settings(&harness).masks.len(), 1, "the same mask");
    assert_eq!(mask.components.len(), 2);
    assert!(mask.components[1].subtract);
    assert_eq!(overlay_at(&harness, 0.3, 0.3), Some(0.0), "cut out");
    assert_eq!(harness.count(Role::TreeItem, "Shape 2: Brush (cut out)"), 1);

    // And add a second ellipse beside it.
    harness.click(Role::Button, "Add")?;
    harness.click(Role::Button, "Radial Gradient")?;
    harness.drag(
        on_canvas(&harness, 0.7, 0.3),
        on_canvas(&harness, 0.76, 0.36),
    )?;
    settle(&mut harness)?;
    let mask = settings(&harness).masks[0].clone();
    assert_eq!(mask.components.len(), 3);
    assert!(!mask.components[2].subtract);
    assert!(overlay_at(&harness, 0.7, 0.3).ok_or("an overlay")? > 0.95);

    // The selected shape is removed again.
    harness.click(Role::Button, "Remove Shape")?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).masks[0].components.len(), 2);
    Ok(())
}
