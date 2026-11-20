//! The grid view: every photo of the list as a square cell, in rows that
//! fill the work area's width.
//!
//! Only the rows in view are laid out, so a list of any length costs the
//! same per frame. A click selects, Command-click toggles, Shift-click
//! extends; a double-click opens the loupe; a drag carries the selection to
//! a collection; a right-click opens the cell's menu.

use egui::{Align, Button, Rect, RichText, ScrollArea, Ui, UiBuilder, Vec2, vec2};
use photo_core::meta::PhotoId;
use photo_engine::{CollectionKind, Filter, PreviewSize, Source, Target};

use super::{cell, drag};
use crate::app::actions::Action;
use crate::app::state::View;
use crate::app::{Cx, dialogs, dispatch, text};
use crate::theme::{palette, space};
use crate::widgets::ThumbnailResponse;

/// Gap between cells, and around them.
const GAP: f32 = 8.0;
/// The smallest a cell gets, however narrow the work area.
const MIN_EDGE: f32 = 64.0;

/// The number of columns and the edge of a cell for a grid `width` points
/// wide whose cells should be about `wanted` points: as many columns as
/// fit, then the cells widened to fill the row.
pub(crate) fn layout(width: f32, wanted: f32) -> (usize, f32) {
    let columns = ((width - GAP) / (wanted + GAP)).floor().max(1.0) as usize;
    let edge = ((width - GAP) / columns as f32 - GAP).floor().max(MIN_EDGE);
    (columns, edge)
}

/// What the user did to a cell this frame.
struct CellEvent {
    photo: PhotoId,
    response: ThumbnailResponse,
}

/// Show the grid, or what to do when it has nothing to show.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let count = cx.engine.photo_count();
    if count == 0 {
        empty(ui, cx);
        return;
    }
    let reveal = cx.ui.grid.reveal_active;
    let active = cx.engine.active_photo().and_then(|photo| cx.engine.index_of(photo));
    let output = ScrollArea::vertical()
        .id_salt("grid")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            let (columns, edge) = layout(ui.available_width(), cx.ui.thumb_size);
            cx.ui.grid.columns = columns;
            let step = edge + GAP;
            let rows = count.div_ceil(columns);
            ui.set_height(GAP + rows as f32 * step);
            let origin = ui.max_rect().min;
            let at = |index: usize| {
                let (row, column) = (index / columns, index % columns);
                let min = origin + vec2(GAP + column as f32 * step, GAP + row as f32 * step);
                Rect::from_min_size(min, Vec2::splat(edge))
            };
            if reveal && let Some(index) = active {
                ui.scroll_to_rect(at(index).expand(GAP), None);
            }
            let first = (((viewport.min.y - GAP) / step).floor().max(0.0) as usize) * columns;
            let last = ((((viewport.max.y - GAP) / step).ceil().max(0.0) as usize + 1) * columns)
                .min(count);
            let photos = cx.engine.photos()[first.min(last)..last].to_vec();
            let mut events = Vec::new();
            for (offset, photo) in photos.iter().enumerate() {
                let index = first + offset;
                let name = text::photo_name(photo);
                let rect = at(index);
                let builder = UiBuilder::new().max_rect(rect).id_salt(("cell", photo.id));
                let response = ui
                    .scope_builder(builder, |ui| {
                        cell(ui, cx, photo, &name, rect.size(), PreviewSize::Thumbnail)
                            .index(index + 1)
                            .show(ui)
                    })
                    .inner;
                menu(cx, &response.response);
                if response.clicked
                    || response.double_clicked
                    || response.secondary_clicked
                    || response.drag_started
                {
                    events.push(CellEvent {
                        photo: photo.id,
                        response,
                    });
                }
            }
            events
        });
    for event in output.inner {
        handle(ui.ctx(), cx, event);
    }
}

/// React to a click, double-click, context click or drag on a cell.
fn handle(ctx: &egui::Context, cx: &mut Cx<'_>, event: CellEvent) {
    let CellEvent { photo, response } = event;
    if response.double_clicked {
        dispatch::open_view(cx, View::Loupe);
    } else if response.drag_started {
        drag::start(ctx, cx, photo);
    } else if response.secondary_clicked {
        // The menu acts on the selection: a photo outside it is selected
        // alone first.
        if !cx.engine.selection().is_selected(photo) {
            cx.act(Action::SelectOnly(photo));
        }
    } else if response.clicked {
        cx.act(select(photo, response.modifiers));
    }
}

