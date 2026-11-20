//! The gallery's centre: the grid of photo cells, the toolbar under it and
//! the filmstrip across the bottom of the window.

use egui::{Margin, Modifiers, RichText, ScrollArea, Stroke, Ui, vec2};
use photo_core::meta::{ColorLabel, Flag, Rating};

use super::textures::Picture;
use super::{GalleryApp, Photo, View};
use crate::theme::{self, palette, size, space};
use crate::widgets::{
    Icon, IconButton, SelectionTier, ThumbImage, Thumbnail, ThumbnailResponse, color_labels,
    flag_toggle, icon_button, rating_stars,
};

/// Edge of a grid cell.
const CELL: f32 = 188.0;
/// Gap between grid cells.
const GAP: f32 = 12.0;
/// Edge of a filmstrip cell.
const FILM_CELL: f32 = 48.0;
/// Gap between filmstrip cells.
const FILM_GAP: f32 = 3.0;
/// Height of the toolbar.
const TOOLBAR: f32 = 34.0;

/// What the user did to one cell this frame.
struct CellEvent {
    index: usize,
    response: ThumbnailResponse,
}

/// The grid of photo cells in the work area.
pub(super) fn grid(app: &mut GalleryApp, ui: &mut Ui) {
    let mut event = None;
    ScrollArea::vertical()
        .id_salt("grid")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width();
            let columns = (((width - GAP) / (CELL + GAP)).floor() as usize).max(1);
            let used = columns as f32 * (CELL + GAP) - GAP;
            let margin = ((width - used) / 2.0).floor().max(0.0);
            ui.spacing_mut().item_spacing = vec2(GAP, GAP);
            ui.add_space(GAP);
            for (row, photos) in app.photos.chunks(columns).enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(margin);
                    for (column, photo) in photos.iter().enumerate() {
                        let index = row * columns + column;
                        let response = cell(photo, &app.pictures, &photo.name, CELL)
                            .index(index + 1)
                            .show(ui);
                        if acted_on(&response) {
                            event = Some(CellEvent { index, response });
                        }
                    }
                });
            }
            ui.add_space(GAP);
        });
    if let Some(event) = event {
        app.handle(event);
    }
}

/// The toolbar under the grid: view buttons and the attributes of the
/// selection.
pub(super) fn toolbar(app: &mut GalleryApp, ui: &mut Ui) {
    let frame = theme::panel_frame().inner_margin(Margin::symmetric(8, 0));
    egui::Panel::bottom("toolbar")
        .exact_size(TOOLBAR)
        .frame(frame)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = space::XS;
                for (view, icon, label) in [
                    (View::Grid, Icon::Grid, "Grid view"),
                    (View::Loupe, Icon::Loupe, "Loupe view"),
                    (View::Compare, Icon::Compare, "Compare view"),
                    (View::Survey, Icon::Survey, "Survey view"),
                ] {
                    let button = IconButton::new(icon, label).selected(app.view == view);
                    if ui.add(button).clicked() {
                        app.view = view;
                    }
                }
                divider(ui);

                let active = app.active().cloned();
                let editable = active.is_some();
                let (mut flag, mut rating, mut label) = active.map_or(
                    (Flag::Unflagged, Rating::UNRATED, ColorLabel::None),
                    |photo| (photo.flag, photo.rating, photo.label),
                );
                ui.add_enabled_ui(editable, |ui| {
                    if flag_toggle(ui, "Flag", &mut flag).changed() {
                        let message = match flag {
                            Flag::Picked => "Flagged as Pick",
                            Flag::Rejected => "Set as Rejected",
                            Flag::Unflagged => "Unflagged",
                        };
                        app.edit_selection(message.to_owned(), |photo| photo.flag = flag);
                    }
                    divider(ui);
                    if rating_stars(ui, "Rating", &mut rating).changed() {
                        let message = format!("Set Rating to {}", rating.stars());
                        app.edit_selection(message, |photo| photo.rating = rating);
                    }
                    divider(ui);
                    if color_labels(ui, "Label", &mut label).changed() {
                        let message = match label {
                            ColorLabel::None => "Removed Label".to_owned(),
                            other => format!("Set Label to {}", other.name()),
                        };
                        app.edit_selection(message, |photo| photo.label = label);
                    }
                    divider(ui);
                    if icon_button(ui, Icon::RotateCcw, "Rotate counter-clockwise").clicked() {
                        app.toasts.push("Rotated Left");
                    }
                    if icon_button(ui, Icon::RotateCw, "Rotate clockwise").clicked() {
                        app.toasts.push("Rotated Right");
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(space::SM);
                    let selected = app
                        .photos
                        .iter()
                        .filter(|photo| photo.tier != SelectionTier::None)
                        .count();
                    let summary = format!("{} photos, {selected} selected", app.photos.len());
                    ui.label(RichText::new(summary).color(palette::TEXT_DIM));
                });
            });
        });
}

