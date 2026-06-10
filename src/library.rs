//! Library path persistence and file-dialog extensions.

use crate::decode::ImageData;
use crate::nav;
use iced::widget::image::Handle as ImageHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct LibraryEntry {
    pub(crate) path: PathBuf,
    pub(crate) filename: String,
    pub(crate) thumbnail_image: Option<Arc<ImageData>>,
    pub(crate) thumbnail_handle: Option<ImageHandle>,
}

pub(crate) fn local_app_storage_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|dir| Path::new(&dir).join("photo"))
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
    content
        .lines()
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect()
}

pub(crate) fn image_file_dialog_extensions() -> &'static [&'static str] {
    nav::image_extensions()
}

pub fn scan_folder_for_images(folder: &Path) -> Vec<PathBuf> {
    nav::scan_images_in_directory(folder)
}