/// What a click with `modifiers` on `photo` does to the selection.
pub(super) fn select(photo: PhotoId, modifiers: egui::Modifiers) -> Action {
    if modifiers.shift {
        Action::ExtendSelection(photo)
    } else if modifiers.command {
        Action::ToggleSelected(photo)
    } else {
        Action::SelectOnly(photo)
    }
}

/// The menu a right-click on a cell opens. It acts on the selection.
fn menu(cx: &mut Cx<'_>, response: &egui::Response) {
    egui::Popup::context_menu(response).show(|ui| {
        let quick = cx
            .engine
            .active_photo()
            .and_then(|photo| cx.engine.summary(photo))
            .is_some_and(|summary| summary.in_quick_collection);
        let quick_label = if quick {
            "Remove from Quick Collection"
        } else {
            "Add to Quick Collection"
        };
        let target = Target::Selected;
        if ui.button(quick_label).clicked() {
            cx.act(Action::ToggleQuickCollection(target.clone()));
        }
        if ui.button("Create Virtual Copy").clicked() {
            cx.act(Action::CreateVirtualCopies(target.clone()));
        }
        ui.separator();
        if ui.button("Rotate Left").clicked() {
            cx.act(Action::RotateLeft(target.clone()));
        }
        if ui.button("Rotate Right").clicked() {
            cx.act(Action::RotateRight(target.clone()));
        }
        ui.separator();
        if let Source::Collection(id) = *cx.engine.source()
            && cx
                .engine
                .collections()
                .iter()
                .any(|c| c.id == id && c.kind == CollectionKind::Regular)
            && ui.button("Remove from Collection").clicked()
        {
            cx.act(Action::RemoveFromCollection(id, target.clone()));
        }
        if ui.button("Remove from Catalog…").clicked() {
            dialogs::open_remove_photos(cx, target);
        }
    });
}

/// The grid has nothing to show: say why, and offer the way on.
fn empty(ui: &mut Ui, cx: &mut Cx<'_>) {
    let catalog_empty = cx.engine.source_counts().all_photos == 0;
    let filtered = !cx.engine.filter().is_empty();
    let (title, hint) = if catalog_empty {
        (
            "Import photos to get started",
            "Photos are added where they are, or copied into folders by date.",
        )
    } else if filtered {
        ("No photo matches the filter", "Loosen the filter, or turn it off.")
    } else {
        match *cx.engine.source() {
            Source::QuickCollection => (
                "The Quick Collection is empty",
                "Press B to put the selected photos in it.",
            ),
            Source::Collection(id) => {
                let kind = cx
                    .engine
                    .collections()
                    .iter()
                    .find(|collection| collection.id == id)
                    .map(|collection| collection.kind);
                match kind {
                    Some(CollectionKind::Smart) => (
                        "No photo matches its rules",
                        "Edit the rules from the smart collection's menu in the Collections panel.",
                    ),
                    Some(CollectionKind::Set) => (
                        "Nothing in this collection set yet",
                        "Its collections' photos show here.",
                    ),
                    _ => (
                        "This collection is empty",
                        "Drag photos onto it in the Collections panel to add them.",
                    ),
                }
            }
            Source::PreviousImport => ("Nothing has been imported yet", ""),
            _ => ("No photos here", ""),
        }
    };
    ui.with_layout(egui::Layout::top_down(Align::Center), |ui| {
        ui.add_space((ui.available_height() / 2.0 - 60.0).max(space::XL));
        ui.label(
            RichText::new(title)
                .size(18.0)
                .color(palette::TEXT_STRONG),
        );
        ui.add_space(space::MD);
        if !hint.is_empty() {
            ui.label(RichText::new(hint).color(palette::TEXT_DIM));
        }
        ui.add_space(space::XL);
        if catalog_empty {
            let button = Button::new(RichText::new("Import Photos…").color(palette::TEXT_STRONG))
                .fill(palette::ACCENT_DIM)
                .min_size(vec2(140.0, 28.0));
            if ui.add(button).clicked() {
                dialogs::open_import(cx);
            }
        } else if filtered && ui.button("Turn Off the Filter").clicked() {
            cx.act(Action::SetFilter(Filter::default()));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_widen_to_fill_the_row() {
        let (columns, edge) = layout(1000.0, 168.0);
        assert_eq!(columns, 5);
        let used = GAP + columns as f32 * (edge + GAP);
        assert!(used <= 1000.0 && 1000.0 - used < columns as f32, "{used}");
        assert!(edge >= 168.0);
    }

    #[test]
    fn a_narrow_grid_still_has_one_column_of_usable_cells() {
        assert_eq!(layout(40.0, 168.0), (1, MIN_EDGE));
        assert_eq!(layout(200.0, 168.0).0, 1);
    }
}
