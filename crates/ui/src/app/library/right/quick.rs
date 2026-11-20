//! Quick Develop: relative edits to the selected photos from the Library
//! (the active photo when none is selected): a white-balance preset, Auto
//! Tone, steps up and down for exposure (a third of a stop and a stop) and
//! for contrast, highlights, shadows, whites, blacks, clarity and vibrance,
//! and Reset All. Each click is one step that undo takes back.

use egui::{Button, ComboBox, RichText, Ui, vec2};
use photo_core::develop::{Adj, WbPreset};
use photo_core::meta::PhotoId;
use photo_engine::Target;

use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::develop::actions::DevelopAction;
use crate::theme::{palette, size, space};
use crate::widgets::{Icon, IconButton};

/// The adjustments Quick Develop steps, with the small and the large step.
const STEPS: [(Adj, f32, f32); 8] = [
    (Adj::Exposure, 1.0 / 3.0, 1.0),
    (Adj::Contrast, 5.0, 20.0),
    (Adj::Highlights, 5.0, 20.0),
    (Adj::Shadows, 5.0, 20.0),
    (Adj::Whites, 5.0, 20.0),
    (Adj::Blacks, 5.0, 20.0),
    (Adj::Clarity, 5.0, 20.0),
    (Adj::Vibrance, 5.0, 20.0),
];

/// Width of the label column.
const LABEL: f32 = 86.0;

/// What the panel remembers: the active photo's white-balance preset, read
/// again when the photo or its settings change.
#[derive(Clone, Debug, Default)]
pub(crate) struct QuickState {
    read: Option<(PhotoId, u64)>,
    preset: Option<WbPreset>,
}

/// A step as written on its button's name: "+1/3", "-1", "+5".
pub(crate) fn step_text(adj: Adj, delta: f32) -> String {
    let sign = if delta < 0.0 { "-" } else { "+" };
    let size = delta.abs();
    if adj == Adj::Exposure && (size - 1.0 / 3.0).abs() < 1.0e-4 {
        format!("{sign}1/3")
    } else {
        format!("{sign}{size}")
    }
}

/// Show the panel.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let Some(active) = cx.engine.active_photo() else {
        ui.label(RichText::new("Select photos to develop them here.").color(palette::TEXT_DIM));
        return;
    };
    let target = Target::Selected;
    let count = cx.engine.selection().selected().len().max(1);
    let editable = !cx.engine.is_read_only();
    ui.add_enabled_ui(editable, |ui| {
        white_balance(ui, cx, active);
        ui.horizontal(|ui| {
            label(ui, "Tone Control");
            if ui.add(Button::new("Auto Tone").min_size(vec2(0.0, size::ROW))).clicked() {
                cx.act(Action::Develop(DevelopAction::QuickAutoTone(target.clone())));
            }
        });
        ui.add_space(space::SM);
        for (adj, small, large) in STEPS {
            ui.horizontal(|ui| {
                label(ui, adj.spec().label);
                for (icon, delta) in [
                    (Icon::ChevronsLeft, -large),
                    (Icon::ChevronLeft, -small),
                    (Icon::ChevronRight, small),
                    (Icon::ChevronsRight, large),
                ] {
                    let name = format!("{} {}", adj.spec().label, step_text(adj, delta));
                    if ui.add(IconButton::new(icon, &name).side(22.0)).clicked() {
                        let action = DevelopAction::QuickAdjust(target.clone(), adj, delta);
                        cx.act(Action::Develop(action));
                    }
                }
            });
        }
        ui.add_space(space::MD);
        let reset = Button::new("Reset All").min_size(vec2(ui.available_width(), 24.0));
        if ui
            .add(reset)
            .on_hover_text("Put every setting of the selected photos back to how they were imported")
            .clicked()
        {
            cx.act(Action::Develop(DevelopAction::ResetSettings(target.clone())));
        }
    });
    if count > 1 {
        ui.add_space(space::XS);
        let note = format!("Acts on the {count} selected photos.");
        ui.label(RichText::new(note).color(palette::TEXT_DIM).small());
    }
}

/// A row's label, in the label column.
fn label(ui: &mut Ui, text: &str) {
    ui.allocate_ui(vec2(LABEL, size::ROW), |ui| {
        ui.set_min_width(LABEL);
        ui.label(RichText::new(text).color(palette::TEXT));
    });
}

/// The white-balance menu, showing the active photo's preset.
fn white_balance(ui: &mut Ui, cx: &mut Cx<'_>, active: PhotoId) {
    let version = cx
        .engine
        .summary(active)
        .map_or(0, |summary| summary.develop_version);
    let quick = &mut cx.ui.right.quick;
    if quick.read != Some((active, version)) {
        quick.read = Some((active, version));
        quick.preset = cx
            .engine
            .develop_settings(active)
            .ok()
            .map(|settings| settings.white_balance.preset);
    }
    let shown = quick.preset.map_or("—", WbPreset::label);
    let mut chosen = None;
    ui.horizontal(|ui| {
        label(ui, "White Balance");
        ComboBox::from_id_salt("quick-white-balance")
            .selected_text(shown)
            .width(ui.available_width() - 4.0)
            .show_ui(ui, |ui| {
                for preset in WbPreset::ALL.into_iter().filter(|preset| *preset != WbPreset::Custom) {
                    if ui.selectable_label(quick_shows(shown, preset), preset.label()).clicked() {
                        chosen = Some(preset);
                    }
                }
            })
            .response
            .accessible_name("Quick white balance");
    });
    if let Some(preset) = chosen {
        cx.act(Action::Develop(DevelopAction::QuickWhiteBalance(Target::Selected, preset)));
    }
}

fn quick_shows(shown: &str, preset: WbPreset) -> bool {
    shown == preset.label()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_named_as_photographers_say_them() {
        assert_eq!(step_text(Adj::Exposure, 1.0 / 3.0), "+1/3");
        assert_eq!(step_text(Adj::Exposure, -1.0), "-1");
        assert_eq!(step_text(Adj::Contrast, 5.0), "+5");
        assert_eq!(step_text(Adj::Clarity, -20.0), "-20");
    }
}
