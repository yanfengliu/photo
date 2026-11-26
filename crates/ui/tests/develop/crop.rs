//! The crop tool: a rectangle with a fixed aspect and an angle is one step
//! and survives undo, redo and a quarter turn; a line drawn along a slope
//! levels it; Escape drops what the tool changed.

use egui::{Key, Modifiers, Role, vec2};
use photo_core::develop::{Crop, CropConstraint};
use photo_core::geom::Point;
use photo_ui::harness::Harness;
use photo_ui::{PhotoApp, Tool};

use crate::{Studio, TestResult, develop, history, scene_files, session, settings, settle};

/// The open photo's size as cropped and turned.
fn output(harness: &Harness<PhotoApp>) -> (u32, u32) {
    session(harness)
        .output_size()
        .expect("the photo is decoded")
}

/// The source position shown at the middle of the output.
fn shown_centre(harness: &Harness<PhotoApp>) -> Point {
    let geometry = session(harness).geometry().expect("the photo is decoded");
    let (width, height) = geometry.output_size();
    geometry.out_to_src(Point::new(width as f32 / 2.0, height as f32 / 2.0))
}

/// What a test of the crop tool starts with: the folder of the scene's
/// files, the catalog, and the app with the PNG open and the tool up.
type Cropping = (tempfile::TempDir, Studio, Harness<PhotoApp>);

fn cropping() -> Result<Cropping, Box<dyn std::error::Error>> {
    let (dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    assert_eq!(output(&harness), (1200, 800));
    harness.press_key(Key::R, Modifiers::NONE)?;
    assert_eq!(harness.app().tool(), Some(Tool::Crop));
    settle(&mut harness)?;
    Ok((dir, studio, harness))
}

#[test]
fn a_crop_with_a_fixed_aspect_and_an_angle_is_one_step_and_survives_undo_redo_and_a_turn()
-> TestResult {
    let (_dir, _studio, mut harness) = cropping()?;
    let steps = history(&harness).len();

    // While cropping the canvas shows the whole photo, the rectangle on it.
    let whole = harness.app().crop_rect().ok_or("the rectangle is drawn")?;
    let photo = harness.app().canvas_photo_rect().ok_or("a photo")?;
    assert!(
        (whole.width() - photo.width()).abs() < 1.0,
        "{whole:?} {photo:?}"
    );

    harness.click(Role::ComboBox, "Crop aspect")?;
    harness.click(Role::Button, "16 : 9")?;
    harness.set_value(Role::Slider, "Crop Angle", 5.0)?;
    settle(&mut harness)?;
    let rect = harness.app().crop_rect().ok_or("the rectangle is drawn")?;
    assert!((rect.aspect_ratio() - 16.0 / 9.0).abs() < 0.01, "{rect:?}");
    assert!(
        rect.width() < whole.width(),
        "shrunk to fit the turned photo"
    );

    // Drag the bottom right corner inwards.
    let corner = rect.right_bottom();
    harness.drag(corner, corner - vec2(120.0, 60.0))?;
    settle(&mut harness)?;
    let dragged = harness.app().crop_rect().ok_or("the rectangle is drawn")?;
    assert!(
        (dragged.aspect_ratio() - 16.0 / 9.0).abs() < 0.01,
        "{dragged:?}"
    );
    assert!(
        (dragged.left_top() - rect.left_top()).length() < 1.0,
        "the other corner stays"
    );
    assert!(
        dragged.width() < rect.width() - 50.0,
        "{dragged:?} {rect:?}"
    );
    assert_eq!(history(&harness).len(), steps, "nothing is recorded yet");
    assert!(session(&harness).is_editing());

    harness.press_key(Key::Enter, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().tool(), None);
    assert_eq!(
        history(&harness).len(),
        steps + 1,
        "one step: {:?}",
        history(&harness)
    );
    let crop = settings(&harness).crop;
    assert_eq!(crop.angle, 5.0);
    assert_eq!(
        crop.constraint,
        CropConstraint::Ratio {
            width: 16.0,
            height: 9.0
        }
    );
    let cropped = output(&harness);
    let shape = cropped.0 as f32 / cropped.1 as f32;
    assert!((shape - 16.0 / 9.0).abs() < 0.01, "{cropped:?}");
    assert!(cropped.0 < 1100 && cropped.0 > 800, "{cropped:?}");
    // The canvas shows the cropped photo: its box has the crop's shape.
    let shown = harness.app().canvas_photo_rect().ok_or("a photo")?;
    assert!((shown.aspect_ratio() - shape).abs() < 0.01, "{shown:?}");
    assert!(harness.app().canvas_shows().current);
    let centre = shown_centre(&harness);

    harness.press_key(Key::Z, Modifiers::COMMAND)?;
    settle(&mut harness)?;
    assert_eq!(output(&harness), (1200, 800), "undone");
    assert_eq!(settings(&harness).crop, Crop::default());
    harness.press_key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)?;
    settle(&mut harness)?;
    assert_eq!(output(&harness), cropped, "redone");
    assert_eq!(settings(&harness).crop, crop);

    // A quarter turn: the same part of the photo, on its side.
    harness.press_key(Key::CloseBracket, Modifiers::COMMAND)?;
    settle(&mut harness)?;
    assert_eq!(output(&harness), (cropped.1, cropped.0));
    let turned = shown_centre(&harness);
    assert!(turned.distance(centre) < 1.5, "{centre:?} -> {turned:?}");
    assert_eq!(history(&harness).len(), steps + 2);
    Ok(())
}

