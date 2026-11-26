//! The healing tool: a click places a spot with the engine's suggested
//! source, a drag sizes one; spots and their sources are moved by dragging
//! them; a spot over the scene's dark patch heals it and changes nothing
//! else; Escape takes everything back.

use egui::{Key, Modifiers, Role, vec2};
use photo_core::develop::SpotMode;
use photo_core::geom::Point;
use photo_ui::harness::Harness;
use photo_ui::{PhotoApp, Tool};

use crate::{
    Studio, TestResult, develop, frame_image, history, scene_files, session, settings, settle,
};

/// The middle of the scene's dark skin patch, as fractions of the photo.
fn dark_patch() -> Point {
    let patch = photo_testkit::patches(1200, 800)
        .into_iter()
        .find(|patch| patch.name == "dark skin")
        .expect("the scene has a dark skin patch");
    Point::new(
        (patch.x as f32 + patch.width as f32 / 2.0) / 1200.0,
        (patch.y as f32 + patch.height as f32 / 2.0) / 800.0,
    )
}

/// Where a point of the photo is on the canvas.
fn on_canvas(harness: &Harness<PhotoApp>, at: Point) -> egui::Pos2 {
    harness
        .app()
        .canvas_position(at)
        .expect("the photo is on the canvas")
}

#[test]
fn a_click_places_a_spot_with_the_suggested_source_and_escape_takes_it_back() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    let steps = history(&harness).len();
    harness.press_key(Key::Q, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().tool(), Some(Tool::Heal));

    let patch = dark_patch();
    let at = on_canvas(&harness, patch);
    harness.click_at(at)?;
    settle(&mut harness)?;
    let spots = settings(&harness).spots;
    assert_eq!(spots.len(), 1);
    let spot = spots[0];
    assert!(spot.target.distance(patch) < 0.002, "{spot:?}");
    assert_eq!((spot.radius, spot.mode), (0.02, SpotMode::Heal));
    let suggested = harness
        .app()
        .engine()
        .suggest_heal_source(spot.target, spot.radius)?;
    assert_eq!(spot.source, suggested, "the engine's suggestion");
    assert_eq!(harness.app().selected_spot(), Some(0));
    assert_eq!(history(&harness).len(), steps, "nothing recorded yet");

    // The panel edits the selected spot.
    harness.click(Role::Button, "Clone")?;
    harness.set_value(Role::Slider, "Spot Size", 3.0)?;
    harness.set_value(Role::Slider, "Spot Opacity", 80.0)?;
    settle(&mut harness)?;
    let spot = settings(&harness).spots[0];
    assert_eq!(spot.mode, SpotMode::Clone);
    assert!((spot.radius - 0.03).abs() < 1.0e-6, "{spot:?}");
    assert_eq!(spot.opacity, 80.0);

    // Drag the source, then the spot itself.
    let source = on_canvas(&harness, spot.source);
    harness.drag(source, source + vec2(40.0, -30.0))?;
    settle(&mut harness)?;
    let moved = settings(&harness).spots[0];
    assert_eq!(moved.target, spot.target, "the spot stays");
    let now = on_canvas(&harness, moved.source);
    assert!(
        (now - (source + vec2(40.0, -30.0))).length() < 1.0,
        "{now:?}"
    );
    let target = on_canvas(&harness, moved.target);
    harness.drag(target, target + vec2(-25.0, 10.0))?;
    settle(&mut harness)?;
    let again = settings(&harness).spots[0];
    assert_eq!(again.source, moved.source, "the source stays");
    let now = on_canvas(&harness, again.target);
    assert!(
        (now - (target + vec2(-25.0, 10.0))).length() < 1.0,
        "{now:?}"
    );

    // Delete removes the selected spot.
    harness.press_key(Key::Delete, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert!(settings(&harness).spots.is_empty());
    assert!(!harness.app().dialog_open(), "no photo is removed");

    // A spot placed and then dropped with Escape leaves no trace.
    harness.click_at(at)?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).spots.len(), 1);
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().tool(), None);
    assert!(settings(&harness).spots.is_empty());
    assert!(!session(&harness).is_editing());
    assert_eq!(history(&harness).len(), steps, "no step");
    Ok(())
}

