//! Quick Develop in the Library: a white-balance preset, Auto Tone and the
//! step buttons acting on the selection, and Reset All.

use egui::{Key, Modifiers, Role};
use photo_core::develop::WbPreset;
use photo_core::meta::FileKind;
use photo_ui::Module;

use crate::{Studio, TestResult, id, scene_files, settle};

#[test]
fn quick_develop_steps_the_selected_photos_and_resets_them() -> TestResult {
    let (_dir, files) = scene_files();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    settle(&mut harness)?;
    harness.press_key(Key::A, Modifiers::COMMAND)?;
    harness.click(Role::DisclosureTriangle, "Quick Develop")?;
    assert_eq!(harness.app().module(), Module::Library);
    let (png, dng) = (id(&harness, "scene.png"), id(&harness, "scene.dng"));
    let both = |harness: &photo_ui::harness::Harness<photo_ui::PhotoApp>| {
        let engine = harness.app().engine();
        [png, dng].map(|photo| engine.develop_settings(photo).expect("settings"))
    };

    harness.click(Role::Button, "Exposure +1/3")?;
    harness.click(Role::Button, "Exposure +1")?;
    harness.click(Role::Button, "Contrast -20")?;
    harness.click(Role::Button, "Vibrance +5")?;
    settle(&mut harness)?;
    for settings in both(&harness) {
        assert!((settings.tone.exposure - 4.0 / 3.0).abs() < 1.0e-4);
        assert_eq!(settings.tone.contrast, -20.0);
        assert_eq!(settings.presence.vibrance, 5.0);
    }

    harness.click(Role::ComboBox, "Quick white balance")?;
    harness.click(Role::Button, "Tungsten")?;
    settle(&mut harness)?;
    let [raster, raw] = both(&harness);
    assert_eq!(raw.white_balance.preset, WbPreset::Tungsten);
    assert_eq!(
        raster.white_balance.preset,
        WbPreset::AsShot,
        "an illuminant means nothing to a rendered file"
    );

    harness.click(Role::Button, "Auto Tone")?;
    settle(&mut harness)?;
    for settings in both(&harness) {
        assert!(
            (settings.tone.exposure - 4.0 / 3.0).abs() > 0.05,
            "set from the photo"
        );
    }

    harness.click(Role::Button, "Reset All")?;
    settle(&mut harness)?;
    let [raster, raw] = both(&harness);
    assert!(raster.is_default_for(FileKind::Raster) && raw.is_default_for(FileKind::Raw));
    harness.press_key(Key::Z, Modifiers::COMMAND)?;
    settle(&mut harness)?;
    assert_eq!(
        both(&harness)[1].white_balance.preset,
        WbPreset::Tungsten,
        "undone"
    );
    Ok(())
}