#[test]
fn the_panels_turn_and_flip_buttons_carry_the_crop_with_the_photo() -> TestResult {
    use photo_core::develop::Orientation;

    let (_dir, _studio, mut harness) = cropping()?;
    let steps = history(&harness).len();
    // A crop off the centre: the bottom right corner pulled in.
    let rect = harness.app().crop_rect().ok_or("the rectangle is drawn")?;
    let corner = rect.right_bottom();
    harness.drag(corner, corner - vec2(240.0, 120.0))?;
    settle(&mut harness)?;
    let size = output(&harness);
    let centre = shown_centre(&harness);
    assert!(size.0 < 1000 && size.1 < 700, "{size:?}");
    assert!(
        centre.distance(Point::new(600.0, 400.0)) > 60.0,
        "{centre:?}"
    );
    let upright = settings(&harness).orientation;

    // Each button is an icon with a name, and each leaves the same part of
    // the photo in the rectangle.
    let sideways = (size.1, size.0);
    for (button, expected) in [
        ("Rotate right", sideways),
        ("Rotate left", size),
        ("Flip horizontal", size),
        ("Flip vertical", size),
    ] {
        harness.click(Role::Button, button)?;
        settle(&mut harness)?;
        assert_eq!(output(&harness), expected, "after {button}");
        let now = shown_centre(&harness);
        assert!(
            now.distance(centre) < 1.5,
            "{button}: {centre:?} -> {now:?}"
        );
        assert_eq!(harness.app().tool(), Some(Tool::Crop), "the tool stays up");
    }
    // Mirrored both ways is the photo turned half round.
    assert_eq!(upright, Orientation::IDENTITY);
    assert_eq!(settings(&harness).orientation, Orientation::new(2, false));
    let row = [
        "Rotate left",
        "Rotate right",
        "Flip horizontal",
        "Flip vertical",
    ]
    .map(|name| {
        harness
            .control(Role::Button, name)
            .map(|button| button.rect)
    });
    let [left, right, across, down] = row;
    let (left, right, across, down) = (left?, right?, across?, down?);
    assert_eq!(left.size(), down.size(), "four buttons of one kind");
    assert!(left.right() <= right.left() && right.right() <= across.left());
    assert!(across.right() <= down.left() && down.center().y == left.center().y);

    assert_eq!(history(&harness).len(), steps, "nothing is recorded yet");
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(history(&harness).len(), steps + 1, "one step for all of it");
    assert_eq!(output(&harness), size);
    Ok(())
}

#[test]
fn an_export_while_cropping_writes_what_is_shown_and_records_the_crop() -> TestResult {
    let (_dir, _studio, mut harness) = cropping()?;
    let steps = history(&harness).len();
    let rect = harness.app().crop_rect().ok_or("the rectangle is drawn")?;
    let corner = rect.right_bottom();
    harness.drag(corner, corner - vec2(240.0, 120.0))?;
    settle(&mut harness)?;
    let shown = output(&harness);
    assert!(shown.0 < 1000 && shown.1 < 700, "{shown:?}");
    assert_eq!(history(&harness).len(), steps, "not recorded yet");

    let out = tempfile::tempdir()?;
    harness.press_key(Key::E, Modifiers::COMMAND | Modifiers::SHIFT)?;
    harness.set_text(
        Role::TextInput,
        "Export folder",
        &out.path().display().to_string(),
    )?;
    harness.click(Role::Button, "Export 1 Photo")?;
    settle(&mut harness)?;
    let written = image::open(out.path().join("scene.jpg"))?;
    assert_eq!((written.width(), written.height()), shown, "as shown");

    // The export took the crop as a step of its own; the tool stays up
    // with nothing of its own left to record or to drop.
    assert_eq!(harness.app().tool(), Some(Tool::Crop));
    assert_eq!(
        history(&harness).len(),
        steps + 1,
        "{:?}",
        history(&harness)
    );
    harness.press_key(Key::Z, Modifiers::COMMAND)?;
    settle(&mut harness)?;
    assert_eq!(
        output(&harness),
        (1200, 800),
        "one undo takes the crop back"
    );
    Ok(())
}

