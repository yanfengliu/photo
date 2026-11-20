//! The filmstrip across the bottom of the window: the source line (where
//! the photos come from, how many pass the filter, how many are selected)
//! and the list's photos as small cells, of which only those in view are
//! laid out.

use egui::{Align, Layout, Margin, Rect, RichText, ScrollArea, Ui, UiBuilder, Vec2, vec2};
use photo_core::meta::PhotoId;
use photo_engine::{Engine, Filter, PreviewSize, Source};

use super::{cell, drag, grid, prefixed};
use crate::app::actions::Action;
use crate::app::shell::Edge;
use crate::app::state::View;
use crate::app::{Cx, dispatch, text};
use crate::theme::{self, palette, space};
use crate::widgets::{IconButton, ThumbnailResponse};

/// Edge of a filmstrip cell.
const CELL: f32 = 64.0;
/// Gap between filmstrip cells.
const GAP: f32 = 4.0;
/// Height of the source line.
const LINE: f32 = 22.0;

/// Show the filmstrip panel.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let frame = theme::panel_frame().inner_margin(Margin::symmetric(8, 4));
    egui::Panel::bottom("filmstrip")
        .exact_size(LINE + CELL + 18.0)
        .resizable(false)
        .frame(frame)
        .show(ui, |ui| {
            source_line(ui, cx);
            strip(ui, cx);
        });
}

/// "Previous Import · 6 of 6 photos, 1 selected", the filter in force, and
/// the button that hides the filmstrip.
fn source_line(ui: &mut Ui, cx: &mut Cx<'_>) {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), LINE));
    let mut line = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    line.label(RichText::new(text::source_name(cx.engine)).color(palette::TEXT));
    line.add_space(space::MD);
    let summary = counts(cx.engine);
    line.label(RichText::new(summary).color(palette::TEXT_DIM));
    if let Some(name) = cx
        .engine
        .active_photo()
        .and_then(|photo| cx.engine.summary(photo))
        .map(text::photo_name)
    {
        line.add_space(space::MD);
        line.label(RichText::new(name).color(palette::TEXT_DIM));
    }
    line.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let (icon, label) = Edge::Bottom.button(true);
        if ui.add(IconButton::new(icon, label).side(18.0)).clicked() {
            cx.ui.layout.filmstrip = false;
        }
        if !cx.engine.filter().is_empty() {
            ui.add_space(space::MD);
            if ui.small_button("Turn Off Filter").clicked() {
                cx.act(Action::SetFilter(Filter::default()));
            }
            ui.label(RichText::new("Filtered").color(palette::ACCENT));
        }
    });
}

/// "N of M photos, K selected": the photos shown, the photos in the source
/// before the filter, and the selection.
pub(super) fn counts(engine: &Engine) -> String {
    let shown = engine.photo_count();
    let total = if engine.filter().is_empty() {
        shown as u64
    } else {
        source_total(engine).unwrap_or(shown as u64)
    };
    let selected = engine.selection().selected().len();
    let total = text::count(
        usize::try_from(total).unwrap_or(usize::MAX),
        "photo",
        "photos",
    );
    format!("{shown} of {total}, {selected} selected")
}

/// How many photos the current source holds before any filter, from the
/// counts the panels already have. `None` where no panel counts it.
fn source_total(engine: &Engine) -> Option<u64> {
    let counts = engine.source_counts();
    match *engine.source() {
        Source::AllPhotos => Some(counts.all_photos),
        Source::QuickCollection => Some(counts.quick_collection),
        Source::PreviousImport => Some(counts.previous_import),
        Source::Collection(id) => engine
            .collections()
            .iter()
            .find(|collection| collection.id == id)
            .map(|collection| collection.photo_count),
        Source::Folder {
            id,
            include_subfolders,
        } => {
            let folder = engine.folders().iter().find(|folder| folder.id == id)?;
            if !include_subfolders {
                return Some(folder.photo_count);
            }
            let total = engine
                .folders()
                .iter()
                .filter(|other| other.path.starts_with(&folder.path))
                .map(|other| other.photo_count)
                .sum();
            Some(total)
        }
        Source::Keyword(_) => None,
    }
}

/// The row of cells, scrolled to the active photo when it changes.
fn strip(ui: &mut Ui, cx: &mut Cx<'_>) {
    let count = cx.engine.photo_count();
    let reveal = cx.ui.grid.reveal_active;
    let active = cx
        .engine
        .active_photo()
        .and_then(|photo| cx.engine.index_of(photo));
    let output = ScrollArea::horizontal()
        .id_salt("filmstrip")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            let step = CELL + GAP;
            ui.set_width(count as f32 * step);
            let origin = ui.max_rect().min;
            let at = |index: usize| {
                Rect::from_min_size(origin + vec2(index as f32 * step, 0.0), Vec2::splat(CELL))
            };
            if reveal && let Some(index) = active {
                ui.scroll_to_rect(at(index), None);
            }
            let first = ((viewport.min.x / step).floor().max(0.0) as usize).min(count);
            let last = ((viewport.max.x / step).ceil().max(0.0) as usize + 1).min(count);
            let photos = cx.engine.photos()[first..last.max(first)].to_vec();
            let mut events: Vec<(PhotoId, ThumbnailResponse)> = Vec::new();
            for (offset, photo) in photos.iter().enumerate() {
                let rect = at(first + offset);
                let name = prefixed("Filmstrip", photo);
                let builder = UiBuilder::new().max_rect(rect).id_salt(("film", photo.id));
                let response = ui
                    .scope_builder(builder, |ui| {
                        cell(ui, cx, photo, &name, rect.size(), PreviewSize::Thumbnail).show(ui)
                    })
                    .inner;
                if response.clicked || response.double_clicked || response.drag_started {
                    events.push((photo.id, response));
                }
            }
            events
        });
    for (photo, response) in output.inner {
        if response.double_clicked {
            dispatch::open_view(cx, View::Loupe);
        } else if response.drag_started {
            drag::start(ui.ctx(), cx, photo);
        } else if response.clicked {
            cx.act(grid::select(photo, response.modifiers));
        }
    }
}
