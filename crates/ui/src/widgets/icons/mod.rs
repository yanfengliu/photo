//! The icon set and the icon button.
//!
//! Icons are drawn in code as vector strokes (see [`Icon::paint`]): there is
//! no icon font and no artwork, they scale to any size and pixel density,
//! and they take whatever colour the control showing them is in.

mod paint;

use egui::{
    Color32, CornerRadius, Response, Role, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo,
};

use crate::theme::{palette, radius, size};

/// One icon of the set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// Grid view: a lattice of cells.
    Grid,
    /// Loupe view: one photo.
    Loupe,
    /// Compare view: two photos side by side.
    Compare,
    /// Survey view: several photos tiled.
    Survey,
    /// A photo flagged as a pick.
    FlagPick,
    /// A photo flagged as rejected.
    FlagReject,
    /// A photo with no flag.
    FlagNone,
    /// A star of a rating that is set.
    StarFilled,
    /// A star of a rating that is not set.
    StarEmpty,
    /// Rotate a quarter turn counter-clockwise.
    RotateCcw,
    /// Rotate a quarter turn clockwise.
    RotateCw,
    /// Mirror horizontally.
    Flip,
    /// Mirror vertically.
    FlipVertical,
    /// The crop tool.
    Crop,
    /// The healing tool.
    Heal,
    /// A linear (graduated) mask.
    LinearMask,
    /// A radial mask.
    RadialMask,
    /// A brushed mask.
    Brush,
    /// Show or hide.
    Eye,
    /// Delete.
    Trash,
    /// Add.
    Plus,
    /// Remove.
    Minus,
    /// Go or collapse to the left.
    ChevronLeft,
    /// Go or collapse to the right.
    ChevronRight,
    /// Go or collapse upwards.
    ChevronUp,
    /// Go or collapse downwards.
    ChevronDown,
    /// A big step down or back.
    ChevronsLeft,
    /// A big step up or on.
    ChevronsRight,
    /// Before and after, split down the middle.
    BeforeAfter,
    /// Pick a colour from the photo: the white-balance picker.
    Picker,
    /// Zoom in.
    Zoom,
    /// Import photos into the catalog.
    Import,
    /// Export photos out of the catalog.
    Export,
    /// Filter the photos shown.
    Filter,
    /// Search.
    Search,
    /// Locked.
    Lock,
    /// Unlocked.
    Unlock,
    /// Something needs attention.
    Warning,
    /// A folder on disk, or a set that holds collections.
    Folder,
    /// A collection of photos.
    Collection,
    /// A smart collection: photos gathered by rules.
    SmartCollection,
    /// Done, included, on.
    Check,
    /// Close, cancel, take away.
    Close,
}

impl Icon {
    /// Every icon, in declaration order.
    pub const ALL: [Self; 43] = [
        Self::Grid,
        Self::Loupe,
        Self::Compare,
        Self::Survey,
        Self::FlagPick,
        Self::FlagReject,
        Self::FlagNone,
        Self::StarFilled,
        Self::StarEmpty,
        Self::RotateCcw,
        Self::RotateCw,
        Self::Flip,
        Self::FlipVertical,
        Self::Crop,
        Self::Heal,
        Self::LinearMask,
        Self::RadialMask,
        Self::Brush,
        Self::Eye,
        Self::Trash,
        Self::Plus,
        Self::Minus,
        Self::ChevronLeft,
        Self::ChevronRight,
        Self::ChevronUp,
        Self::ChevronDown,
        Self::ChevronsLeft,
        Self::ChevronsRight,
        Self::BeforeAfter,
        Self::Picker,
        Self::Zoom,
        Self::Import,
        Self::Export,
        Self::Filter,
        Self::Search,
        Self::Lock,
        Self::Unlock,
        Self::Warning,
        Self::Folder,
        Self::Collection,
        Self::SmartCollection,
        Self::Check,
        Self::Close,
    ];

