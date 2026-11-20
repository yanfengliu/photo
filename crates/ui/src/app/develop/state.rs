//! What the Develop module remembers between frames: how the canvas is
//! zoomed and compared, which panels are open, the modes of the multi-part
//! panels, and what the engine was last told, so each change is told once.

use std::collections::HashSet;

use photo_core::develop::GroupSet;
use photo_core::meta::PhotoId;
use photo_engine::{BeforeSettings, CanvasView, Engine, HistoryStep, PresetInfo, PresetKey, Snapshot};

use super::canvas::{CanvasState, Placement};
use crate::widgets::{ClippingToggles, PanelGroup};

/// How the before view is shown with the edit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BeforeAfter {
    /// The edit alone.
    #[default]
    Off,
    /// The before view in the edit's place (`\`).
    Before,
    /// The before view and the edit side by side (Y).
    SideBySide,
    /// One picture split down the middle: the before view on the left, the
    /// edit on the right (Shift+Y).
    Split,
}

impl BeforeAfter {
    /// Whether the before view is on screen.
    pub fn shows_before(self) -> bool {
        self != Self::Off
    }
}

/// Which curve the Tone Curve panel edits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum CurveChannel {
    /// The region curve: four sliders and three splits.
    #[default]
    Parametric,
    /// The point curve of all three channels.
    Rgb,
    /// The red channel's point curve.
    Red,
    /// The green channel's point curve.
    Green,
    /// The blue channel's point curve.
    Blue,
}

/// Which sliders the Color Mixer shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum MixerView {
    #[default]
    Hue,
    Saturation,
    Luminance,
    All,
}

/// What the engine was last told about the open session, so that a change
/// is told once rather than every frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Told {
    /// The photo the rest was told for; when the session changes, all of it
    /// is told again.
    pub(crate) photo: Option<PhotoId>,
    pub(crate) canvas: Option<CanvasView>,
    pub(crate) before: Option<Option<BeforeSettings>>,
    pub(crate) look: Option<Option<PresetKey>>,
}

impl Told {
    /// A session of `photo` just asked for: it has a before view and a
    /// preset preview only once told, and no canvas view yet.
    pub(crate) fn fresh(photo: Option<PhotoId>) -> Self {
        Self {
            photo,
            canvas: None,
            before: Some(None),
            look: Some(None),
        }
    }
}

/// The presets, and the open photo's snapshots and history, read again
/// only when something may have changed them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Lists {
    key: Option<(Option<PhotoId>, u64)>,
    pub(crate) presets: Vec<PresetInfo>,
    pub(crate) snapshots: Vec<Snapshot>,
    pub(crate) history: Vec<HistoryStep>,
}

impl Lists {
    /// The lists for `photo` as of `generation`.
    pub(crate) fn refresh(&mut self, engine: &Engine, photo: Option<PhotoId>, generation: u64) {
        if self.key == Some((photo, generation)) {
            return;
        }
        self.key = Some((photo, generation));
        self.presets = engine.presets().unwrap_or_else(|error| {
            log::warn!("cannot read the presets: {error}");
            Vec::new()
        });
        let Some(photo) = photo else {
            self.snapshots.clear();
            self.history.clear();
            return;
        };
        self.snapshots = engine.snapshots(photo).unwrap_or_else(|error| {
            log::warn!("cannot read the snapshots of photo {photo}: {error}");
            Vec::new()
        });
        self.history = engine.history(photo).unwrap_or_else(|error| {
            log::warn!("cannot read the history of photo {photo}: {error}");
            Vec::new()
        });
    }
}

/// The titles of the Develop panel columns.
pub(crate) const LEFT_PANELS: [&str; 4] = ["Navigator", "Presets", "Snapshots", "History"];

/// The right column's panels, top to bottom.
pub(crate) const RIGHT_PANELS: [&str; 10] = [
    "Histogram",
    "Basic",
    "Tone Curve",
    "Color Mixer",
    "Color Grading",
    "Detail",
    "Lens Corrections",
    "Transform",
    "Effects",
    "Calibration",
];

/// The keys the panel columns' open state is kept under in the catalog.
pub(crate) const LEFT_KEY: &str = "develop.panels.left";
pub(crate) const RIGHT_KEY: &str = "develop.panels.right";

/// All of the Develop module's presentation state.
pub(crate) struct DevelopState {
    pub(crate) canvas: CanvasState,
    pub(crate) before_after: BeforeAfter,
    /// What the before view shows of the photo.
    pub(crate) before_choice: BeforeSettings,
    pub(crate) clipping: ClippingToggles,
    /// The white-balance picker is waiting for a click on the photo.
    pub(crate) picker: bool,
    pub(crate) curve: CurveChannel,
    pub(crate) mixer: MixerView,
    pub(crate) left_panels: PanelGroup,
    pub(crate) right_panels: PanelGroup,
    /// The panel state as last stored in the catalog.
    pub(crate) stored_panels: (String, String),
    /// Preset groups the user has closed.
    pub(crate) closed_groups: HashSet<String>,
    /// The preset under the pointer, for the Navigator's preview.
    pub(crate) hovered_preset: Option<PresetKey>,
    /// The groups the Copy Settings dialog offers ticked.
    pub(crate) copy_groups: GroupSet,
    pub(crate) told: Told,
    pub(crate) lists: Lists,
    /// The part of the photo in view as fractions, while some is hidden;
    /// drawn on the Navigator.
    pub(crate) seen: Option<egui::Rect>,
    /// Where the edit was placed in the last frame, for the zoom keys.
    pub(crate) last_placement: Option<Placement>,
    /// The Navigator's picture box in pixels: the size preset previews are
    /// rendered at.
    pub(crate) navigator_box: (u32, u32),
}

