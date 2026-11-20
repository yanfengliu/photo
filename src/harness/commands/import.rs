//! Getting photos in: `import_folder` through the real Import dialog, and
//! `open`, which starts the app again with a photo as `photo PATH` does.

use std::path::Path;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use serde_json::{Value, json};

use super::{Session, performed};
use crate::harness::driver::{Driver, finish};
use crate::harness::keys::parse_modifiers;
use crate::harness::protocol::{CmdError, Code, Params};
use crate::harness::tree::Control;

/// The Library's button that opens the dialog.
const IMPORT_BUTTON: &str = "Import…";
/// The dialog's folder field.
const SOURCE_FIELD: &str = "Source folder";
/// The dialog's subfolders switch.
const SUBFOLDERS: &str = "Include subfolders";
/// The dialog's way out.
const CANCEL: &str = "Cancel";
/// How long the folder's scan may take unless told otherwise.
const DEFAULT_SCAN_WAIT: Duration = Duration::from_secs(120);
/// How long to block on the engine between looks at the scan.
const SCAN_SLICE: Duration = Duration::from_millis(50);

/// `import_folder {path, recursive}`: open the Import dialog, type the
/// folder's path, wait for the scan to list its photos, and press Import.
/// The answer comes once Import is pressed; `wait_idle` waits for the
/// import itself.
pub fn import_folder(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["path", "recursive", "timeout_ms"])?;
    let folder = session.photo_path(params.string("path")?);
    let recursive = params.opt_bool("recursive")?.unwrap_or(true);
    let timeout = params
        .opt_u64("timeout_ms")?
        .map_or(DEFAULT_SCAN_WAIT, Duration::from_millis);
    if !folder.is_dir() {
        return Err(CmdError::not_found(format!(
            "there is no folder at {}",
            folder.display()
        )));
    }
    let typed = folder.display().to_string();
    let driver = &mut *session.driver;
    let mut warnings = open_dialog(driver)?;

    let switch = driver
        .harness_ref()
        .control(Role::CheckBox, SUBFOLDERS)
        .map_err(|error| {
            CmdError::internal(format!(
                "the Import dialog has no subfolders switch: {error}"
            ))
        })?;
    if switch.toggled != Some(recursive) {
        warnings.extend(finish(driver.harness().click(Role::CheckBox, SUBFOLDERS))?);
    }

    // Typed as a user types it: into the field, over what it held, then
    // Enter, which looks in the folder.
    let select_all = parse_modifiers(&["cmd".to_owned()])?;
    warnings.extend(finish(
        driver.harness().click(Role::TextInput, SOURCE_FIELD),
    )?);
    warnings.extend(finish(driver.harness().press_key(Key::A, select_all))?);
    warnings.extend(finish(driver.harness().type_text(&typed))?);
    warnings.extend(finish(
        driver.harness().press_key(Key::Enter, Modifiers::NONE),
    )?);
    let field = driver
        .harness_ref()
        .control(Role::TextInput, SOURCE_FIELD)
        .map_err(|error| CmdError::internal(error.to_string()))?;
    if field.text.as_deref() != Some(typed.as_str()) {
        return Err(CmdError::internal(format!(
            "after typing {typed:?} the Source folder field holds {:?}",
            field.text
        )));
    }

    wait_for_scan(driver, &folder, recursive, timeout)?;
    let (found, in_catalog, duplicates) = driver
        .engine()
        .import_scan()
        .map(|scan| {
            let count = |keep: fn(&photo_engine::ImportCandidate) -> bool| {
                scan.candidates
                    .iter()
                    .filter(|candidate| keep(candidate))
                    .count()
            };
            (
                scan.candidates.len(),
                count(|candidate| candidate.already_in_catalog),
                count(|candidate| candidate.suspected_duplicate),
            )
        })
        .unwrap_or_default();
    warnings.extend(driver.settle());

    let controls = driver.controls()?;
    let (button, ticked) = confirm_button(&controls)
        .ok_or_else(|| CmdError::internal("the Import dialog shows no `Import N Photos` button"))?;
    let name = button.name.clone().unwrap_or_default();
    let mut data = json!({
        "folder": folder,
        "recursive": recursive,
        "found": found,
        "already_in_catalog": in_catalog,
        "suspected_duplicates": duplicates,
        "ticked": ticked,
    });
    if ticked == 0 || !button.enabled {
        warnings.extend(finish(driver.harness().click(Role::Button, CANCEL))?);
        data["import_started"] = json!(false);
        data["reason"] = json!(format!(
            "nothing to import: {found} found, {in_catalog} already in the catalog, {duplicates} suspected duplicates; the dialog was cancelled"
        ));
        return Ok(performed(data, warnings));
    }
    warnings.extend(finish(driver.harness().click(Role::Button, &name))?);
    if driver.app().dialog_open() {
        return Err(CmdError::internal(format!(
            "the dialog stayed open after {name:?} was pressed"
        )));
    }
    data["pressed"] = json!(name);
    data["import_started"] = json!(true);
    Ok(performed(data, warnings))
}

