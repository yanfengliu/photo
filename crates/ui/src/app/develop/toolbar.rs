//! The Develop toolbar under the canvas: how the before view is shown, the
//! clipping overlays, and the canvas's zoom.

use egui::{Align, Button, Layout, Margin, Stroke, Ui, UiBuilder, vec2};

use super::canvas::{self, CanvasZoom};
use super::state::BeforeAfter;
use crate::app::Cx;
use crate::theme::{self, palette, size, space};
use crate::widgets::{Icon, IconButton};

/// Height of the toolbar.
const TOOLBAR: f32 = 36.0;

/// Show the toolbar panel.
pub(crate) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let frame = theme::panel_frame().inner_margin(Margin::symmetric(8, 0));
    egui::Panel::bottom("develop-toolbar")
        .exact_size(TOOLBAR)
        .resizable(false)
        .frame(frame)
        .show(ui, |ui| {
            let bar = ui.max_rect();
            let mut right = ui.new_child(
                UiBuilder::new()
                    .max_rect(bar)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            right.spacing_mut().item_spacing.x = space::SM;
            zoom(&mut right, cx);
            let taken = right.min_rect().width();
            let rest = bar.with_max_x(bar.right() - taken - space::LG);
            let mut left = ui.new_child(
                UiBuilder::new()
                    .max_rect(rest)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            left.set_clip_rect(rest.intersect(ui.clip_rect()));
            left.spacing_mut().item_spacing.x = space::XS;
            before_after(&mut left, cx);
            divider(&mut left);
            let clipping = cx.ui.develop.clipping;
            let on = clipping.shadows || clipping.highlights;
            if left
                .add(Button::selectable(on, "Clipping").min_size(vec2(0.0, size::ROW)))
                .on_hover_text("Show where shadows and highlights clip (J)")
                .clicked()
            {
                cx.ui.develop.toggle_clipping();
            }
        });
}

fn before_after(ui: &mut Ui, cx: &mut Cx<'_>) {
    let mode = cx.ui.develop.before_after;
    let edit = IconButton::new(Icon::Loupe, "Show the edit").selected(mode == BeforeAfter::Off);
    if ui.add(edit).clicked() {
        cx.ui.develop.before_after = BeforeAfter::Off;
    }
    let before = Button::selectable(mode == BeforeAfter::Before, "Before").min_size(vec2(0.0, size::ROW));
    if ui
        .add(before)
        .on_hover_text("Show the photo as it was imported (\\)")
        .clicked()
    {
        cx.ui.develop.toggle_before_after(BeforeAfter::Before);
    }
    for (choice, icon, name) in [
        (BeforeAfter::SideBySide, Icon::Compare, "Before and after side by side"),
        (BeforeAfter::Split, Icon::BeforeAfter, "Before and after split"),
    ] {
        if ui.add(IconButton::new(icon, name).selected(mode == choice)).clicked() {
            cx.ui.develop.toggle_before_after(choice);
        }
    }
}

fn zoom(ui: &mut Ui, cx: &mut Cx<'_>) {
    // Right to left: the closest zoom is added first.
    for zoom in CanvasZoom::ALL.into_iter().rev() {
        let selected = cx.ui.develop.canvas.zoom == zoom;
        if ui
            .add(Button::selectable(selected, zoom.label()).min_size(vec2(36.0, size::ROW)))
            .clicked()
        {
            canvas::set_zoom(cx, zoom);
        }
    }
}

/// A thin vertical line between groups of controls.
fn divider(ui: &mut Ui) {
    let (_, rect) = ui.allocate_space(vec2(9.0, size::ROW));
    let line = Stroke::new(1.0, palette::OUTLINE);
    ui.painter()
        .vline(rect.center().x, rect.y_range().shrink(3.0), line);
}
