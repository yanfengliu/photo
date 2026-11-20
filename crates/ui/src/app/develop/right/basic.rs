//! The Basic panel: treatment and profile, white balance (preset, Temp and
//! Tint, the picker and Auto), tone (Exposure to Blacks, with Auto) and
//! presence (Texture to Saturation).

use egui::{Button, ComboBox, RichText, Ui, vec2};
use photo_core::develop::{Adj, DevelopSettings, Profile, Treatment, WbPreset, WhiteBalance};
use photo_core::meta::FileKind;

use super::{Editing, caption, edited, heading, sliders};
use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::develop::actions::DevelopAction;
use crate::app::develop::drafts::Pending;
use crate::theme::{palette, size};
use crate::widgets::{AdjSlider, Icon, IconButton, Scale};

/// The tone sliders, top to bottom.
const TONE: [Adj; 6] = [
    Adj::Exposure,
    Adj::Contrast,
    Adj::Highlights,
    Adj::Shadows,
    Adj::Whites,
    Adj::Blacks,
];

/// The presence sliders, top to bottom.
const PRESENCE: [Adj; 5] = [
    Adj::Texture,
    Adj::Clarity,
    Adj::Dehaze,
    Adj::Vibrance,
    Adj::Saturation,
];

/// Show the panel.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    treatment(ui, cx, editing);
    profile(ui, cx, editing);
    ui.add_space(size::ROW / 4.0);
    white_balance(ui, cx, editing);
    let decoded = editing.decoded;
    caption(ui, "Tone", |ui| {
        let auto = ui
            .add_enabled(decoded, Button::new("Auto").small())
            .accessible_name("Auto Tone")
            .on_hover_text("Set exposure, highlights, shadows, whites and blacks from the photo");
        if auto.clicked() {
            cx.act(Action::Develop(DevelopAction::AutoTone));
        }
    });
    sliders(ui, cx, editing, &TONE, None);
    heading(ui, "Presence");
    sliders(ui, cx, editing, &PRESENCE, None);
}

/// A row with a label in the sliders' label column.
fn row(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui(vec2(size::SLIDER_LABEL, size::ROW), |ui| {
            ui.set_min_width(size::SLIDER_LABEL);
            ui.label(RichText::new(label).color(palette::TEXT));
        });
        content(ui);
    });
}

/// `settings` with one change, committed at once.
fn apply(cx: &mut Cx<'_>, editing: &Editing, change: impl FnOnce(&mut DevelopSettings)) {
    let mut settings = editing.settings.clone();
    change(&mut settings);
    if settings != editing.settings {
        cx.act(Action::Develop(DevelopAction::Apply(Box::new(settings))));
    }
}

fn treatment(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    let current = editing.settings.treatment;
    row(ui, "Treatment", |ui| {
        for (treatment, label) in [
            (Treatment::Color, "Color"),
            (Treatment::BlackAndWhite, "Black & White"),
        ] {
            let button =
                Button::selectable(current == treatment, label).min_size(vec2(0.0, size::ROW));
            if ui.add(button).clicked() && current != treatment {
                apply(cx, editing, |settings| settings.treatment = treatment);
            }
        }
    });
}

fn profile(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    let raw = editing.kind == FileKind::Raw;
    let current = editing.settings.profile;
    let mut chosen = current;
    row(ui, "Profile", |ui| {
        ui.add_enabled_ui(raw, |ui| {
            ComboBox::from_id_salt("develop-profile")
                .selected_text(current.label())
                .width(ui.available_width() - 4.0)
                .show_ui(ui, |ui| {
                    for profile in Profile::ALL {
                        ui.selectable_value(&mut chosen, profile, profile.label());
                    }
                })
                .response
                .accessible_name("Profile")
                .on_disabled_hover_text(
                    "Profiles render camera raw files; this file is already rendered.",
                );
        });
    });
    if chosen != current {
        apply(cx, editing, |settings| settings.profile = chosen);
    }
}

/// The presets a photo of `kind` can take: the illuminants only mean
/// something for a camera raw file.
fn presets(kind: FileKind) -> Vec<WbPreset> {
    WbPreset::ALL
        .into_iter()
        .filter(|preset| kind == FileKind::Raw || preset.illuminant().is_none())
        .collect()
}

