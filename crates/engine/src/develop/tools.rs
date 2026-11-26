//! What the Develop tools ask of the open photo: automatic tone and white
//! balance, the white-balance picker, the before view and the preset hover
//! preview. Each needs the decoded photo.

use photo_core::develop::DevelopSettings;
use photo_core::geom::Point;
use photo_core::meta::{HistoryId, SnapshotId};
use photo_develop::{Quality, Source};

use super::worker::Purpose;
use crate::engine::Engine;
use crate::error::EngineError;
use crate::looks::PresetKey;

/// What the before view shows, beside or instead of the edit. Whatever it
/// is, it is shown with the edit's crop, orientation, transform and lens
/// distortion, so the two sides line up point for point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BeforeSettings {
    /// The settings a newly imported photo of this kind starts with.
    Default,
    /// A snapshot of the photo.
    Snapshot(SnapshotId),
    /// A step of the photo's history.
    HistoryStep(HistoryId),
}

/// `before` laid out like `after`: the before view's settings.
pub(crate) fn with_geometry(before: &DevelopSettings, after: &DevelopSettings) -> DevelopSettings {
    let mut shown = before.clone();
    shown.orientation = after.orientation;
    shown.crop = after.crop;
    shown.transform = after.transform;
    shown.lens.distortion = after.lens.distortion;
    shown
}

/// Whether the two settings place the photo differently in the output.
pub(crate) fn geometry_differs(a: &DevelopSettings, b: &DevelopSettings) -> bool {
    a.orientation != b.orientation
        || a.crop != b.crop
        || a.transform != b.transform
        || a.lens.distortion != b.lens.distortion
}

impl Engine {
    /// The open photo's decoded source and current settings.
    fn decoded(&self) -> Result<(Source, DevelopSettings), EngineError> {
        let session = self.develop.as_ref().ok_or(EngineError::NoDevelopSession)?;
        let source = session.decoded().ok_or(EngineError::StillLoading)?;
        Ok((source.clone(), session.settings.clone()))
    }

    /// Choose what the before view shows, or `None` to turn it off. The
    /// before frame is rendered for the canvas's view and kept while the
    /// edit changes; it is rendered again only when the view, the choice or
    /// the edit's geometry changes.
    pub fn set_before(&mut self, choice: Option<BeforeSettings>) -> Result<(), EngineError> {
        let session = self.develop.as_ref().ok_or(EngineError::NoDevelopSession)?;
        let base = match choice {
            Some(choice) => Some((
                choice,
                self.before_base(session.photo, session.kind, choice)?,
            )),
            None => None,
        };
        let issued = self.next_render;
        if let Some(session) = self.develop.as_mut() {
            if session.before == base {
                return Ok(());
            }
            session.before = base;
            session.before_frame = None;
            // A frame asked for the choice before this one is not kept.
            session.before_since = issued;
        }
        self.render_session(Purpose::Before, Quality::Full);
        Ok(())
    }

    /// Set the open photo's tone automatically (exposure, highlights,
    /// shadows, whites, blacks; contrast is kept) and commit it as one step.
    pub fn auto_tone(&mut self) -> Result<(), EngineError> {
        self.editable_session()?;
        let (source, mut settings) = self.decoded()?;
        settings.tone = photo_develop::auto_tone(&source, &settings);
        self.apply_settings(settings)
    }

    /// Set the open photo's white balance automatically and commit it.
    pub fn auto_white_balance(&mut self) -> Result<(), EngineError> {
        self.editable_session()?;
        let (source, mut settings) = self.decoded()?;
        settings.white_balance = photo_develop::auto_white_balance(&source);
        self.apply_settings(settings)
    }

    /// The white-balance picker: make what sits at `at` (in full-resolution
    /// output pixels) neutral, and commit it. Returns false, changing
    /// nothing, when the point is off the photo or has no colour to
    /// neutralise (a pure black or clipped sample).
    pub fn pick_white_balance(&mut self, at: Point) -> Result<bool, EngineError> {
        self.editable_session()?;
        let (source, mut settings) = self.decoded()?;
        let Some(balance) = photo_develop::pick_white_balance(&source, &settings, at) else {
            return Ok(false);
        };
        settings.white_balance = balance;
        self.apply_settings(settings)?;
        Ok(true)
    }

    /// Where a new healing spot at `target` with `radius` should take its
    /// patch from, as photo-imaging suggests it: a nearby place, inside the
    /// photo and clear of the target, whose surroundings look like the
    /// target's. Both points are in normalized source coordinates and the
    /// radius a fraction of the long edge, as in a `Spot`. It reads a few
    /// dozen samples, so it is cheap enough to call on the click that
    /// places the spot. Fails when no photo is open or it is still loading.
    pub fn suggest_heal_source(&self, target: Point, radius: f32) -> Result<Point, EngineError> {
        let session = self.develop.as_ref().ok_or(EngineError::NoDevelopSession)?;
        let source = session.decoded().ok_or(EngineError::StillLoading)?;
        Ok(photo_imaging::suggest_heal_source(
            source.pyramid(),
            target,
            radius,
        ))
    }

    /// Render the open photo with a preset applied, at most `width` x
    /// `height` pixels, for the Navigator's hover preview; `None` stops
    /// showing one. The frame arrives in
    /// [`DevelopSession::preset_preview`](crate::DevelopSession::preset_preview).
    pub fn preview_preset(
        &mut self,
        preset: Option<PresetKey>,
        width: u32,
        height: u32,
    ) -> Result<(), EngineError> {
        let session = self.develop.as_ref().ok_or(EngineError::NoDevelopSession)?;
        let look = match preset {
            Some(key) => {
                let applied = self
                    .resolve_preset(&key)?
                    .applied_to(&session.settings, session.kind);
                Some((key, applied, (width.max(1), height.max(1))))
            }
            None => None,
        };
        let shown = look.is_some();
        let issued = self.next_render;
        if let Some(session) = self.develop.as_mut() {
            if session.look == look {
                return Ok(());
            }
            session.look = look;
            session.look_since = issued;
            if !shown {
                session.look_frame = None;
            }
        }
        self.render_session(Purpose::Look, Quality::Full);
        Ok(())
    }
}
