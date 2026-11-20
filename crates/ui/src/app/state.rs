//! The presentation state: what the window shows and how, and nothing the
//! engine owns. Losing all of it loses no work.

use egui::Rect;
use photo_core::meta::PhotoId;
use photo_engine::{Engine, PhotoRecord, Target};

use super::dialogs::{Dialog, ImportSettings};
use super::library::compare::ComparePair;
use super::library::filter_bar::FilterBarState;
use super::library::{LeftState, RightState};
use crate::widgets::PanelGroup;

/// The app's modules, as the module picker lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Module {
    /// Organising: import, browse, rate, collect.
    Library,
    /// Developing one photo. Shown in the picker, not yet available.
    Develop,
}

/// The views of the Library's work area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum View {
    /// Every photo of the list as a cell.
    #[default]
    Grid,
    /// The active photo, large.
    Loupe,
    /// The active photo beside a candidate.
    Compare,
    /// The selected photos, tiled to fit.
    Survey,
}

impl View {
    /// The photos a rating, flag or label key applies to in this view: the
    /// whole selection in the grid, the photo being looked at elsewhere.
    pub(crate) fn target(self) -> Target {
        match self {
            Self::Grid => Target::Selected,
            Self::Loupe | Self::Compare | Self::Survey => Target::Active,
        }
    }
}

/// How the loupe fits the photo into the work area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Zoom {
    /// The whole photo, as large as fits.
    #[default]
    Fit,
    /// The work area filled, the overflow cropped (drag to pan).
    Fill,
    /// One pixel of the preview to one pixel of the screen (drag to pan).
    OneToOne,
}

/// Which parts of the window around the work area are shown. Each can be
/// hidden on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Layout {
    /// The bar across the top: the app name and the module picker.
    pub top_bar: bool,
    /// The left panel column.
    pub left: bool,
    /// The right panel column.
    pub right: bool,
    /// The filmstrip across the bottom, with its source line.
    pub filmstrip: bool,
    /// The toolbar under the work area.
    pub toolbar: bool,
}

impl Default for Layout {
    /// Everything shown.
    fn default() -> Self {
        Self {
            top_bar: true,
            left: true,
            right: true,
            filmstrip: true,
            toolbar: true,
        }
    }
}

impl Layout {
    /// Tab: hide both side columns if either is shown, else show both.
    pub(crate) fn toggle_sides(&mut self) {
        let show = !(self.left || self.right);
        self.left = show;
        self.right = show;
    }

    /// Shift+Tab: hide all four edges if any is shown, else show all four.
    /// The toolbar is not an edge and stays as it is.
    pub(crate) fn toggle_edges(&mut self) {
        let show = !(self.top_bar || self.left || self.right || self.filmstrip);
        self.top_bar = show;
        self.left = show;
        self.right = show;
        self.filmstrip = show;
    }
}

/// The loupe's zoom and where the photo is panned to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LoupeState {
    pub(crate) zoom: Zoom,
    /// The zoom Space and Z go back to from 1:1.
    pub(crate) fitted: Zoom,
    /// How far the photo's centre is from the work area's centre, in
    /// points.
    pub(crate) pan: egui::Vec2,
}

impl LoupeState {
    /// Space or Z: between the fitted zoom and 1:1, centred.
    pub(crate) fn toggle(&mut self) {
        if self.zoom == Zoom::OneToOne {
            self.zoom = self.fitted;
        } else {
            self.fitted = self.zoom;
            self.zoom = Zoom::OneToOne;
        }
        self.pan = egui::Vec2::ZERO;
    }

    /// Choose a zoom from the toolbar.
    pub(crate) fn set(&mut self, zoom: Zoom) {
        if zoom != Zoom::OneToOne {
            self.fitted = zoom;
        }
        self.zoom = zoom;
        self.pan = egui::Vec2::ZERO;
    }
}

/// What the grid remembers between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct GridState {
    /// Cells per row in the last frame, for the Up and Down keys.
    pub(crate) columns: usize,
    /// The active photo of the last frame: when it changes, the grid and
    /// the filmstrip scroll to it.
    pub(crate) last_active: Option<PhotoId>,
    /// Scroll the grid and the filmstrip to the active photo this frame.
    pub(crate) reveal_active: bool,
}

