//! Auto Tone, Auto white balance and the picker; the before view in place,
//! side by side and split; the clipping overlays.

use egui::{Key, Modifiers, PointerButton, Role, pos2};
use photo_core::develop::WbPreset;
use photo_engine::BeforeSettings;
use photo_ui::{BeforeAfter, Lights};

use crate::{
    Studio, TestResult, develop, history, mean_luma, mean_rgb, scene_files, session, settings,
    settle,
};

/// The middle of the scene's grey ramp, as fractions of the photo.
fn ramp_middle() -> (f32, f32) {
    let (x, y, width, height) = photo_testkit::grey_ramp(1200, 800);
    (
        (x + width / 2) as f32 / 1200.0,
        (y + height / 2) as f32 / 800.0,
    )
}

/// The red minus blue, and green minus blue, of the ramp's middle in the
/// frame on screen.
fn ramp_cast(harness: &photo_ui::harness::Harness<photo_ui::PhotoApp>) -> (f64, f64) {
    let frame = session(harness).frame().expect("a frame");
    let image = frame.image();
    let (u, v) = ramp_middle();
    let x = (u * image.width as f32) as u32;
    let y = (v * image.height as f32) as u32;
    let [red, green, blue] = mean_rgb(image, x - 2, y - 1, 5, 3);
    (red - blue, green - blue)
}

#[test]
fn auto_tone_and_auto_white_balance_set_the_photo_from_itself() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    harness.set_value(Role::Slider, "Exposure", -2.0)?;
    settle(&mut harness)?;
    let steps = history(&harness).len();
    harness.click(Role::Button, "Auto Tone")?;
    settle(&mut harness)?;
    let tone = settings(&harness).tone;
    assert!(tone.exposure > -1.5, "brought up: {tone:?}");
    assert_eq!(history(&harness).len(), steps + 1, "one step");

    harness.set_value(Role::Slider, "Temp", 60.0)?;
    settle(&mut harness)?;
    harness.click(Role::ComboBox, "White balance")?;
    harness.click(Role::Button, "Auto")?;
    settle(&mut harness)?;
    let balance = settings(&harness).white_balance;
    assert_eq!(balance.preset, WbPreset::Auto);
    assert_ne!(balance.temp, 60.0, "estimated from the photo: {balance:?}");

    // The button beside the menu does the same.
    harness.set_value(Role::Slider, "Temp", -70.0)?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).white_balance.preset, WbPreset::Custom);
    harness.click(Role::Button, "Auto White Balance")?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).white_balance, balance);
    Ok(())
}

#[test]
fn the_picker_makes_a_grey_patch_neutral() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    harness.set_value(Role::Slider, "Temp", -50.0)?;
    harness.set_value(Role::Slider, "Tint", 30.0)?;
    settle(&mut harness)?;
    let (cool, _) = ramp_cast(&harness);
    assert!(cool < -15.0, "blue before: {cool}");

    harness.press_key(Key::W, Modifiers::NONE)?;
    assert!(harness.app().picking_white_balance());
    assert_eq!(
        harness
            .control(Role::Button, "White balance picker")?
            .toggled,
        Some(true)
    );
    let photo = harness.app().canvas_photo_rect().ok_or("a photo")?;
    let (u, v) = ramp_middle();
    let at = pos2(
        photo.left() + u * photo.width(),
        photo.top() + v * photo.height(),
    );
    harness.click_at(at)?;
    settle(&mut harness)?;
    assert!(!harness.app().picking_white_balance(), "one pick and done");
    assert_eq!(settings(&harness).white_balance.preset, WbPreset::Custom);
    let (red, green) = ramp_cast(&harness);
    assert!(
        red.abs() < 3.0 && green.abs() < 3.0,
        "neutral: {red} {green}"
    );

    // Escape puts the picker away without a pick.
    harness.press_key(Key::W, Modifiers::NONE)?;
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    assert!(!harness.app().picking_white_balance());
    Ok(())
}

