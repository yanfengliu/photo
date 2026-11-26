//! Opening a photo by its path: imported in place if the catalog does not
//! hold it yet, and made the active photo. [`open_in_engine`] does it on an
//! engine before there is an app (`photo PATH`); [`PhotoApp::open_path`]
//! does it in the running app and shows the photo, without starting the app
//! again, so the panels, the view's settings and the undo stack are as they
//! were.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use photo_core::meta::PhotoId;
use photo_engine::{Engine, Filter, ImportMode, ImportRequest, Source};

use super::PhotoApp;
use super::state::{Module, View};

/// Why a photo could not be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// There is no file at the path.
    Missing(PathBuf),
    /// The file is not of a kind Photo reads.
    Unsupported(PathBuf),
    /// The import did not finish in time.
    Timeout(PathBuf),
    /// The import or the catalog refused it, for this reason.
    Failed(String),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(path) => write!(f, "there is no file at {}", path.display()),
            Self::Unsupported(path) => {
                write!(f, "{} is not a kind of image Photo reads", path.display())
            }
            Self::Timeout(path) => write!(f, "importing {} did not finish in time", path.display()),
            Self::Failed(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for OpenError {}

/// The photo [`open_in_engine`] or [`PhotoApp::open_path`] opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// Its id in the catalog.
    pub photo: PhotoId,
    /// Its file's name.
    pub file_name: String,
    /// Its file, as an absolute path.
    pub path: PathBuf,
    /// Whether this opening imported it.
    pub imported: bool,
}

/// The absolute path of a photo that can be opened, or why it cannot: there
/// is no file there, or it is not a kind of image Photo reads. Nothing is
/// imported.
pub fn openable(path: &Path) -> Result<PathBuf, OpenError> {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if !path.is_file() {
        return Err(OpenError::Missing(path));
    }
    if !photo_engine::is_supported(&path) {
        return Err(OpenError::Unsupported(path));
    }
    Ok(path)
}

/// Open the photo at `path` in `engine`: import it in place unless the
/// catalog holds it, and make it the active photo, alone in the selection.
///
/// A photo the list already shows is opened where it is. A fresh import
/// leaves the Previous Import showing, as every import does; a photo that
/// is in the catalog but not in the list shown is found in All Photographs
/// with no filter. The call waits (up to `timeout`) for the import, which
/// reads the file's metadata.
pub fn open_in_engine(
    engine: &mut Engine,
    path: &Path,
    timeout: Duration,
) -> Result<Opened, OpenError> {
    let path = openable(path)?;
    let mut imported = false;
    if listed(engine, &path).is_none() {
        imported = import_in_place(engine, &path, timeout)?;
    }
    let (photo, file_name) = listed(engine, &path).ok_or_else(|| {
        OpenError::Failed(format!(
            "{} is in the catalog but not in the photo list",
            path.display()
        ))
    })?;
    engine.select_only(photo);
    Ok(Opened {
        photo,
        file_name,
        path,
        imported,
    })
}

/// The photo of the list shown whose file is `path` (not a virtual copy of
/// it).
fn listed(engine: &Engine, path: &Path) -> Option<(PhotoId, String)> {
    engine
        .photos()
        .iter()
        .find(|photo| photo.path == path && !photo.is_virtual_copy())
        .map(|photo| (photo.id, photo.file_name.clone()))
}

/// Import the file at `path` where it is and wait for it. Returns whether
/// it was new to the catalog; when it was not, the list is changed to all
/// photographs, unfiltered, so that it is in it.
fn import_in_place(engine: &mut Engine, path: &Path, timeout: Duration) -> Result<bool, OpenError> {
    let failed = |what: &str, error: &dyn fmt::Display| {
        OpenError::Failed(format!("cannot {what} {}: {error}", path.display()))
    };
    let request = ImportRequest {
        files: vec![path.to_path_buf()],
        mode: ImportMode::Add,
        skip_suspected_duplicates: false,
        keywords: Vec::new(),
        read_sidecars: true,
    };
    let task = engine
        .start_import(request)
        .map_err(|error| failed("import", &error))?;
    if !engine.wait_idle(timeout) {
        return Err(OpenError::Timeout(path.to_path_buf()));
    }
    let summary = engine
        .import_summary(task)
        .cloned()
        .ok_or_else(|| OpenError::Failed("the import left no summary".to_owned()))?;
    if let Some(issue) = summary.errors.first() {
        return Err(failed("import", &issue.message));
    }
    let imported = !summary.imported.is_empty();
    if !imported {
        engine
            .set_source(Source::AllPhotos)
            .and_then(|()| engine.set_filter(Filter::default()))
            .map_err(|error| failed("show", &error))?;
    }
    Ok(imported)
}

impl PhotoApp {
    /// Open the photo at `path` without starting the app again, as
    /// [`open_in_engine`] does, and show it: the Library shows it in the
    /// loupe; Develop stays Develop and opens it there. Run a frame
    /// afterwards to see the photo.
    pub fn open_path(&mut self, path: &Path, timeout: Duration) -> Result<Opened, OpenError> {
        let opened = open_in_engine(&mut self.engine, path, timeout);
        // The engine may have imported or changed its list even when the
        // photo was not opened in the end.
        self.generation += 1;
        let opened = opened?;
        if self.ui.module == Module::Library {
            self.ui.view = View::Loupe;
        }
        Ok(opened)
    }
}
