//! The gallery's left column: navigator, catalog lists and the icon sheet.

use egui::{Align2, CornerRadius, Rect, RichText, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use super::GalleryApp;
use super::textures::Picture;
use crate::theme::{self, palette, radius, size, space};
use crate::widgets::{
    Dialog, DialogOutcome, Icon, IconButton, Panel, PanelGroup, icon_button, panel_column,
};

/// Show the left column.
pub(super) fn show(app: &mut GalleryApp, ui: &mut Ui) {
    let active = app
        .active()
        .map(|photo| (photo.name.clone(), photo.picture));
    let GalleryApp {
        pictures,
        left_panels: panels,
        collections,
        new_collection,
        ..
    } = app;

    panel_column(ui, "left-panels", |ui| {
        Panel::new("Navigator").show(ui, panels, |ui| {
            let picture = active
                .as_ref()
                .and_then(|(_, picture)| picture.and_then(|index| pictures.get(index)));
            navigator(ui, picture);
            let name = active
                .as_ref()
                .map_or("No photo selected", |(name, _)| name.as_str());
            ui.label(RichText::new(name).color(palette::TEXT_DIM).small());
        });
        Panel::new("Catalog").show(ui, panels, |ui| {
            row(ui, 0.0, "All Photographs", "1,284", true);
            row(ui, 0.0, "Quick Collection", "12", false);
            row(ui, 0.0, "Previous Import", "96", false);
        });
        Panel::new("Folders").show(ui, panels, |ui| {
            row(ui, 0.0, "Pictures", "1,284", false);
            row(ui, 14.0, "2026", "308", false);
            row(ui, 28.0, "2026-08 Iceland", "212", false);
            row(ui, 28.0, "2026-09 Harbour", "96", false);
            row(ui, 14.0, "2025", "976", false);
        });
        Panel::new("Collections").show(ui, panels, |ui| {
            for name in collections.iter() {
                row(ui, 0.0, name, "", false);
            }
            ui.add_space(space::XS);
            ui.horizontal(|ui| {
                if icon_button(ui, Icon::Plus, "New collection").clicked() {
                    *new_collection = Some(String::new());
                }
                ui.label(RichText::new("New collection").color(palette::TEXT_DIM));
            });
        });
        icon_sheet(ui, panels);
    });
}

/// The New Collection dialog, while it is open.
pub(super) fn new_collection_dialog(app: &mut GalleryApp, ctx: &egui::Context) {
    let Some(name) = &mut app.new_collection else {
        return;
    };
    let response = Dialog::new("New Collection")
        .confirm_label("Create")
        .can_confirm(!name.trim().is_empty())
        .show(ctx, |ui| {
            ui.label(RichText::new("Name").color(palette::TEXT_DIM));
            ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY))
                .accessible_name("Collection name");
        });
    match response.outcome {
        DialogOutcome::Open => {}
        DialogOutcome::Cancelled => app.new_collection = None,
        DialogOutcome::Confirmed => {
            let name = app.new_collection.take().unwrap_or_default();
            let name = name.trim().to_owned();
            app.toasts.push(format!("Created Collection \"{name}\""));
            app.collections.push(name);
        }
    }
}

/// The active photo, fitted into a well.
fn navigator(ui: &mut Ui, picture: Option<&Picture>) {
    let (_, well) = ui.allocate_space(vec2(ui.available_width(), 150.0));
    if !ui.is_rect_visible(well) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(well, CornerRadius::same(radius::MD), palette::WELL);
    let Some(picture) = picture else {
        return;
    };
    let frame = well.shrink(space::LG);
    let size = if frame.width() / frame.height() > picture.aspect {
        vec2(frame.height() * picture.aspect, frame.height())
    } else {
        vec2(frame.width(), frame.width() / picture.aspect)
    };
    let image = Rect::from_center_size(frame.center(), size);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter.image(picture.texture.id(), image, uv, egui::Color32::WHITE);
    let outline = Stroke::new(1.0, palette::SEPARATOR);
    painter.rect_stroke(image, 0.0, outline, StrokeKind::Outside);
}

/// One row of a list: a name, indented, with a count at the right.
fn row(ui: &mut Ui, indent: f32, name: &str, count: &str, selected: bool) {
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), size::ROW));
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    if selected {
        let highlight = rect.expand2(vec2(space::SM, 0.0));
        painter.rect_filled(highlight, CornerRadius::same(radius::MD), palette::RAISED);
    }
    let color = if selected {
        palette::TEXT_STRONG
    } else {
        palette::TEXT
    };
    painter.text(
        rect.left_center() + vec2(indent, 0.0),
        Align2::LEFT_CENTER,
        name,
        theme::body_font(),
        color,
    );
    painter.text(
        rect.right_center(),
        Align2::RIGHT_CENTER,
        count,
        theme::small_font(),
        palette::TEXT_DIM,
    );
}

/// Every icon as a button, and one icon in each state a button can be in.
fn icon_sheet(ui: &mut Ui, panels: &mut PanelGroup) {
    Panel::new("Icons").show(ui, panels, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(space::XS);
            for icon in Icon::ALL {
                icon_button(ui, icon, &format!("{} icon", icon.name()));
            }
        });
        ui.add_space(space::MD);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::XS;
            ui.label(RichText::new("States").color(palette::TEXT_DIM));
            ui.add_space(space::MD);
            ui.add(IconButton::new(Icon::Eye, "Idle button"));
            ui.add(IconButton::new(Icon::Eye, "Selected button").selected(true));
            ui.add_enabled(false, IconButton::new(Icon::Eye, "Disabled button"));
        });
    });
}
