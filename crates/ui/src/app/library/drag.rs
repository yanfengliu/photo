//! Dragging photos from the grid or the filmstrip onto a collection.

use egui::{Align2, Area, Frame, Margin, Order, RichText};
use photo_core::meta::PhotoId;

use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::text;
use crate::theme::{palette, radius};

/// The photos a drag carries, in list order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DraggedPhotos(pub(crate) Vec<PhotoId>);

/// A drag began on the cell of `photo`: it carries the whole selection if
/// the photo is part of it, and the photo alone (selected alone) otherwise.
pub(super) fn start(ctx: &egui::Context, cx: &mut Cx<'_>, photo: PhotoId) {
    let photos = if cx.engine.selection().is_selected(photo) {
        cx.engine.selection().selected().to_vec()
    } else {
        cx.act(Action::SelectOnly(photo));
        vec![photo]
    };
    egui::DragAndDrop::set_payload(ctx, DraggedPhotos(photos));
}

/// While photos are dragged, say how many beside the pointer.
pub(super) fn preview(ctx: &egui::Context) {
    let Some(dragged) = egui::DragAndDrop::payload::<DraggedPhotos>(ctx) else {
        return;
    };
    let Some(pointer) = ctx.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    Area::new(egui::Id::unique("photo-drag"))
        .order(Order::Tooltip)
        .pivot(Align2::LEFT_TOP)
        .fixed_pos(pointer + egui::vec2(14.0, 10.0))
        .interactable(false)
        .show(ctx, |ui| {
            Frame::new()
                .fill(palette::ACCENT_DIM)
                .corner_radius(radius::MD)
                .inner_margin(Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    let label = text::count(dragged.0.len(), "photo", "photos");
                    ui.label(RichText::new(label).color(palette::TEXT_STRONG));
                });
        });
}
