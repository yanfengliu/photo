//! Going into Develop and out again, what the canvas shows while a photo
//! loads, the canvas's zoom and the renders it asks for, the panel columns
//! kept in the catalog, and the keys.

use std::time::{Duration, Instant};

use egui::{Key, Modifiers, Role, vec2};
use photo_engine::{DevelopLoad, PreviewOrigin, PreviewSize};
use photo_testkit::TempLibrary;
use photo_ui::{CanvasImage, CanvasZoom, Module, View};

use crate::{
    PATIENCE, Studio, TestResult, develop, id, scene_files, session, settings, settle, shows,
};

/// The app on the fixture library, as the Previous Import.
fn library_app(
    library: &TempLibrary,
    studio: &Studio,
) -> photo_ui::harness::Harness<photo_ui::PhotoApp> {
    let files = library.entries().iter().map(|e| e.path.clone()).collect();
    studio.app_with(files)
}

#[test]
fn d_develops_the_active_photo_and_g_and_e_go_back_to_the_library() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = library_app(&library, &studio);
    develop(&mut harness, "dawn.jpg")?;
    let dawn = id(&harness, "dawn.jpg");
    assert_eq!(session(&harness).photo(), dawn);
    assert_eq!(session(&harness).load_state(), &DevelopLoad::Rendered);
    let now = harness.app().canvas_shows();
    assert_eq!((now.image, now.photo), (CanvasImage::Rendered, Some(dawn)));
    assert!(now.current, "the frame of the current settings");
    assert_eq!(
        harness.control(Role::Button, "Develop")?.toggled,
        Some(true)
    );
    assert_eq!(harness.count(Role::DisclosureTriangle, "Basic"), 1);

    harness.press_key(Key::G, Modifiers::NONE)?;
    assert_eq!(harness.app().module(), Module::Library);
    assert_eq!(harness.app().view(), View::Grid);
    assert!(
        harness.app().engine().develop().is_none(),
        "the session closed"
    );

    harness.press_key(Key::D, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().module(), Module::Develop);
    harness.press_key(Key::E, Modifiers::NONE)?;
    assert_eq!(
        (harness.app().module(), harness.app().view()),
        (Module::Library, View::Loupe)
    );

    harness.click(Role::Button, "Develop")?;
    assert_eq!(harness.app().module(), Module::Develop);
    harness.click(Role::Button, "Library")?;
    assert_eq!(
        (harness.app().module(), harness.app().view()),
        (Module::Library, View::Loupe),
        "back to the view it left"
    );
    Ok(())
}

#[test]
fn the_filmstrip_and_the_arrow_keys_choose_the_photo_developed() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = library_app(&library, &studio);
    develop(&mut harness, "dawn.jpg")?;
    harness.press_key(Key::ArrowRight, Modifiers::NONE)?;
    settle(&mut harness)?;
    let ridge = id(&harness, "ridge.jpg");
    assert_eq!(session(&harness).photo(), ridge, "the next in the list");
    let lake = harness
        .control(Role::ListBoxOption, "Filmstrip lake.png")?
        .rect
        .center();
    harness.click_at(lake)?;
    settle(&mut harness)?;
    assert_eq!(session(&harness).photo(), id(&harness, "lake.png"));
    assert_eq!(harness.app().engine().previous_photo(), Some(ridge));
    Ok(())
}

