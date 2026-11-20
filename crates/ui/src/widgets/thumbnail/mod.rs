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

mod paint;

use egui::{
    Color32, Modifiers, PointerButton, Response, Role, Sense, TextureId, Ui, Vec2, WidgetInfo,
};
use photo_core::meta::{ColorLabel, Flag, Rating};

use super::{end_click_burst, in_click_burst, meant_double_click};

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