    /// A short name for the icon, usable as a label.
    pub fn name(self) -> &'static str {
        match self {
            Self::Grid => "Grid",
            Self::Loupe => "Loupe",
            Self::Compare => "Compare",
            Self::Survey => "Survey",
            Self::FlagPick => "Pick",
            Self::FlagReject => "Reject",
            Self::FlagNone => "Unflagged",
            Self::StarFilled => "Star",
            Self::StarEmpty => "No star",
            Self::RotateCcw => "Rotate left",
            Self::RotateCw => "Rotate right",
            Self::Flip => "Flip",
            Self::FlipVertical => "Flip vertical",
            Self::Crop => "Crop",
            Self::Heal => "Heal",
            Self::LinearMask => "Linear mask",
            Self::RadialMask => "Radial mask",
            Self::Brush => "Brush",
            Self::Eye => "Show",
            Self::Trash => "Delete",
            Self::Plus => "Add",
            Self::Minus => "Remove",
            Self::ChevronLeft => "Left",
            Self::ChevronRight => "Right",
            Self::ChevronUp => "Up",
            Self::ChevronDown => "Down",
            Self::ChevronsLeft => "Far left",
            Self::ChevronsRight => "Far right",
            Self::BeforeAfter => "Before and after",
            Self::Picker => "Picker",
            Self::Zoom => "Zoom",
            Self::Import => "Import",
            Self::Export => "Export",
            Self::Filter => "Filter",
            Self::Search => "Search",
            Self::Lock => "Lock",
            Self::Unlock => "Unlock",
            Self::Warning => "Warning",
            Self::Folder => "Folder",
            Self::Collection => "Collection",
            Self::SmartCollection => "Smart collection",
            Self::Check => "Check",
            Self::Close => "Close",
        }
    }
}

/// A square button that shows only an icon.
///
/// `label` is not drawn: it is the accessible name (and the tooltip), which
/// is the only way a test, a script or a screen reader can tell one icon
/// button from another.
#[must_use = "add it with `ui.add(..)`"]
pub struct IconButton<'a> {
    icon: Icon,
    label: &'a str,
    selected: Option<bool>,
    side: f32,
}

impl<'a> IconButton<'a> {
    /// A button showing `icon`, named `label`.
    pub fn new(icon: Icon, label: &'a str) -> Self {
        Self {
            icon,
            label,
            selected: None,
            side: size::ICON_BUTTON,
        }
    }

    /// Make this a toggle that is currently on or off (a view mode, a tool):
    /// an "on" button stays lit, and the state is exposed to accessibility.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    /// The edge of the button in points. The icon scales with it.
    pub fn side(mut self, side: f32) -> Self {
        self.side = side;
        self
    }
}

impl egui::Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.side), Sense::click());
        let enabled = ui.is_enabled();
        response.widget_info(|| match self.selected {
            Some(selected) => WidgetInfo::selected(Role::Button, enabled, selected, self.label),
            None => WidgetInfo::labeled(Role::Button, enabled, self.label),
        });

        if ui.is_rect_visible(rect) {
            let on = self.selected == Some(true);
            let pressed = response.is_pointer_button_down_on();
            let fill = if pressed {
                Some(palette::PRESSED)
            } else if response.hovered() {
                Some(palette::HOVER)
            } else if on {
                Some(palette::RAISED)
            } else {
                None
            };
            let rounding = CornerRadius::same(radius::MD);
            if let Some(fill) = fill {
                ui.painter().rect_filled(rect, rounding, fill);
            }
            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect,
                    rounding,
                    Stroke::new(1.0, palette::ACCENT),
                    StrokeKind::Inside,
                );
            }
            let color = icon_color(enabled, on, response.hovered() || pressed);
            let glyph = rect.shrink((self.side * (1.0 - size::ICON / size::ICON_BUTTON)) / 2.0);
            self.icon.paint(ui.painter(), glyph, color);
        }
        response.on_hover_text(self.label)
    }
}

/// The colour of an icon in a control: off when disabled, strong when it is
/// on or under the pointer, quiet otherwise.
fn icon_color(enabled: bool, on: bool, lit: bool) -> Color32 {
    if !enabled {
        palette::TEXT_OFF
    } else if on || lit {
        palette::TEXT_STRONG
    } else {
        palette::TEXT
    }
}

/// Show an icon button. See [`IconButton`] for toggles and other sizes.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str) -> Response {
    ui.add(IconButton::new(icon, label))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_icon_is_listed_once_and_has_its_own_name() {
        let icons: HashSet<Icon> = Icon::ALL.into_iter().collect();
        assert_eq!(icons.len(), Icon::ALL.len());
        let names: HashSet<&str> = Icon::ALL.into_iter().map(Icon::name).collect();
        assert_eq!(names.len(), Icon::ALL.len());
        assert!(names.iter().all(|name| !name.trim().is_empty()));
    }

    #[test]
    fn a_disabled_icon_is_dimmer_than_an_idle_one_which_is_dimmer_than_a_lit_one() {
        let level = |color: Color32| color.r();
        assert!(level(icon_color(false, true, true)) < level(icon_color(true, false, false)));
        assert!(level(icon_color(true, false, false)) < level(icon_color(true, false, true)));
        assert_eq!(icon_color(true, true, false), icon_color(true, false, true));
    }
}