#[test]
fn the_before_view_shows_the_photo_as_imported_in_place_beside_the_edit_or_split() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    harness.set_value(Role::Slider, "Exposure", 1.0)?;
    settle(&mut harness)?;

    harness.press_key(Key::Backslash, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().before_after(), BeforeAfter::Before);
    let held = session(&harness);
    assert_eq!(held.before_settings(), Some(BeforeSettings::Default));
    let before = held.before_frame().ok_or("a before frame")?;
    let after = held.frame().ok_or("a frame")?;
    assert_eq!(before.view(), after.view(), "laid out alike");
    assert!(
        mean_luma(before.image()) + 20.0 < mean_luma(after.image()),
        "the before view is the photo as imported, darker than the edit"
    );
    assert_eq!(harness.control(Role::Button, "Before")?.toggled, Some(true));

    harness.press_key(Key::Y, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().before_after(), BeforeAfter::SideBySide);
    let held = session(&harness);
    let half = held.frame().ok_or("a frame")?.view();
    assert_eq!(held.before_frame().ok_or("a before frame")?.view(), half);
    assert!(
        half.width() < 600,
        "each side gets half the canvas: {}",
        half.width()
    );

    harness.press_key(Key::Y, Modifiers::SHIFT)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().before_after(), BeforeAfter::Split);
    assert!(session(&harness).before_frame().is_some());
    harness.press_key(Key::Y, Modifiers::SHIFT)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().before_after(), BeforeAfter::Off);
    assert_eq!(session(&harness).before_settings(), None);

    // A history step as the before view.
    let first = history(&harness)[0].clone();
    let row = harness
        .control(Role::TreeItem, &format!("History 1: {first}"))?
        .rect
        .center();
    harness.click_at_with(row, PointerButton::Secondary, Modifiers::NONE)?;
    harness.click(Role::Button, "Show as Before")?;
    settle(&mut harness)?;
    assert_eq!(harness.app().before_after(), BeforeAfter::SideBySide);
    assert!(matches!(
        session(&harness).before_settings(),
        Some(BeforeSettings::HistoryStep(_))
    ));
    Ok(())
}

#[test]
fn j_and_the_histogram_toggles_show_where_the_photo_clips() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    harness.set_value(Role::Slider, "Exposure", 3.0)?;
    settle(&mut harness)?;

    harness.press_key(Key::J, Modifiers::NONE)?;
    let clipping = harness.app().clipping();
    assert!(clipping.shadows && clipping.highlights);
    for toggle in ["Show shadow clipping", "Show highlight clipping"] {
        assert_eq!(harness.control(Role::Button, toggle)?.toggled, Some(true));
    }
    assert_eq!(
        harness.control(Role::Button, "Clipping")?.toggled,
        Some(true)
    );
    settle(&mut harness)?;
    if harness.can_render() {
        harness.pointer_leave()?;
        let image = harness.render()?;
        let photo = harness.app().canvas_photo_rect().ok_or("a photo")?;
        // The sun, white in the file and white still.
        let sun = photo.left_top() + egui::vec2(photo.width() * 0.76, photo.height() * 0.2);
        let pixel = image.get_pixel(sun.x as u32, sun.y as u32);
        let red = photo_ui::theme::palette::CLIP_HIGHLIGHTS;
        let marked = [pixel[0], pixel[1], pixel[2]]
            .iter()
            .zip([red.r(), red.g(), red.b()])
            .all(|(shown, wanted)| shown.abs_diff(wanted) <= 2);
        assert!(marked, "the clipped sun is marked: {pixel:?}");
    }

    harness.click(Role::Button, "Show shadow clipping")?;
    let clipping = harness.app().clipping();
    assert!(!clipping.shadows && clipping.highlights);
    harness.press_key(Key::J, Modifiers::NONE)?;
    let clipping = harness.app().clipping();
    assert!(
        !clipping.shadows && !clipping.highlights,
        "J turns any overlay off"
    );
    Ok(())
}

#[test]
fn l_darkens_everything_but_the_photo_and_a_click_on_the_dark_lights_it_again() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    let panel = harness.control(Role::Slider, "Exposure")?.rect.center();
    harness.press_key(Key::L, Modifiers::NONE)?;
    assert_eq!(harness.app().lights(), Lights::Dim);
    harness.press_key(Key::L, Modifiers::NONE)?;
    assert_eq!(harness.app().lights(), Lights::Out);
    if harness.can_render() {
        harness.pointer_leave()?;
        let image = harness.render()?;
        let photo = harness.app().canvas_photo_rect().ok_or("a photo")?;
        let dark = image.get_pixel(panel.x as u32, panel.y as u32);
        assert_eq!(
            [dark[0], dark[1], dark[2]],
            [0, 0, 0],
            "the panels are black"
        );
        let middle = image.get_pixel(photo.center().x as u32, photo.center().y as u32);
        assert!(
            middle[0] > 0 || middle[1] > 0 || middle[2] > 0,
            "the photo is not"
        );
    }
    // The dark takes the click: the slider under it does not move.
    harness.click_at(panel)?;
    assert_eq!(harness.app().lights(), Lights::On);
    assert_eq!(settings(&harness).tone.exposure, 0.0);
    Ok(())
}
