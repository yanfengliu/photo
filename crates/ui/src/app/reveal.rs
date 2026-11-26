//! Showing a folder to the user in the system's file manager.

use std::path::Path;
use std::process::Command;

/// Open `folder` in the system's file manager. Nothing waits for it to
/// show; a failure is logged.
pub(crate) fn in_file_manager(folder: &Path) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    match Command::new(program).arg(folder).spawn() {
        Ok(mut child) => {
            // Reaped off the UI thread, so no finished process is left
            // waiting for the app to end.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => log::warn!("cannot show {}: {error}", folder.display()),
    }
}
