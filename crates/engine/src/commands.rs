//! The one path every user-made catalog change takes: apply, record on the
//! undo stack with a label, forget what could be redone, confirm with a
//! toast, and refresh what the change made stale. Undo and redo go back
//! along the same path.

use std::collections::HashSet;

use photo_catalog::{Catalog, CatalogError};
use photo_core::meta::PhotoId;

use crate::browse::Refresh;
use crate::engine::Engine;
use crate::error::EngineError;
use crate::undo::{Command, History};

/// Which photos an action applies to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// The active photo only.
    Active,
    /// The selected photos; the active photo when none is selected.
    Selected,
    /// These photos, whether or not they are in the current list.
    Photos(Vec<PhotoId>),
}

/// Commands applied so far while building one undoable step, so that a
/// failure part way can take them back.
#[derive(Debug, Default)]
pub(crate) struct Applied(Vec<Command>);

impl Applied {
    /// Add a command that has already been applied.
    pub(crate) fn push(&mut self, command: Command) {
        self.0.push(command);
    }

    /// Apply a command that moves between two recorded states (its redo is
    /// its first application) and add it.
    pub(crate) fn apply(
        &mut self,
        catalog: &mut Catalog,
        mut command: Command,
    ) -> Result<(), CatalogError> {
        command.redo(catalog)?;
        self.0.push(command);
        Ok(())
    }

    /// Undo everything applied so far, newest first, after a failure.
    pub(crate) fn roll_back(self, catalog: &mut Catalog) {
        for mut command in self.0.into_iter().rev() {
            if let Err(error) = command.undo(catalog) {
                log::error!("could not take back part of a failed change: {error}");
            }
        }
    }

    /// The commands as one step, or `None` when nothing was applied.
    fn into_command(mut self) -> Option<Command> {
        match self.0.len() {
            0 => None,
            1 => self.0.pop(),
            _ => Some(Command::Batch(self.0)),
        }
    }
}

impl Engine {
    /// The photos `target` stands for, each once, in order.
    pub(crate) fn resolve(&self, target: &Target) -> Vec<PhotoId> {
        match target {
            Target::Active => self.selection.active().into_iter().collect(),
            Target::Selected if self.selection.selected().is_empty() => {
                self.selection.active().into_iter().collect()
            }
            Target::Selected => self.selection.selected().to_vec(),
            Target::Photos(photos) => {
                let mut seen = HashSet::new();
                photos
                    .iter()
                    .copied()
                    .filter(|photo| seen.insert(*photo))
                    .collect()
            }
        }
    }

    /// Record a command that has just been applied, and confirm it.
    pub(crate) fn record(&mut self, label: String, command: Command) {
        self.toasts.info(label.clone());
        self.record_silently(label, command);
    }

    /// Record a command that has just been applied, without a toast: for
    /// the commits of the develop sliders, which the canvas confirms.
    pub(crate) fn record_silently(&mut self, label: String, command: Command) {
        let mut changes = Vec::new();
        command.changes(&mut changes);
        self.history.record(label, command);
        self.refresh_after(&changes);
    }

    /// Record what `applied` holds as one step; with nothing applied, only
    /// confirm the action.
    pub(crate) fn record_applied(&mut self, label: String, applied: Applied) {
        match applied.into_command() {
            Some(command) => self.record(label, command),
            None => self.toasts.info(label),
        }
    }

    /// Apply a command that moves between two recorded states and record
    /// it; with no command (nothing would change), only confirm.
    pub(crate) fn apply_and_record(
        &mut self,
        label: String,
        command: Option<Command>,
    ) -> Result<(), EngineError> {
        let mut applied = Applied::default();
        if let Some(command) = command {
            applied.apply(&mut self.catalog, command)?;
        }
        self.record_applied(label, applied);
        Ok(())
    }

    /// Take back the most recent change. Returns false when there is
    /// nothing to undo. A change that can no longer be undone (what it
    /// touched was changed outside the undo stack, or deleted) is dropped
    /// with an error.
    ///
    /// An edit still open in Develop is kept: when the change to take back
    /// reaches the open photo, the edit is committed first. It is then the
    /// most recent change, so it is the one taken back, and a redo brings
    /// it back.
    pub fn undo(&mut self) -> Result<bool, EngineError> {
        self.commit_edit_reached_by(History::next_undo)?;
        let Some(mut entry) = self.history.pop_undo() else {
            return Ok(false);
        };
        match entry.command.undo(&mut self.catalog) {
            Ok(()) => {
                let mut changes = Vec::new();
                entry.command.changes(&mut changes);
                self.toasts.info(format!("Undo {}", entry.label));
                self.history.undone(entry);
                self.refresh_after(&changes);
                Ok(true)
            }
            Err(error) => Err(self.lost(&entry.label, "undo", error)),
        }
    }

    /// Make the most recently undone change again. Returns false when there
    /// is nothing to redo.
    ///
    /// An edit still open in Develop is kept: when the change to make again
    /// reaches the open photo, the edit is committed first. Like any change
    /// made since the undo, it ends what could be redone, so nothing is
    /// redone over it.
    pub fn redo(&mut self) -> Result<bool, EngineError> {
        self.commit_edit_reached_by(History::next_redo)?;
        let Some(mut entry) = self.history.pop_redo() else {
            return Ok(false);
        };
        match entry.command.redo(&mut self.catalog) {
            Ok(()) => {
                let mut changes = Vec::new();
                entry.command.changes(&mut changes);
                self.toasts.info(format!("Redo {}", entry.label));
                self.history.redone(entry);
                self.refresh_after(&changes);
                Ok(true)
            }
            Err(error) => Err(self.lost(&entry.label, "redo", error)),
        }
    }

    /// Commit the edit still open in Develop when the command `next` picks
    /// from the history would change the open photo's settings or take the
    /// photo out of the catalog. Undoing or redoing it puts the catalog's
    /// settings back in the session, or ends the session; an edit not yet
    /// committed would be gone for good. A command that does not reach the
    /// open photo leaves its edit as it is, in progress.
    fn commit_edit_reached_by(
        &mut self,
        next: fn(&History) -> Option<&Command>,
    ) -> Result<(), EngineError> {
        let editing = self
            .develop
            .as_ref()
            .filter(|session| session.is_editing())
            .map(|session| session.photo);
        let reached = editing
            .is_some_and(|photo| next(&self.history).is_some_and(|command| command.reaches(photo)));
        if reached {
            self.commit_edit()?;
        }
        Ok(())
    }

    /// The label of the change [`Engine::undo`] would take back.
    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }

    /// The label of the change [`Engine::redo`] would make again.
    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo_label()
    }

    /// Report a change that could not be undone or redone. Part of it may
    /// have been, so everything is read again.
    fn lost(&mut self, label: &str, action: &str, error: CatalogError) -> EngineError {
        self.toasts
            .error(format!("Could not {action} {label}: {error}"));
        if let Err(refresh) = self.refresh(&Refresh::all()) {
            log::error!("could not refresh after a failed {action}: {refresh}");
        }
        error.into()
    }
}
