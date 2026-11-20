//! Photo's user interface.
//!
//! The app is [`PhotoApp`]: the shell (top bar, side columns, filmstrip,
//! toolbar) around the Library module, driving one `photo_engine::Engine`.
//! It is built from the visual system ([`theme`]) and the widget kit
//! ([`widgets`]); [`harness`] drives it (or any other `eframe::App`)
//! without a window, and [`gallery`] is a showcase of every widget.
//!
//! The crate holds no business logic. Every user action is one engine call;
//! the UI keeps only presentation state (which panels are open, the view,
//! zoom and scroll, dialog fields, drags in progress).

#![warn(missing_docs)]

mod app;
pub mod gallery;
pub mod harness;
pub mod theme;
pub mod widgets;

use std::path::PathBuf;

pub use app::{Layout, Module, PhotoApp, View, Zoom};
use photo_engine::{Engine, EngineConfig, EngineError};

/// Where the app keeps its data. Every path is chosen by the caller:
/// nothing in this crate decides a location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppOptions {
    /// The catalog file; created if it does not exist.
    pub catalog_path: PathBuf,
    /// The folder preview files are kept in; created if it does not exist.
    pub previews_dir: PathBuf,
    /// Where exports go unless the user chooses another folder.
    pub export_dir: PathBuf,
}

impl AppOptions {
    /// Open the engine on these paths, with a worker per spare processor
    /// (at least one). Fails as `Engine::open` does: a catalog that cannot
    /// be opened or a previews folder that cannot be made.
    pub fn open_engine(&self) -> Result<Engine, EngineError> {
        let worker_threads = std::thread::available_parallelism()
            .map(|count| count.get().saturating_sub(1).max(1))
            .unwrap_or(2);
        Engine::open(EngineConfig {
            catalog_path: self.catalog_path.clone(),
            previews_dir: self.previews_dir.clone(),
            worker_threads,
        })
    }
}

/// Open the catalog `options` names and show the app in a window. Returns
/// when the window is closed, or with the reason the catalog could not be
/// opened or the window could not be made.
pub fn run(options: AppOptions) -> Result<(), Box<dyn std::error::Error>> {
    let engine = options.open_engine()?;
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
        Box::new(move |cc| Ok(Box::new(PhotoApp::new(cc, engine, options)))),
    )?;
    Ok(())
}
