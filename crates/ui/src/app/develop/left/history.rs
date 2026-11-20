//! The History panel: every step of the photo's settings, newest first, the
//! one the photo is at marked. A click goes to a step, as one more step
//! that undo takes back; the menu shows a step as the before view.

use egui::{RichText, Ui};
use photo_engine::BeforeSettings;

use super::looks::show_as_before;
use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::develop::actions::DevelopAction;
use crate::app::develop::right::Editing;
use crate::theme::palette;
use crate::widgets::ListRow;

/// The accessible name of the history row of step `number` (from 1, the
/// oldest).
pub(crate) fn row_name(number: usize, label: &str) -> String {
    format!("History {number}: {label}")
}

/// Show the panel.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>, editing: Option<&Editing>) {
    let Some(editing) = editing else {
        return;
    };
    let steps = cx.ui.develop.lists.history.clone();
    if steps.is_empty() {
        ui.label(
            RichText::new("Each change to the photo is listed here.")
                .color(palette::TEXT_DIM)
                .small(),
        );
        return;
    }
    let editable = editing.editable();
    for (index, step) in steps.iter().enumerate().rev() {
        let name = row_name(index + 1, &step.label);
        let row = ui
            .add_enabled_ui(editable, |ui| {
                ListRow::new(&name, &step.label)
                    .selected(step.is_current)
                    .show(ui)
            })
            .inner;
        if row.clicked && !step.is_current {
            cx.act(Action::Develop(DevelopAction::GoToHistoryStep(
                editing.photo,
                step.id,
            )));
        }
        egui::Popup::context_menu(&row.response).show(|ui| {
            if ui.button("Show as Before").clicked() {
                show_as_before(cx, BeforeSettings::HistoryStep(step.id));
            }
        });
    }
}
