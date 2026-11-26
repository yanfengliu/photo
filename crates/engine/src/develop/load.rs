//! Opening and closing a develop session, and loading its photo: the RAW
//! embedded preview first, then the decode through the shared cache of
//! decoded sources. Each stage is a job on the urgent lane and lands in
//! [`Engine::poll`] only while the session it was started for is open.
//! Opening another photo, or closing, cancels the load of the one before:
//! a stage still queued never runs, and one that was running hands nothing
//! back, so stepping quickly through photos decodes only where it stops.

use std::sync::Arc;

use photo_catalog::CatalogError;
use photo_core::develop::DevelopSettings;
use photo_core::image::Rgba8Image;
use photo_core::meta::{FileKind, PhotoId};
use photo_develop::{Quality, Source};

use super::worker::Purpose;
use super::{
    CanvasView, DevelopLoad, DevelopSession, NEWER_SETTINGS, READ_ONLY_CATALOG, RenderCounts,
    overlay,
};
use crate::engine::{Engine, JobFailure};
use crate::error::EngineError;
use crate::jobs::{CancelToken, Lane};
use crate::previews::render;
use crate::stamp::Stamp;

/// Long edge of the embedded preview a session shows while it loads.
const EMBEDDED_EDGE: u32 = 2048;

/// The start of the reason a load fails with when the file has gone.
const MISSING: &str = "the file is missing";

/// What a load cancelled by a newer one reports.
const STOPPED: &str = "loading was stopped";

impl Engine {
    /// Open `photo` in Develop. Its settings are read at once; a RAW
    /// photo's embedded preview, then its decoded pixels, follow in the
    /// background (see [`DevelopSession::load_history`]). An edit pending
    /// on the photo open before is committed first. Opening the photo
    /// already open does nothing.
    ///
    /// A photo whose settings were saved by a newer version opens with the
    /// settings a new photo starts with, and cannot be edited; nor can any
    /// photo of a write-protected catalog. [`DevelopSession::read_only_reason`]
    /// says why.
    pub fn open_develop(&mut self, photo: PhotoId) -> Result<(), EngineError> {
        if self
            .develop
            .as_ref()
            .is_some_and(|session| session.photo == photo)
        {
            return Ok(());
        }
        let record = self.catalog.photo(photo)?;
        let (settings, newer) = match self.catalog.develop_settings(photo) {
            Ok(settings) => (settings, false),
            Err(CatalogError::NewerFormat { .. }) => {
                (DevelopSettings::default_for(record.kind), true)
            }
            Err(error) => return Err(error.into()),
        };
        self.commit_edit()?;
        if let Some(previous) = self.develop.as_ref().map(|session| session.photo) {
            self.previous_active = Some(previous);
        }
        self.close_develop();
        self.develop_generation += 1;
        self.develop_loading = Some(CancelToken::default());
        let read_only = if newer {
            Some(NEWER_SETTINGS.to_string())
        } else if self.catalog.is_read_only() {
            Some(READ_ONLY_CATALOG.to_string())
        } else {
            None
        };
        self.develop = Some(DevelopSession {
            generation: self.develop_generation,
            photo,
            kind: record.kind,
            path: record.path,
            committed: settings.clone(),
            settings,
            version: 1,
            load: vec![DevelopLoad::Nothing],
            embedded: None,
            source: None,
            canvas: None,
            frame: None,
            before: None,
            before_frame: None,
            before_since: 0,
            look: None,
            look_frame: None,
            look_since: 0,
            overlay_mask: None,
            overlay: None,
            read_only,
            counts: RenderCounts::default(),
        });
        match record.kind {
            FileKind::Raw => self.load_embedded(),
            FileKind::Raster => self.load_source(),
        }
        Ok(())
    }

    /// Close the develop session, committing a pending edit; a commit that
    /// fails is reported with a toast.
    pub fn close_develop(&mut self) {
        if let Err(error) = self.commit_edit() {
            self.toasts
                .error(format!("Could not save the edit: {error}"));
        }
        self.end_session();
    }

    /// The photo open in Develop, if any.
    pub fn develop(&self) -> Option<&DevelopSession> {
        self.develop.as_ref()
    }

    /// Say what the canvas shows; the canvas renders it.
    pub fn set_canvas_view(&mut self, canvas: CanvasView) {
        let Some(session) = self.develop.as_mut() else {
            return;
        };
        if session.canvas == Some(canvas) {
            return;
        }
        session.canvas = Some(canvas);
        self.render_session(Purpose::Overlay, Quality::Full);
        self.render_session(Purpose::Main, Quality::Full);
        self.render_session(Purpose::Before, Quality::Full);
    }

