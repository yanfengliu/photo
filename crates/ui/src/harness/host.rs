//! A minimal app for exercising widgets on their own.

use crate::theme;

/// The closure a [`Host`] calls every frame with the UI and its state.
type Show<S> = Box<dyn FnMut(&mut egui::Ui, &mut S)>;

/// An [`eframe::App`] made of a state value and a closure that shows it.
///
/// Widget tests need somewhere to put a widget and somewhere to record what
/// it reported; this is both. The closure runs inside a panel with the app's
/// theme applied, so the widget looks and measures as it does in the real
/// app. Built by [`super::HarnessBuilder::build_ui`].
pub struct Host<S> {
    /// The state handed to the closure every frame.
    pub state: S,
    show: Show<S>,
}

impl<S> Host<S> {
    /// Apply the theme to `ctx` and wrap `state` and `show` into an app.
    pub fn new(
        ctx: &egui::Context,
        state: S,
        show: impl FnMut(&mut egui::Ui, &mut S) + 'static,
    ) -> Self {
        theme::apply(ctx);
        Self {
            state,
            show: Box::new(show),
        }
    }
}

impl<S> eframe::App for Host<S> {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let frame = theme::panel_frame().inner_margin(theme::space::LG);
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            (self.show)(ui, &mut self.state);
        });
    }
}
