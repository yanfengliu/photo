//! Editing the open photo. While a control moves, the settings change in
//! memory only and the canvas renders drafts; letting go commits once: one
//! catalog write, one history step labelled with what changed, one undo
//! entry, then a full-quality render.

use photo_catalog::DevelopEdit;
use photo_core::develop::{
    Adj, DevelopSettings, LocalAdj, WbPreset, WhiteBalance, describe_change,
};
use photo_develop::Quality;

use super::worker::Purpose;
use super::{DevelopSession, READ_ONLY_CATALOG, overlay, tools};
use crate::engine::Engine;
use crate::error::EngineError;
use crate::undo::Command;

impl Engine {
    /// The open session, if its photo may be edited.
    pub(crate) fn editable_session(&self) -> Result<&DevelopSession, EngineError> {
        let session = self.develop.as_ref().ok_or(EngineError::NoDevelopSession)?;
        if self.catalog.is_read_only() {
            return Err(EngineError::NotEditable(READ_ONLY_CATALOG.to_string()));
        }
        match &session.read_only {
            Some(reason) => Err(EngineError::NotEditable(reason.clone())),
            None => Ok(session),
        }
    }

    /// Set one adjustment of the open photo, in memory, and render a draft.
    /// The value is held to the slider's range for the photo's kind. Moving
    /// temperature or tint away from a preset makes the white balance
    /// custom, starting from the values the preset showed.
    ///
    /// Fails, changing nothing, when no photo is open, when it cannot be
    /// edited, or for temperature and tint of a RAW photo still loading.
    pub fn adjust(&mut self, adj: Adj, value: f32) -> Result<(), EngineError> {
        let session = self.editable_session()?;
        let mut settings = session.settings.clone();
        if matches!(adj, Adj::Temperature | Adj::Tint)
            && settings.white_balance.preset != WbPreset::Custom
        {
            let (temp, tint) = session
                .shown_white_balance()
                .ok_or(EngineError::StillLoading)?;
            settings.white_balance = WhiteBalance {
                preset: WbPreset::Custom,
                temp,
                tint,
            };
        }
        adj.set(&mut settings, adj.spec_for(session.kind).clamp(value));
        self.update_session(settings);
        Ok(())
    }

    /// Set one local adjustment of mask `mask` of the open photo, in
    /// memory, and render a draft. Fails, changing nothing, when there is
    /// no such mask.
    pub fn adjust_local(
        &mut self,
        mask: usize,
        adj: LocalAdj,
        value: f32,
    ) -> Result<(), EngineError> {
        let session = self.editable_session()?;
        let mut settings = session.settings.clone();
        let target = settings
            .masks
            .get_mut(mask)
            .ok_or(EngineError::NoSuchMask(mask))?;
        adj.set(&mut target.adjust, adj.spec().clamp(value));
        self.update_session(settings);
        Ok(())
    }

    /// Replace the open photo's settings, in memory, and render a draft: for
    /// tools that change several values at once (a crop handle, a mask, a
    /// curve). The settings are first held to their valid ranges.
    pub fn edit_settings(&mut self, settings: DevelopSettings) -> Result<(), EngineError> {
        let kind = self.editable_session()?.kind;
        let mut settings = settings;
        settings.sanitize_for(kind);
        self.update_session(settings);
        Ok(())
    }

    /// Commit what changed since the last commit: one catalog write, one
    /// history step and one undo entry, both labelled with what changed,
    /// then a full-quality render. Returns false when nothing had changed.
    pub fn commit_edit(&mut self) -> Result<bool, EngineError> {
        let Some(session) = self.develop.as_ref() else {
            return Ok(false);
        };
        if !session.is_editing() {
            return Ok(false);
        }
        let session = self.editable_session()?;
        let (photo, kind) = (session.photo, session.kind);
        let after = session.settings.clone();
        let label = describe_change(&session.committed, &after, kind);
        let edit = DevelopEdit {
            photo,
            settings: &after,
            history_label: Some(&label),
        };
        let changes = self.catalog.apply_develop_edits(&[edit])?;
        if let Some(session) = self.develop.as_mut() {
            session.committed = after;
        }
        self.record_silently(label, Command::Develop(changes));
        self.render_session(Purpose::Main, Quality::Full);
        Ok(true)
    }

    /// Drop what changed since the last commit and render the committed
    /// settings. Returns false when nothing had changed.
    pub fn cancel_edit(&mut self) -> bool {
        let Some(session) = self.develop.as_ref() else {
            return false;
        };
        if !session.is_editing() {
            return false;
        }
        let committed = session.committed.clone();
        self.update_session(committed);
        self.render_session(Purpose::Main, Quality::Full);
        true
    }

    /// Replace the open photo's settings and commit at once: for discrete
    /// edits (a crop committed, a mask added, a spot placed).
    pub fn apply_settings(&mut self, settings: DevelopSettings) -> Result<(), EngineError> {
        self.edit_settings(settings)?;
        self.commit_edit()?;
        Ok(())
    }

    /// Show `settings` for the open photo, uncommitted, and render a draft;
    /// the before view follows when the geometry moved.
    pub(crate) fn update_session(&mut self, settings: DevelopSettings) {
        let Some(session) = self.develop.as_mut() else {
            return;
        };
        if session.settings == settings {
            return;
        }
        let moved = tools::geometry_differs(&session.settings, &settings);
        let overlay_moves = session
            .overlay_mask
            .is_some_and(|mask| overlay::overlay_moves(&session.settings, &settings, mask));
        session.settings = settings;
        session.version += 1;
        if overlay_moves {
            self.render_session(Purpose::Overlay, Quality::Draft);
        }
        self.render_session(Purpose::Main, Quality::Draft);
        if moved {
            self.render_session(Purpose::Before, Quality::Draft);
        }
    }
}