impl Default for DevelopState {
    fn default() -> Self {
        Self {
            canvas: CanvasState::default(),
            before_after: BeforeAfter::Off,
            before_choice: BeforeSettings::Default,
            clipping: ClippingToggles::default(),
            picker: false,
            curve: CurveChannel::default(),
            mixer: MixerView::default(),
            left_panels: PanelGroup::with_open(LEFT_PANELS),
            right_panels: PanelGroup::with_open(["Histogram", "Basic"]),
            stored_panels: (String::new(), String::new()),
            closed_groups: HashSet::new(),
            hovered_preset: None,
            copy_groups: GroupSet::default_copy(),
            told: Told::default(),
            lists: Lists::default(),
            seen: None,
            last_placement: None,
            navigator_box: (480, 320),
        }
    }
}

impl DevelopState {
    /// The state with the panel columns as the catalog last stored them.
    pub(crate) fn with_stored_panels(engine: &Engine) -> Self {
        let mut state = Self::default();
        let stored = |key: &str| match engine.ui_setting(key) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("cannot read {key}: {error}");
                None
            }
        };
        if let Some(text) = stored(LEFT_KEY) {
            state.left_panels = decode_panels(&text, &LEFT_PANELS);
        }
        if let Some(text) = stored(RIGHT_KEY) {
            state.right_panels = decode_panels(&text, &RIGHT_PANELS);
        }
        // What is shown now counts as stored: the defaults are not written
        // until the user changes them.
        state.stored_panels = (
            encode_panels(&state.left_panels),
            encode_panels(&state.right_panels),
        );
        state
    }

    /// J: both clipping overlays on, or both off if either is on.
    pub(crate) fn toggle_clipping(&mut self) {
        let on = !(self.clipping.shadows || self.clipping.highlights);
        self.clipping = ClippingToggles {
            shadows: on,
            highlights: on,
        };
    }

    /// `\`, Y and Shift+Y: show `mode`, or the edit alone if it is shown.
    pub(crate) fn toggle_before_after(&mut self, mode: BeforeAfter) {
        self.before_after = if self.before_after == mode {
            BeforeAfter::Off
        } else {
            mode
        };
    }

    /// Forget what belongs to the photo that was open.
    pub(crate) fn photo_changed(&mut self) {
        self.told = Told::default();
        self.before_choice = BeforeSettings::Default;
        self.hovered_preset = None;
        self.picker = false;
    }
}

/// A panel column's open state as text: `solo` or `-` on the first line,
/// then the open panels' titles, one per line.
pub(crate) fn encode_panels(group: &PanelGroup) -> String {
    let mut text = String::from(if group.solo() { "solo" } else { "-" });
    for title in group.open_panels() {
        text.push('\n');
        text.push_str(title);
    }
    text
}

/// The open state stored as [`encode_panels`] wrote it. Titles that are not
/// among `known` (a panel that was renamed or removed) are left out.
pub(crate) fn decode_panels(text: &str, known: &[&str]) -> PanelGroup {
    let mut lines = text.lines();
    let solo = lines.next() == Some("solo");
    let mut group = PanelGroup::new();
    for title in lines.filter(|title| known.contains(title)) {
        group.set_open(title, true);
    }
    group.set_solo(solo);
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_state_reads_back_as_it_was_written() {
        let mut group = PanelGroup::with_open(["Basic", "Detail"]);
        let text = encode_panels(&group);
        assert_eq!(decode_panels(&text, &RIGHT_PANELS), group);
        group.set_solo(true);
        let solo = decode_panels(&encode_panels(&group), &RIGHT_PANELS);
        assert!(solo.solo());
        assert_eq!(solo.open_panels().collect::<Vec<_>>(), ["Detail"]);
        let unknown = decode_panels("-\nBasic\nNo Such Panel", &RIGHT_PANELS);
        assert_eq!(unknown.open_panels().collect::<Vec<_>>(), ["Basic"]);
        assert!(decode_panels("", &RIGHT_PANELS).open_panels().next().is_none());
    }

    #[test]
    fn j_turns_both_overlays_on_and_any_overlay_off() {
        let mut state = DevelopState::default();
        state.toggle_clipping();
        assert!(state.clipping.shadows && state.clipping.highlights);
        state.clipping.highlights = false;
        state.toggle_clipping();
        assert!(!state.clipping.shadows && !state.clipping.highlights);
    }

    #[test]
    fn a_before_after_key_shows_its_mode_or_goes_back_to_the_edit() {
        let mut state = DevelopState::default();
        state.toggle_before_after(BeforeAfter::SideBySide);
        assert_eq!(state.before_after, BeforeAfter::SideBySide);
        state.toggle_before_after(BeforeAfter::Before);
        assert_eq!(state.before_after, BeforeAfter::Before);
        state.toggle_before_after(BeforeAfter::Before);
        assert_eq!(state.before_after, BeforeAfter::Off);
    }
}