/// Open the Import dialog as the user does: its button under the left
/// panels, or ⇧⌘I when those are hidden. An Import dialog already open is
/// used as it is.
fn open_dialog(driver: &mut Driver) -> Result<Vec<String>, CmdError> {
    if driver.app().dialog_open() {
        if driver.harness_ref().count(Role::TextInput, SOURCE_FIELD) == 1 {
            return Ok(Vec::new());
        }
        return Err(CmdError::unavailable(
            "another dialog is open; close it first (key escape)",
        ));
    }
    let result = if driver.harness_ref().count(Role::Button, IMPORT_BUTTON) == 1 {
        driver.harness().click(Role::Button, IMPORT_BUTTON)
    } else {
        let shortcut = parse_modifiers(&["cmd".to_owned(), "shift".to_owned()])?;
        driver.harness().press_key(Key::I, shortcut)
    };
    let warnings = finish(result)?;
    if !driver.app().dialog_open() || driver.harness_ref().count(Role::TextInput, SOURCE_FIELD) != 1
    {
        return Err(CmdError::internal("the Import dialog did not open"));
    }
    Ok(warnings)
}

/// Run frames until the scan of `folder` has listed every file.
fn wait_for_scan(
    driver: &mut Driver,
    folder: &Path,
    recursive: bool,
    timeout: Duration,
) -> Result<(), CmdError> {
    let deadline = Instant::now() + timeout;
    loop {
        let done = driver.engine().import_scan().is_some_and(|scan| {
            scan.root == folder && scan.recursive == recursive && scan.finished
        });
        if done {
            return Ok(());
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(CmdError::new(
                Code::Timeout,
                format!(
                    "the scan of {} had not finished after {} ms",
                    folder.display(),
                    timeout.as_millis()
                ),
            ));
        }
        driver
            .harness()
            .app_mut()
            .app
            .wait_idle(left.min(SCAN_SLICE));
        driver.harness().step(1);
    }
}

/// The dialog's confirm button, `Import 6 Photos`, and the count it gives.
fn confirm_button(controls: &[Control]) -> Option<(&Control, usize)> {
    controls
        .iter()
        .filter(|control| control.role == Role::Button)
        .find_map(|control| {
            let rest = control.name.as_deref()?.strip_prefix("Import ")?;
            let count = rest
                .strip_suffix(" Photos")
                .or_else(|| rest.strip_suffix(" Photo"))?;
            count.parse().ok().map(|count| (control, count))
        })
}

/// `open {path}`: start the app again on the same storage with the photo
/// open, exactly as `photo PATH` does: imported in place if the catalog
/// does not hold it, made active, shown in the loupe. The presentation
/// state and the undo stack start afresh, as after a restart.
pub fn open(params: &Params<'_>, session: &mut Session<'_>) -> Result<Value, CmdError> {
    params.only(&["path"])?;
    let path = session.photo_path(params.string("path")?);
    let opened = session.driver.relaunch(&path)?;
    Ok(json!({
        "opened": {
            "id": opened.id,
            "file_name": opened.file_name,
            "path": opened.path,
            "imported": opened.imported,
        },
        "relaunched": true,
        "view": "loupe",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(name: &str) -> Control {
        Control {
            role: Role::Button,
            name: Some(name.to_owned()),
            value: None,
            numeric: None,
            min: None,
            max: None,
            step: None,
            enabled: true,
            rect: None,
            toggled: None,
            selected: None,
            expanded: None,
            focused: false,
            interactive: true,
            description: None,
        }
    }

    #[test]
    fn the_confirm_button_is_found_by_its_count_and_not_a_candidate() {
        let mut candidate = button("Import 6 dawn.jpg");
        candidate.role = Role::ListBoxOption;
        let controls = [button("Import…"), candidate, button("Import 1 Photo")];
        let (found, count) = confirm_button(&controls).unwrap();
        assert_eq!((found.name.as_deref(), count), (Some("Import 1 Photo"), 1));
        assert_eq!(confirm_button(&[button("Import 12 Photos")]).unwrap().1, 12);
        assert!(confirm_button(&[button("Import…")]).is_none());
    }
}