#[test]
fn a_raw_photo_shows_its_camera_preview_until_the_render_takes_its_place_at_the_same_zoom()
-> TestResult {
    let dir = tempfile::tempdir()?;
    let files = vec![
        photo_testkit::write_png8(dir.path(), "first.png", 1500, 1000),
        photo_testkit::write_dng(dir.path(), "second.dng", 3000, 2000),
    ];
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "first.png")?;
    harness.click(Role::Button, "1:1")?;
    let inside = harness
        .app()
        .canvas_photo_rect()
        .ok_or("a photo on the canvas")?
        .center();
    harness.drag(inside, inside + vec2(-180.0, 90.0))?;
    settle(&mut harness)?;
    let center = harness.app().develop_center();
    assert_ne!(center, vec2(0.5, 0.5), "panned");

    // Results are let in one frame at a time, to see every stage.
    harness.app_mut().hold_results(true);
    let raw = harness
        .control(Role::ListBoxOption, "Filmstrip second.dng")?
        .rect
        .center();
    harness.click_at(raw)?;
    let mut seen = Vec::new();
    let mut preview_rect = None;
    // Bounded by time, not frames: a busy machine reads the file slowly.
    let started = Instant::now();
    while started.elapsed() < PATIENCE {
        std::thread::sleep(Duration::from_millis(1));
        harness.app_mut().hold_results(false);
        harness.step(1);
        harness.app_mut().hold_results(true);
        let now = harness.app().canvas_shows();
        if seen.last() != Some(&now.image) {
            seen.push(now.image);
        }
        // The load indicator says the camera's preview is shown in exactly
        // the frames in which the raw file is still being read. Whether
        // any frame falls in that stage depends on how fast the file is
        // read: on the Windows runner the preview and the decoded file
        // have arrived in one frame (CI run 45922096867), which the stage
        // history below still records.
        let reading = *session(&harness).load_state() == DevelopLoad::EmbeddedPreview;
        assert_eq!(
            shows(&harness, "Camera preview: the raw file is being read"),
            reading,
            "the load indicator follows the load: {:?}",
            session(&harness).load_state()
        );
        if now.image == CanvasImage::EmbeddedPreview {
            preview_rect = harness.app().canvas_photo_rect();
            assert_eq!(harness.app().develop_zoom(), CanvasZoom::OneToOne);
        }
        if now.image == CanvasImage::Rendered && now.current {
            break;
        }
    }
    harness.app_mut().hold_results(false);
    settle(&mut harness)?;
    let at = seen
        .iter()
        .position(|image| *image == CanvasImage::EmbeddedPreview)
        .ok_or_else(|| format!("the camera preview was never shown: {seen:?}"))?;
    assert_eq!(seen.last(), Some(&CanvasImage::Rendered), "{seen:?}");
    assert!(
        !seen[at..].contains(&CanvasImage::Nothing),
        "nothing blank after the preview: {seen:?}"
    );
    assert_eq!(
        session(&harness).load_history(),
        [
            DevelopLoad::Nothing,
            DevelopLoad::EmbeddedPreview,
            DevelopLoad::Decoded,
            DevelopLoad::Rendered
        ]
    );
    assert_eq!(
        harness.app().develop_zoom(),
        CanvasZoom::OneToOne,
        "the zoom stays"
    );
    assert_eq!(
        harness.app().develop_center(),
        center,
        "and so does the place"
    );
    assert_eq!(
        harness.app().canvas_photo_rect(),
        preview_rect,
        "the rendered frame lands where the camera preview was"
    );
    assert!(!shows(
        &harness,
        "Camera preview: the raw file is being read"
    ));
    Ok(())
}

