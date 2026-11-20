//! The Color Mixer (hue, saturation and luminance per band, or the black
//! and white mix when the treatment is black and white) and Color Grading
//! (the shadows, midtones, highlights and global wheels with blending and
//! balance).

use egui::{Align, Button, Layout, RichText, Ui, vec2};
use photo_core::develop::{Adj, ColorBand, GradeWheel, Treatment};

use super::{Editing, heading, slider, sliders};
use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::develop::actions::DevelopAction;
use crate::app::develop::state::MixerView;
use crate::theme::{palette, space};
use crate::widgets::ColorWheel;

/// The mixer's views: what each button shows and its accessible name.
const VIEWS: [(MixerView, &str, &str); 4] = [
    (MixerView::Hue, "Hue", "Mixer Hue"),
    (MixerView::Saturation, "Saturation", "Mixer Saturation"),
    (MixerView::Luminance, "Luminance", "Mixer Luminance"),
    (MixerView::All, "All", "Mixer All"),
];

/// The Color Mixer panel.
pub(super) fn mixer(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    if editing.settings.treatment == Treatment::BlackAndWhite {
        ui.label(
            RichText::new("How light each colour turns in black and white.")
                .color(palette::TEXT_DIM)
                .small(),
        );
        ui.add_space(space::SM);
        band_sliders(ui, cx, editing, Adj::MixBw, "B&W");
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        for (view, text, name) in VIEWS {
            let selected = cx.ui.develop.mixer == view;
            if ui
                .add(Button::selectable(selected, text))
                .accessible_name(name)
                .clicked()
            {
                cx.ui.develop.mixer = view;
            }
        }
    });
    ui.add_space(space::SM);
    let families: [(MixerView, fn(ColorBand) -> Adj, &str); 3] = [
        (MixerView::Hue, Adj::MixHue, "Hue"),
        (MixerView::Saturation, Adj::MixSaturation, "Saturation"),
        (MixerView::Luminance, Adj::MixLuminance, "Luminance"),
    ];
    let shown = cx.ui.develop.mixer;
    for (view, family, name) in families {
        if shown == MixerView::All {
            heading(ui, name);
        }
        if shown == view || shown == MixerView::All {
            band_sliders(ui, cx, editing, family, name);
        }
    }
}

/// One slider per band of `family`, named after `prefix` and the band.
fn band_sliders(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    editing: &Editing,
    family: fn(ColorBand) -> Adj,
    prefix: &str,
) {
    for band in ColorBand::ALL {
        slider(ui, cx, editing, family(band), Some(prefix));
    }
}

/// The Color Grading panel.
pub(super) fn grading(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    let mut settings = editing.settings.clone();
    let mut changed = false;
    let mut finished = false;
    {
        let grading = &mut settings.color_grading;
        ui.horizontal_top(|ui| {
            // Whole points, so the three columns sit on the pixel grid.
            let gaps = 2.0 * ui.spacing().item_spacing.x;
            let column = vec2(((ui.available_width() - gaps) / 3.0).floor(), 0.0);
            let diameter = (column.x - 12.0).clamp(56.0, 96.0);
            for (name, wheel, luminance) in [
                ("Shadows", &mut grading.shadows, Adj::GradeShadowsLuminance),
                ("Midtones", &mut grading.midtones, Adj::GradeMidtonesLuminance),
                ("Highlights", &mut grading.highlights, Adj::GradeHighlightsLuminance),
            ] {
                ui.allocate_ui_with_layout(column, Layout::top_down(Align::Min), |ui| {
                    let (now_changed, now_finished) = wheel_ui(ui, name, wheel, luminance, diameter);
                    changed |= now_changed;
                    finished |= now_finished;
                });
            }
        });
        ui.add_space(space::MD);
        ui.vertical_centered(|ui| {
            let width = (ui.available_width() * 0.5).floor();
            ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
                let diameter = (width - 12.0).clamp(72.0, 120.0);
                let (now_changed, now_finished) = wheel_ui(
                    ui,
                    "Global",
                    &mut grading.global,
                    Adj::GradeGlobalLuminance,
                    diameter,
                );
                changed |= now_changed;
                finished |= now_finished;
            });
        });
    }
    if changed {
        cx.act(Action::Develop(DevelopAction::Edit(Box::new(settings))));
    }
    if finished {
        cx.act(Action::Develop(DevelopAction::Commit));
    }
    ui.add_space(space::SM);
    sliders(ui, cx, editing, &[Adj::GradeBlending, Adj::GradeBalance], None);
}

/// One wheel. Returns whether it changed and whether an edit finished.
fn wheel_ui(
    ui: &mut Ui,
    name: &str,
    wheel: &mut GradeWheel,
    luminance: Adj,
    diameter: f32,
) -> (bool, bool) {
    let response = ColorWheel::new(name, wheel, luminance.spec())
        .diameter(diameter)
        .show(ui);
    (response.changed, response.finished)
}
