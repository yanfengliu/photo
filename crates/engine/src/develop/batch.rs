//! Develop changes to many photos at once (presets, paste, sync, reset,
//! Quick Develop): one catalog write and one undo entry however many photos
//! they touch.

use photo_catalog::{CatalogError, DevelopEdit};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{FileKind, PhotoId};

use super::{NEWER_SETTINGS, READ_ONLY_CATALOG};
use crate::engine::Engine;
use crate::error::EngineError;
use crate::undo::Command;

/// The photos a change can be made to, with their current settings.
pub(crate) struct Editable {
    pub(crate) photos: Vec<(PhotoId, FileKind, DevelopSettings)>,
    /// Photos left out because their settings were saved by a newer build.
    pub(crate) skipped: usize,
}

impl Engine {
    /// A photo's settings as they stand, to copy them or save them as a
    /// preset or snapshot: an edit of it still open in Develop is committed
    /// first. Settings saved by a newer version fail with the catalog's
    /// `NewerFormat`, since what they hold cannot be read here.
    pub(crate) fn settings_now(&mut self, photo: PhotoId) -> Result<DevelopSettings, EngineError> {
        if self
            .develop
            .as_ref()
            .is_some_and(|session| session.photo == photo)
        {
            self.commit_edit()?;
        }
        Ok(self.catalog.develop_settings(photo)?)
    }

    /// The current settings of `photos`, for a change to all of them. An
    /// edit of the open photo not yet committed is committed first. Photos
    /// whose settings come from a newer build are left out and counted;
    /// when that leaves none, or the catalog is write-protected, the change
    /// is refused.
    pub(crate) fn editable_settings(
        &mut self,
        photos: &[PhotoId],
    ) -> Result<Editable, EngineError> {
        if self.catalog.is_read_only() {
            return Err(EngineError::NotEditable(READ_ONLY_CATALOG.to_string()));
        }
        self.commit_edit()?;
        let records = if photos.is_empty() {
            Vec::new()
        } else {
            self.catalog.photos(photos)?
        };
        let mut editable = Editable {
            photos: Vec::with_capacity(records.len()),
            skipped: 0,
        };
        for record in records {
            match self.catalog.develop_settings(record.id) {
                Ok(settings) => editable.photos.push((record.id, record.kind, settings)),
                Err(CatalogError::NewerFormat { .. }) => editable.skipped += 1,
                Err(error) => return Err(error.into()),
            }
        }
        if editable.photos.is_empty() && editable.skipped > 0 {
            return Err(EngineError::NotEditable(NEWER_SETTINGS.to_string()));
        }
        Ok(editable)
    }

    /// Store `edit(photo, settings, kind)` for every editable photo in one
    /// catalog write and record it as one undo entry, labelled `label` there
    /// and in each photo's history. Photos whose settings would not change
    /// are left out; when none would, the action is only confirmed.
    pub(crate) fn write_develop(
        &mut self,
        label: &str,
        editable: Editable,
        edit: impl Fn(PhotoId, &DevelopSettings, FileKind) -> DevelopSettings,
    ) -> Result<(), EngineError> {
        let changed: Vec<(PhotoId, DevelopSettings)> = editable
            .photos
            .iter()
            .filter_map(|(photo, kind, current)| {
                let mut next = edit(*photo, current, *kind);
                next.sanitize_for(*kind);
                (next != *current).then_some((*photo, next))
            })
            .collect();
        if editable.skipped > 0 {
            self.toasts.error(format!(
                "{} photos were left alone: their settings were saved by a newer version",
                editable.skipped
            ));
        }
        if changed.is_empty() {
            self.toasts.info(label);
            return Ok(());
        }
        let edits: Vec<DevelopEdit<'_>> = changed
            .iter()
            .map(|(photo, settings)| DevelopEdit {
                photo: *photo,
                settings,
                history_label: Some(label),
            })
            .collect();
        let changes = self.catalog.apply_develop_edits(&edits)?;
        self.record(label.to_string(), Command::Develop(changes));
        Ok(())
    }
}
