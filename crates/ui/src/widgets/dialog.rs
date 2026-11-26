//! The modal dialog frame every dialog (Import, Export, Copy Settings, ...)
//! is built in, so they all look and behave alike.
//!
//! The frame gives a title, a body the caller fills, and a row with a confirm
//! and a cancel button in the platform's order (confirm on the right, except
//! on Windows). Enter confirms and Escape cancels: the topmost dialog only,
//! and never with the key press that opened it. The rest of the window is
//! dimmed and takes no pointer input while the dialog is up, and a key the
//! dialog answers is taken out of the input for whatever is shown after it.

use egui::os::OperatingSystem;
use egui::{
    Align, Button, Color32, CornerRadius, Event, Frame, Id, Key, Layout, Margin, Modal, Modifiers,
    RichText, Stroke, Ui, vec2,
};

use super::{recall, remember};
use crate::theme::{self, palette, radius, size, space};

/// What happened to a dialog this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogOutcome {
    /// Nothing: keep showing it.
    Open,
    /// The confirm button or Enter: do the thing and close.
    Confirmed,
    /// The cancel button or Escape: close without doing anything.
    Cancelled,
}

/// What a dialog reports for one frame.
pub struct DialogResponse<R> {
    /// Whether the user confirmed or cancelled.
    pub outcome: DialogOutcome,
    /// What the body closure returned.
    pub body: R,
}

/// A modal dialog. See the [module documentation](self).
#[must_use = "call `.show(ctx, body)`"]
pub struct Dialog<'a> {
    title: &'a str,
    confirm: &'a str,
    cancel: &'a str,
    can_confirm: bool,
    width: f32,
}

impl<'a> Dialog<'a> {
    /// A dialog titled `title` with "OK" and "Cancel" buttons. The title also
    /// names the dialog for accessibility and must be unique among the
    /// dialogs that can be open at once.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            confirm: "OK",
            cancel: "Cancel",
            can_confirm: true,
            width: 420.0,
        }
    }

    /// The text of the confirm button: say what it does ("Import", "Export").
    pub fn confirm_label(mut self, label: &'a str) -> Self {
        self.confirm = label;
        self
    }

    /// The text of the cancel button.
    pub fn cancel_label(mut self, label: &'a str) -> Self {
        self.cancel = label;
        self
    }

    /// Whether the dialog can be confirmed right now. While it cannot, the
    /// confirm button is disabled and Enter does nothing.
    pub fn can_confirm(mut self, can_confirm: bool) -> Self {
        self.can_confirm = can_confirm;
        self
    }

    /// The width of the dialog's content in points.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Show the dialog for this frame. Call it every frame while the dialog
    /// is open, and stop when the outcome is no longer [`DialogOutcome::Open`].
    pub fn show<R>(
        self,
        ctx: &egui::Context,
        body: impl FnOnce(&mut Ui) -> R,
    ) -> DialogResponse<R> {
        let id = Id::unique(("photo-dialog", self.title));
        let frame = Frame::new()
            .fill(palette::PANEL)
            .stroke(Stroke::new(1.0, palette::OUTLINE))
            .corner_radius(CornerRadius::same(radius::LG))
            .shadow(ctx.global_style().visuals.window_shadow)
            .inner_margin(Margin::same(18));
        let mut modal = Modal::new(id)
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha(150));
        modal.area = Modal::default_area(id).accessible_name(self.title);

        let confirm_first = confirm_is_rightmost(ctx.os());
        let shown = modal.show(ctx, |ui| {
            ui.set_width(self.width);
            ui.label(
                RichText::new(self.title)
                    .font(theme::title_font())
                    .color(palette::TEXT_STRONG),
            );
            ui.add_space(space::LG);
            let body = body(ui);
            ui.add_space(space::XL);

            let (mut confirmed, mut cancelled) = (false, false);
            // A row exactly as tall as its buttons: laid out in whatever
            // height is left, they would hang in the space a body that got
            // shorter left behind.
            let row = vec2(ui.available_width(), BUTTON.y);
            ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = space::LG - space::XS;
                // In a right-to-left row the first button added is rightmost.
                for is_confirm in [confirm_first, !confirm_first] {
                    if is_confirm {
                        let label = RichText::new(self.confirm).color(palette::TEXT_STRONG);
                        let button = Button::new(label).fill(palette::ACCENT_DIM);
                        confirmed = ui
                            .add_enabled(self.can_confirm, button.min_size(BUTTON))
                            .clicked();
                    } else {
                        cancelled = ui.add(Button::new(self.cancel).min_size(BUTTON)).clicked();
                    }
                }
            });
            (body, confirmed, cancelled)
        });

        let (body, confirmed, cancelled) = shown.inner;
        // The keys are this dialog's only if it is the one on top with no
        // menu open over it, and only if it was already up last frame: the
        // Enter that opened it (by confirming another dialog, say) is not an
        // answer to it. (egui itself reports a new modal as on top only from
        // its second frame; this does not rely on that.) Keys that are this
        // dialog's are taken out of the input whether or not they act, so
        // nothing shown after the dialog acts on them too.
        let frame = ctx.cumulative_frame_nr();
        let opened: u64 = recall(ctx, id).unwrap_or(frame);
        remember(ctx, id, Some(opened));
        let live = opened < frame && shown.is_top_modal && !shown.any_popup_open;
        // A focused button keeps Enter to itself; a text field gives up
        // focus on Enter, so by now nothing is focused and Enter confirms.
        let unfocused = ctx.memory(|memory| memory.focused().is_none());
        let enter = live && take_press(ctx, Key::Enter) && self.can_confirm && unfocused;
        let escape = live && take_press(ctx, Key::Escape);
        let outcome = if cancelled {
            DialogOutcome::Cancelled
        } else if confirmed {
            DialogOutcome::Confirmed
        } else if escape {
            DialogOutcome::Cancelled
        } else if enter {
            DialogOutcome::Confirmed
        } else {
            DialogOutcome::Open
        };
        DialogResponse { outcome, body }
    }
}

/// Take a fresh press of `key` out of this frame's input; says whether there
/// was one. A key held down repeats, and those repeats do not count: holding
/// Enter must not answer one dialog after another.
fn take_press(ctx: &egui::Context, key: Key) -> bool {
    ctx.input_mut(|input| {
        let before = input.events.len();
        input.events.retain(|event| {
            !matches!(
                event,
                Event::Key { key: pressed, pressed: true, repeat: false, modifiers, .. }
                    if *pressed == key && modifiers.matches_logically(Modifiers::NONE)
            )
        });
        input.events.len() != before
    })
}

/// The least size of a dialog button.
const BUTTON: egui::Vec2 = vec2(84.0, size::ICON_BUTTON);

/// Whether the confirm button goes to the right of the cancel button. It
/// does everywhere but on Windows, where OK comes first.
fn confirm_is_rightmost(os: OperatingSystem) -> bool {
    os != OperatingSystem::Windows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_confirm_button_is_rightmost_except_on_windows() {
        assert!(confirm_is_rightmost(OperatingSystem::Mac));
        assert!(confirm_is_rightmost(OperatingSystem::Nix));
        assert!(!confirm_is_rightmost(OperatingSystem::Windows));
    }
}
