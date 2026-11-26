//! The harness itself: input reaches the app, lookups fail politely, and a
//! machine without a GPU is an error value, not a crash.

use std::sync::Arc;

use egui::{Key, Modifiers, PointerButton, Role};
use photo_ui::harness::{HarnessBuilder, HarnessError, RenderError};
use photo_ui::widgets::Toasts;

use crate::TestResult;

/// What the probe app saw.
#[derive(Default)]
struct Seen {
    keys: Vec<(Key, Modifiers)>,
    clicks: Vec<(PointerButton, Modifiers)>,
    text: String,
    presses: u32,
    double_presses: u32,
    frames: u64,
}

fn probe(ui: &mut egui::Ui, seen: &mut Seen) {
    seen.frames += 1;
    ui.input(|input| {
        for event in &input.events {
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => seen.keys.push((*key, *modifiers)),
                egui::Event::PointerButton {
                    button,
                    pressed: false,
                    ..
                } => seen.clicks.push((*button, input.modifiers)),
                _ => {}
            }
        }
    });
    let button = ui.button("Press me");
    seen.presses += u32::from(button.clicked());
    seen.double_presses += u32::from(button.double_clicked());
    ui.add(egui::TextEdit::singleline(&mut seen.text))
        .accessible_name("Notes");
}

#[test]
fn key_presses_reach_the_app_with_their_modifiers() -> TestResult {
    let mut harness = HarnessBuilder::new().build_ui(Seen::default(), probe);
    harness.run()?;

    harness.press_key(Key::G, Modifiers::NONE)?;
    harness.press_key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)?;

    let keys = &harness.app().state.keys;
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert_eq!(keys[0], (Key::G, Modifiers::NONE));
    assert_eq!(keys[1].0, Key::Z);
    assert!(keys[1].1.command && keys[1].1.shift && !keys[1].1.alt);
    Ok(())
}

#[test]
fn pointer_clicks_carry_their_button_and_modifiers() -> TestResult {
    let mut harness = HarnessBuilder::new().build_ui(Seen::default(), probe);
    harness.run()?;
    let spot = egui::pos2(400.0, 300.0);

    harness.click_at(spot)?;
    harness.secondary_click_at(spot)?;
    harness.click_at_with(spot, PointerButton::Primary, Modifiers::SHIFT)?;

    let clicks = &harness.app().state.clicks;
    assert_eq!(clicks.len(), 3, "{clicks:?}");
    assert_eq!(clicks[0], (PointerButton::Primary, Modifiers::NONE));
    assert_eq!(clicks[1], (PointerButton::Secondary, Modifiers::NONE));
    assert_eq!(clicks[2].0, PointerButton::Primary);
    assert!(clicks[2].1.shift);
    Ok(())
}

#[test]
fn controls_are_found_and_operated_by_role_and_name() -> TestResult {
    let mut harness = HarnessBuilder::new().build_ui(Seen::default(), probe);
    harness.run()?;

    harness.click(Role::Button, "Press me")?;
    harness.invoke(Role::Button, "Press me")?;
    assert_eq!(harness.app().state.presses, 2);

    harness.click(Role::TextInput, "Notes")?;
    harness.type_text("dusk")?;
    assert_eq!(harness.app().state.text, "dusk");
    harness.set_text(Role::TextInput, "Notes", "dawn")?;
    assert_eq!(harness.app().state.text, "dawn");
    assert_eq!(
        harness.control(Role::TextInput, "Notes")?.text.as_deref(),
        Some("dawn")
    );
    Ok(())
}

/// What the listing test shows: a text, a number and a switch.
struct Listed {
    text: String,
    amount: f32,
    on: bool,
}

