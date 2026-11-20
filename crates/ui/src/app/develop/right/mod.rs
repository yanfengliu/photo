//! The Develop module's right column: the histogram, then the editing
//! panels in their fixed order, with Previous (Sync when several photos
//! are selected) and Reset at the bottom.
//!
//! Every slider is built from the `Adj` table: a drag edits in memory with
//! draft renders and commits once when it is let go; a reset, a nudge or a
//! typed number is a complete edit at once. While the photo cannot be
//! edited, every editing control is disabled and the reason is shown.

mod basic;
pub(crate) mod curve_math;
mod color;
mod curve;
mod detail;

use egui::{Align, Button, Layout, Margin, RichText, Ui, vec2};
use photo_core::develop::{Adj, DevelopSettings};
use photo_core::meta::{FileKind, PhotoId};
use photo_engine::{DevelopSession, Target};

use super::actions::DevelopAction;
use crate::app::actions::Action;
use crate::app::{Cx, text};
use crate::theme::{self, palette, space};
use crate::widgets::{AdjSlider, HistogramData, Icon, Panel, histogram, panel_column};

/// Width of the column unless the user drags it wider or narrower.
const WIDTH: f32 = 316.0;
/// Height of the histogram.
const HISTOGRAM: f32 = 112.0;

/// What the panels edit: the open photo's settings as shown, including a
/// drag not yet committed.
pub(crate) struct Editing {
    pub(crate) photo: PhotoId,
    pub(crate) kind: FileKind,
    pub(crate) settings: DevelopSettings,
    /// The temperature and tint the Basic panel shows; `None` while a RAW
    /// photo shot with its camera's balance is still being read.
    pub(crate) shown_white_balance: Option<(f32, f32)>,
    /// What As Shot means for this photo, once known.
    pub(crate) as_shot: Option<(f32, f32)>,
    /// Why the photo cannot be edited, if it cannot.
    pub(crate) read_only: Option<String>,
    /// Whether the file is decoded (the automatic tools need it).
    pub(crate) decoded: bool,
}

impl Editing {
    /// What the panels edit in `session`.
    pub(crate) fn of(session: &DevelopSession) -> Self {
        let as_shot = match session.kind() {
            FileKind::Raster => Some((0.0, 0.0)),
            FileKind::Raw => session.as_shot_white_balance(),
        };
        Self {
            photo: session.photo(),
            kind: session.kind(),
            settings: session.settings().clone(),
            shown_white_balance: session.shown_white_balance(),
            as_shot,
            read_only: session.read_only_reason().map(str::to_owned),
            decoded: session.output_size().is_some(),
        }
    }

    pub(crate) fn editable(&self) -> bool {
        self.read_only.is_none()
    }
}

/// Show the right column.
pub(crate) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let editing = cx
        .engine
        .develop()
        .filter(|session| Some(session.photo()) == cx.engine.active_photo())
        .map(Editing::of);
    egui::Panel::right("develop-right-column")
        .resizable(true)
        .size_range(260.0..=460.0)
        .default_size(WIDTH)
        .frame(theme::panel_frame())
        .show(ui, |ui| {
            bottom(ui, cx, editing.as_ref());
            if let Some(reason) = editing.as_ref().and_then(|editing| editing.read_only.as_deref()) {
                read_only_banner(ui, reason);
            }
            let mut panels = std::mem::take(&mut cx.ui.develop.right_panels);
            panel_column(ui, "develop-right-panels", |ui| {
                Panel::new("Histogram").show(ui, &mut panels, |ui| histogram_panel(ui, cx));
                // Part 2 puts the tool strip (crop, healing, masking) here,
                // under the histogram.
                let Some(editing) = editing.as_ref() else {
                    ui.add_space(space::LG);
                    ui.label(RichText::new("Open a photo to edit it.").color(palette::TEXT_DIM));
                    return;
                };
                ui.add_enabled_ui(editing.editable(), |ui| {
                    Panel::new("Basic").show(ui, &mut panels, |ui| basic::show(ui, cx, editing));
                    Panel::new("Tone Curve").show(ui, &mut panels, |ui| curve::show(ui, cx, editing));
                    Panel::new("Color Mixer").show(ui, &mut panels, |ui| color::mixer(ui, cx, editing));
                    Panel::new("Color Grading")
                        .show(ui, &mut panels, |ui| color::grading(ui, cx, editing));
                    Panel::new("Detail").show(ui, &mut panels, |ui| detail::detail(ui, cx, editing));
                    Panel::new("Lens Corrections")
                        .show(ui, &mut panels, |ui| detail::lens(ui, cx, editing));
                    Panel::new("Transform")
                        .show(ui, &mut panels, |ui| detail::transform(ui, cx, editing));
                    Panel::new("Effects").show(ui, &mut panels, |ui| detail::effects(ui, cx, editing));
                    Panel::new("Calibration")
                        .show(ui, &mut panels, |ui| detail::calibration(ui, cx, editing));
                });
            });
            cx.ui.develop.right_panels = panels;
        });
}

