//! Still renders: one photo through the develop pipeline at full
//! resolution (or capped to a long edge), on the calling thread, for the
//! automation harness, which holds the engine only by shared reference.

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::DevelopSettings;
use photo_core::image::{Rgb16Image, Rgba8Image};
use photo_core::meta::PhotoId;
use photo_develop::{CancelToken, Quality, RenderRequest, Renderer};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::stamp::Stamp;

/// Which develop settings a still render uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StillSettings {
    /// What the photo shows now: for the photo open in Develop, its
    /// settings with any edit not yet committed; otherwise the catalog's.
    Current,
    /// The settings a newly imported photo of its kind starts with.
    Default,
}

impl Engine {
    /// Render `photo` with `settings`, as opaque 8-bit sRGB RGBA, at full
    /// resolution or, with `long_edge`, at most that many pixels along its
    /// longer side (never enlarged). The size is that of the photo's output
    /// as cropped and turned.
    ///
    /// This blocks the calling thread for the whole render, and the decode
    /// too unless the file is in the engine's cache of decoded files (where
    /// it is kept for later renders). It is for the automation harness, not
    /// for a frame of the UI.
    pub fn render_photo_rgba8(
        &self,
        photo: PhotoId,
        settings: StillSettings,
        long_edge: Option<u32>,
    ) -> Result<Rgba8Image, EngineError> {
        self.render_still(photo, settings, long_edge, |renderer, request, cancel| {
            renderer.render_rgba8(request, cancel)
        })
    }

    /// As [`Engine::render_photo_rgba8`], as 16-bit sRGB RGB: the
    /// pipeline's full export precision. Blocks the same way.
    pub fn render_photo_rgb16(
        &self,
        photo: PhotoId,
        settings: StillSettings,
        long_edge: Option<u32>,
    ) -> Result<Rgb16Image, EngineError> {
        self.render_still(photo, settings, long_edge, |renderer, request, cancel| {
            renderer.render_rgb16(request, cancel)
        })
    }

    fn render_still<T>(
        &self,
        photo: PhotoId,
        choice: StillSettings,
        long_edge: Option<u32>,
        render: impl FnOnce(&mut Renderer, &RenderRequest<'_>, &CancelToken) -> Option<T>,
    ) -> Result<T, EngineError> {
        let record = self.catalog.photo(photo)?;
        let settings = match choice {
            StillSettings::Default => DevelopSettings::default_for(record.kind),
            StillSettings::Current => match &self.develop {
                Some(session) if session.photo == photo => session.settings.clone(),
                _ => self.catalog.develop_settings(photo)?,
            },
        };
        let unreadable = |reason: String| EngineError::Render(reason);
        let stamp = Stamp::read(&record.path).map_err(|error| {
            unreadable(format!("cannot read {}: {error}", record.path.display()))
        })?;
        let source = self
            .sources
            .decode(&record.path, stamp)
            .map_err(|error| unreadable(error.to_string()))?;
        let (width, height) = photo_develop::output_size(&source, &settings);
        let (box_width, box_height) = match long_edge {
            Some(edge) => (edge.max(1), edge.max(1)),
            None => (width, height),
        };
        let request = RenderRequest {
            source: &source,
            settings: &settings,
            view: photo_develop::fit_view(&source, &settings, box_width, box_height),
            quality: Quality::Full,
            space: RgbSpace::Srgb,
        };
        render(&mut Renderer::new(), &request, &CancelToken::new())
            .ok_or_else(|| unreadable("the render was stopped".to_string()))
    }
}
