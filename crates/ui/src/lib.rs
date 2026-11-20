//! Photo's user interface.
//!
//! This crate is the foundation the Library and Develop screens are built
//! on: the visual system ([`theme`]), the widget kit ([`widgets`]), a
//! headless driver for tests and automation ([`harness`]), and a [`gallery`]
//! app that shows every widget in a realistic arrangement.
//!
//! The crate holds no business logic. Widgets edit plain `photo-core` values
//! and report what happened; deciding what that means is the engine's job.

#![warn(missing_docs)]

pub mod gallery;
pub mod harness;
pub mod theme;
pub mod widgets;

/// Open the app window. Returns when the window is closed.
///
/// Until the Library and Develop screens exist, the window shows the widget
/// [`gallery`].
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Photo")
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Photo",
        options,
        Box::new(|cc| Ok(Box::new(gallery::GalleryApp::new(cc)))),
    )?;
    Ok(())
}