#[test]
fn a_spot_stays_on_its_content_when_the_photo_is_turned_and_cropped() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    harness.press_key(Key::Q, Modifiers::NONE)?;
    settle(&mut harness)?;

    // A spot in the middle of the blue patch; a corner of the patch, clear
    // of the spot's circles, says where the patch is on screen.
    let blue = photo_testkit::patches(1200, 800)
        .into_iter()
        .find(|patch| patch.name == "blue")
        .ok_or("the scene has a blue patch")?;
    let middle = Point::new(
        (blue.x as f32 + blue.width as f32 / 2.0) / 1200.0,
        (blue.y as f32 + blue.height as f32 / 2.0) / 800.0,
    );
    let corner = Point::new(
        (blue.x as f32 + 5.0) / 1200.0,
        (blue.y as f32 + 5.0) / 800.0,
    );
    harness.click_at(on_canvas(&harness, middle))?;
    settle(&mut harness)?;
    let spot = settings(&harness).spots[0];
    assert!(spot.target.distance(middle) < 0.002, "{spot:?}");
    let apart = (on_canvas(&harness, corner) - on_canvas(&harness, spot.target)).length();
    let ring = (on_canvas(
        &harness,
        Point::new(spot.target.x + spot.radius, spot.target.y),
    ) - on_canvas(&harness, spot.target))
    .length();
    assert!(apart > ring + 6.0, "the corner is clear of the ring");

    // The mark is drawn where `canvas_position` puts the spot; the picture
    // there is the blue patch, however the photo lies.
    let check = |harness: &mut Harness<PhotoApp>, when: &str| -> TestResult {
        assert_eq!(
            settings(harness).spots,
            [spot],
            "{when}: the spot is as placed"
        );
        let mark = on_canvas(harness, spot.target);
        let back = harness.app().canvas_source(mark).ok_or("a photo")?;
        assert!(back.distance(spot.target) < 0.002, "{when}: {back:?}");
        if harness.can_render() {
            harness.pointer_leave()?;
            let image = harness.render()?;
            let at = on_canvas(harness, corner);
            let pixel = image.get_pixel(at.x.round() as u32, at.y.round() as u32);
            let (red, green, blue) = (
                i32::from(pixel[0]),
                i32::from(pixel[1]),
                i32::from(pixel[2]),
            );
            assert!(
                blue > red + 60 && blue > green + 60,
                "{when}: {pixel:?} at {at:?} is not the blue patch"
            );
        }
        Ok(())
    };
    check(&mut harness, "upright")?;
    let upright = on_canvas(&harness, spot.target);

    // A quarter turn.
    harness.press_key(Key::CloseBracket, Modifiers::COMMAND)?;
    settle(&mut harness)?;
    let turned = on_canvas(&harness, spot.target);
    assert!((turned - upright).length() > 50.0, "the mark moved with it");
    check(&mut harness, "turned")?;

    // A square crop about the middle, which keeps the patch.
    harness.press_key(Key::R, Modifiers::NONE)?;
    settle(&mut harness)?;
    harness.click(Role::ComboBox, "Crop aspect")?;
    harness.click(Role::Button, "1 : 1")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    settle(&mut harness)?;
    let size = session(&harness).output_size().ok_or("decoded")?;
    assert_eq!(size, (800, 800));
    harness.press_key(Key::Q, Modifiers::NONE)?;
    settle(&mut harness)?;
    let cropped = on_canvas(&harness, spot.target);
    assert!((cropped - turned).length() > 10.0, "and with the crop");
    check(&mut harness, "cropped")?;
    Ok(())
}

#[test]
fn a_spot_dragged_over_the_dark_patch_heals_it_and_changes_nothing_else() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    let steps = history(&harness).len();
    let before = frame_image(&harness);
    harness.press_key(Key::Q, Modifiers::NONE)?;
    settle(&mut harness)?;

    // Press in the middle of the patch and drag out past its corners: the
    // drag is the spot's size.
    let patch = dark_patch();
    let reach = Point::new(patch.x + 90.0 / 1200.0, patch.y);
    harness.drag(on_canvas(&harness, patch), on_canvas(&harness, reach))?;
    settle(&mut harness)?;
    let spots = settings(&harness).spots;
    assert_eq!(spots.len(), 1);
    let spot = spots[0];
    assert!(spot.target.distance(patch) < 0.002, "{spot:?}");
    assert!((spot.radius - 90.0 / 1200.0).abs() < 0.002, "{spot:?}");
    let suggested = harness
        .app()
        .engine()
        .suggest_heal_source(spot.target, spot.radius)?;
    assert_eq!(spot.source, suggested, "the engine's suggestion");
    let apart = Point::new(spot.source.x * 1200.0, spot.source.y * 800.0)
        .distance(Point::new(spot.target.x * 1200.0, spot.target.y * 800.0));
    assert!(
        apart >= 2.0 * spot.radius * 1200.0 * 0.99,
        "clear of the spot: {apart}"
    );

    harness.press_key(Key::Enter, Modifiers::NONE)?;
    settle(&mut harness)?;
    let labels = history(&harness);
    assert_eq!(labels.len(), steps + 1, "{labels:?}");
    assert_eq!(labels[labels.len() - 1], "Add Spot Removal");

    // The frame before and after: the patch is healed, the rest untouched.
    let after = frame_image(&harness);
    assert_eq!((after.width, after.height), (before.width, before.height));
    let scale = after.width as f32 / 1200.0;
    let centre = (
        spot.target.x * 1200.0 * scale,
        spot.target.y * 800.0 * scale,
    );
    let radius = spot.radius * 1200.0 * scale;
    let (mut inside, mut changed, mut outside) = (0u32, 0u32, 0u32);
    for y in 0..after.height {
        for x in 0..after.width {
            let distance = (x as f32 + 0.5 - centre.0).hypot(y as f32 + 0.5 - centre.1);
            let (was, is) = (before.pixel(x, y), after.pixel(x, y));
            let difference = (0..3)
                .map(|channel| was[channel].abs_diff(is[channel]))
                .max()
                .unwrap_or(0);
            if distance < radius * 0.4 {
                inside += 1;
                changed += u32::from(difference > 6);
            } else if distance > radius + 2.0 {
                outside += u32::from(difference > 1);
            }
        }
    }
    assert!(inside > 100, "{inside} pixels looked at");
    assert!(
        changed * 2 > inside,
        "the patch is replaced: {changed} of {inside} changed"
    );
    assert_eq!(outside, 0, "pixels outside the spot changed");
    Ok(())
}