fn white_balance(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    let current = editing.settings.white_balance.preset;
    let mut chosen = current;
    ui.horizontal(|ui| {
        let picker =
            IconButton::new(Icon::Picker, "White balance picker").selected(cx.ui.develop.picker);
        let response = ui.add_enabled(editing.decoded, picker);
        if response.clicked() {
            cx.ui.develop.picker = !cx.ui.develop.picker;
        }
        ui.allocate_ui(
            vec2(size::SLIDER_LABEL - size::ICON_BUTTON - 6.0, size::ROW),
            |ui| {
                ui.label(RichText::new("WB").color(palette::TEXT));
            },
        );
        // The Auto button sits at the right edge, the menu fills between.
        let auto_width = 40.0;
        ComboBox::from_id_salt("develop-white-balance")
            .selected_text(current.label())
            .width(ui.available_width() - auto_width - 2.0 * ui.spacing().item_spacing.x)
            .show_ui(ui, |ui| {
                for preset in presets(editing.kind) {
                    ui.selectable_value(&mut chosen, preset, preset.label());
                }
            })
            .response
            .accessible_name("White balance");
        let auto = ui
            .add_enabled(
                editing.decoded,
                Button::new("Auto").small().min_size(vec2(auto_width, 0.0)),
            )
            .accessible_name("Auto White Balance")
            .on_hover_text("Set the white balance from the photo");
        if auto.clicked() {
            cx.act(Action::Develop(DevelopAction::AutoWhiteBalance));
        }
    });
    if chosen != current {
        choose_white_balance(cx, editing, chosen);
    }
    temperature_and_tint(ui, cx, editing);
}

/// A white-balance preset chosen from the menu.
fn choose_white_balance(cx: &mut Cx<'_>, editing: &Editing, preset: WbPreset) {
    match (preset, preset.illuminant()) {
        (WbPreset::Auto, _) => cx.act(Action::Develop(DevelopAction::AutoWhiteBalance)),
        (WbPreset::AsShot, _) => apply(cx, editing, |settings| {
            settings.white_balance = WhiteBalance::default();
        }),
        (_, Some((temp, tint))) => apply(cx, editing, |settings| {
            settings.white_balance = WhiteBalance { preset, temp, tint };
        }),
        (_, None) => {
            if let Some((temp, tint)) = editing.shown_white_balance {
                apply(cx, editing, |settings| {
                    settings.white_balance = WhiteBalance {
                        preset: WbPreset::Custom,
                        temp,
                        tint,
                    };
                });
            }
        }
    }
}

/// Temp and Tint. They show the camera's balance while it is As Shot, and
/// a double-click puts them back to it: when both are back where the
/// camera had them, the balance is As Shot again.
fn temperature_and_tint(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    let Some((temp, tint)) = editing.shown_white_balance else {
        for label in ["Temp", "Tint"] {
            row(ui, label, |ui| {
                ui.label(
                    RichText::new("Reading the camera's balance…")
                        .color(palette::TEXT_DIM)
                        .small(),
                );
            });
        }
        return;
    };
    let as_shot = editing.as_shot.unwrap_or((temp, tint));
    for (adj, value, default, other, other_default) in [
        (Adj::Temperature, temp, as_shot.0, tint, as_shot.1),
        (Adj::Tint, tint, as_shot.1, temp, as_shot.0),
    ] {
        let spec = adj.spec_for(editing.kind);
        let mut shown = value;
        let mut widget = AdjSlider::new(spec, &mut shown, spec.clamp(default));
        if adj == Adj::Temperature && editing.kind == FileKind::Raw {
            // Kelvin spans 2000 to 50000: a linear track would squeeze the
            // useful range into its first tenth.
            widget = widget.scale(Scale::LOG);
        }
        let response = widget.show(ui);
        let one_step = response.changed && response.finished;
        if one_step && shown == spec.clamp(default) && other == other_default {
            apply(cx, editing, |settings| {
                settings.white_balance = WhiteBalance::default()
            });
            continue;
        }
        let draft = response.changed.then_some(Pending::Adjust(adj, shown));
        edited(cx, draft, response.finished);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rendered_file_is_offered_only_the_presets_that_mean_something_for_it() {
        assert_eq!(
            presets(FileKind::Raster),
            [WbPreset::AsShot, WbPreset::Auto, WbPreset::Custom]
        );
        assert_eq!(presets(FileKind::Raw).len(), WbPreset::ALL.len());
    }
}
