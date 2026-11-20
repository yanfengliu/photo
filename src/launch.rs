//! Starting Photo: the command line, and opening the photo it names.
//!
//! ```text
//! photo [PATH]                                  the app in a window
//! photo --harness[=PORT] [--harness-real-storage] [PATH]
//!                                               the app driven over localhost
//! ```
//!
//! A path opens that photo at startup, harness or not: imported in place if
//! the catalog does not hold it yet, made the active photo, and shown in
//! the loupe.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Event, Key, Modifiers};
use photo_engine::{Engine, ImportMode, ImportRequest, Source};
use photo_ui::{AppOptions, PhotoApp};

/// The port `--harness` listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 7878;

/// How long opening a photo may take: its import is a metadata read.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(120);

/// What the command line asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// Run headless behind the agent harness, on this port (0: any free one).
    pub harness: Option<u16>,
    /// Let a harness session use the user's real catalog.
    pub real_storage: bool,
    /// The photo to open at startup.
    pub open: Option<PathBuf>,
}

/// How the command line is used, for error messages.
pub const USAGE: &str = "usage: photo [--harness[=PORT] [--harness-real-storage]] [PATH]";

impl Args {
    /// Read the arguments after the program name.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut paths_only = false;
        for arg in args {
            let text = arg.to_string_lossy().into_owned();
            if paths_only || !text.starts_with("--") {
                if parsed.open.is_some() {
                    return Err(format!("only one photo can be opened\n{USAGE}"));
                }
                parsed.open = Some(PathBuf::from(arg));
                continue;
            }
            match text.split_once('=') {
                _ if text == "--" => paths_only = true,
                _ if text == "--harness" => parsed.harness = Some(DEFAULT_PORT),
                Some(("--harness", port)) => {
                    let port = port
                        .parse()
                        .map_err(|_| format!("`{port}` is not a port number\n{USAGE}"))?;
                    parsed.harness = Some(port);
                }
                _ if text == "--harness-real-storage" => parsed.real_storage = true,
                _ => return Err(format!("unknown option `{text}`\n{USAGE}")),
            }
        }
        if parsed.real_storage && parsed.harness.is_none() {
            return Err(format!(
                "--harness-real-storage only goes with --harness\n{USAGE}"
            ));
        }
        Ok(parsed)
    }
}

/// Why a photo could not be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// There is no file at the path.
    Missing(PathBuf),
    /// The file is not of a kind Photo reads.
    Unsupported(PathBuf),
    /// The import did not finish in time.
    Timeout(PathBuf),
    /// The import or the catalog refused it.
    Failed(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

/// The photo [`open_in_engine`] opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// Its id in the catalog.
    pub id: i64,
    /// Its file's name.
    pub file_name: String,
    /// Its file.
    pub path: PathBuf,
    /// Whether this opening imported it.
    pub imported: bool,
}

/// The absolute path of a photo that can be opened, or why it cannot.
pub fn openable(path: &Path) -> Result<PathBuf, OpenError> {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if !absolute.is_file() {
        return Err(OpenError::Missing(absolute));
    }
    if !photo_engine::is_supported(&absolute) {
        return Err(OpenError::Unsupported(absolute));
    }
    Ok(absolute)
}

