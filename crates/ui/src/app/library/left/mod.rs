//! The left panel column: Navigator, Catalog, Folders and Collections, with
//! the Import button at the bottom.

mod collections;
mod folders;

use std::collections::HashSet;

use egui::{
    Align, Button, Color32, CornerRadius, Layout, Margin, Rect, RichText, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
use photo_core::meta::{CollectionId, FolderId};
use photo_engine::{PreviewSize, Source, Target};

use super::DraggedPhotos;
use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::{dialogs, text};
use crate::theme::{self, palette, radius, space};
use crate::widgets::{Icon, ListRow, Panel, panel_column};

/// Width of the column unless the user drags it wider or narrower.
const WIDTH: f32 = 264.0;
/// Height of the navigator's well.
const NAVIGATOR: f32 = 150.0;

/// What the left column remembers between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct LeftState {
    /// Collection sets the user has closed.
    pub(crate) closed_sets: HashSet<CollectionId>,
    /// Folders the user has closed.
    pub(crate) closed_folders: HashSet<FolderId>,
}

/// Show the left column.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    egui::Panel::left("left-column")
        .resizable(true)
        .size_range(220.0..=420.0)
        .default_size(WIDTH)
        .frame(theme::panel_frame())
        .show(ui, |ui| {
            let frame = theme::panel_frame().inner_margin(Margin::same(10));
            egui::Panel::bottom("left-bottom")
                .resizable(false)
                .show_separator_line(false)
                .frame(frame)
                .show(ui, |ui| {
                    // Centred: a wide button lays its label out by the
                    // layout's cross alignment.
                    ui.with_layout(Layout::top_down(Align::Center), |ui| {
                        let label = RichText::new("Import…").color(palette::TEXT_STRONG);
                        let button = Button::new(label)
                            .fill(palette::ACCENT_DIM)
                            .min_size(vec2(ui.available_width(), 26.0));
                        if ui.add(button).clicked() {
                            dialogs::open_import(cx);
                        }
                    });
                });
            let mut panels = std::mem::take(&mut cx.ui.left_panels);
            panel_column(ui, "left-panels", |ui| {
                Panel::new("Navigator").show(ui, &mut panels, |ui| navigator(ui, cx));
                Panel::new("Catalog").show(ui, &mut panels, |ui| catalog(ui, cx));
                Panel::new("Folders").show(ui, &mut panels, |ui| folders::show(ui, cx));
                Panel::new("Collections").show(ui, &mut panels, |ui| collections::show(ui, cx));
            });
            cx.ui.left_panels = panels;
        });
}

/// The active photo, small, with the part the loupe shows outlined.
fn navigator(ui: &mut Ui, cx: &mut Cx<'_>) {
    let (_, well) = ui.allocate_space(vec2(ui.available_width(), NAVIGATOR));
    ui.painter()
        .rect_filled(well, CornerRadius::same(radius::MD), palette::WELL);
    let active = cx
        .engine
        .active_photo()
        .and_then(|photo| cx.engine.summary(photo).cloned());
    let Some(photo) = active else {
        ui.painter().text(
            well.center(),
            egui::Align2::CENTER_CENTER,
            "No photo selected",
            theme::small_font(),
            palette::TEXT_OFF,
        );
        return;
    };
    let shown = cx
        .textures
        .photo(ui.ctx(), cx.engine, photo.id, PreviewSize::Thumbnail);
    let frame = well.shrink(space::LG);
    if let Some(shown) = shown {
        let size = if frame.width() / frame.height() > shown.aspect {
            vec2(frame.height() * shown.aspect, frame.height())
        } else {
            vec2(frame.width(), frame.width() / shown.aspect)
        };
        let image = Rect::from_center_size(frame.center(), size);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(shown.id, image, uv, Color32::WHITE);
        ui.painter().rect_stroke(
            image,
            0.0,
            Stroke::new(1.0, palette::SEPARATOR),
            StrokeKind::Outside,
        );
        if let Some(region) = cx.ui.loupe_region {
            let seen = Rect::from_min_max(
                image.lerp_inside(region.min.to_vec2()),
                image.lerp_inside(region.max.to_vec2()),
            );
            ui.painter().rect_stroke(
                seen,
                0.0,
                Stroke::new(1.0, palette::TEXT_STRONG),
                StrokeKind::Inside,
            );
        }
    }
    ui.add_space(space::XS);
    ui.label(
        RichText::new(text::photo_name(&photo))
            .color(palette::TEXT_DIM)
            .small(),
    );
}

/// All Photographs, the Quick Collection (which takes drops) and Previous
/// Import, with their counts.
fn catalog(ui: &mut Ui, cx: &mut Cx<'_>) {
    let counts = cx.engine.source_counts();
    let current = *cx.engine.source();
    for (source, name, count) in [
        (Source::AllPhotos, "All Photographs", counts.all_photos),
        (Source::QuickCollection, "Quick Collection", counts.quick_collection),
        (Source::PreviousImport, "Previous Import", counts.previous_import),
    ] {
        let quick = source == Source::QuickCollection;
        let row = ListRow::new(name, name)
            .icon(if quick { Icon::Collection } else { Icon::Grid })
            .count(count)
            .selected(current == source)
            .accepts_drop(quick)
            .show(ui);
        if quick && let Some(dragged) = row.response.dnd_release_payload::<DraggedPhotos>() {
            let quick = cx.engine.quick_collection();
            cx.act(Action::AddToCollection(quick, Target::Photos(dragged.0.clone())));
        } else if row.clicked {
            cx.act(Action::SetSource(source));
        }
    }
}
