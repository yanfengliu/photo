//! The develop session: one photo open for editing.
//!
//! Opening a photo moves it through explicit load states, each recorded in
//! order (see [`DevelopLoad`]): a RAW photo shows its embedded camera
//! preview while the file is decoded in the background, then renders (see
//! `load`). The UI says what it shows ([`CanvasView`]); the engine turns
//! that into a develop view and keeps the newest frame for it, rendered by
//! the session's own render worker (see `worker` and `frames`). Edits are in
//! `edit`, changes to many photos at once in `batch`, the automatic and
//! picked values and the before view in `tools`.

mod batch;
mod edit;
mod frames;
mod load;
mod tools;
pub(crate) mod worker;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use photo_core::develop::{DevelopSettings, WbPreset};
use photo_core::geom::{Point, Rect};
use photo_core::image::Rgba8Image;
use photo_core::meta::{FileKind, PhotoId};
use photo_develop::{Quality, Source};
use photo_imaging::{Geometry, Histogram, View};

pub use tools::BeforeSettings;

use crate::engine::Engine;
use crate::looks::PresetKey;

/// Why editing is refused on a write-protected catalog.
pub(crate) const READ_ONLY_CATALOG: &str =
    "the catalog file is write-protected, so nothing in it can be changed";

/// Why editing is refused on settings from a newer build.
pub(crate) const NEWER_SETTINGS: &str = "this photo's develop settings were saved by a newer version of Photo and cannot be read here, so it is shown without them and cannot be edited";

/// What the develop canvas shows, as the UI describes it. The engine turns
/// it into a develop view of the photo's output, as cropped and turned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanvasView {
    /// The whole photo fitted inside a box of this many pixels, never
    /// enlarged.
    Fit {
        /// Box width in pixels.
        width: u32,
        /// Box height in pixels.
        height: u32,
    },
    /// One screen pixel per photo pixel: a box of this many pixels centred
    /// on an output point (in full-resolution output pixels).
    ActualSize {
        /// The output point at the centre of the box.
        center: Point,
        /// Box width in pixels.
        width: u32,
        /// Box height in pixels.
        height: u32,
    },
    /// An output region at a scale: `scale` screen pixels per output pixel
    /// (2 is 200%).
    Region {
        /// The output rectangle shown.
        roi: Rect,
        /// Screen pixels per output pixel.
        scale: f32,
    },
}

/// How far an open photo has got. A session moves forward through these
/// and never back, except that an edit after [`DevelopLoad::Rendered`]
/// keeps it there while the next frame renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DevelopLoad {
    /// Nothing to show yet.
    Nothing,
    /// The camera's embedded preview is shown (RAW files only).
    EmbeddedPreview,
    /// The file is decoded; the first render is on its way.
    Decoded,
    /// A rendered frame is shown.
    Rendered,
    /// The file could not be read, for this reason.
    Failed(String),
}