#[test]
fn every_control_is_listed_with_its_role_name_value_state_and_place() -> TestResult {
    use egui::accesskit::Toggled;
    use photo_core::develop::Adj;
    use photo_ui::widgets::AdjSlider;

    let state = Listed {
        text: "dusk".to_owned(),
        amount: 0.25,
        on: true,
    };
    let mut harness = HarnessBuilder::new().build_ui(state, |ui, state| {
        let _ = ui.button("Press me");
        ui.add_enabled(false, egui::Button::new("Not now"));
        ui.add(egui::TextEdit::singleline(&mut state.text))
            .accessible_name("Notes");
        AdjSlider::new(Adj::Exposure.spec(), &mut state.amount, 0.0).show(ui);
        ui.checkbox(&mut state.on, "Sharpen");
        ui.label("Plain words");
    });
    harness.run()?;

    let controls = harness.controls();
    let find = |role: Role, name: &str| {
        controls
            .iter()
            .position(|control| control.role == role && control.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no {role:?} named {name:?} is listed"))
    };
    let button = &controls[find(Role::Button, "Press me")];
    assert!(button.enabled && button.interactive && !button.is_layout());
    assert_eq!(
        button.rect,
        Some(harness.control(Role::Button, "Press me")?.rect),
        "in points, as the lookups give it"
    );
    assert!(!controls[find(Role::Button, "Not now")].enabled);
    let notes = &controls[find(Role::TextInput, "Notes")];
    assert_eq!(notes.value.as_deref(), Some("dusk"));
    let slider = &controls[find(Role::Slider, "Exposure")];
    assert_eq!(
        (slider.numeric, slider.min, slider.max),
        (Some(0.25), Some(-5.0), Some(5.0))
    );
    assert_eq!(
        controls[find(Role::CheckBox, "Sharpen")].toggled,
        Some(Toggled::True)
    );
    let words = &controls[find(Role::Label, "Plain words")];
    assert!(
        !words.interactive && words.value.is_none(),
        "a label's text is its name"
    );
    assert!(words.rect.is_some());

    // In the order they are on screen, and the window itself left out.
    assert!(find(Role::Button, "Press me") < find(Role::Button, "Not now"));
    assert!(find(Role::Button, "Not now") < find(Role::TextInput, "Notes"));
    assert!(controls.iter().all(|control| control.role != Role::Window));

    // The listing follows the screen.
    harness.click(Role::CheckBox, "Sharpen")?;
    let controls = harness.controls();
    let switch = controls
        .iter()
        .find(|control| control.name.as_deref() == Some("Sharpen"))
        .ok_or("the switch is listed")?;
    assert_eq!(switch.toggled, Some(Toggled::False));
    Ok(())
}

#[test]
fn a_drag_holds_its_modifiers_from_the_press_to_the_release() -> TestResult {
    #[derive(Default)]
    struct Held {
        during: Vec<bool>,
        after: bool,
    }
    let mut harness = HarnessBuilder::new().build_ui(Held::default(), |ui, held| {
        let (down, alt) = ui.input(|input| (input.pointer.primary_down(), input.modifiers.alt));
        if down {
            held.during.push(alt);
        }
        held.after = alt;
        ui.label("Drag here");
    });
    harness.run()?;
    let from = egui::pos2(300.0, 300.0);
    harness.drag_with(from, from + egui::vec2(90.0, 20.0), Modifiers::ALT)?;
    let held = &harness.app().state;
    assert!(held.during.len() >= 5 && held.during.iter().all(|alt| *alt));
    assert!(!held.after, "let go with the button");
    Ok(())
}

#[test]
fn a_missing_control_is_an_error_not_a_panic() -> TestResult {
    let mut harness = HarnessBuilder::new().build_ui(Seen::default(), probe);
    harness.run()?;

    assert_eq!(harness.count(Role::Button, "Nope"), 0);
    let error = harness.click(Role::Button, "Nope").unwrap_err();
    assert_eq!(
        error,
        HarnessError::NotFound {
            role: Role::Button,
            name: "Nope".to_owned()
        }
    );
    Ok(())
}

#[test]
fn two_controls_with_one_name_are_reported_as_ambiguous() -> TestResult {
    let mut harness = HarnessBuilder::new().build_ui((), |ui, ()| {
        let _ = ui.button("Twin");
        let _ = ui.button("Twin");
    });
    harness.run()?;

    assert_eq!(harness.count(Role::Button, "Twin"), 2);
    let error = harness.click(Role::Button, "Twin").unwrap_err();
    assert!(
        matches!(error, HarnessError::Ambiguous { count: 2, .. }),
        "{error}"
    );
    Ok(())
}

#[test]
fn an_unnamed_control_is_reported_when_the_ui_settles() {
    let mut harness = HarnessBuilder::new().build_ui(String::new(), |ui, text| {
        ui.text_edit_singleline(text);
    });
    let error = harness.run().unwrap_err();
    assert!(
        matches!(&error, HarnessError::UnnamedControls(list) if list.len() == 1),
        "{error}"
    );
}

#[test]
fn a_ui_that_never_rests_is_reported() {
    let mut harness = HarnessBuilder::new()
        .max_steps(8)
        .build_ui((), |ui, ()| ui.request_repaint());
    let error = harness.run().unwrap_err();
    assert!(matches!(error, HarnessError::NotSettled(_)), "{error}");
}

#[test]
fn run_until_waits_for_the_apps_own_condition() -> TestResult {
    let mut harness = HarnessBuilder::new().build_ui(Seen::default(), probe);
    let start = harness.app().state.frames;
    let frames = harness.run_until(50, |app| app.state.frames >= start + 5)?;
    assert_eq!(frames, 5);
    assert_eq!(
        harness.run_until(3, |app| app.state.frames >= start + 8)?,
        3
    );

    let start = harness.app().state.frames;
    assert!(harness.run_until(3, |_| false).is_err());
    assert_eq!(
        harness.app().state.frames,
        start + 3,
        "no frame past the limit"
    );
    Ok(())
}

#[test]
fn clicks_on_one_spot_stay_separate_clicks() -> TestResult {
    let mut harness = HarnessBuilder::new().build_ui(Seen::default(), probe);
    harness.run()?;

    // In real time these would be a double-click; a script means two clicks.
    harness.click(Role::Button, "Press me")?;
    harness.click(Role::Button, "Press me")?;
    assert_eq!(harness.app().state.presses, 2);
    assert_eq!(harness.app().state.double_presses, 0);

    // And a double-click right after is a double-click, not a triple.
    harness.double_click(Role::Button, "Press me")?;
    assert_eq!(harness.app().state.presses, 4);
    assert_eq!(harness.app().state.double_presses, 1);
    Ok(())
}

#[test]
fn a_drag_between_two_clicks_does_not_merge_them() -> TestResult {
    let mut harness = HarnessBuilder::new().build_ui(Seen::default(), probe);
    harness.run()?;
    let button = harness.control(Role::Button, "Press me")?.rect.center();
    let elsewhere = egui::pos2(600.0, 400.0);

    harness.click_at(button)?;
    // The release that ends a drag is no click, and must not make the
    // harness forget where the click before it was.
    harness.pointer_down(elsewhere)?;
    harness.pointer_move(elsewhere + egui::vec2(60.0, 0.0))?;
    harness.pointer_up(elsewhere + egui::vec2(60.0, 0.0))?;
    harness.click_at(button)?;
    assert_eq!(harness.app().state.presses, 2);
    assert_eq!(harness.app().state.double_presses, 0);
    Ok(())
}

#[test]
fn a_container_without_a_rectangle_is_an_error_not_a_panic() -> TestResult {
    let mut toasts = Toasts::new();
    toasts.push("Saved");
    let mut harness = HarnessBuilder::new().build_ui(toasts, |ui, toasts| {
        toasts.show(ui.ctx(), ui.max_rect());
    });
    harness.run()?;

    // The toast area is in the tree, but it is not a control with a place.
    assert_eq!(harness.count(Role::Alert, "Notifications"), 1);
    let nowhere = HarnessError::NoBounds {
        role: Role::Alert,
        name: "Notifications".to_owned(),
    };
    assert_eq!(
        harness.control(Role::Alert, "Notifications"),
        Err(nowhere.clone())
    );
    assert_eq!(harness.click(Role::Alert, "Notifications"), Err(nowhere));
    Ok(())
}

#[test]
fn a_drag_lets_go_of_the_button_even_when_a_step_reports_an_error() {
    let mut harness = HarnessBuilder::new().build_ui(String::new(), |ui, text| {
        ui.text_edit_singleline(text);
    });
    let from = egui::pos2(200.0, 200.0);

    let error = harness
        .drag(from, from + egui::vec2(80.0, 0.0))
        .unwrap_err();
    assert!(matches!(error, HarnessError::UnnamedControls(_)), "{error}");
    assert!(!harness.ctx().input(|input| input.pointer.any_down()));
}

#[test]
fn the_check_for_names_can_be_turned_off() -> TestResult {
    let mut harness =
        HarnessBuilder::new()
            .check_names(false)
            .build_ui(String::new(), |ui, text| {
                ui.text_edit_singleline(text);
            });
    harness.run()?;
    harness.click_at(egui::pos2(200.0, 200.0))?;
    Ok(())
}

#[test]
fn nonsense_options_are_held_to_what_egui_can_take() -> TestResult {
    let mut harness = HarnessBuilder::new()
        .size([f32::NAN, -5.0])
        .pixels_per_point(0.0)
        .step_dt(-1.0)
        .build_ui(Seen::default(), probe);
    harness.run()?;
    harness.click_at(egui::pos2(10.0, 0.5))?;
    harness.advance(f32::NAN);

    assert_eq!(harness.ctx().pixels_per_point(), 0.25);
    assert_eq!(harness.ctx().content_rect().size(), egui::vec2(1280.0, 1.0));
    // A quarter of a pixel high: there is no such frame to render.
    assert_cannot_render(harness.render(), harness.can_render());

    // A harness that may not run a single frame still does not panic.
    let mut harness = HarnessBuilder::new()
        .max_steps(0)
        .build_ui(Seen::default(), probe);
    let _ = harness.run();

    // The longest frame is short enough for a press and release to be a
    // click, and however long it is asked to wait, it comes back.
    let mut harness = HarnessBuilder::new()
        .step_dt(f32::INFINITY)
        .build_ui(Seen::default(), probe);
    harness.run()?;
    harness.click(Role::Button, "Press me")?;
    harness.double_click(Role::Button, "Press me")?;
    assert_eq!(harness.app().state.presses, 3);
    assert_eq!(harness.app().state.double_presses, 1);
    let start = harness.app().state.frames;
    harness.advance(f32::INFINITY);
    assert_eq!(
        harness.app().state.frames,
        start + 1200,
        "a minute at twenty frames a second"
    );
    Ok(())
}

#[test]
fn a_frame_too_large_to_render_is_an_error_not_a_panic() -> TestResult {
    let mut harness = HarnessBuilder::new()
        .size([16_384.0, 16_384.0])
        .pixels_per_point(8.0)
        .build_ui(Seen::default(), probe);
    harness.run()?;
    assert_cannot_render(harness.render(), harness.can_render());
    Ok(())
}

/// A frame that cannot be made is refused with [`RenderError::Failed`]; on a
/// machine without a GPU every frame is refused as [`RenderError::NoAdapter`].
fn assert_cannot_render(result: Result<image::RgbaImage, RenderError>, gpu: bool) {
    match result {
        Err(RenderError::Failed(reason)) => assert!(reason.contains("pixels"), "{reason}"),
        Err(RenderError::NoAdapter(reason)) => assert!(!gpu, "{reason}"),
        Err(other) => panic!("refused for the wrong reason: {other}"),
        Ok(image) => panic!("rendered {} x {}", image.width(), image.height()),
    }
}

#[test]
fn rendering_gives_an_image_of_the_requested_pixel_size() -> TestResult {
    let mut harness = HarnessBuilder::new()
        .size([320.0, 200.0])
        .pixels_per_point(2.0)
        .build_ui(Seen::default(), probe);
    harness.run()?;
    match harness.render() {
        Ok(image) => assert_eq!((image.width(), image.height()), (640, 400)),
        Err(RenderError::NoAdapter(reason)) => eprintln!("skipped, no GPU adapter: {reason}"),
        Err(other) => return Err(other.into()),
    }

    // Controls are found and clicked in points at any pixel density.
    harness.click(Role::Button, "Press me")?;
    assert_eq!(harness.app().state.presses, 1);
    Ok(())
}

#[test]
fn a_missing_gpu_adapter_is_reported_instead_of_panicking() -> TestResult {
    // Stand in for a machine with no GPU: an adapter selector that finds none.
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.native_adapter_selector = Some(Arc::new(|_adapters, _surface| {
        Err("no adapters on this machine".to_owned())
    }));
    let mut harness = HarnessBuilder::new()
        .wgpu_setup(setup.into())
        .build_ui(Seen::default(), probe);

    // Everything except rendering still works.
    harness.run()?;
    harness.click(Role::Button, "Press me")?;
    assert_eq!(harness.app().state.presses, 1);

    assert!(!harness.can_render());
    let error = harness.render().unwrap_err();
    assert!(
        matches!(&error, RenderError::NoAdapter(reason) if reason.contains("no adapters")),
        "{error}"
    );
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("frame.png");
    assert!(matches!(
        harness.save_png(&path),
        Err(RenderError::NoAdapter(_))
    ));
    assert!(!path.exists());
    Ok(())
}
