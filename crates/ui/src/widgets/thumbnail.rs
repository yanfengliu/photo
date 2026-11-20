//! The photo cell of the grid and the filmstrip.
//!
//! ```text
//! ┌──────────────────┐
//! │ 12            ⚑ │  index number, flag
//! │  ┌────────────┐  │
//! │  │   photo    │  │  the image, fitted; badges on its corners
//! │  └────────────┘  │
//! │ ★★★           ■ │  rating, colour label
//! └──────────────────┘
//! ```
//!
//! The cell sits on a mat whose lightness is its selection tier, tinted by
//! the photo's colour label. Cells too small for the two strips (a filmstrip)
//! drop them and mark rating, label and flag on the edge of the image; of the
//! badges they keep only the missing-file warning.
//!
//! A photo shown large (Compare, Survey) is better without a mat: a large
//! light surround would bias how its colours are judged. A
//! [loose](Thumbnail::loose) cell stands on the work area itself and shows
//! its selection tier as a line around the image instead.

use egui::emath::GuiRounding as _;
use egui::epaint::Shape;
use egui::{
    Align2, Color32, CornerRadius, Modifiers, PointerButton, Rect, Response, Role, Sense, Stroke,
    StrokeKind, TextureId, Ui, Vec2, WidgetInfo, pos2, vec2,
};
use photo_core::meta::{ColorLabel, Flag, Rating};

use super::color_label::label_color;
use super::icons::Icon;
use super::{end_click_burst, in_click_burst, meant_double_click};
use crate::theme::{self, palette, radius, space};

/// Cells shorter than this drop the index and rating strips.
const COMPACT_BELOW: f32 = 110.0;
/// Height of the strip above and below the image in a full cell.
const STRIP: f32 = 16.0;
/// The dark plate a badge is drawn on so it reads over any photo.
const BADGE_PLATE: Color32 = Color32::from_black_alpha(170);

/// How selected a photo is. Each tier is drawn on a lighter mat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SelectionTier {
    /// Not selected.
    #[default]
    None,
    /// Part of the selection.
    Selected,
    /// The active photo: the one the panels describe and edits apply to.
    Active,
}

/// What a cell shows where the photo goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThumbImage {
    /// A texture, with the aspect ratio (width / height) to fit it at.
    Texture {
        /// The texture to draw.
        id: TextureId,
        /// Width divided by height of the photo.
        aspect: f32,
    },
    /// No thumbnail yet: an empty frame is drawn, in the shape set with
    /// [`Thumbnail::placeholder_aspect`], so the cell does not change shape
    /// when the picture arrives.
    Placeholder,
}

/// The small marks a cell can carry on its image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Badges {
    /// The photo has develop adjustments.
    pub edited: bool,
    /// The photo is a virtual copy of another.
    pub virtual_copy: bool,
    /// The photo's file cannot be found.
    pub missing: bool,
    /// The photo is in the quick collection: a dot beside the flag.
    pub quick_collection: bool,
}

/// What a cell reports for one frame.
#[derive(Clone, Debug)]
pub struct ThumbnailResponse {
    /// The cell's response (hover, focus, rect, drag-and-drop helpers).
    pub response: Response,
    /// A single click with the primary button (or Enter or Space with focus).
    /// A double-click reports its first click here and its second in
    /// [`Self::double_clicked`].
    pub clicked: bool,
    /// The second click of a double-click.
    pub double_clicked: bool,
    /// A click with the secondary button: open the context menu.
    pub secondary_clicked: bool,
    /// A drag with the primary button began on this cell this frame.
    pub drag_started: bool,
    /// The modifier keys held when the above happened (Shift to extend a
    /// selection, Command or Ctrl to toggle one photo in it).
    pub modifiers: Modifiers,
}

/// One photo cell. See the [module documentation](self).
#[must_use = "call `.show(ui)`"]
pub struct Thumbnail<'a> {
    name: &'a str,
    size: Vec2,
    image: ThumbImage,
    tier: SelectionTier,
    rating: Rating,
    flag: Flag,
    label: ColorLabel,
    badges: Badges,
    index: Option<usize>,
    placeholder_aspect: f32,
    loose: bool,
}

/// The shape a placeholder takes when nothing better is known.
const DEFAULT_ASPECT: f32 = 1.5;

impl<'a> Thumbnail<'a> {
    /// A cell of `size` points for the photo called `name` (its file name,
    /// say). The name is not drawn; it is how the cell is found by tests and
    /// read by assistive technology.
    pub fn new(name: &'a str, size: impl Into<Vec2>) -> Self {
        Self {
            name,
            size: size.into(),
            image: ThumbImage::Placeholder,
            tier: SelectionTier::None,
            rating: Rating::UNRATED,
            flag: Flag::Unflagged,
            label: ColorLabel::None,
            badges: Badges::default(),
            index: None,
            placeholder_aspect: DEFAULT_ASPECT,
            loose: false,
        }
    }