    /// Bring the open photo's settings back in line with the catalog after
    /// a command changed them (undo, redo, paste, a preset), and render.
    /// Does nothing while the catalog still holds what was last committed,
    /// so an edit in progress survives a refresh that is not about it (an
    /// import, a virtual copy, a removal).
    pub(crate) fn reload_develop(&mut self) {
        let Some(session) = self.develop.as_mut() else {
            return;
        };
        let Ok(settings) = self.catalog.develop_settings(session.photo) else {
            return;
        };
        if settings == session.committed {
            return;
        }
        let overlay_moves = session
            .overlay_mask
            .is_some_and(|mask| overlay::overlay_moves(&session.settings, &settings, mask));
        session.committed = settings.clone();
        session.settings = settings;
        session.version += 1;
        if overlay_moves {
            self.render_session(Purpose::Overlay, Quality::Full);
        }
        self.render_session(Purpose::Main, Quality::Full);
        self.render_session(Purpose::Before, Quality::Full);
    }

    /// Close the session when its photo has left the catalog.
    pub(crate) fn drop_removed_develop(&mut self) {
        let removed = self
            .develop
            .as_ref()
            .is_some_and(|session| self.catalog.photo(session.photo).is_err());
        if removed {
            self.end_session();
        }
    }

    /// Forget the session and the renders asked for it, and stop loading
    /// its photo.
    fn end_session(&mut self) {
        if let Some(loading) = self.develop_loading.take() {
            loading.cancel();
        }
        for dropped in self.renderer.clear() {
            self.renders.remove(&dropped);
        }
        self.develop = None;
    }

    /// The first stage of a RAW photo: its embedded preview, then the
    /// decode.
    fn load_embedded(&mut self) {
        let Some(session) = &self.develop else {
            return;
        };
        let (generation, path) = (session.generation, session.path.clone());
        let orientation = session.settings.orientation;
        let loading = self.develop_loading.clone();
        self.submit(
            Lane::Urgent,
            None,
            loading,
            move |context| {
                let preview = render::embedded(&path, FileKind::Raw, orientation, EMBEDDED_EDGE)
                    .ok()
                    .filter(|_| !context.should_stop())
                    .map(|rendered| rendered.image);
                Box::new(move |engine: &mut Engine| engine.embedded_loaded(generation, preview))
            },
            move |engine, _| engine.embedded_loaded(generation, None),
        );
    }

    fn embedded_loaded(&mut self, generation: u64, preview: Option<Rgba8Image>) -> bool {
        let Some(session) = self.session_of(generation) else {
            return false;
        };
        if let Some(image) = preview
            && session.source.is_none()
        {
            session.embedded = Some(Arc::new(image));
            session.reach(DevelopLoad::EmbeddedPreview);
        }
        self.load_source();
        true
    }

    /// Decode the open photo, through the shared cache of decoded sources.
    fn load_source(&mut self) {
        let Some(session) = &self.develop else {
            return;
        };
        let (generation, path) = (session.generation, session.path.clone());
        let sources = self.sources.clone();
        let loading = self.develop_loading.clone();
        self.submit(
            Lane::Urgent,
            None,
            loading,
            move |context| {
                let decoded = match Stamp::read(&path) {
                    Ok(stamp) => sources
                        .decode(&path, stamp)
                        .map_err(|error| error.to_string()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        Err(format!("{MISSING}: {}", path.display()))
                    }
                    Err(error) => Err(error.to_string()),
                };
                // A photo closed meanwhile hands nothing back: its pixels
                // stay in the bounded cache only, not in a result waiting
                // to be polled in.
                let decoded = if context.should_stop() {
                    Err(STOPPED.to_string())
                } else {
                    decoded
                };
                Box::new(move |engine: &mut Engine| engine.source_loaded(generation, decoded))
            },
            move |engine, failure| {
                let reason = match failure {
                    JobFailure::Skipped => STOPPED.to_string(),
                    JobFailure::Panicked(message) => message,
                };
                engine.source_loaded(generation, Err(reason))
            },
        );
    }

    fn source_loaded(&mut self, generation: u64, decoded: Result<Source, String>) -> bool {
        let Some(session) = self.session_of(generation) else {
            return false;
        };
        match decoded {
            Ok(source) => {
                session.source = Some(source);
                session.reach(DevelopLoad::Decoded);
                self.render_session(Purpose::Overlay, Quality::Full);
                self.render_session(Purpose::Main, Quality::Full);
                self.render_session(Purpose::Before, Quality::Full);
            }
            Err(reason) => {
                let (photo, missing) = (session.photo, reason.starts_with(MISSING));
                session.reach(DevelopLoad::Failed(reason));
                if missing {
                    self.missing.push((photo, true));
                }
            }
        }
        true
    }
}