/// The active photo's full record, read again only when something may
/// have changed it (another photo, or a new generation of changes).
#[derive(Clone, Debug, Default)]
pub(crate) struct RecordCache {
    key: Option<(PhotoId, u64)>,
    record: Option<PhotoRecord>,
}

impl RecordCache {
    /// The record of `photo` as of `generation`.
    pub(crate) fn get(
        &mut self,
        engine: &Engine,
        photo: PhotoId,
        generation: u64,
    ) -> Option<&PhotoRecord> {
        if self.key != Some((photo, generation)) {
            self.key = Some((photo, generation));
            self.record = match engine.photo(photo) {
                Ok(record) => Some(record),
                Err(error) => {
                    log::warn!("cannot read photo {photo}: {error}");
                    None
                }
            };
        }
        self.record.as_ref()
    }
}

/// All presentation state of the window.
pub(crate) struct UiState {
    pub(crate) view: View,
    pub(crate) layout: Layout,
    pub(crate) left_panels: PanelGroup,
    pub(crate) right_panels: PanelGroup,
    /// The edge of a grid cell the size slider asks for, in points.
    pub(crate) thumb_size: f32,
    pub(crate) loupe: LoupeState,
    /// The part of the photo the loupe shows, as fractions of its size,
    /// while part of it is out of view; drawn on the navigator.
    pub(crate) loupe_region: Option<Rect>,
    pub(crate) compare: ComparePair,
    pub(crate) grid: GridState,
    pub(crate) filter_bar: FilterBarState,
    pub(crate) left: LeftState,
    pub(crate) right: RightState,
    pub(crate) record: RecordCache,
    pub(crate) dialog: Option<Dialog>,
    /// The Import dialog's choices, kept from one import to the next.
    pub(crate) import_settings: ImportSettings,
}

/// The size of grid cells unless the slider says otherwise.
pub(crate) const DEFAULT_THUMB_SIZE: f32 = 168.0;
/// The range of the thumbnail-size slider.
pub(crate) const THUMB_SIZES: std::ops::RangeInclusive<f32> = 96.0..=320.0;

impl Default for UiState {
    fn default() -> Self {
        Self {
            view: View::Grid,
            layout: Layout::default(),
            left_panels: PanelGroup::with_open(["Navigator", "Catalog", "Folders", "Collections"]),
            right_panels: PanelGroup::with_open([
                "Histogram",
                "Keywording",
                "Keyword List",
                "Metadata",
            ]),
            thumb_size: DEFAULT_THUMB_SIZE,
            loupe: LoupeState::default(),
            loupe_region: None,
            compare: ComparePair::default(),
            grid: GridState::default(),
            filter_bar: FilterBarState::default(),
            left: LeftState::default(),
            right: RightState::default(),
            record: RecordCache::default(),
            dialog: None,
            import_settings: ImportSettings::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_toggles_the_sides_together_and_shift_tab_every_edge() {
        let mut layout = Layout::default();
        layout.toggle_sides();
        assert!(!layout.left && !layout.right && layout.top_bar && layout.filmstrip);
        layout.toggle_sides();
        assert!(layout.left && layout.right);

        layout.left = false;
        layout.toggle_sides();
        assert!(
            !layout.left && !layout.right,
            "either side shown hides both"
        );

        let mut layout = Layout::default();
        layout.toggle_edges();
        assert!(!layout.top_bar && !layout.left && !layout.right && !layout.filmstrip);
        assert!(layout.toolbar, "the toolbar is not an edge");
        layout.toggle_edges();
        assert_eq!(layout, Layout::default());
    }

    #[test]
    fn space_goes_between_the_fitted_zoom_and_one_to_one() {
        let mut loupe = LoupeState::default();
        loupe.set(Zoom::Fill);
        loupe.pan = egui::vec2(30.0, 0.0);
        loupe.toggle();
        assert_eq!((loupe.zoom, loupe.pan), (Zoom::OneToOne, egui::Vec2::ZERO));
        loupe.toggle();
        assert_eq!(loupe.zoom, Zoom::Fill, "back to the zoom it came from");
    }

    #[test]
    fn keys_apply_to_the_selection_in_the_grid_and_to_the_photo_elsewhere() {
        assert_eq!(View::Grid.target(), Target::Selected);
        for view in [View::Loupe, View::Compare, View::Survey] {
            assert_eq!(view.target(), Target::Active);
        }
    }
}
