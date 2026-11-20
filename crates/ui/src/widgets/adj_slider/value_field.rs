//! The number at the right of a develop slider: shown as plain text, and a
//! text field while it has keyboard focus.

use egui::accesskit::{Action, ActionData};
use egui::text::CCursorRange;
use egui::{Align2, Id, Key, Margin, Rect, Sense, TextEdit, Ui, WidgetInfo};
use photo_core::develop::AdjSpec;

use super::{Edit, round_to_decimals};
use crate::theme::{self, palette};
use crate::widgets::{recall, remember};

/// Everything the value field needs to know about its slider.
pub(super) struct ValueField<'a> {
    pub id: Id,
    pub name: &'a str,
    pub spec: AdjSpec,
    pub default: f32,
}

/// The text of the field while it has keyboard focus.
#[derive(Clone)]
struct Typing {
    /// The number as the field showed it when the text was taken from it.
    shown: String,
    /// What the field holds now.
    text: String,
}

impl Typing {
    fn of(shown: &str) -> Self {
        Self {
            shown: shown.to_owned(),
            text: shown.to_owned(),
        }
    }

    /// Whether anything was typed. Only then is there something to commit:
    /// merely passing through the field must not round or clamp the value.
    fn edited(&self) -> bool {
        self.text != self.shown
    }
}

impl ValueField<'_> {
    /// Whether a number has been typed into the field and not yet committed.
    pub fn is_typing(&self, ui: &Ui) -> bool {
        recall::<Typing>(ui.ctx(), self.id).is_some_and(|typing| typing.edited())
    }

    /// Show the number in `rect` and apply any edit to `value`.
    ///
    /// Click (or tab to) the number to edit it: Enter or leaving the field
    /// commits what was typed, clamped to the range; Escape discards it.
    pub fn show(&self, ui: &mut Ui, rect: Rect, value: &mut f32, edit: &mut Edit) {
        let shown = self.spec.format(*value);
        let enabled = ui.is_enabled();
        // Declare interest before asking, so focus arriving by Tab shows the
        // text field in the same frame.
        let editing = enabled
            && ui.memory_mut(|memory| {
                memory.interested_in_focus(self.id, ui.layer_id());
                memory.has_focus(self.id)
            });
        let typing: Option<Typing> = recall(ui.ctx(), self.id);
        let escaped = ui.input(|input| input.key_pressed(Key::Escape));
        let mut keep: Option<Typing> = None;

        if editing {
            // Until something is typed the field follows the value, which can
            // still change under it (an undo, a drag of the track).
            let mut typing = typing
                .filter(|typing| typing.edited() || typing.shown == shown)
                .unwrap_or_else(|| Typing::of(&shown));
            let response = ui
                .place(
                    rect,
                    TextEdit::singleline(&mut typing.text)
                        .id(self.id)
                        .font(theme::body_font())
                        .align(Align2::RIGHT_CENTER)
                        .margin(Margin::symmetric(3, 2))
                        .desired_width(rect.width())
                        .min_size(rect.size()),
                )
                .accessible_name(self.name);
            if response.gained_focus() {
                select_all(ui.ctx(), self.id, &typing.text);
            }
            if !response.lost_focus() {
                keep = Some(typing);
            } else if !escaped {
                self.commit_typed(&typing, value, edit);
            }
        } else {
            // Focus left since the last frame (Tab, a click elsewhere): what
            // was typed still counts, unless Escape threw it away or the
            // slider was disabled in the meantime.
            if let Some(typing) = typing.filter(|_| enabled && !escaped) {
                self.commit_typed(&typing, value, edit);
            }
            // `place` above gives the field a child `Ui`, which takes one of
            // the parent's automatic ids. Take one here too, so the ids of
            // the widgets after the slider do not depend on the field's mode.
            ui.skip_ahead_auto_ids(1);
            self.show_static(ui, rect, value, edit);
        }

        remember(ui.ctx(), self.id, keep);
    }

    fn commit_typed(&self, typing: &Typing, value: &mut f32, edit: &mut Edit) {
        if typing.edited() {
            self.commit(&typing.text, value, edit);
        }
    }

    fn show_static(&self, ui: &Ui, rect: Rect, value: &mut f32, edit: &mut Edit) {
        let response = ui.interact(rect, self.id, Sense::click());

        // A text value set through the accessibility tree is a complete edit.
        let typed = ui.input(|input| {
            input
                .accesskit_action_requests(self.id, Action::SetValue)
                .filter_map(|request| match &request.data {
                    Some(ActionData::Value(text)) => Some(text.to_string()),
                    _ => None,
                })
                .last()
        });
        let enabled = ui.is_enabled();
        if let Some(text) = typed.filter(|_| enabled) {
            self.commit(&text, value, edit);
        }

        let shown = self.spec.format(*value);
        if response.clicked() {
            response.request_focus();
            select_all(ui.ctx(), self.id, &shown);
        }

        response.widget_info(|| WidgetInfo {
            label: Some(self.name.to_owned()),
            current_text_value: Some(shown.clone()),
            enabled,
            ..WidgetInfo::new(egui::Role::TextInput)
        });
        ui.ctx()
            .accesskit_node_builder(self.id, |node| node.add_action(Action::SetValue));

        if ui.is_rect_visible(rect) {
            let color = if response.hovered() || *value != self.default {
                palette::TEXT_STRONG
            } else {
                palette::TEXT
            };
            if response.hovered() {
                ui.painter()
                    .rect_filled(rect, theme::radius::MD, palette::HEADER);
            }
            ui.painter().text(
                rect.right_center() - egui::vec2(3.0, 0.0),
                Align2::RIGHT_CENTER,
                &shown,
                theme::body_font(),
                color,
            );
        }
    }

    fn commit(&self, text: &str, value: &mut f32, edit: &mut Edit) {
        if let Some(number) = parse(text) {
            let target = self.spec.clamp(round_to_decimals(&self.spec, number));
            edit.commit(value, target);
        }
    }
}

/// Read a number the way people type one: surrounding spaces, a leading plus
/// sign and a decimal comma are all fine. Anything else is not a number.
fn parse(text: &str) -> Option<f32> {
    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            ',' => '.',
            '\u{2212}' => '-',
            other => other,
        })
        .collect();
    cleaned
        .parse::<f32>()
        .ok()
        .filter(|number| number.is_finite())
}

fn select_all(ctx: &egui::Context, id: Id, text: &str) {
    let mut state = TextEdit::load_state(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::select_all_str(text)));
    state.store(ctx, id);
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn numbers_parse_as_people_type_them() {
        assert_eq!(parse("12"), Some(12.0));
        assert_eq!(parse(" +0.35 "), Some(0.35));
        assert_eq!(parse("-1,5"), Some(-1.5));
        assert_eq!(parse("\u{2212}20"), Some(-20.0));
        assert_eq!(parse("5 600"), Some(5600.0));
    }

    #[test]
    fn text_that_is_not_a_number_is_rejected() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("abc"), None);
        assert_eq!(parse("1.2.3"), None);
        assert_eq!(parse("inf"), None);
        assert_eq!(parse("NaN"), None);
    }
}