#[test]
fn a_virtual_copy_made_while_cropping_is_cropped_as_shown() -> TestResult {
    let (_dir, _studio, mut harness) = cropping()?;
    let steps = history(&harness).len();
    let rect = harness.app().crop_rect().ok_or("the rectangle is drawn")?;
    let corner = rect.right_bottom();
    harness.drag(corner, corner - vec2(240.0, 120.0))?;
    settle(&mut harness)?;
    let shown = output(&harness);
    let crop = settings(&harness).crop;
    let master = session(&harness).photo();
    assert!(shown.0 < 1000 && shown.1 < 700, "{shown:?}");

    harness.press_key(Key::Quote, Modifiers::COMMAND)?;
    settle(&mut harness)?;
    let copy = session(&harness).photo();
    assert_ne!(copy, master, "the copy is the photo developed now");
    assert_eq!(
        settings(&harness).crop,
        crop,
        "it has the crop that was shown"
    );
    assert_eq!(output(&harness), shown);
    // And the master kept it, as a step of its own.
    let engine = harness.app().engine();
    let recorded = engine.history(master)?;
    assert_eq!(recorded.len(), steps + 1, "{recorded:?}");
    Ok(())
}

#[test]
fn the_lock_holds_a_shape_custom_numbers_set_one_and_reset_gives_the_photo_back() -> TestResult {
    let (_dir, _studio, mut harness) = cropping()?;
    let drawn =
        |harness: &Harness<PhotoApp>| harness.app().crop_rect().ok_or("the rectangle is drawn");

    // A square, held: an edge dragged in takes the other side with it.
    harness.click(Role::ComboBox, "Crop aspect")?;
    harness.click(Role::Button, "1 : 1")?;
    settle(&mut harness)?;
    let square = drawn(&harness)?;
    assert!((square.aspect_ratio() - 1.0).abs() < 0.01, "{square:?}");
    let lock = harness.control(Role::Button, "Lock aspect")?;
    assert_eq!(lock.toggled, Some(true), "a chosen shape is held");
    let edge = square.right_center();
    harness.drag(edge, edge - vec2(100.0, 0.0))?;
    settle(&mut harness)?;
    let held = drawn(&harness)?;
    assert!((held.aspect_ratio() - 1.0).abs() < 0.01, "{held:?}");
    assert!(held.width() < square.width() - 50.0, "{held:?}");

    // Let go of, the same drag changes the shape.
    harness.click(Role::Button, "Lock aspect")?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).crop.constraint, CropConstraint::Free);
    let edge = held.right_center();
    harness.drag(edge, edge - vec2(80.0, 0.0))?;
    settle(&mut harness)?;
    let freed = drawn(&harness)?;
    assert!((freed.height() - held.height()).abs() < 1.0, "{freed:?}");
    assert!(freed.width() < held.width() - 40.0, "{freed:?}");

    // Custom: its two numbers are the shape.
    harness.click(Role::ComboBox, "Crop aspect")?;
    harness.click(Role::Button, "Custom")?;
    harness.set_value(Role::SpinButton, "Custom aspect width", 3.0)?;
    harness.set_value(Role::SpinButton, "Custom aspect height", 1.0)?;
    settle(&mut harness)?;
    let wide = drawn(&harness)?;
    assert!((wide.aspect_ratio() - 3.0).abs() < 0.03, "{wide:?}");
    assert_eq!(
        settings(&harness).crop.constraint,
        CropConstraint::Ratio {
            width: 3.0,
            height: 1.0
        }
    );
    let cropped = output(&harness);
    assert!(
        (cropped.0 as f32 / cropped.1 as f32 - 3.0).abs() < 0.02,
        "{cropped:?}"
    );

    // Reset: the whole photo again, not turned, the shape still held.
    harness.set_value(Role::Slider, "Crop Angle", 3.0)?;
    settle(&mut harness)?;
    harness.click(Role::Button, "Reset Crop")?;
    settle(&mut harness)?;
    let crop = settings(&harness).crop;
    assert_eq!((crop.rect, crop.angle), (None, 0.0));
    assert_eq!(output(&harness), (1200, 800));
    assert!(harness.control(Role::Button, "Reset Crop")?.disabled);
    Ok(())
}