/// A rendered frame. Cloning is cheap: the pixels are shared.
#[derive(Clone, Debug)]
pub struct DevelopFrame {
    image: Arc<Rgba8Image>,
    view: View,
    version: u64,
    quality: Quality,
    stage_times: Arc<[(&'static str, Duration)]>,
    render_time: Duration,
    histogram: Arc<Histogram>,
}

impl DevelopFrame {
    /// The pixels: opaque sRGB RGBA8, exactly the size of [`Self::view`].
    pub fn image(&self) -> &Arc<Rgba8Image> {
        &self.image
    }

    /// The develop view these pixels show.
    pub fn view(&self) -> View {
        self.view
    }

    /// The settings version shown (see [`DevelopSession::version`]).
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Draft while a slider moves, full otherwise.
    pub fn quality(&self) -> Quality {
        self.quality
    }

    /// How long each stage that ran took, in order, for profiling.
    pub fn stage_times(&self) -> &[(&'static str, Duration)] {
        &self.stage_times
    }

    /// How long the whole render took.
    pub fn render_time(&self) -> Duration {
        self.render_time
    }

    /// The histogram of exactly these pixels.
    pub fn histogram(&self) -> &Arc<Histogram> {
        &self.histogram
    }
}

/// What happened to the session's render requests: once the session is
/// idle, every request was rendered, superseded by a newer one (dropped
/// while waiting, or cancelled while running), or failed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RenderCounts {
    /// Requests made.
    pub requested: u64,
    /// Requests that produced a frame.
    pub rendered: u64,
    /// Requests a newer one replaced before they finished.
    pub superseded: u64,
    /// Requests whose render failed (each reported with a toast).
    pub failed: u64,
}

/// The photo open in Develop.
#[derive(Debug)]
pub struct DevelopSession {
    pub(crate) generation: u64,
    pub(crate) photo: PhotoId,
    pub(crate) kind: FileKind,
    pub(crate) path: PathBuf,
    /// What is shown and edited, including a drag not yet committed.
    pub(crate) settings: DevelopSettings,
    /// What the catalog holds.
    pub(crate) committed: DevelopSettings,
    pub(crate) version: u64,
    pub(crate) load: Vec<DevelopLoad>,
    pub(crate) embedded: Option<Arc<Rgba8Image>>,
    pub(crate) source: Option<Source>,
    pub(crate) canvas: Option<CanvasView>,
    pub(crate) frame: Option<DevelopFrame>,
    pub(crate) before: Option<(BeforeSettings, DevelopSettings)>,
    pub(crate) before_frame: Option<DevelopFrame>,
    pub(crate) look: Option<(PresetKey, DevelopSettings, (u32, u32))>,
    pub(crate) look_frame: Option<(PresetKey, DevelopFrame)>,
    pub(crate) read_only: Option<String>,
    pub(crate) counts: RenderCounts,
}

impl DevelopSession {
    /// The photo.
    pub fn photo(&self) -> PhotoId {
        self.photo
    }

    /// Raster or RAW.
    pub fn kind(&self) -> FileKind {
        self.kind
    }

    /// The settings shown: the catalog's, plus any edit not yet committed.
    pub fn settings(&self) -> &DevelopSettings {
        &self.settings
    }

    /// True while an edit has not been committed.
    pub fn is_editing(&self) -> bool {
        self.settings != self.committed
    }

    /// Increases with every change to [`Self::settings`]; a frame says
    /// which version it shows.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Where loading has got.
    pub fn load_state(&self) -> &DevelopLoad {
        self.load.last().unwrap_or(&DevelopLoad::Nothing)
    }

    /// Every load state the session has been in since it was opened, in
    /// order.
    pub fn load_history(&self) -> &[DevelopLoad] {
        &self.load
    }

    /// The embedded camera preview, upright, shown until the first frame.
    pub fn embedded_preview(&self) -> Option<&Arc<Rgba8Image>> {
        self.embedded.as_ref()
    }

    /// The newest frame of the canvas.
    pub fn frame(&self) -> Option<&DevelopFrame> {
        self.frame.as_ref()
    }

    /// The newest frame of the before view, when one is chosen.
    pub fn before_frame(&self) -> Option<&DevelopFrame> {
        self.before_frame.as_ref()
    }

    /// What the before view shows, when one is chosen.
    pub fn before_settings(&self) -> Option<BeforeSettings> {
        self.before.as_ref().map(|(choice, _)| *choice)
    }

    /// The newest preset hover preview, with the preset it shows.
    pub fn preset_preview(&self) -> Option<(&PresetKey, &DevelopFrame)> {
        self.look_frame.as_ref().map(|(key, frame)| (key, frame))
    }

    /// Why this photo cannot be edited, if it cannot.
    pub fn read_only_reason(&self) -> Option<&str> {
        self.read_only.as_deref()
    }

    /// What became of the render requests.
    pub fn render_counts(&self) -> RenderCounts {
        self.counts
    }

    /// The decoded photo, once decoded.
    pub(crate) fn decoded(&self) -> Option<&Source> {
        self.source.as_ref()
    }

    /// The camera's as-shot white balance as `(kelvin, tint)`: for a RAW
    /// photo once decoded; `None` for a raster file, whose as-shot state is
    /// the file itself.
    pub fn as_shot_white_balance(&self) -> Option<(f32, f32)> {
        self.source
            .as_ref()
            .and_then(photo_develop::as_shot_white_balance)
    }

    /// The temperature and tint the Basic panel shows: the as-shot values
    /// when the white balance is as shot (`(0, 0)` for a raster file),
    /// otherwise the setting's own. `None` for a RAW photo still loading
    /// with an as-shot white balance.
    pub fn shown_white_balance(&self) -> Option<(f32, f32)> {
        let balance = &self.settings.white_balance;
        match (balance.preset, self.kind) {
            (WbPreset::AsShot, FileKind::Raster) => Some((0.0, 0.0)),
            (WbPreset::AsShot, FileKind::Raw) => self.as_shot_white_balance(),
            _ => Some((balance.temp, balance.tint)),
        }
    }

    /// The full-resolution size of the photo as cropped and turned: what
    /// every view, overlay and export of it agrees on. `None` until decoded.
    pub fn output_size(&self) -> Option<(u32, u32)> {
        let source = self.source.as_ref()?;
        Some(photo_develop::output_size(source, &self.settings))
    }

    /// The mapping between the decoded source and the output for the
    /// current settings, for tool overlays. `None` until decoded.
    pub fn geometry(&self) -> Option<Geometry> {
        let source = self.source.as_ref()?;
        Some(photo_develop::geometry(source, &self.settings))
    }

    /// The develop view the canvas renders. `None` until decoded and given a
    /// [`CanvasView`].
    pub fn view(&self) -> Option<View> {
        let source = self.source.as_ref()?;
        Some(develop_view(source, &self.settings, self.canvas?))
    }

    fn reach(&mut self, state: DevelopLoad) {
        if self.load.last() != Some(&state) {
            self.load.push(state);
        }
    }
}

/// The develop view for what the canvas shows.
pub(crate) fn develop_view(
    source: &Source,
    settings: &DevelopSettings,
    canvas: CanvasView,
) -> View {
    match canvas {
        CanvasView::Fit { width, height } => {
            photo_develop::fit_view(source, settings, width, height)
        }
        CanvasView::ActualSize {
            center,
            width,
            height,
        } => {
            let (half_width, half_height) = (width as f32 / 2.0, height as f32 / 2.0);
            photo_develop::actual_size_view(Rect::new(
                center.x - half_width,
                center.y - half_height,
                center.x + half_width,
                center.y + half_height,
            ))
        }
        CanvasView::Region { roi, scale } => photo_develop::scaled_view(roi, scale),
    }
}

impl Engine {
    /// The session, when it is still the one opened as `generation`.
    fn session_of(&mut self, generation: u64) -> Option<&mut DevelopSession> {
        self.develop
            .as_mut()
            .filter(|session| session.generation == generation)
    }
}