/// Previous (or Sync with several photos selected) and Reset.
fn bottom(ui: &mut Ui, cx: &mut Cx<'_>, editing: Option<&Editing>) {
    let frame = theme::panel_frame().inner_margin(Margin::same(10));
    let editable = editing.is_some_and(Editing::editable);
    let several = cx.engine.selection().selected().len() > 1;
    let previous = cx.engine.previous_photo().is_some();
    egui::Panel::bottom("develop-right-bottom")
        .resizable(false)
        .show_separator_line(false)
        .frame(frame)
        .show(ui, |ui| {
            ui.columns(2, |columns| {
                let width = columns[0].available_width();
                let button = |text: &str| Button::new(text).min_size(vec2(width, 26.0));
                columns[0].add_enabled_ui(editable, |ui| {
                    if several {
                        if ui.add(button("Sync…")).clicked() {
                            crate::app::dialogs::open_sync(cx);
                        }
                    } else if ui
                        .add_enabled(previous, button("Previous"))
                        .on_hover_text("Give this photo all the settings of the photo before it")
                        .clicked()
                    {
                        cx.act(Action::Develop(DevelopAction::PasteFromPrevious(Target::Active)));
                    }
                });
                columns[1].add_enabled_ui(editable, |ui| {
                    if ui
                        .add(button("Reset"))
                        .on_hover_text("Put every setting of this photo back to how it was imported")
                        .clicked()
                    {
                        cx.act(Action::Develop(DevelopAction::ResetSettings(Target::Active)));
                    }
                });
            });
        });
}

/// Why nothing can be edited, above the panels.
fn read_only_banner(ui: &mut Ui, reason: &str) {
    egui::Frame::new()
        .fill(theme::mix(palette::PANEL, palette::WARNING, 0.12))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let (_, icon) = ui.allocate_space(vec2(16.0, 16.0));
                Icon::Lock.paint(ui.painter(), icon, palette::WARNING);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Read only").color(palette::WARNING).strong());
                    ui.label(RichText::new(text::sentence(reason)).color(palette::TEXT));
                });
            });
        });
}