#[test]
fn a_line_drawn_along_a_slope_levels_it() -> TestResult {
    let (_dir, _studio, mut harness) = cropping()?;
    harness.click(Role::Button, "Straighten")?;
    let photo = harness.app().canvas_photo_rect().ok_or("a photo")?;
    // A line that falls 60 points over 400: 8.53 degrees.
    let from = photo.center() + vec2(-200.0, -30.0);
    let to = photo.center() + vec2(200.0, 30.0);
    let app = harness.app();
    let ends = [from, to].map(|at| app.canvas_source(at).expect("the photo is mapped"));
    harness.drag(from, to)?;
    settle(&mut harness)?;
    let angle = settings(&harness).crop.angle;
    assert!((angle - 8.53).abs() < 0.05, "{angle}");
    let slider = harness.control(Role::Slider, "Crop Angle")?;
    assert_eq!(slider.value, Some(f64::from(angle)));

    harness.press_key(Key::Enter, Modifiers::NONE)?;
    settle(&mut harness)?;
    // Both ends of the line are at one height in the photo as it is now.
    let geometry = session(&harness).geometry().ok_or("decoded")?;
    let (width, height) = geometry.source_size();
    let level: Vec<Point> = ends
        .iter()
        .map(|end| {
            let source = Point::new(end.x * width as f32, end.y * height as f32);
            geometry.src_to_out(source).expect("on the photo")
        })
        .collect();
    assert!((level[0].y - level[1].y).abs() < 0.75, "{level:?}");
    assert!(level[1].x > level[0].x + 300.0, "{level:?}");
    let (cropped, _) = output(&harness);
    assert!(cropped < 1200, "cut to what the turn left: {cropped}");
    Ok(())
}

#[test]
fn escape_drops_what_the_crop_tool_changed_and_records_nothing() -> TestResult {
    let (_dir, _studio, mut harness) = cropping()?;
    let steps = history(&harness).len();
    harness.set_value(Role::Slider, "Crop Angle", -7.0)?;
    settle(&mut harness)?;
    let rect = harness.app().crop_rect().ok_or("the rectangle is drawn")?;
    harness.drag(rect.center(), rect.center() + vec2(-40.0, 10.0))?;
    settle(&mut harness)?;
    assert!(session(&harness).is_editing());
    assert_eq!(settings(&harness).crop.angle, -7.0);

    harness.press_key(Key::Escape, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().tool(), None);
    assert!(!session(&harness).is_editing());
    assert_eq!(settings(&harness).crop, Crop::default());
    assert_eq!(history(&harness).len(), steps, "no step");
    assert_eq!(output(&harness), (1200, 800));
    Ok(())
}

#[test]
fn another_tool_or_a_panels_slider_records_the_crop_as_a_step_of_its_own() -> TestResult {
    let (_dir, _studio, mut harness) = cropping()?;
    let steps = history(&harness).len();
    harness.set_value(Role::Slider, "Crop Angle", 3.0)?;
    settle(&mut harness)?;
    // Q: the healing tool opens, and the crop is kept.
    harness.press_key(Key::Q, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().tool(), Some(Tool::Heal));
    assert_eq!(history(&harness).len(), steps + 1);
    assert_eq!(settings(&harness).crop.angle, 3.0);
    assert!(!session(&harness).is_editing());

    // Back in the crop tool, a slider of the Basic panel is moved.
    harness.press_key(Key::R, Modifiers::NONE)?;
    harness.set_value(Role::Slider, "Crop Angle", 6.0)?;
    settle(&mut harness)?;
    harness.set_value(Role::Slider, "Exposure", 0.5)?;
    settle(&mut harness)?;
    let labels = history(&harness);
    assert_eq!(labels.len(), steps + 3, "{labels:?}");
    assert_eq!(labels[labels.len() - 1], "Exposure +0.50");
    assert_eq!(
        harness.app().tool(),
        Some(Tool::Crop),
        "the tool stays open"
    );
    assert_eq!(settings(&harness).crop.angle, 6.0);

    // The swap key lays the rectangle the other way.
    harness.click(Role::ComboBox, "Crop aspect")?;
    harness.click(Role::Button, "4 : 5")?;
    settle(&mut harness)?;
    let wide = harness.app().crop_rect().ok_or("the rectangle is drawn")?;
    assert!((wide.aspect_ratio() - 5.0 / 4.0).abs() < 0.01, "{wide:?}");
    harness.press_key(Key::X, Modifiers::NONE)?;
    settle(&mut harness)?;
    let tall = harness.app().crop_rect().ok_or("the rectangle is drawn")?;
    assert!((tall.aspect_ratio() - 4.0 / 5.0).abs() < 0.01, "{tall:?}");
    assert_eq!(
        settings(&harness).crop.constraint,
        CropConstraint::Ratio {
            width: 4.0,
            height: 5.0
        }
    );
    Ok(())
}
