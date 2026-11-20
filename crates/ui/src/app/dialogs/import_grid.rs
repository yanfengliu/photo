//! The Import dialog's candidates: a cell per file found, with its
//! embedded preview, ticked (lighter, with a check mark) when it will be
//! imported. A click ticks or unticks it. Files already in the catalog, and
//! suspected duplicates while those are skipped, are shown but cannot be
//! ticked. Only the rows in view are laid out.

use egui::{Align2, Color32, CornerRadius, Rect, ScrollArea, Ui, UiBuilder, Vec2, vec2};
use photo_engine::ImportCandidate;

use super::import::ImportDialog;
use crate::app::Cx;
use crate::app::text;
use crate::theme::{self, palette, radius};
use crate::widgets::{Icon, SelectionTier, ThumbImage, Thumbnail};

/// The smallest edge of a candidate's cell; cells widen to fill a row.
const CELL: f32 = 120.0;
/// Gap between cells.
const GAP: f32 = 6.0;
/// Height of the grid.
const HEIGHT: f32 = 330.0;

/// Show the candidates of `dialog`.
pub(super) fn show(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    dialog: &mut ImportDialog,
    candidates: &[ImportCandidate],
) {
    let output = ScrollArea::vertical()
        .id_salt("import-candidates")
        .auto_shrink([false, false])
        .max_height(HEIGHT)
        .min_scrolled_height(HEIGHT)
        .show_viewport(ui, |ui, viewport| {
            let width = ui.available_width();
            let columns = ((width - GAP) / (CELL + GAP)).floor().max(1.0) as usize;
            // Widened to fill the row.
            let edge = ((width - GAP) / columns as f32 - GAP).floor().max(CELL);
            let step = edge + GAP;
            let rows = candidates.len().div_ceil(columns);
            ui.set_height((GAP + rows as f32 * step).max(HEIGHT - GAP));
            let origin = ui.max_rect().min;
            let first = ((viewport.min.y / step).floor().max(0.0) as usize) * columns;
            let last = (((viewport.max.y / step).ceil().max(0.0) as usize + 1) * columns)
                .min(candidates.len());
            let mut toggled = Vec::new();
            for (index, candidate) in candidates
                .iter()
                .enumerate()
                .take(last)
                .skip(first.min(last))
            {
                let (row, column) = (index / columns, index % columns);
                let min = origin + vec2(GAP + column as f32 * step, GAP + row as f32 * step);
                let rect = Rect::from_min_size(min, Vec2::splat(edge));
                if cell(ui, cx, dialog, candidate, rect) {
                    toggled.push(candidate.path.clone());
                }
            }
            toggled
        });
    for path in output.inner {
        dialog.toggle(&path);
    }
}

/// One candidate's cell. Returns whether a click asked to tick or untick
/// it.
fn cell(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    dialog: &ImportDialog,
    candidate: &ImportCandidate,
    rect: Rect,
) -> bool {
    let ticked = dialog.is_ticked(candidate);
    let tickable = dialog.is_tickable(candidate);
    let image = cx
        .textures
        .candidate(ui.ctx(), cx.engine, &candidate.path)
        .map_or(ThumbImage::Placeholder, |shown| ThumbImage::Texture {
            id: shown.id,
            aspect: shown.aspect,
        });
    let file = text::file_name(&candidate.path);
    let name = format!("Import {file}");
    let tier = if ticked {
        SelectionTier::Selected
    } else {
        SelectionTier::None
    };
    let builder = UiBuilder::new()
        .max_rect(rect)
        .id_salt(("candidate", &candidate.path));
    let response = ui
        .scope_builder(builder, |ui| {
            Thumbnail::new(&name, rect.size())
                .image(image)
                .tier(tier)
                .show(ui)
        })
        .inner;

    let painter = ui.painter();
    if ticked {
        let plate = Rect::from_min_size(rect.min + vec2(5.0, 5.0), Vec2::splat(16.0));
        painter.rect_filled(plate, CornerRadius::same(radius::SM), palette::ACCENT);
        Icon::Check.paint(painter, plate.shrink(2.0), palette::TEXT_STRONG);
    }
    // The file's name, in the strip under the picture.
    let ink = if ticked {
        palette::TEXT_STRONG
    } else {
        palette::TEXT_DIM
    };
    let galley = text::fitted(painter, &file, theme::small_font(), ink, rect.width() - 12.0);
    let at = Align2::CENTER_BOTTOM.anchor_size(rect.center_bottom() - vec2(0.0, 4.0), galley.size());
    painter.galley(at.min, galley, ink);
    let note = if candidate.already_in_catalog {
        Some("In catalog")
    } else if candidate.suspected_duplicate {
        Some("Duplicate?")
    } else {
        None
    };
    if let Some(note) = note {
        let at = rect.center_bottom() - vec2(0.0, 24.0);
        let galley = painter.layout_no_wrap(note.to_owned(), theme::small_font(), palette::TEXT);
        let plate = Align2::CENTER_BOTTOM
            .anchor_size(at, galley.size())
            .expand2(vec2(4.0, 1.0));
        painter.rect_filled(plate, 2.0, Color32::from_black_alpha(170));
        painter.galley(plate.min + vec2(4.0, 1.0), galley, palette::TEXT);
    }
    response.clicked && tickable
}
