//! How a cell is painted: its mat, the image, the strips of marks and the
//! badges.

use egui::emath::GuiRounding as _;
use egui::epaint::Shape;
use egui::{
    Align2, Color32, CornerRadius, Rect, Response, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use photo_core::meta::{ColorLabel, Flag};

use super::{
    BADGE_PLATE, COMPACT_BELOW, DEFAULT_ASPECT, STRIP, SelectionTier, ThumbImage, Thumbnail,
};
use crate::theme::{self, palette, radius, space};
use crate::widgets::color_label::label_color;
use crate::widgets::icons::Icon;

impl Thumbnail<'_> {
    /// Paint the cell into `rect`, lit as `response` says.
    pub(super) fn paint(&self, ui: &Ui, response: &Response, rect: Rect) {
        let painter = ui.painter();
        let rounding = CornerRadius::same(radius::MD);
        // A loose cell's "mat" is the work area it stands on.
        let mat = if self.loose {
            palette::WORK_AREA
        } else {
            let mat = mat_color(self.tier, self.label, response.hovered());
            painter.rect_filled(rect, rounding, mat);
            mat
        };
        if self.tier == SelectionTier::Active && !self.loose {
            let edge = Stroke::new(1.0, palette::TEXT);
            painter.rect_stroke(rect, rounding, edge, StrokeKind::Inside);
        }
        if response.has_focus() {
            let ring = Stroke::new(1.0, palette::ACCENT_STRONG);
            painter.rect_stroke(rect.expand(1.0), rounding, ring, StrokeKind::Outside);
        }

        let compact = rect.height() < COMPACT_BELOW;
        let pad = if compact { space::SM } else { space::MD };
        let inner = rect.shrink(pad);
        let frame = if compact {
            inner.with_max_y(inner.bottom() - space::MD)
        } else {
            inner.shrink2(vec2(0.0, STRIP))
        };
        let image = self.paint_image(ui, frame);
        if self.loose {
            let line = match self.tier {
                SelectionTier::Active => Some(Stroke::new(2.0, palette::TEXT_STRONG)),
                SelectionTier::Selected => Some(Stroke::new(1.0, palette::TEXT_DIM)),
                SelectionTier::None => None,
            };
            if let Some(line) = line {
                painter.rect_stroke(image.expand(2.0), 0.0, line, StrokeKind::Outside);
            }
        }

        // Marks are drawn in whichever of white or black the mat leaves legible.
        let ink = if mat.r() > 150 {
            palette::SEPARATOR
        } else {
            palette::TEXT
        };
        if compact {
            self.paint_compact_marks(ui, inner, image, ink);
        } else {
            // With no mat to frame them, the strips hug the image.
            let strips = if self.loose {
                image.expand2(vec2(0.0, STRIP))
            } else {
                inner
            };
            self.paint_strips(ui, strips, ink);
            self.paint_badges(ui, image, mat);
        }
    }

    /// Paint the photo (or its placeholder) fitted into `frame`; returns the
    /// rect it took.
    fn paint_image(&self, ui: &Ui, frame: Rect) -> Rect {
        let painter = ui.painter();
        let aspect = match self.image {
            ThumbImage::Texture { aspect, .. } if aspect.is_finite() && aspect > 0.0 => aspect,
            ThumbImage::Texture { .. } => DEFAULT_ASPECT,
            ThumbImage::Placeholder => self.placeholder_aspect,
        };
        let size = if frame.width() / frame.height() > aspect {
            vec2(frame.height() * aspect, frame.height())
        } else {
            vec2(frame.width(), frame.width() / aspect)
        };
        let rect =
            Rect::from_center_size(frame.center(), size).round_to_pixels(ui.pixels_per_point());
        match self.image {
            ThumbImage::Texture { id, .. } => {
                // A rejected or missing photo is shown dimmed.
                let dim = self.flag == Flag::Rejected || self.badges.missing;
                let tint = if dim {
                    Color32::from_gray(96)
                } else {
                    Color32::WHITE
                };
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter.image(id, rect, uv, tint);
            }
            ThumbImage::Placeholder => {
                painter.rect_filled(rect, 0.0, palette::WORK_AREA);
                let glyph = Rect::from_center_size(rect.center(), Vec2::splat(16.0));
                Icon::Loupe.paint(painter, glyph, palette::TEXT_OFF);
            }
        }
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, palette::SEPARATOR),
            StrokeKind::Outside,
        );
        rect
    }

    /// The strips of a full cell: index and flag above, rating and label below.
    fn paint_strips(&self, ui: &Ui, inner: Rect, ink: Color32) {
        let painter = ui.painter();
        let top = Rect::from_min_size(inner.min, vec2(inner.width(), STRIP - space::XS));
        let bottom = Rect::from_min_max(
            pos2(inner.left(), inner.bottom() - STRIP + space::XS),
            inner.max,
        );

        if let Some(index) = self.index {
            painter.text(
                top.left_center(),
                Align2::LEFT_CENTER,
                index,
                theme::small_font(),
                ink.gamma_multiply(0.7),
            );
        }
        let flag =
            Rect::from_center_size(pos2(top.right() - 6.0, top.center().y), Vec2::splat(12.0));
        match self.flag {
            Flag::Picked => Icon::FlagPick.paint(painter, flag, ink),
            Flag::Rejected => Icon::FlagReject.paint(painter, flag, ink.gamma_multiply(0.7)),
            Flag::Unflagged => {}
        }
        if self.badges.quick_collection {
            let dot = pos2(top.right() - 20.0, top.center().y);
            painter.circle_filled(dot, 3.0, ink);
        }

        for star in 0..self.rating.stars() {
            let center = pos2(
                bottom.left() + 5.0 + 10.0 * f32::from(star),
                bottom.center().y,
            );
            let glyph = Rect::from_center_size(center, Vec2::splat(10.0));
            Icon::StarFilled.paint(painter, glyph, ink);
        }
        if let Some(color) = label_color(self.label) {
            let swatch = Rect::from_center_size(
                pos2(bottom.right() - 5.0, bottom.center().y),
                Vec2::splat(8.0),
            );
            painter.rect_filled(swatch, CornerRadius::same(radius::SM), color);
            painter.rect_stroke(
                swatch,
                CornerRadius::same(radius::SM),
                Stroke::new(1.0, palette::SEPARATOR),
                StrokeKind::Outside,
            );
        }
    }

    /// The marks of a compact cell, in the margin under the image and on its
    /// top-left corner.
    fn paint_compact_marks(&self, ui: &Ui, inner: Rect, image: Rect, ink: Color32) {
        let painter = ui.painter();
        let y = inner.bottom() - 2.0;
        for star in 0..self.rating.stars() {
            let center = pos2(inner.left() + 2.0 + 5.0 * f32::from(star), y);
            painter.circle_filled(center, 1.5, ink);
        }
        if let Some(color) = label_color(self.label) {
            let swatch = Rect::from_center_size(pos2(inner.right() - 5.0, y), vec2(10.0, 3.0));
            painter.rect_filled(swatch, 1.0, color);
        }
        if self.badges.quick_collection {
            painter.circle_filled(pos2(inner.right() - 15.0, y), 1.8, ink);
        }
        if self.flag != Flag::Unflagged {
            let corner = image.left_top();
            let mark = vec![corner, corner + vec2(9.0, 0.0), corner + vec2(0.0, 9.0)];
            let color = if self.flag == Flag::Picked {
                palette::TEXT_STRONG
            } else {
                palette::SEPARATOR
            };
            painter.add(Shape::convex_polygon(mark, color, Stroke::NONE));
        }
        // Of the badges only the one that needs acting on is kept: the others
        // would cover most of an image this small.
        if self.badges.missing {
            let badge =
                Rect::from_min_size(image.right_top() + vec2(-12.0, 1.0), Vec2::splat(11.0));
            painter.rect_filled(badge, CornerRadius::same(radius::SM), BADGE_PLATE);
            Icon::Warning.paint(painter, badge.shrink(1.0), palette::WARNING);
        }
    }

    /// The badges on the corners of the image of a full cell.
    fn paint_badges(&self, ui: &Ui, image: Rect, mat: Color32) {
        let painter = ui.painter();
        let rounding = CornerRadius::same(radius::SM);
        if self.badges.missing {
            let badge =
                Rect::from_min_size(image.right_top() + vec2(-17.0, 3.0), Vec2::splat(14.0));
            painter.rect_filled(badge, rounding, BADGE_PLATE);
            Icon::Warning.paint(painter, badge.shrink(1.5), palette::WARNING);
        }
        if self.badges.edited {
            // Two tiny sliders: the photo has adjustments.
            let badge =
                Rect::from_min_size(image.right_bottom() + vec2(-17.0, -17.0), Vec2::splat(14.0));
            painter.rect_filled(badge, rounding, BADGE_PLATE);
            for (row, knob) in [(4.5, 9.0), (9.5, 5.0)] {
                let y = badge.top() + row;
                let line = Stroke::new(1.0, palette::TEXT);
                painter.hline(badge.left() + 3.0..=badge.right() - 3.0, y, line);
                let knob = Rect::from_center_size(pos2(badge.left() + knob, y), Vec2::splat(3.0));
                painter.rect_filled(knob, 0.0, palette::TEXT_STRONG);
            }
        }
        if self.badges.virtual_copy {
            // A turned-up corner: this is a copy, not a file of its own.
            let corner = image.left_bottom();
            let fold = 11.0;
            let cut = vec![corner, corner + vec2(fold, 0.0), corner + vec2(0.0, -fold)];
            painter.add(Shape::convex_polygon(cut, mat, Stroke::NONE));
            let flap = vec![
                corner + vec2(fold, 0.0),
                corner + vec2(fold, -fold),
                corner + vec2(0.0, -fold),
            ];
            painter.add(Shape::convex_polygon(flap, palette::TEXT, Stroke::NONE));
        }
    }
}

