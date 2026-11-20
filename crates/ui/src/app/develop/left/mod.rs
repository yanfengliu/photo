//! The Develop module's left column: the Navigator (the photo small, the
//! part in view outlined, and the preview of the preset under the
//! pointer), Presets, Snapshots and History, with Copy and Paste at the
//! bottom.

mod history;
mod looks;

use egui::{Button, Color32, CornerRadius, Margin, Rect, RichText, Stroke, StrokeKind, Ui, pos2, vec2};
use photo_engine::{PreviewSize, Target};

use super::actions::DevelopAction;
use super::right::Editing;
use crate::app::actions::Action;
use crate::app::{Cx, dialogs, text};
use crate::theme::{self, palette, radius, space};
use crate::widgets::{Panel, panel_column};

/// Width of the column unless the user drags it wider or narrower.
const WIDTH: f32 = 264.0;
/// Height of the Navigator's well.
const NAVIGATOR: f32 = 168.0;

/// Show the left column.
pub(crate) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let editing = cx
        .engine
        .develop()
        .filter(|session| Some(session.photo()) == cx.engine.active_photo())
        .map(Editing::of);
    cx.ui.develop.hovered_preset = None;
    egui::Panel::left("develop-left-column")
        .resizable(true)
        .size_range(220.0..=420.0)
        .default_size(WIDTH)
        .frame(theme::panel_frame())
        .show(ui, |ui| {
            bottom(ui, cx, editing.as_ref());
            let mut panels = std::mem::take(&mut cx.ui.develop.left_panels);
            panel_column(ui, "develop-left-panels", |ui| {
                Panel::new("Navigator").show(ui, &mut panels, |ui| navigator(ui, cx));
                Panel::new("Presets").show(ui, &mut panels, |ui| looks::presets(ui, cx, editing.as_ref()));
                Panel::new("Snapshots")
                    .show(ui, &mut panels, |ui| looks::snapshots(ui, cx, editing.as_ref()));
                Panel::new("History")
                    .show(ui, &mut panels, |ui| history::show(ui, cx, editing.as_ref()));
            });
            cx.ui.develop.left_panels = panels;
        });
}

/// Copy… and Paste.
fn bottom(ui: &mut Ui, cx: &mut Cx<'_>, editing: Option<&Editing>) {
    let frame = theme::panel_frame().inner_margin(Margin::same(10));
    let editable = editing.is_some_and(Editing::editable);
    let can_copy = editing.is_some();
    let can_paste = editable && cx.engine.clipboard().is_some();
    egui::Panel::bottom("develop-left-bottom")
        .resizable(false)
        .show_separator_line(false)
        .frame(frame)
        .show(ui, |ui| {
            ui.columns(2, |columns| {
                let width = columns[0].available_width();
                let button = |text: &str| Button::new(text).min_size(vec2(width, 26.0));
                if columns[0]
                    .add_enabled(can_copy, button("Copy…"))
                    .on_hover_text("Choose settings of this photo to paste onto others")
                    .clicked()
                {
                    dialogs::open_copy_settings(cx);
                }
                if columns[1]
                    .add_enabled(can_paste, button("Paste"))
                    .on_hover_text("Paste the copied settings onto the selected photos")
                    .clicked()
                {
                    cx.act(Action::Develop(DevelopAction::PasteSettings(Target::Selected)));
                }
            });
        });
}

/// The photo small, with the part the canvas shows outlined; while the
/// pointer is on a preset, that preset's preview instead.
fn navigator(ui: &mut Ui, cx: &mut Cx<'_>) {
    let (_, well) = ui.allocate_space(vec2(ui.available_width(), NAVIGATOR));
    ui.painter()
        .rect_filled(well, CornerRadius::same(radius::MD), palette::WELL);
    let frame = well.shrink(space::LG);
    let ppp = ui.ctx().pixels_per_point();
    cx.ui.develop.navigator_box = (
        (frame.width() * ppp).round().max(1.0) as u32,
        (frame.height() * ppp).round().max(1.0) as u32,
    );
    let Some(photo) = cx
        .engine
        .active_photo()
        .and_then(|photo| cx.engine.summary(photo).cloned())
    else {
        ui.painter().text(
            well.center(),
            egui::Align2::CENTER_CENTER,
            "No photo selected",
            theme::small_font(),
            palette::TEXT_OFF,
        );
        return;
    };
    let ours = cx
        .engine
        .develop()
        .is_some_and(|session| session.photo() == photo.id);
    let hovered = cx.ui.develop.hovered_preset;
    let look = hovered.and_then(|key| {
        cx.frames
            .look
            .as_ref()
            .filter(|(shown, _)| ours && *shown == key)
            .map(|(_, look)| (look.id(), look.size()))
    });
    let caption = match (hovered, look) {
        (Some(key), _) => cx
            .ui
            .develop
            .lists
            .presets
            .iter()
            .find(|preset| preset.key == key)
            .map_or_else(String::new, |preset| format!("Preset: {}", preset.name)),
        _ => text::photo_name(&photo),
    };
    let shown = match look {
        Some(look) => Some((look, None)),
        None => {
            let whole = ours
                .then(|| cx.frames.whole.as_ref().map(|whole| (whole.id(), whole.size())))
                .flatten();
            let image = whole.or_else(|| {
                cx.textures
                    .photo(ui.ctx(), cx.engine, photo.id, PreviewSize::Thumbnail)
                    .map(|shown| (shown.id, shown.pixels))
            });
            image.map(|image| (image, cx.ui.develop.seen))
        }
    };
    if let Some(((id, pixels), seen)) = shown {
        let aspect = pixels.x / pixels.y.max(1.0);
        let size = if frame.width() / frame.height() > aspect {
            vec2(frame.height() * aspect, frame.height())
        } else {
            vec2(frame.width(), frame.width() / aspect)
        };
        let image = Rect::from_center_size(frame.center(), size);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(id, image, uv, Color32::WHITE);
        ui.painter().rect_stroke(
            image,
            0.0,
            Stroke::new(1.0, palette::SEPARATOR),
            StrokeKind::Outside,
        );
        if let Some(region) = seen {
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
    ui.label(RichText::new(caption).color(palette::TEXT_DIM).small());
}