/// Import the photo at `path` in place unless the catalog holds it already,
/// and make it the active photo, alone in the selection. A fresh import
/// leaves the Previous Import showing; a photo already in the catalog is
/// found in All Photographs.
pub fn open_in_engine(
    engine: &mut Engine,
    path: &Path,
    timeout: Duration,
) -> Result<Opened, OpenError> {
    let path = openable(path)?;
    let request = ImportRequest {
        files: vec![path.clone()],
        mode: ImportMode::Add,
        skip_suspected_duplicates: false,
        keywords: Vec::new(),
        read_sidecars: true,
    };
    let task = engine
        .start_import(request)
        .map_err(|error| OpenError::Failed(format!("cannot import {}: {error}", path.display())))?;
    if !engine.wait_idle(timeout) {
        return Err(OpenError::Timeout(path));
    }
    let summary = engine
        .import_summary(task)
        .cloned()
        .ok_or_else(|| OpenError::Failed("the import left no summary".to_owned()))?;
    if let Some(issue) = summary.errors.first() {
        return Err(OpenError::Failed(format!(
            "cannot import {}: {}",
            issue.path.display(),
            issue.message
        )));
    }
    let imported = !summary.imported.is_empty();
    if !imported {
        engine
            .set_source(Source::AllPhotos)
            .map_err(|error| OpenError::Failed(format!("cannot show all photos: {error}")))?;
    }
    let found = engine
        .photos()
        .iter()
        .find(|photo| photo.path == path && !photo.is_virtual_copy())
        .map(|photo| (photo.id, photo.file_name.clone()))
        .ok_or_else(|| {
            OpenError::Failed(format!(
                "{} is in the catalog but not in the photo list",
                path.display()
            ))
        })?;
    let (id, file_name) = found;
    engine.select_only(id);
    Ok(Opened {
        id: id.0,
        file_name,
        path,
        imported,
    })
}

/// The key press that shows the loupe, as a keyboard sends it.
pub fn loupe_key_events() -> [Event; 2] {
    [true, false].map(|pressed| Event::Key {
        key: Key::E,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    })
}

/// The app in a window with the photo at `path` open in the loupe.
///
/// `photo_ui::run` opens its own engine and starts in the grid, so this
/// opens the engine, opens the photo in it, and starts the window with the
/// app it would build, pressing E (the loupe) in the first frame. The
/// window's options are those `photo_ui::run` uses.
pub fn run_window_with(options: AppOptions, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = options.open_engine()?;
    let opened = open_in_engine(&mut engine, path, OPEN_TIMEOUT)?;
    log::info!("opened {} (photo {})", opened.path.display(), opened.id);
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Photo")
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Photo",
        native,
        Box::new(move |cc| {
            let app = PhotoApp::new(cc, engine, options);
            Ok(Box::new(InLoupe { app, pending: true }))
        }),
    )?;
    Ok(())
}

/// The app, with E pressed in its first frame.
struct InLoupe {
    app: PhotoApp,
    pending: bool,
}

impl eframe::App for InLoupe {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if std::mem::take(&mut self.pending) {
            raw_input.events.extend(loupe_key_events());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn the_harness_listens_on_7878_unless_given_a_port() {
        assert_eq!(parse(&[]).unwrap(), Args::default());
        assert_eq!(parse(&["--harness"]).unwrap().harness, Some(DEFAULT_PORT));
        assert_eq!(parse(&["--harness=0"]).unwrap().harness, Some(0));
        assert_eq!(parse(&["--harness=9001"]).unwrap().harness, Some(9001));
        assert!(parse(&["--harness=port"]).is_err());
        assert!(parse(&["--harness=70000"]).is_err());
    }

    #[test]
    fn a_bare_path_is_the_photo_to_open_harness_or_not() {
        let window = parse(&["shot.jpg"]).unwrap();
        assert_eq!(
            (window.harness, window.open),
            (None, Some(PathBuf::from("shot.jpg")))
        );
        let harness = parse(&["--harness", "--harness-real-storage", "raw.dng"]).unwrap();
        assert!(harness.real_storage);
        assert_eq!(harness.open, Some(PathBuf::from("raw.dng")));
        assert_eq!(
            parse(&["--", "--odd-name.jpg"]).unwrap().open,
            Some(PathBuf::from("--odd-name.jpg"))
        );
        assert!(parse(&["a.jpg", "b.jpg"]).is_err());
    }

    #[test]
    fn real_storage_only_goes_with_the_harness_and_unknown_options_are_refused() {
        assert!(parse(&["--harness-real-storage"]).is_err());
        assert!(parse(&["--fullscreen"]).is_err());
    }

    #[test]
    fn the_loupe_key_is_pressed_and_released() {
        let [press, release] = loupe_key_events();
        assert!(matches!(
            press,
            Event::Key {
                key: Key::E,
                pressed: true,
                ..
            }
        ));
        assert!(matches!(
            release,
            Event::Key {
                key: Key::E,
                pressed: false,
                ..
            }
        ));
    }
}
