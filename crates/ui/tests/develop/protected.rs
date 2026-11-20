//! A photo that cannot be edited: Develop shows the engine's reason, every
//! editing control is disabled, and nothing reaches the catalog.

use egui::{Key, Modifiers, Role};

use crate::{Studio, TestResult, develop, import, protect, scene_files, settings, settle, shows};

#[test]
fn a_write_protected_catalog_says_why_and_every_editing_control_is_off() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    {
        let mut engine = studio.options.open_engine()?;
        import(&mut engine, files);
    }
    if !protect(&studio.options.catalog_path) {
        eprintln!("skipped: this user can write a read-only file");
        return Ok(());
    }
    let bytes = std::fs::read(&studio.options.catalog_path)?;
    let mut harness = studio.app();
    develop(&mut harness, "scene.dng")?;
    let reason = crate::session(&harness)
        .read_only_reason()
        .ok_or("a reason")?
        .to_owned();
    assert!(reason.contains("write-protected"), "{reason}");
    assert!(shows(
        &harness,
        "The catalog file is write-protected, so nothing in it can be changed."
    ));
    assert!(shows(&harness, "Read only"));

    for slider in ["Exposure", "Temp", "Shadows", "Texture"] {
        assert!(harness.control(Role::Slider, slider)?.disabled, "{slider}");
    }
    for button in [
        "Auto Tone",
        "Reset",
        "Previous",
        "Paste",
        "White balance picker",
    ] {
        assert!(harness.control(Role::Button, button)?.disabled, "{button}");
    }
    assert!(
        harness
            .control(Role::TreeItem, "Preset Vivid Colour")?
            .disabled
    );
    assert!(
        !harness
            .control(Role::DisclosureTriangle, "Tone Curve")?
            .disabled,
        "the panels can still be opened and looked at"
    );

    let before = settings(&harness);
    let track = harness.control(Role::Slider, "Exposure")?.rect;
    harness.drag(track.center(), track.right_center())?;
    harness.press_key(Key::W, Modifiers::NONE)?;
    assert!(!harness.app().picking_white_balance());
    harness.press_key(Key::R, Modifiers::COMMAND | Modifiers::SHIFT)?;
    settle(&mut harness)?;
    assert_eq!(settings(&harness), before, "nothing changed");
    assert!(
        harness
            .app()
            .toasts()
            .any(|toast| toast.contains("write-protected")),
        "a refusal says why"
    );
    drop(harness);
    assert_eq!(
        std::fs::read(&studio.options.catalog_path)?,
        bytes,
        "the file is as it was"
    );
    Ok(())
}
