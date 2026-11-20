//! A ready-made folder of mixed photographs for import tests.

use std::path::{Path, PathBuf};

use photo_core::meta::FileKind;
use photo_core::time::DateTime;
use tempfile::TempDir;

use crate::dng::{DngSpec, write_dng_with};
use crate::exif::ExifSpec;
use crate::writers::{fixture_path, write_bytes, write_jpeg_with_exif, write_png8, write_tiff16};

/// One photograph in a [`TempLibrary`], with what a correct reader should
/// find out about it.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    /// Absolute path of the file, inside the library's temporary directory.
    pub path: PathBuf,
    /// Whether the file is a raster image or a camera raw file.
    pub kind: FileKind,
    /// Upright width, after the file's own orientation is applied.
    pub width: u32,
    /// Upright height, after the file's own orientation is applied.
    pub height: u32,
    /// Capture time recorded in the file, `None` when it carries no EXIF.
    pub capture_time: Option<DateTime>,
}

/// A temporary directory holding a small photo library: six photographs of
/// mixed formats in two dated folders, plus one file that is not an image.
///
/// ```text
/// 2024-03-15/dawn.jpg       JPEG with EXIF, landscape
/// 2024-03-15/harbour.dng    Bayer DNG with EXIF and an embedded preview
/// 2024-03-15/ridge.jpg      JPEG with EXIF, stored sideways (orientation 6)
/// 2024-07-04/fireworks.jpg  JPEG with EXIF
/// 2024-07-04/lake.png       8-bit PNG, no metadata
/// 2024-07-04/meadow.tif     16-bit TIFF, no metadata
/// 2024-07-04/notes.txt      not an image
/// ```
///
/// The directory and everything in it is deleted when the value is dropped.
pub struct TempLibrary {
    dir: TempDir,
    entries: Vec<LibraryEntry>,
    stray_file: PathBuf,
}

impl TempLibrary {
    /// Creates the library in a fresh temporary directory.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("create a temporary library directory");
        let spring = dated_folder(dir.path(), "2024-03-15");
        let summer = dated_folder(dir.path(), "2024-07-04");
        let at = |month, day, hour, minute, second| {
            DateTime::new(2024, month, day, hour, minute, second).expect("valid fixture date")
        };

        let dawn_time = at(3, 15, 7, 42, 10);
        let ridge_time = at(3, 15, 8, 5, 33);
        let harbour_time = at(3, 15, 9, 20, 0);
        let fireworks_time = at(7, 4, 21, 30, 45);
        let sideways = ExifSpec {
            orientation: 6,
            ..ExifSpec::sample_at(ridge_time)
        };
        let raw = DngSpec {
            exif: ExifSpec::sample_at(harbour_time),
            ..DngSpec::default()
        };

        let entry = |path, kind, width, height, capture_time| LibraryEntry {
            path,
            kind,
            width,
            height,
            capture_time,
        };
        let mut entries = vec![
            entry(
                write_jpeg_with_exif(&spring, "dawn.jpg", 96, 64, &ExifSpec::sample_at(dawn_time)),
                FileKind::Raster,
                96,
                64,
                Some(dawn_time),
            ),
            entry(
                write_jpeg_with_exif(&spring, "ridge.jpg", 64, 96, &sideways),
                FileKind::Raster,
                64,
                96,
                Some(ridge_time),
            ),
            entry(
                write_dng_with(&spring, "harbour.dng", 96, 64, &raw),
                FileKind::Raw,
                96,
                64,
                Some(harbour_time),
            ),
            entry(
                write_jpeg_with_exif(
                    &summer,
                    "fireworks.jpg",
                    80,
                    60,
                    &ExifSpec::sample_at(fireworks_time),
                ),
                FileKind::Raster,
                80,
                60,
                Some(fireworks_time),
            ),
            entry(
                write_png8(&summer, "lake.png", 72, 48),
                FileKind::Raster,
                72,
                48,
                None,
            ),
            entry(
                write_tiff16(&summer, "meadow.tif", 48, 72),
                FileKind::Raster,
                48,
                72,
                None,
            ),
        ];
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        let stray_file = fixture_path(&summer, "notes.txt");
        write_bytes(&stray_file, b"Remember to charge the batteries.\n");

        Self {
            dir,
            entries,
            stray_file,
        }
    }

    /// The directory an import should be pointed at.
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Every photograph in the library, sorted by path.
    pub fn entries(&self) -> &[LibraryEntry] {
        &self.entries
    }

    /// The dated sub-folders, sorted.
    pub fn folders(&self) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = self
            .entries
            .iter()
            .filter_map(|entry| entry.path.parent().map(Path::to_path_buf))
            .collect();
        folders.dedup();
        folders
    }

    /// The one file in the library that is not an image and must be skipped.
    pub fn stray_file(&self) -> &Path {
        &self.stray_file
    }
}

impl Default for TempLibrary {
    fn default() -> Self {
        Self::new()
    }
}

fn dated_folder(root: &Path, name: &str) -> PathBuf {
    let folder = fixture_path(root, name);
    std::fs::create_dir(&folder)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", folder.display()));
    folder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_holds_six_photographs_in_two_dated_folders() {
        let library = TempLibrary::new();
        let names: Vec<String> = library
            .entries()
            .iter()
            .map(|entry| {
                let relative = entry.path.strip_prefix(library.root()).unwrap();
                relative.to_string_lossy().replace('\\', "/")
            })
            .collect();
        assert_eq!(
            names,
            [
                "2024-03-15/dawn.jpg",
                "2024-03-15/harbour.dng",
                "2024-03-15/ridge.jpg",
                "2024-07-04/fireworks.jpg",
                "2024-07-04/lake.png",
                "2024-07-04/meadow.tif",
            ]
        );
        assert_eq!(library.folders().len(), 2);
        for entry in library.entries() {
            assert!(entry.path.is_file(), "{}", entry.path.display());
        }
        assert!(library.stray_file().is_file());
        assert_eq!(
            library
                .entries()
                .iter()
                .filter(|e| e.kind == FileKind::Raw)
                .count(),
            1
        );
    }

    #[test]
    fn folder_names_match_the_capture_dates_inside_them() {
        let library = TempLibrary::new();
        for entry in library.entries() {
            if let Some(time) = entry.capture_time {
                let folder = entry
                    .path
                    .parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                assert_eq!(folder, time.date_string());
            }
        }
    }

    #[test]
    fn raster_entries_report_their_upright_size() {
        let library = TempLibrary::new();
        for entry in library
            .entries()
            .iter()
            .filter(|e| e.kind == FileKind::Raster)
        {
            let (stored_width, stored_height) = image::image_dimensions(&entry.path).unwrap();
            let sideways = entry.path.ends_with("ridge.jpg");
            let upright = if sideways {
                (stored_height, stored_width)
            } else {
                (stored_width, stored_height)
            };
            assert_eq!(
                upright,
                (entry.width, entry.height),
                "{}",
                entry.path.display()
            );
        }
    }

    #[test]
    fn the_directory_is_removed_on_drop() {
        let library = TempLibrary::new();
        let root = library.root().to_path_buf();
        assert!(root.is_dir());
        drop(library);
        assert!(!root.exists());
    }
}
