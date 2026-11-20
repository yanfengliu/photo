//! The red mask overlay: the coverage of one mask of the open photo over
//! the canvas view, computed by the render worker like a frame and kept in
//! step with the mask's shape, the photo's geometry and the view.

use std::sync::Arc;

use photo_core::develop::DevelopSettings;
use photo_develop::Quality;
use photo_imaging::{Plane, View};

use super::worker::Purpose;
use super::{DevelopSession, tools};
use crate::engine::Engine;
use crate::error::EngineError;

/// The coverage of one mask over the canvas view: what the red mask overlay
/// shows. Cloning is cheap: the plane is shared.
#[derive(Clone, Debug)]
pub struct MaskOverlay {
    pub(crate) mask: usize,
    pub(crate) alpha: Arc<Plane>,
    pub(crate) view: View,
    pub(crate) version: u64,
}

impl MaskOverlay {
    /// Which mask: its index in the settings' list of masks.
    pub fn mask(&self) -> usize {
        self.mask
    }

    /// The coverage, 0 to 1, one value per pixel of [`Self::view`], row by
    /// row: the mask's shape (components and invert) whether or not it is
    /// enabled, exactly as `photo_develop::mask_alpha` gives it.
    pub fn alpha(&self) -> &Arc<Plane> {
        &self.alpha
    }

    /// The develop view the coverage is of.
    pub fn view(&self) -> View {
        self.view
    }

    /// The settings version it was computed from (see
    /// [`DevelopSession::version`]). It holds for later versions until the
    /// mask's shape, the photo's geometry or the view change, when a new
    /// one is computed.
    pub fn version(&self) -> u64 {
        self.version
    }
}

impl DevelopSession {
    /// The newest mask overlay, while one is shown (see
    /// [`Engine::show_mask_overlay`]).
    pub fn mask_overlay(&self) -> Option<&MaskOverlay> {
        self.overlay.as_ref()
    }
}

/// Whether going from `before` to `after` changes what the overlay of mask
/// `mask` shows.
pub(crate) fn overlay_moves(
    before: &DevelopSettings,
    after: &DevelopSettings,
    mask: usize,
) -> bool {
    let shape = before
        .masks
        .get(mask)
        .map(|mask| (&mask.components, mask.invert));
    let moved = after
        .masks
        .get(mask)
        .map(|mask| (&mask.components, mask.invert));
    shape != moved || tools::geometry_differs(before, after)
}

impl Engine {
    /// Show the overlay of mask `mask` of the open photo, or none. Its
    /// coverage is computed in the background for the canvas view and
    /// arrives in [`DevelopSession::mask_overlay`], again whenever the
    /// mask's shape changes (during a drag too), the photo's geometry or the
    /// view does. Fails, changing nothing, when there is no such mask.
    pub fn show_mask_overlay(&mut self, mask: Option<usize>) -> Result<(), EngineError> {
        let session = self.develop.as_mut().ok_or(EngineError::NoDevelopSession)?;
        if let Some(index) = mask
            && index >= session.settings.masks.len()
        {
            return Err(EngineError::NoSuchMask(index));
        }
        if session.overlay_mask != mask {
            session.overlay = None;
        }
        session.overlay_mask = mask;
        self.render_session(Purpose::Overlay, Quality::Full);
        Ok(())
    }
}
