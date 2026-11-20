//! Rating stars.

use egui::{Rect, Response, Role, Sense, Ui, Vec2, WidgetInfo, vec2};
use photo_core::meta::Rating;

use super::icons::Icon;
use crate::theme::{palette, size};

/// Width of one star's click target.
const CELL_WIDTH: f32 = 16.0;
/// Edge of the star drawn inside it.
const STAR: f32 = 13.0;

/// Five stars that show and edit `rating`.
///
/// Clicking star N sets the rating to N; clicking the star of the current
/// rating clears it. Each star is a button named `"{name} {N}"`, so with the
/// name "Rating" the third star is "Rating 3". The returned response is
/// marked changed when the rating was edited.
pub fn rating_stars(ui: &mut Ui, name: &str, rating: &mut Rating) -> Response {
    let enabled = ui.is_enabled();
    let (row, mut whole) = ui.allocate_exact_size(
        vec2(CELL_WIDTH * f32::from(Rating::MAX), size::ROW),
        Sense::hover(),
    );

    let mut stars = Vec::with_capacity(usize::from(Rating::MAX));
    for n in 1..=Rating::MAX {
        let cell = Rect::from_min_size(
            row.min + vec2(CELL_WIDTH * f32::from(n - 1), 0.0),
            vec2(CELL_WIDTH, row.height()),
        );
        let response = ui.interact(cell, whole.id.with(n), Sense::click());
        if response.clicked() {
            *rating = if rating.stars() == n {
                Rating::UNRATED
            } else {
                Rating::new(n)
            };
            whole.mark_changed();
        }
        stars.push((n, cell, response));
    }

    // The pointer previews the rating a click would give.
    let preview = stars
        .iter()
        .find(|(_, _, response)| response.hovered())
        .map(|(n, _, _)| *n);
    for (n, cell, response) in &stars {
        let set = *n <= rating.stars();
        response.widget_info(|| {
            WidgetInfo::selected(Role::Button, enabled, set, format!("{name} {n}"))
        });
        if ui.is_rect_visible(*cell) {
            let (icon, color) = match (set, preview) {
                (true, Some(_)) => (Icon::StarFilled, palette::TEXT_STRONG),
                (true, None) => (Icon::StarFilled, palette::TEXT),
                (false, Some(up_to)) if *n <= up_to => (Icon::StarEmpty, palette::TEXT),
                (false, _) => (Icon::StarEmpty, palette::TEXT_OFF),
            };
            let glyph = Rect::from_center_size(cell.center(), Vec2::splat(STAR));
            icon.paint(ui.painter(), glyph, color);
        }
        whole = whole.union(response.clone());
    }
    whole
}
