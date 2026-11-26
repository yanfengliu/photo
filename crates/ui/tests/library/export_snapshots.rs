//! Frames of the Export dialog and of an export running and done, rendered
//! on the GPU at 1600 x 1000 points, at 1x and 2x, to
//! `target/ui-snapshots/export-*.png`, for a person to look at. On a machine
//! with no GPU nothing is rendered and the tests only drive the states.

use egui::{Key, Modifiers, Role};
use photo_ui::PhotoApp;
use photo_ui::harness::Harness;

use crate::snapshots::{save, shoot};
use crate::{Studio, TestResult, click_cell, settle};

/// The resting frame as `export-{name}.png`: nothing hovered, no toast.
fn rest(harness: &mut Harness<PhotoApp>, name: &str, scale: u32) -> TestResult {
    settle(harness)?;
    harness.pointer_leave()?;
    harness.advance(3.0);
    harness.run()?;
    save(harness, &format!("export-{name}"), scale)
}

/// Every state, at `scale` pixels per point.
fn every_state(scale: u32) -> TestResult {
    let (shoot_dir, files) = shoot()?;
    let studio = Studio::new();
    let mut harness = studio.app_with_at(files, scale as f32);
    settle(&mut harness)?;
    click_cell(&mut harness, "DSC_4100.jpg", Modifiers::NONE)?;
    click_cell(&mut harness, "DSC_4103.jpg", Modifiers::COMMAND)?;

    // The dialog as it opens from the Library's Export… button.
    harness.click(Role::Button, "Export…")?;
    rest(&mut harness, "dialog", scale)?;

    // Every section in use.
    let out = tempfile::tempdir()?;
    let target = out.path().join("Portfolio");
    harness.set_text(
        Role::TextInput,
        "Export folder",
        &target.display().to_string(),
    )?;
    harness.click(Role::CheckBox, "Put in a subfolder")?;
    harness.set_text(Role::TextInput, "Export subfolder", "Web")?;
    harness.set_text(
        Role::TextInput,
        "File name template",
        "{date}-{text}-{seq:3}",
    )?;
    harness.set_text(Role::TextInput, "Custom text", "coast")?;
    harness.click(Role::ComboBox, "Export format")?;
    harness.click(Role::Button, "TIFF")?;
    harness.click(Role::RadioButton, "16 bits")?;
    harness.click(Role::ComboBox, "Resize")?;
    harness.click(Role::Button, "Fit within")?;
    harness.click(Role::ComboBox, "Output sharpening")?;
    harness.click(Role::Button, "Standard")?;
    harness.click(Role::ComboBox, "Metadata")?;
    harness.click(Role::Button, "Copyright and contact only")?;
    harness.click(Role::CheckBox, "Add a watermark")?;
    harness.set_text(Role::TextInput, "Watermark text", "A. Photographer")?;
    rest(&mut harness, "dialog-filled", scale)?;

    // A menu open: what can happen to a name that is taken.
    harness.click(Role::ComboBox, "Existing files")?;
    save(&mut harness, "export-dialog-existing", scale)?;
    harness.press_key(Key::Escape, Modifiers::NONE)?;

    // Naming a preset to save.
    harness.click(Role::Button, "Save Preset…")?;
    harness.set_text(Role::TextInput, "Export preset name", "Portfolio TIFF")?;
    rest(&mut harness, "dialog-preset", scale)?;
    harness.click(Role::Button, "Save")?;
    settle(&mut harness)?;

    // A folder too deep for one line: the example goes on to the next.
    let mut deep = out.path().to_path_buf();
    for level in [
        "Clients",
        "2024 Coast and Hills",
        "Selects for the web",
        "Round 2",
    ] {
        deep.push(level);
    }
    harness.set_text(
        Role::TextInput,
        "Export folder",
        &deep.display().to_string(),
    )?;
    rest(&mut harness, "dialog-long-folder", scale)?;
    harness.set_text(
        Role::TextInput,
        "Export folder",
        &target.display().to_string(),
    )?;

    // Beside the originals: no folder, nothing replaced.
    harness.click(Role::RadioButton, "Beside each original")?;
    harness.click(Role::ComboBox, "Export format")?;
    harness.click(Role::Button, "JPEG")?;
    harness.set_text(Role::TextInput, "File name template", "{name}")?;
    rest(&mut harness, "dialog-beside", scale)?;
    harness.click(Role::RadioButton, "A folder")?;

    // Running: progress and Cancel in the top bar.
    harness.app_mut().hold_results(true);
    harness.click(Role::Button, "Export 2 Photos")?;
    save(&mut harness, "export-running", scale)?;
    harness.app_mut().hold_results(false);
    settle(&mut harness)?;
    // Done: the toast, and the way to the folder.
    save(&mut harness, "export-done", scale)?;
    rest(&mut harness, "done-notice", scale)?;

    // From Develop, on the photo being developed.
    click_cell(&mut harness, "IMG_2211.jpg", Modifiers::NONE)?;
    harness.press_key(Key::D, Modifiers::NONE)?;
    settle(&mut harness)?;
    harness.click(Role::Button, "Export…")?;
    rest(&mut harness, "dialog-develop", scale)?;
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    drop(shoot_dir);
    Ok(())
}

#[test]
fn every_export_state_renders_at_1x() -> TestResult {
    every_state(1)
}

#[test]
fn every_export_state_renders_at_2x() -> TestResult {
    every_state(2)
}