    /// Leave out the mat: the photo stands on whatever is behind the cell,
    /// a line around the image shows the selection tier (bright for the
    /// active photo, dim for a selected one), and the colour label shows
    /// only as its swatch. For photos shown large.
    pub fn loose(mut self, loose: bool) -> Self {
        self.loose = loose;
        self
    }

    /// The shape (width / height) of the empty frame drawn while there is
    /// no picture: the photo's own, when known. Ignored for a texture, which
    /// brings its own; a value that is not a positive number is ignored.
    pub fn placeholder_aspect(mut self, aspect: f32) -> Self {
        if aspect.is_finite() && aspect > 0.0 {
            self.placeholder_aspect = aspect;
        }
        self
    }

    /// The image to show.
    pub fn image(mut self, image: ThumbImage) -> Self {
        self.image = image;
        self
    }

    /// The selection tier.
    pub fn tier(mut self, tier: SelectionTier) -> Self {
        self.tier = tier;
        self
    }

    /// The star rating to show.
    pub fn rating(mut self, rating: Rating) -> Self {
        self.rating = rating;
        self
    }

    /// The flag to show.
    pub fn flag(mut self, flag: Flag) -> Self {
        self.flag = flag;
        self
    }

    /// The colour label: it tints the mat and shows as a swatch.
    pub fn label(mut self, label: ColorLabel) -> Self {
        self.label = label;
        self
    }

    /// The badges to show.
    pub fn badges(mut self, badges: Badges) -> Self {
        self.badges = badges;
        self
    }

    /// The photo's position in the list, shown as a number (starting at 1).
    pub fn index(mut self, index: usize) -> Self {
        self.index = Some(index);
        self
    }

    /// Show the cell.
    pub fn show(self, ui: &mut Ui) -> ThumbnailResponse {
        let (rect, response) = ui.allocate_exact_size(self.size, Sense::click_and_drag());
        let (enabled, selected) = (ui.is_enabled(), self.tier != SelectionTier::None);
        let summary = self.describe();
        response.widget_info(|| WidgetInfo {
            label: Some(self.name.to_owned()),
            current_text_value: Some(summary.clone()),
            enabled,
            ..WidgetInfo::new(Role::ListBoxOption)
        });
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_selected(selected));

        if ui.is_rect_visible(rect) {
            self.paint(ui, &response, rect);
        }
        let burst = in_click_burst(ui.ctx(), response.id);
        let double_clicked = meant_double_click(&response, burst);
        if double_clicked {
            end_click_burst(ui.ctx(), response.id);
        }
        ThumbnailResponse {
            clicked: response.clicked() && !double_clicked,
            double_clicked,
            secondary_clicked: response.secondary_clicked(),
            drag_started: response.drag_started_by(PointerButton::Primary),
            modifiers: ui.input(|input| input.modifiers),
            response,
        }
    }

    /// The cell's state in words, for the accessibility tree.
    fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        match self.tier {
            SelectionTier::None => {}
            SelectionTier::Selected => parts.push("selected".to_owned()),
            SelectionTier::Active => parts.push("active".to_owned()),
        }
        if self.rating.stars() > 0 {
            parts.push(format!("{} stars", self.rating.stars()));
        }
        match self.flag {
            Flag::Picked => parts.push("pick".to_owned()),
            Flag::Rejected => parts.push("rejected".to_owned()),
            Flag::Unflagged => {}
        }
        if self.label != ColorLabel::None {
            parts.push(self.label.name().to_lowercase());
        }
        for (set, word) in [
            (self.badges.edited, "edited"),
            (self.badges.virtual_copy, "virtual copy"),
            (self.badges.missing, "missing"),
            (self.badges.quick_collection, "in quick collection"),
        ] {
            if set {
                parts.push(word.to_owned());
            }
        }
        parts.join(", ")
    }

    fn paint(&self, ui: &Ui, response: &Response, rect: Rect) {
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
            self.paint_strips(ui, inner, ink);
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

    #[test]
    fn the_description_names_every_state_that_is_set() {
        let quiet = Thumbnail::new("a.jpg", [160.0, 160.0]);
        assert_eq!(quiet.describe(), "");
        let busy = Thumbnail::new("b.jpg", [160.0, 160.0])
            .tier(SelectionTier::Active)
            .rating(Rating::new(4))
            .flag(Flag::Picked)
            .label(ColorLabel::Blue)
            .badges(Badges {
                edited: true,
                virtual_copy: true,
                missing: false,
                quick_collection: true,
            });
        assert_eq!(
            busy.describe(),
            "active, 4 stars, pick, blue, edited, virtual copy, in quick collection"
        );
    }

    #[test]
    fn only_a_usable_aspect_shapes_the_placeholder() {
        let cell = |aspect| Thumbnail::new("a.jpg", [160.0, 160.0]).placeholder_aspect(aspect);
        assert_eq!(cell(0.75).placeholder_aspect, 0.75);
        for nonsense in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(cell(nonsense).placeholder_aspect, DEFAULT_ASPECT);
        }
    }
}