/// The mat a cell sits on: lighter for each selection tier, tinted by the
/// colour label, and a touch lighter under the pointer.
fn mat_color(tier: SelectionTier, label: ColorLabel, hovered: bool) -> Color32 {
    let base = match tier {
        SelectionTier::None => palette::CELL,
        SelectionTier::Selected => palette::CELL_SELECTED,
        SelectionTier::Active => palette::CELL_ACTIVE,
    };
    let tinted = match label_color(label) {
        Some(color) => theme::mix(base, color, 0.24),
        None => base,
    };
    if hovered {
        theme::mix(tinted, Color32::WHITE, 0.07)
    } else {
        tinted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_selection_tier_sits_on_a_lighter_mat() {
        let level = |tier| mat_color(tier, ColorLabel::None, false).r();
        assert!(level(SelectionTier::None) < level(SelectionTier::Selected));
        assert!(level(SelectionTier::Selected) < level(SelectionTier::Active));
        assert!(
            mat_color(SelectionTier::None, ColorLabel::None, true).r() > level(SelectionTier::None),
            "hover lightens the mat"
        );
    }

    #[test]
    fn a_colour_label_tints_the_mat_towards_its_colour() {
        let plain = mat_color(SelectionTier::None, ColorLabel::None, false);
        let red = mat_color(SelectionTier::None, ColorLabel::Red, false);
        assert!(plain.r() == plain.g() && plain.g() == plain.b());
        assert!(red.r() > red.g() && red.r() > plain.r());
    }
}
