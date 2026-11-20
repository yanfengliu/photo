//! The colour-label swatch row.

use egui::{
    Color32, CornerRadius, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo,
    vec2,
};
use photo_core::meta::ColorLabel;

use crate::theme::{palette, radius, size};

/// The labels that have a colour, in the order they are shown.
const LABELS: [ColorLabel; 5] = [
    ColorLabel::Red,
    ColorLabel::Yellow,
    ColorLabel::Green,
    ColorLabel::Blue,
    ColorLabel::Purple,
];
/// Width of one swatch's click target.
const CELL_WIDTH: f32 = 16.0;
/// Edge of the swatch drawn inside it.
const SWATCH: f32 = 10.0;

/// The colour a label is shown in; `None` for [`ColorLabel::None`].
pub fn label_color(label: ColorLabel) -> Option<Color32> {
    match label {
        ColorLabel::None => None,
        ColorLabel::Red => Some(palette::LABEL_RED),
        ColorLabel::Yellow => Some(palette::LABEL_YELLOW),
        ColorLabel::Green => Some(palette::LABEL_GREEN),
        ColorLabel::Blue => Some(palette::LABEL_BLUE),
        ColorLabel::Purple => Some(palette::LABEL_PURPLE),
    }
}

/// Five swatches that show and edit `label`.
///
/// Clicking a swatch sets that label; clicking the one that is already set
/// removes the label. Each swatch is a toggle button named after its colour:
/// `"{name} red"`, `"{name} yellow"` and so on. The returned response is
/// marked changed when the label was edited.
pub fn color_labels(ui: &mut Ui, name: &str, label: &mut ColorLabel) -> Response {
    let enabled = ui.is_enabled();
    let (row, mut whole) = ui.allocate_exact_size(
        vec2(CELL_WIDTH * LABELS.len() as f32, size::ROW),
        Sense::hover(),
    );
    for (index, swatch) in LABELS.into_iter().enumerate() {
        let cell = Rect::from_min_size(
            row.min + vec2(CELL_WIDTH * index as f32, 0.0),
            vec2(CELL_WIDTH, row.height()),
        );
        let response = ui.interact(cell, whole.id.with(index), Sense::click());
        if response.clicked() {
            *label = if *label == swatch {
                ColorLabel::None
            } else {
                swatch
            };
            whole.mark_changed();
        }
        let set = *label == swatch;
        response.widget_info(|| {
            let color = swatch.name().to_lowercase();
            WidgetInfo::selected(Role::Button, enabled, set, format!("{name} {color}"))
        });
        if ui.is_rect_visible(cell)
            && let Some(color) = label_color(swatch)
        {
            let rounding = CornerRadius::same(radius::SM);
            let side = if response.hovered() {
                SWATCH + 2.0
            } else {
                SWATCH
            };
            let square = Rect::from_center_size(cell.center(), Vec2::splat(side));
            ui.painter().rect_filled(square, rounding, color);
            if set {
                ui.painter().rect_stroke(
                    square.expand(2.0),
                    rounding,
                    Stroke::new(1.0, palette::TEXT_STRONG),
                    StrokeKind::Inside,
                );
            }
        }
        whole = whole.union(response);
    }
    whole
}

/// Six toggles that show and edit a set of labels, such as the labels a
/// filter lets through: one swatch per colour and a hollow one for photos
/// with no label.
///
/// Clicking a swatch adds its label to `labels` or takes it out. The
/// swatches are toggle buttons named like [`color_labels`]' (`"{name} red"`,
/// ...) and `"{name} unlabelled"` for the hollow one. The set keeps the order
/// of [`ColorLabel::ALL`]. The returned response is marked changed when the
/// set was edited.
pub fn color_label_set(ui: &mut Ui, name: &str, labels: &mut Vec<ColorLabel>) -> Response {
    let enabled = ui.is_enabled();
    let offered = ColorLabel::ALL;
    let (row, mut whole) = ui.allocate_exact_size(
        vec2(CELL_WIDTH * offered.len() as f32, size::ROW),
        Sense::hover(),
    );
    for (index, swatch) in offered.into_iter().enumerate() {
        let cell = Rect::from_min_size(
            row.min + vec2(CELL_WIDTH * index as f32, 0.0),
            vec2(CELL_WIDTH, row.height()),
        );
        let response = ui.interact(cell, whole.id.with(index), Sense::click());
        if response.clicked() {
            toggle_member(labels, swatch);
            whole.mark_changed();
        }
        let set = labels.contains(&swatch);
        response.widget_info(|| {
            let color = match swatch {
                ColorLabel::None => "unlabelled".to_owned(),
                other => other.name().to_lowercase(),
            };
            WidgetInfo::selected(Role::Button, enabled, set, format!("{name} {color}"))
        });
        if ui.is_rect_visible(cell) {
            paint_set_swatch(ui, cell, swatch, set, response.hovered());
        }
        whole = whole.union(response);
    }
    whole
}

/// Add `label` to `labels` or take it out, keeping the order of
/// [`ColorLabel::ALL`].
fn toggle_member(labels: &mut Vec<ColorLabel>, label: ColorLabel) {
    if labels.contains(&label) {
        labels.retain(|other| *other != label);
    } else {
        labels.push(label);
        labels.sort_by_key(|other| ColorLabel::ALL.iter().position(|all| all == other));
    }
}

fn paint_set_swatch(ui: &Ui, cell: Rect, swatch: ColorLabel, set: bool, hovered: bool) {
    let rounding = CornerRadius::same(radius::SM);
    let side = if hovered { SWATCH + 2.0 } else { SWATCH };
    let square = Rect::from_center_size(cell.center(), Vec2::splat(side));
    let painter = ui.painter();
    match label_color(swatch) {
        Some(color) => {
            painter.rect_filled(square, rounding, color);
        }
        None => {
            let outline = Stroke::new(1.0, palette::TEXT_DIM);
            painter.rect_stroke(square.shrink(0.5), rounding, outline, StrokeKind::Inside);
        }
    }
    if set {
        painter.rect_stroke(
            square.expand(2.0),
            rounding,
            Stroke::new(1.0, palette::TEXT_STRONG),
            StrokeKind::Inside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_set_toggles_members_and_keeps_the_label_order() {
        let mut labels = Vec::new();
        toggle_member(&mut labels, ColorLabel::Blue);
        toggle_member(&mut labels, ColorLabel::Red);
        toggle_member(&mut labels, ColorLabel::None);
        assert_eq!(
            labels,
            [ColorLabel::None, ColorLabel::Red, ColorLabel::Blue]
        );
        toggle_member(&mut labels, ColorLabel::Red);
        assert_eq!(labels, [ColorLabel::None, ColorLabel::Blue]);
    }

    #[test]
    fn every_label_but_none_has_a_distinct_colour() {
        assert_eq!(label_color(ColorLabel::None), None);
        let colors: Vec<Color32> = LABELS.into_iter().filter_map(label_color).collect();
        assert_eq!(colors.len(), LABELS.len());
        for (index, color) in colors.iter().enumerate() {
            assert!(!colors[index + 1..].contains(color));
        }
    }

    #[test]
    fn the_swatch_row_offers_every_label_that_has_a_colour() {
        let offered: Vec<ColorLabel> = ColorLabel::ALL
            .into_iter()
            .filter(|label| label_color(*label).is_some())
            .collect();
        assert_eq!(offered, LABELS);
    }
}