/// The histogram of the frame on screen, with its clipping toggles, and the
/// camera's readings under it.
fn histogram_panel(ui: &mut Ui, cx: &mut Cx<'_>) {
    let active = cx.engine.active_photo();
    let data = match frame_histogram(cx) {
        Some(data) => data,
        None => active
            .and_then(|photo| cx.engine.histogram(photo))
            .map(|histogram| counts(&histogram))
            .unwrap_or_default(),
    };
    histogram(ui, &data, &mut cx.ui.develop.clipping, HISTOGRAM);
    ui.add_space(space::XS);
    let generation = cx.generation;
    let readings: Vec<String> = active
        .and_then(|photo| cx.ui.record.get(cx.engine, photo, generation))
        .map(|record| {
            let metadata = &record.metadata;
            metadata
                .iso
                .map(|iso| format!("ISO {iso}"))
                .into_iter()
                .chain(text::focal_length(metadata))
                .chain(metadata.aperture.map(|f| format!("f/{f}")))
                .chain(
                    metadata
                        .shutter_seconds
                        .map(photo_core::meta::format_shutter)
                        .filter(|shutter| !shutter.is_empty())
                        .map(|shutter| format!("{shutter} s")),
                )
                .collect()
        })
        .unwrap_or_default();
    ui.horizontal(|ui| {
        if readings.is_empty() {
            ui.label(RichText::new(" ").small());
        }
        for reading in readings {
            ui.label(RichText::new(reading).color(palette::TEXT_DIM).small());
            ui.add_space(space::MD);
        }
    });
}

/// The histogram of the open photo's frame on screen.
pub(crate) fn frame_histogram(cx: &Cx<'_>) -> Option<HistogramData> {
    let active = cx.engine.active_photo();
    cx.engine
        .develop()
        .filter(|session| Some(session.photo()) == active)
        .and_then(|session| session.frame())
        .map(|frame| counts(frame.histogram()))
}

/// The engine's histogram as the widget draws it.
fn counts(histogram: &photo_engine::Histogram) -> HistogramData {
    HistogramData {
        red: histogram.red,
        green: histogram.green,
        blue: histogram.blue,
        luma: histogram.luma,
    }
}

/// The label column of the panels whose labels run long.
pub(crate) const WIDE_LABEL: f32 = 104.0;

/// One develop slider bound to `adj`: a drag is a draft until it is let
/// go, which commits it. `prefix` tells sliders with the same label apart.
pub(crate) fn slider(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    editing: &Editing,
    adj: Adj,
    prefix: Option<&str>,
) {
    slider_in(ui, cx, editing, adj, prefix, None);
}

/// [`slider`] with a label column `label_width` wide.
pub(crate) fn slider_in(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    editing: &Editing,
    adj: Adj,
    prefix: Option<&str>,
    label_width: Option<f32>,
) {
    let spec = adj.spec_for(editing.kind);
    let mut value = adj.get(&editing.settings);
    let mut widget = AdjSlider::new(spec, &mut value, adj.default_for(editing.kind));
    if let Some(prefix) = prefix {
        widget = widget.name_prefix(prefix);
    }
    if let Some(width) = label_width {
        widget = widget.label_width(width);
    }
    let response = widget.show(ui);
    if response.changed {
        cx.act(Action::Develop(DevelopAction::Adjust(adj, value)));
    }
    if response.finished {
        cx.act(Action::Develop(DevelopAction::Commit));
    }
}

/// Several sliders, one under the other, with one prefix.
pub(crate) fn sliders(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    editing: &Editing,
    adjs: &[Adj],
    prefix: Option<&str>,
) {
    for adj in adjs {
        slider(ui, cx, editing, *adj, prefix);
    }
}

/// [`sliders`] with a label column `label_width` wide.
pub(crate) fn sliders_in(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    editing: &Editing,
    adjs: &[Adj],
    label_width: f32,
) {
    for adj in adjs {
        slider_in(ui, cx, editing, *adj, None, Some(label_width));
    }
}

/// A caption over a group of sliders, with room at its right for a button
/// (`extra`).
pub(crate) fn caption<R>(ui: &mut Ui, text: &str, extra: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(space::SM);
    ui.horizontal(|ui| {
        ui.label(RichText::new(text).color(palette::TEXT_DIM).small());
        ui.with_layout(Layout::right_to_left(Align::Center), extra)
            .inner
    })
    .inner
}

/// A caption with nothing beside it.
pub(crate) fn heading(ui: &mut Ui, text: &str) {
    caption(ui, text, |_| ());
}
