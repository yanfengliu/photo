//! Photo's app as the harness runs it: the real [`PhotoApp`], wrapped so the
//! harness can see what a frame put in the accessibility tree and which
//! toasts came and went.
//!
//! `photo_ui::harness` finds one control at a time by role and name; to
//! list every control, the wrapper copies egui's AccessKit nodes at the end
//! of a frame the harness asks it to watch, once the app has drawn
//! everything (see `tree`).

use std::collections::VecDeque;

use photo_ui::PhotoApp;

use super::tree::Tree;

/// How many toasts `observe` can look back on.
const TOAST_HISTORY: usize = 20;

/// The app, and what the harness watches of it.
pub struct Observed {
    /// The app itself.
    pub app: PhotoApp,
    capture: bool,
    captured: Option<Tree>,
    showing: Vec<String>,
    history: VecDeque<SeenToast>,
    next_toast: u64,
}

/// A toast the harness saw appear.
#[derive(Clone, Debug, PartialEq)]
pub struct SeenToast {
    /// Its message.
    pub message: String,
    /// Counts toasts from the start of the app.
    pub seq: u64,
    /// The frame it appeared in.
    pub frame: u64,
}

impl Observed {
    /// Wrap `app`.
    pub fn new(app: PhotoApp) -> Self {
        Self {
            app,
            capture: false,
            captured: None,
            showing: Vec::new(),
            history: VecDeque::new(),
            next_toast: 1,
        }
    }

    /// Copy the accessibility tree at the end of the next frame.
    pub fn capture_next_frame(&mut self) {
        self.capture = true;
        self.captured = None;
    }

    /// The tree the last watched frame left, taken.
    pub fn take_tree(&mut self) -> Option<Tree> {
        self.captured.take()
    }

    /// The toasts seen appearing, oldest first, at most the last twenty.
    pub fn toast_history(&self) -> impl Iterator<Item = &SeenToast> {
        self.history.iter()
    }

    /// Note the toasts that appeared since the last frame.
    fn note_toasts(&mut self, frame: u64) {
        let now: Vec<String> = self.app.toasts().map(str::to_owned).collect();
        for message in &now[still_showing(&self.showing, &now)..] {
            if self.history.len() == TOAST_HISTORY {
                self.history.pop_front();
            }
            self.history.push_back(SeenToast {
                message: message.clone(),
                seq: self.next_toast,
                frame,
            });
            self.next_toast += 1;
        }
        self.showing = now;
    }
}

/// How many of the toasts showing `now` were showing `before` too. The app
/// adds toasts at the end of its queue and drops them from the front, so
/// they are the longest end of `before` that `now` starts with; the rest of
/// `now` is new.
fn still_showing(before: &[String], now: &[String]) -> usize {
    (0..=before.len())
        .map(|start| &before[start..])
        .find(|tail| now.starts_with(tail))
        .map_or(0, <[String]>::len)
}

impl eframe::App for Observed {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        let ctx = ui.ctx().clone();
        self.note_toasts(ctx.cumulative_frame_nr());
        if std::mem::take(&mut self.capture) {
            self.captured = Tree::capture(&ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(before: &[&str], now: &[&str]) -> usize {
        let owned =
            |list: &[&str]| -> Vec<String> { list.iter().map(|s| (*s).to_owned()).collect() };
        still_showing(&owned(before), &owned(now))
    }

    #[test]
    fn toasts_are_seen_once_as_they_come_and_go() {
        assert_eq!(kept(&[], &["a"]), 0, "a first toast is new");
        assert_eq!(kept(&["a"], &["a", "b"]), 1, "b joined a");
        assert_eq!(kept(&["a", "b"], &["b", "c"]), 1, "a went, c came");
        assert_eq!(kept(&["a", "b"], &["c"]), 0, "both went, c came");
        assert_eq!(kept(&["a", "b"], &["a", "b"]), 2, "nothing changed");
        assert_eq!(
            kept(&["a"], &["a", "a"]),
            1,
            "the same message twice is two toasts"
        );
    }
}
