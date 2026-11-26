//! Behaviour contracts of the UI foundation, checked the way a user or an
//! automation script meets them: through the headless harness, finding
//! controls by role and accessible name.

mod attributes;
mod dialog;
mod gallery;
mod harness;
mod number;
mod panel;
mod plots;
mod slider;
mod surfaces;

/// The result type of a contract test: any harness or render error fails it.
pub type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A point `fraction` of the way across `rect`, vertically centred.
pub fn across(rect: egui::Rect, fraction: f32) -> egui::Pos2 {
    egui::pos2(
        egui::lerp(rect.left()..=rect.right(), fraction),
        rect.center().y,
    )
}