#[test]
fn the_engine_renders_what_the_canvas_shows_at_the_screens_size() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.png")?;
    let photo = harness.app().canvas_photo_rect().ok_or("a photo")?;
    let fitted = session(&harness).frame().ok_or("a frame")?.view();
    assert!(
        (fitted.width() as f32 - photo.width()).abs() <= 1.0,
        "fitted: {} frame pixels for {} points",
        fitted.width(),
        photo.width()
    );
    assert!(
        fitted.width() < 1200,
        "smaller than the photo: sized to the canvas"
    );

    // Z over the photo: 1:1 around the pointer.
    let pointer = photo.left_top() + vec2(photo.width() * 0.25, photo.height() * 0.75);
    harness.hover_at(pointer)?;
    harness.press_key(Key::Z, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().develop_zoom(), CanvasZoom::OneToOne);
    let view = session(&harness).view().ok_or("a view")?;
    assert_eq!(view.scale(), 1.0);
    let roi = view.roi();
    let zoomed = harness.app().canvas_photo_rect().ok_or("a photo")?;
    assert!(
        (zoomed.width() - 1200.0).abs() < 1.0,
        "one point per pixel at 1x"
    );
    // The photo is wider than the canvas but not taller: across, the point
    // under the pointer stays; down, the photo is centred.
    let under = |rect: egui::Rect| (pointer - rect.min) / rect.size();
    assert!(
        (under(zoomed).x - under(photo).x).abs() < 0.002,
        "the point under the pointer stays: {:?} -> {:?}",
        under(photo),
        under(zoomed)
    );
    assert!((zoomed.center().y - photo.center().y).abs() < 1.0);
    let held = under(zoomed) * vec2(1200.0, 800.0);
    assert!(
        (roi.x0..roi.x1).contains(&held.x) && (roi.y0..roi.y1).contains(&held.y),
        "the render covers it: {roi:?}"
    );
    assert_eq!(
        session(&harness).frame().ok_or("a frame")?.view(),
        view,
        "the frame on screen is the view asked for"
    );

    // A drag pans, and the render follows.
    let middle = photo.center();
    harness.drag(middle, middle + vec2(-120.0, 0.0))?;
    settle(&mut harness)?;
    let panned = session(&harness).view().ok_or("a view")?.roi();
    assert!(panned.x0 > roi.x0 + 100.0, "{roi:?} -> {panned:?}");

    harness.click(Role::Button, "2:1")?;
    settle(&mut harness)?;
    assert_eq!(session(&harness).view().ok_or("a view")?.scale(), 2.0);
    harness.press_key(Key::Space, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(
        harness.app().develop_zoom(),
        CanvasZoom::Fit,
        "back to the fitted zoom"
    );
    assert_eq!(session(&harness).view(), Some(fitted));
    Ok(())
}

#[test]
fn the_panel_columns_open_as_they_were_left_after_a_restart() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    {
        let mut harness = library_app(&library, &studio);
        develop(&mut harness, "dawn.jpg")?;
        assert!(harness.app().right_panels().is_open("Basic"));
        harness.click(Role::DisclosureTriangle, "Basic")?;
        harness.click(Role::DisclosureTriangle, "Detail")?;
        harness.click(Role::DisclosureTriangle, "History")?;
    }
    let mut harness = studio.app();
    develop(&mut harness, "dawn.jpg")?;
    let right = harness.app().right_panels();
    assert!(right.is_open("Detail") && !right.is_open("Basic"));
    assert!(!harness.app().left_panels().is_open("History"));
    assert_eq!(harness.count(Role::Slider, "Sharpening Amount"), 1);
    Ok(())
}

#[test]
fn keys_wait_while_a_number_is_typed_and_the_number_commits_on_enter() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = library_app(&library, &studio);
    develop(&mut harness, "dawn.jpg")?;
    let field = harness.control(Role::TextInput, "Exposure")?.rect.center();
    harness.click_at(field)?;
    harness.press_key(Key::J, Modifiers::NONE)?;
    harness.press_key(Key::W, Modifiers::NONE)?;
    harness.press_key(Key::G, Modifiers::NONE)?;
    assert_eq!(
        harness.app().module(),
        Module::Develop,
        "G typed, not pressed"
    );
    assert!(!harness.app().clipping().highlights, "J typed, not pressed");
    assert!(!harness.app().picking_white_balance());

    harness.press_key(Key::A, Modifiers::COMMAND)?;
    harness.type_text("0.5")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness).tone.exposure, 0.5);
    assert_eq!(
        crate::history(&harness).last().map(String::as_str),
        Some("Exposure +0.50")
    );

    // With the field left, the keys are shortcuts again.
    harness.press_key(Key::J, Modifiers::NONE)?;
    assert!(harness.app().clipping().highlights);
    Ok(())
}

#[test]
fn an_edit_of_a_raw_photo_reaches_its_thumbnail_as_a_render_not_the_camera_preview() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    develop(&mut harness, "scene.dng")?;
    harness.set_value(Role::Slider, "Exposure", 1.0)?;
    settle(&mut harness)?;
    let raw = id(&harness, "scene.dng");
    let version = harness
        .app()
        .engine()
        .summary(raw)
        .map(|summary| summary.develop_version)
        .ok_or("the photo")?;
    assert_eq!(
        harness.app().preview_shown(raw, PreviewSize::Thumbnail),
        Some((version, PreviewOrigin::Rendered)),
        "the filmstrip shows the edit"
    );
    Ok(())
}
