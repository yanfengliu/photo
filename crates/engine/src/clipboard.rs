//! Moving settings between photos: copy and paste, sync, the previous
//! photo's settings, and reset. Each is one undo entry and one catalog
//! write however many photos it touches.

use photo_core::develop::{DevelopSettings, GroupSet, SettingsGroup};
use photo_core::meta::PhotoId;

use crate::commands::Target;
use crate::engine::Engine;
use crate::error::EngineError;

/// Settings copied from one photo, and which groups of them to paste.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsClipboard {
    from: PhotoId,
    groups: GroupSet,
    settings: DevelopSettings,
}

impl SettingsClipboard {
    /// The photo the settings were copied from.
    pub fn from_photo(&self) -> PhotoId {
        self.from
    }

    /// The groups a paste applies.
    pub fn groups(&self) -> GroupSet {
        self.groups
    }

    /// The settings as they were copied.
    pub fn settings(&self) -> &DevelopSettings {
        &self.settings
    }
}

impl Engine {
    /// Copy the active photo's settings, to paste the chosen `groups` of
    /// them later.
    pub fn copy_settings(&mut self, groups: GroupSet) -> Result<(), EngineError> {
        let from = self.selection.active().ok_or(EngineError::NoActivePhoto)?;
        let settings = self.settings_now(from)?;
        self.clipboard = Some(SettingsClipboard {
            from,
            groups,
            settings,
        });
        self.toasts.info("Copy Settings");
        Ok(())
    }

    /// What was copied last.
    pub fn clipboard(&self) -> Option<&SettingsClipboard> {
        self.clipboard.as_ref()
    }

    /// Paste the copied groups onto every target photo.
    pub fn paste_settings(&mut self, target: Target) -> Result<(), EngineError> {
        let clipboard = self.clipboard.clone().ok_or(EngineError::NothingToPaste)?;
        let photos = self.resolve(&target);
        self.spread(
            "Paste Settings",
            &clipboard.settings,
            clipboard.groups,
            &photos,
        )
    }

    /// Give every other selected photo the chosen `groups` of the active
    /// photo's settings.
    pub fn sync_settings(&mut self, groups: GroupSet) -> Result<(), EngineError> {
        let from = self.selection.active().ok_or(EngineError::NoActivePhoto)?;
        let settings = self.settings_now(from)?;
        let others: Vec<PhotoId> = self
            .selection
            .selected()
            .iter()
            .copied()
            .filter(|photo| *photo != from)
            .collect();
        self.spread("Sync Settings", &settings, groups, &others)
    }

    /// The photo that was active (or open in Develop) before the current
    /// one, whose settings [`Engine::paste_from_previous`] takes.
    pub fn previous_photo(&self) -> Option<PhotoId> {
        self.previous_active
    }

    /// Give every target photo all the settings of the previous photo.
    pub fn paste_from_previous(&mut self, target: Target) -> Result<(), EngineError> {
        let from = self.previous_active.ok_or(EngineError::NoPreviousPhoto)?;
        let settings = self.settings_now(from)?;
        let photos: Vec<PhotoId> = self
            .resolve(&target)
            .into_iter()
            .filter(|photo| *photo != from)
            .collect();
        self.spread("Paste from Previous", &settings, GroupSet::all(), &photos)
    }

    /// Put every target photo back to the settings a newly imported photo
    /// of its kind starts with.
    pub fn reset_settings(&mut self, target: Target) -> Result<(), EngineError> {
        let photos = self.resolve(&target);
        let editable = self.editable_settings(&photos)?;
        self.write_develop("Reset Settings", editable, |_, _, kind| {
            DevelopSettings::default_for(kind)
        })
    }

    /// Put one group of every target photo's settings back to its starting
    /// values.
    pub fn reset_group(&mut self, target: Target, group: SettingsGroup) -> Result<(), EngineError> {
        let photos = self.resolve(&target);
        let editable = self.editable_settings(&photos)?;
        let label = format!("Reset {}", group.label());
        self.write_develop(&label, editable, |_, current, kind| {
            let mut reset = current.clone();
            reset.copy_groups_from(
                &DevelopSettings::default_for(kind),
                GroupSet::NONE.with(group),
            );
            reset
        })
    }

    fn spread(
        &mut self,
        label: &str,
        from: &DevelopSettings,
        groups: GroupSet,
        photos: &[PhotoId],
    ) -> Result<(), EngineError> {
        let editable = self.editable_settings(photos)?;
        self.write_develop(label, editable, |_, current, _| {
            let mut pasted = current.clone();
            pasted.copy_groups_from(from, groups);
            pasted
        })
    }
}
