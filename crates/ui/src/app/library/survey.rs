//! Survey: the selected photos tiled to fill the work area, each as large
//! as the count allows. A click makes a photo the active one; its close
//! button takes it out of the selection, and so out of the survey.

use egui::{Align2, Rect, Ui, UiBuilder, Vec2, vec2};
use photo_engine::PreviewSize;

use super::{cell, prefixed, summaries};
use crate::app::Cx;
use crate::app::actions::Action;
use crate::theme::{self, palette};
use crate::widgets::{Icon, IconButton};

/// Gap between tiles, and around them.
const GAP: f32 = 10.0;
/// Height of the caption above each tile: the name and the close button.
const CAPTION: f32 = 20.0;
/// The most photos tiled at once.
pub(crate) const MAX_TILES: usize = 100;
/// Tiles at least this large show the standard preview.
const STANDARD_ABOVE: f32 = 320.0;

/// The columns and rows that give `count` tiles of the shape `aspect`
/// (width / height) the largest size in an area of `size`.
pub(crate) fn tiling(count: usize, size: Vec2, aspect: f32) -> (usize, usize) {
    let mut best = (1, count.max(1), 0.0_f32);
    for columns in 1..=count.max(1) {
        let rows = count.div_ceil(columns).max(1);
        let cell = vec2(
            (size.x - GAP) / columns as f32 - GAP,
            (size.y - GAP) / rows as f32 - GAP - CAPTION,
        );
        // The photo fitted into the cell: limited by its width or height.
        let scale = (cell.x / aspect).min(cell.y);
        if scale > best.2 {
            best = (columns, rows, scale);
        }
    }
    (best.0, best.1)
}

/// Show the selected photos.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let selected = cx.engine.selection().selected().to_vec();
    let area = ui.max_rect();
    if selected.is_empty() {
        ui.painter().text(
            area.center(),
            Align2::CENTER_CENTER,
            "Select photos to survey them side by side.",
            theme::body_font(),
            palette::TEXT_DIM,
        );
        return;
    }
    let photos = summaries(cx, &selected[..selected.len().min(MAX_TILES)]);
    let (columns, rows) = tiling(photos.len(), area.size(), 1.5);
    let tile = vec2(
        ((area.width() - GAP) / columns as f32 - GAP).floor(),
        ((area.height() - GAP) / rows as f32 - GAP).floor(),
    );
    let cell_size = tile - vec2(0.0, CAPTION);
    let preview = if cell_size.min_elem() >= STANDARD_ABOVE {
        PreviewSize::Standard
    } else {
        PreviewSize::Thumbnail
    };
    for (index, photo) in photos.iter().enumerate() {
        let (row, column) = (index / columns, index % columns);
        let min = area.min
            + vec2(
                GAP + column as f32 * (tile.x + GAP),
                GAP + row as f32 * (tile.y + GAP),
            );
        let caption = Rect::from_min_size(min, vec2(tile.x, CAPTION));
        let display = crate::app::text::photo_name(photo);
        let galley = crate::app::text::fitted(
            ui.painter(),
            &display,
            theme::small_font(),
            palette::TEXT_DIM,
            tile.x - CAPTION - 4.0,
        );
        let at = Align2::LEFT_CENTER.anchor_size(caption.left_center(), galley.size());
        ui.painter().galley(at.min, galley, palette::TEXT_DIM);
        let close = Rect::from_min_size(
            caption.right_top() + vec2(-18.0, 1.0),
            Vec2::splat(18.0),
        );
        let label = format!("Remove {display} from the survey");
        let closed = ui
            .put(close, IconButton::new(Icon::Close, &label).side(18.0))
            .clicked();

        let rect = Rect::from_min_size(min + vec2(0.0, CAPTION), cell_size);
        let name = prefixed("Survey", photo);
        let builder = UiBuilder::new().max_rect(rect).id_salt(("survey", photo.id));
        let response = ui
            .scope_builder(builder, |ui| {
                cell(ui, cx, photo, &name, rect.size(), preview)
                    .loose(true)
                    .show(ui)
            })
            .inner;
        if closed {
            cx.act(Action::ToggleSelected(photo.id));
        } else if response.clicked {
            cx.act(Action::MakeActive(photo.id));
        }
    }
    if selected.len() > MAX_TILES {
        let note = format!(
            "Showing the first {MAX_TILES} of {} selected photos.",
            selected.len()
        );
        ui.painter().text(
            area.center_bottom() - vec2(0.0, 6.0),
            Align2::CENTER_BOTTOM,
            note,
            theme::small_font(),
            palette::TEXT_DIM,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_photo_fills_the_area_and_more_are_tiled() {
        let area = vec2(1200.0, 800.0);
        assert_eq!(tiling(1, area, 1.5), (1, 1));
        assert_eq!(tiling(2, area, 1.5), (2, 1));
        let (columns, rows) = tiling(9, area, 1.5);
        assert!(columns * rows >= 9 && columns >= rows, "{columns} x {rows}");
        assert_eq!(tiling(0, area, 1.5), (1, 1));
    }
}
