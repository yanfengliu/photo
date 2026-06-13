//! Library path persistence and file-dialog extensions.

use crate::decode::ImageData;
use crate::loading::BaseImageSource;
use crate::nav;
use iced::widget::image::Handle as ImageHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct LibraryEntry {
    pub(crate) path: PathBuf,
    pub(crate) filename: String,
    pub(crate) thumbnail_image: Option<Arc<ImageData>>,
    pub(crate) thumbnail_handle: Option<ImageHandle>,
    pub(crate) thumbnail_base_source: BaseImageSource,
}

// Test-only override for the per-user app storage directory. A thread-local
// (rather than the global mutex used in `repo.rs`) is the right tool here:
// library/collection storage is only ever read or written synchronously on the
// calling thread, never from a spawned task, so per-test-thread state gives
// race-free isolation with no serialization. Defaults to `None` so the suite
// never touches the real `%LOCALAPPDATA%/photo` location unless a test opts in
// via `with_test_app_storage_dir`.
#[cfg(test)]
thread_local! {
    static TEST_APP_STORAGE_DIR: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Per-user app storage directory (`%LOCALAPPDATA%/photo` in production), home of
/// `library.txt` and `collections.json`. In test builds this resolves to the
/// thread-local override (default `None`) so the suite cannot clobber real user
/// data — a save against `None` is a no-op, and a load returns empty.
pub(crate) fn local_app_storage_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        TEST_APP_STORAGE_DIR.with(|dir| dir.borrow().clone())
    }
    #[cfg(not(test))]
    {
        std::env::var_os("LOCALAPPDATA").map(|dir| Path::new(&dir).join("photo"))
    }
}

/// Runs `f` with the app storage directory pointed at `dir`, restoring the
/// default (no override) afterwards even if `f` panics. Persistence tests use
/// this to exercise real save/load round-trips against an isolated temp dir.
#[cfg(test)]
pub(crate) fn with_test_app_storage_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_APP_STORAGE_DIR.with(|d| *d.borrow_mut() = None);
        }
    }
    TEST_APP_STORAGE_DIR.with(|d| *d.borrow_mut() = Some(dir.to_path_buf()));
    let _reset = Reset;
    f()
}

pub(crate) fn library_file_path() -> Option<PathBuf> {
    local_app_storage_dir().map(|dir| dir.join("library.txt"))
}

pub(crate) fn save_library(library: &[LibraryEntry]) {
    let Some(path) = library_file_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content: String = library
        .iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(&path, content);
}

pub(crate) fn load_library() -> Vec<PathBuf> {
    let Some(path) = library_file_path() else {
        return Vec::new();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    parse_library_content(&content)
}

/// Library membership is user intent. Entries whose media is currently offline
/// (unplugged card, remapped drive) must survive startup, or the next library
/// save would permanently drop them.
pub(crate) fn parse_library_content(content: &str) -> Vec<PathBuf> {
    content
        .lines()
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub(crate) fn image_file_dialog_extensions() -> &'static [&'static str] {
    nav::image_extensions()
}

pub fn scan_folder_for_images(folder: &Path) -> Vec<PathBuf> {
    nav::scan_images_in_directory(folder)
}
