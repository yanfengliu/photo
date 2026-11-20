//! The right panel column: Histogram, Quick Develop, Keywording, Keyword
//! List and Metadata, all about the active photo (Quick Develop and
//! Keywording about the selection).

mod keywords;
mod metadata;
mod quick;

use egui::{RichText, Ui};
use photo_engine::Histogram;

pub(crate) use self::keywords::KeywordState;
pub(crate) use self::metadata::MetadataDraft;
use self::quick::QuickState;
use crate::app::{Cx, text};
use crate::theme::{self, palette, space};
use crate::widgets::{HistogramData, Panel, histogram_readout, panel_column};

/// Width of the column unless the user drags it wider or narrower.
const WIDTH: f32 = 300.0;
/// Height of the histogram.
const HISTOGRAM: f32 = 96.0;

/// What the right column remembers between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct RightState {
    pub(crate) keywords: KeywordState,
    pub(crate) metadata: MetadataDraft,
    pub(crate) quick: QuickState,
}

/// Show the right column.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    egui::Panel::right("right-column")
        .resizable(true)
        .size_range(240.0..=440.0)
        .default_size(WIDTH)
        .frame(theme::panel_frame())
        .show(ui, |ui| {
            let mut panels = std::mem::take(&mut cx.ui.right_panels);
            panel_column(ui, "right-panels", |ui| {
                Panel::new("Histogram").show(ui, &mut panels, |ui| histogram_panel(ui, cx));
                Panel::new("Quick Develop").show(ui, &mut panels, |ui| quick::show(ui, cx));
                Panel::new("Keywording").show(ui, &mut panels, |ui| keywords::keywording(ui, cx));
                Panel::new("Keyword List").show(ui, &mut panels, |ui| keywords::list(ui, cx));
                Panel::new("Metadata").show(ui, &mut panels, |ui| metadata::show(ui, cx));
            });
            cx.ui.right_panels = panels;
        });
}

/// The histogram of the active photo's standard preview, and its camera
/// readings.
fn histogram_panel(ui: &mut Ui, cx: &mut Cx<'_>) {
    let active = cx.engine.active_photo();
    let data = active
        .and_then(|photo| cx.engine.histogram(photo))
        .map(|histogram| counts(&histogram))
        .unwrap_or_default();
    histogram_readout(ui, &data, HISTOGRAM);
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
                        .filter(|shutter| !shutter.is_empty()),
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

/// The engine's histogram as the widget draws it.
fn counts(histogram: &Histogram) -> HistogramData {
    HistogramData {
        red: histogram.red,
        green: histogram.green,
        blue: histogram.blue,
        luma: histogram.luma,
    }
}