/// The filmstrip: the same photos as small cells, with a source line above.
pub(super) fn filmstrip(app: &mut GalleryApp, ui: &mut Ui) {
    let frame = theme::panel_frame().inner_margin(Margin::symmetric(8, 6));
    let mut event = None;
    egui::Panel::bottom("filmstrip")
        .exact_size(FILM_CELL + 38.0)
        .frame(frame)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("All Photographs").small());
                ui.add_space(space::LG);
                let source = "Pictures / 2026 / 2026-08 Iceland";
                ui.label(RichText::new(source).color(palette::TEXT_DIM).small());
            });
            ui.add_space(space::XS);
            ScrollArea::horizontal()
                .id_salt("filmstrip")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = FILM_GAP;
                        for (index, photo) in app.photos.iter().enumerate() {
                            let name = format!("Filmstrip {}", photo.name);
                            let response = cell(photo, &app.pictures, &name, FILM_CELL).show(ui);
                            if acted_on(&response) {
                                event = Some(CellEvent { index, response });
                            }
                        }
                    });
                });
        });
    if let Some(event) = event {
        app.handle(event);
    }
}

impl GalleryApp {
    /// React to a click, double-click, context click or drag on a cell.
    fn handle(&mut self, event: CellEvent) {
        let CellEvent { index, response } = event;
        let name = self.photos[index].name.clone();
        if response.double_clicked {
            self.toasts.push(format!("Opened {name}"));
        } else if response.secondary_clicked {
            self.toasts.push(format!("Context menu for {name}"));
        } else if response.drag_started {
            // A drag takes the whole selection along, but only if it starts
            // on one of its photos: any other photo is dragged alone.
            if self.photos[index].tier == SelectionTier::None {
                self.select(index, Modifiers::NONE);
            }
            let count = self
                .photos
                .iter()
                .filter(|photo| photo.tier != SelectionTier::None)
                .count();
            let noun = if count == 1 { "photo" } else { "photos" };
            self.toasts.push(format!("Dragging {count} {noun}"));
        } else if response.clicked {
            self.select(index, response.modifiers);
        }
    }

    /// Change the selection the way a click on photo `index` does: a plain
    /// click makes it the only selected photo, Command or Ctrl adds or
    /// removes it, and Shift extends the selection to it from the active one.
    /// A selection always has an active photo: when Command takes the active
    /// one out, the first photo still selected takes its place.
    fn select(&mut self, index: usize, modifiers: Modifiers) {
        let active = self
            .photos
            .iter()
            .position(|photo| photo.tier == SelectionTier::Active);
        if modifiers.shift
            && let Some(active) = active
        {
            let range = active.min(index)..=active.max(index);
            for (position, photo) in self.photos.iter_mut().enumerate() {
                if range.contains(&position) && photo.tier == SelectionTier::None {
                    photo.tier = SelectionTier::Selected;
                }
            }
        } else if modifiers.command {
            let photo = &mut self.photos[index];
            photo.tier = match photo.tier {
                SelectionTier::None => SelectionTier::Selected,
                SelectionTier::Selected | SelectionTier::Active => SelectionTier::None,
            };
            if active.is_none_or(|active| active == index) {
                let selected = |photo: &Photo| photo.tier == SelectionTier::Selected;
                let heir = if selected(&self.photos[index]) {
                    Some(index)
                } else {
                    self.photos.iter().position(selected)
                };
                if let Some(heir) = heir {
                    self.photos[heir].tier = SelectionTier::Active;
                }
            }
        } else {
            for (position, photo) in self.photos.iter_mut().enumerate() {
                photo.tier = if position == index {
                    SelectionTier::Active
                } else {
                    SelectionTier::None
                };
            }
        }
    }
}

/// A cell for `photo`, `edge` points square, named `name`.
fn cell<'a>(photo: &Photo, pictures: &[Picture], name: &'a str, edge: f32) -> Thumbnail<'a> {
    let image = photo.picture.and_then(|index| pictures.get(index)).map_or(
        ThumbImage::Placeholder,
        |picture| ThumbImage::Texture {
            id: picture.texture.id(),
            aspect: picture.aspect,
        },
    );
    Thumbnail::new(name, [edge, edge])
        .image(image)
        .tier(photo.tier)
        .rating(photo.rating)
        .flag(photo.flag)
        .label(photo.label)
        .badges(photo.badges)
}

fn acted_on(response: &ThumbnailResponse) -> bool {
    response.clicked
        || response.double_clicked
        || response.secondary_clicked
        || response.drag_started
}

/// A thin vertical line between groups of toolbar controls.
fn divider(ui: &mut Ui) {
    let (_, rect) = ui.allocate_space(vec2(9.0, size::ROW));
    let line = Stroke::new(1.0, palette::OUTLINE);
    ui.painter()
        .vline(rect.center().x, rect.y_range().shrink(3.0), line);
}
