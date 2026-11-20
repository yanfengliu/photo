//! Develop settings and history: the parametric edit of each photo.
//!
//! Every mutation here returns a [`DevelopChange`], a compact record of
//! exactly what it altered. Handing that record to [`Catalog::undo_develop`]
//! or [`Catalog::redo_develop`] moves the photo's settings and history
//! between the two states exactly, history step ids included, so a caller's
//! undo stack never has to reconstruct anything.

use photo_core::develop::DevelopSettings;
use photo_core::meta::{HistoryId, PhotoId};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::catalog::Catalog;
use crate::codec::{kind_from_sql, unix_seconds};
use crate::error::{CatalogError, Entity, Result};
use crate::photos::{kind_of, require_photo};

/// One write of develop settings, for [`Catalog::apply_develop_edits`].
#[derive(Clone, Copy, Debug)]
pub struct DevelopEdit<'a> {
    /// The photo to edit.
    pub photo: PhotoId,
    /// Its new settings, stored exactly as given.
    pub settings: &'a DevelopSettings,
    /// `Some(label)` records the write as a history step with that label;
    /// `None` changes the settings without touching the history.
    pub history_label: Option<&'a str>,
}

/// One step in a photo's develop history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryStep {
    /// The step's id. Later steps of a photo have larger ids, and an undone
    /// step comes back under the id it had.
    pub id: HistoryId,
    /// The label the step was recorded with.
    pub label: String,
    /// When the step was recorded, in Unix seconds.
    pub created_at: i64,
    /// True for the step the photo's settings were last taken from: the
    /// newest step after an edit, or the step last gone to.
    pub is_current: bool,
}

/// What one develop mutation changed (or several consecutive ones, folded
/// together with [`DevelopChange::merge`]), in a form that can be undone and
/// redone exactly. Opaque; it only makes sense for the catalog it came from.
#[derive(Clone, Debug)]
pub struct DevelopChange {
    photo: PhotoId,
    before: Point,
    after: Point,
    /// History steps the change discarded.
    removed: Vec<Step>,
    /// History steps the change recorded.
    added: Vec<Step>,
}

impl DevelopChange {
    /// The photo that was changed.
    pub fn photo(&self) -> PhotoId {
        self.photo
    }

    /// The newest history step this change recorded, if it recorded one.
    pub fn history_step(&self) -> Option<HistoryId> {
        self.added.last().map(|step| HistoryId(step.id))
    }

    /// Fold `next`, the change made to the same photo directly after this
    /// one, into this change, so that the two undo and redo as one. A run of
    /// writes (a slider being dragged, then let go) then costs an undo stack
    /// one change instead of one per write.
    ///
    /// Fails with [`CatalogError::StaleChange`], leaving this change as it
    /// was, if `next` does not start from the settings and history this
    /// change left behind. Only the two changes are compared, not the
    /// catalog, so fold changes as they are made, while both are applied:
    /// a merged change that no longer fits the catalog's state is refused
    /// by undo and redo with the same error, like any stale change.
    pub fn merge(&mut self, next: &DevelopChange) -> Result<()> {
        if self.photo != next.photo || !self.after.is_state(&next.before) {
            return Err(CatalogError::StaleChange(self.photo));
        }
        // A step this change recorded and the next one discarded was never
        // there, as far as the two together are concerned.
        let short_lived: Vec<i64> = next
            .removed
            .iter()
            .map(|step| step.id)
            .filter(|id| self.added.iter().any(|ours| ours.id == *id))
            .collect();
        self.added.retain(|step| !short_lived.contains(&step.id));
        let discarded = next
            .removed
            .iter()
            .filter(|step| !short_lived.contains(&step.id));
        self.removed.extend(discarded.cloned());
        self.added.extend(next.added.iter().cloned());
        self.after = next.after.clone();
        Ok(())
    }
}

/// A photo's develop state at one moment.
#[derive(Clone, Debug)]
struct Point {
    settings: Option<String>,
    has_edits: bool,
    edited_at: Option<i64>,
    current_step: Option<i64>,
    /// The photo's newest history step, whether or not it is the current one.
    newest_step: Option<i64>,
}

impl Point {
    /// True if `other` has the same stored settings at the same step of the
    /// same history. Step ids are never reused, so the newest step's id
    /// stands for the exact set of steps that led up to it. The edit time is
    /// not compared: writing the same settings again leaves the same state.
    fn is_state(&self, other: &Point) -> bool {
        self.settings == other.settings
            && self.current_step == other.current_step
            && self.newest_step == other.newest_step
    }
}

/// A stored history row.
#[derive(Clone, Debug)]
struct Step {
    id: i64,
    label: String,
    settings: String,
    created_at: i64,
}

/// Settings as the JSON the catalog stores. Refuses settings that would not
/// read back (a NaN or infinity is written as `null`), so that nothing
/// unreadable is ever stored.
pub(crate) fn encode_settings(settings: &DevelopSettings) -> Result<String> {
    let unstorable = |source| CatalogError::Unstorable {
        what: "develop settings",
        source,
    };
    let json = serde_json::to_string(settings).map_err(unstorable)?;
    serde_json::from_str::<DevelopSettings>(&json).map_err(unstorable)?;
    Ok(json)
}

/// Stored JSON back into settings. A failure is reported, never papered over
/// with defaults, because the next write would then destroy the stored edit.
pub(crate) fn decode_settings(json: &str) -> Result<DevelopSettings> {
    serde_json::from_str(json).map_err(|source| CatalogError::Corrupt {
        what: "develop settings",
        source,
    })
}

impl Catalog {
    /// The photo's develop settings: what was last stored for it, or the
    /// default for its kind if it has never been edited.
    pub fn develop_settings(&self, photo: PhotoId) -> Result<DevelopSettings> {
        let stored: Option<(Option<String>, i64)> = self
            .conn
            .prepare_cached("SELECT develop, kind FROM photos WHERE id = ?1")?
            .query_row([photo.0], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        match stored {
            None => Err(CatalogError::not_found(Entity::Photo, photo.0)),
            Some((Some(json), _)) => decode_settings(&json),
            Some((None, kind)) => Ok(DevelopSettings::default_for(kind_from_sql(kind))),
        }
    }

    /// Store new develop settings for one photo. See
    /// [`Catalog::apply_develop_edits`].
    pub fn set_develop_settings(
        &mut self,
        photo: PhotoId,
        settings: &DevelopSettings,
        history_label: Option<&str>,
    ) -> Result<DevelopChange> {
        let edit = DevelopEdit {
            photo,
            settings,
            history_label,
        };
        self.write(|tx, now| write_edit(tx, now, &edit))
    }

    /// Store new develop settings for any number of photos in one
    /// transaction, returning one change per edit in the same order.
    ///
    /// Each edit stores the settings exactly as given, bumps the photo's
    /// `develop_version`, stamps its edit time and recomputes `has_edits`. An
    /// edit with a history label also records a step and makes it current;
    /// if the current step was not the newest (the photo had been taken back
    /// to an earlier step), the steps after it are discarded first, as the
    /// new edit replaces them. Several edits may name the same photo and
    /// apply in order.
    pub fn apply_develop_edits(&mut self, edits: &[DevelopEdit<'_>]) -> Result<Vec<DevelopChange>> {
        self.write(|tx, now| edits.iter().map(|edit| write_edit(tx, now, edit)).collect())
    }

    /// The photo's history steps, oldest first.
    pub fn history(&self, photo: PhotoId) -> Result<Vec<HistoryStep>> {
        require_photo(&self.conn, photo)?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, label, created_at, is_current FROM history WHERE photo_id = ?1 ORDER BY id",
        )?;
        let steps = stmt
            .query_map([photo.0], |row| {
                Ok(HistoryStep {
                    id: HistoryId(row.get(0)?),
                    label: row.get(1)?,
                    created_at: unix_seconds(row.get(2)?),
                    is_current: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(steps)
    }

    /// The settings recorded by a history step.
    pub fn history_settings(&self, step: HistoryId) -> Result<DevelopSettings> {
        let json: Option<String> = self
            .conn
            .prepare_cached("SELECT settings FROM history WHERE id = ?1")?
            .query_row([step.0], |row| row.get(0))
            .optional()?;
        decode_settings(&json.ok_or_else(|| CatalogError::not_found(Entity::HistoryStep, step.0))?)
    }

    /// Take the photo back (or forward) to the settings of one of its own
    /// history steps and make that step current. No step is added or
    /// removed: the later steps stay available until the next labelled edit
    /// replaces them.
    pub fn go_to_history_step(&mut self, photo: PhotoId, step: HistoryId) -> Result<DevelopChange> {
        self.write(|tx, now| {
            let before = current_point(tx, photo)?;
            let json: Option<String> = tx
                .prepare_cached("SELECT settings FROM history WHERE id = ?1 AND photo_id = ?2")?
                .query_row([step.0, photo.0], |row| row.get(0))
                .optional()?;
            let json = json.ok_or_else(|| CatalogError::not_found(Entity::HistoryStep, step.0))?;
            let after = Point {
                has_edits: !decode_settings(&json)?.is_default_for(kind_of(tx, photo)?),
                settings: Some(json),
                edited_at: Some(now),
                current_step: Some(step.0),
                newest_step: before.newest_step,
            };
            apply_point(tx, photo, &after)?;
            Ok(DevelopChange {
                photo,
                before,
                after,
                removed: Vec::new(),
                added: Vec::new(),
            })
        })
    }

    /// Undo changes, newest first (so pass them in the order they were
    /// made): each photo's settings, `has_edits`, edit time and history
    /// return to what they were before the change. `develop_version` still
    /// moves forward, because a version number is never reused for
    /// different settings.
    ///
    /// Changes can only be undone in the reverse of the order they were
    /// made, as an undo stack does, and none may be left out (to keep fewer,
    /// fold them together with [`DevelopChange::merge`]). If a photo's
    /// settings or history are not what the change left behind, the call
    /// fails with [`CatalogError::StaleChange`] and undoes nothing, rather
    /// than mix two histories.
    pub fn undo_develop(&mut self, changes: &[DevelopChange]) -> Result<()> {
        self.write(|tx, _| {
            changes.iter().rev().try_for_each(|change| {
                require_point(tx, change.photo, &change.after)?;
                swap_steps(tx, change.photo, &change.added, &change.removed)?;
                apply_point(tx, change.photo, &change.before)
            })
        })
    }

    /// Reapply undone changes, oldest first: the exact state each change
    /// produced, with the same history step ids. Fails with
    /// [`CatalogError::StaleChange`], redoing nothing, if a photo's develop
    /// state is not the state the change started from.
    pub fn redo_develop(&mut self, changes: &[DevelopChange]) -> Result<()> {
        self.write(|tx, _| {
            changes.iter().try_for_each(|change| {
                require_point(tx, change.photo, &change.before)?;
                swap_steps(tx, change.photo, &change.removed, &change.added)?;
                apply_point(tx, change.photo, &change.after)
            })
        })
    }
}

/// Fail unless the photo's develop state is `expected`: the same stored
/// settings, at the same step of the same history.
fn require_point(tx: &Transaction<'_>, photo: PhotoId, expected: &Point) -> Result<()> {
    if current_point(tx, photo)?.is_state(expected) {
        Ok(())
    } else {
        Err(CatalogError::StaleChange(photo))
    }
}

fn write_edit(tx: &Transaction<'_>, now: i64, edit: &DevelopEdit<'_>) -> Result<DevelopChange> {
    let photo = edit.photo;
    let before = current_point(tx, photo)?;
    let json = encode_settings(edit.settings)?;
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let (mut current_step, mut newest_step) = (before.current_step, before.newest_step);
    if let Some(label) = edit.history_label {
        if let Some(current) = before.current_step {
            removed = steps_after(tx, photo, current)?;
            swap_steps(tx, photo, &removed, &[])?;
        }
        tx.prepare_cached(
            "INSERT INTO history (photo_id, label, settings, created_at) VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute(params![photo.0, label, json, now])?;
        let step = Step {
            id: tx.last_insert_rowid(),
            label: label.to_string(),
            settings: json.clone(),
            created_at: now,
        };
        (current_step, newest_step) = (Some(step.id), Some(step.id));
        added.push(step);
    }
    let after = Point {
        has_edits: !edit.settings.is_default_for(kind_of(tx, photo)?),
        settings: Some(json),
        edited_at: Some(now),
        current_step,
        newest_step,
    };
    apply_point(tx, photo, &after)?;
    Ok(DevelopChange {
        photo,
        before,
        after,
        removed,
        added,
    })
}

/// The photo's develop state right now. Fails if the photo does not exist.
fn current_point(tx: &Transaction<'_>, photo: PhotoId) -> Result<Point> {
    let stored = tx
        .prepare_cached("SELECT develop, has_edits, edited_at FROM photos WHERE id = ?1")?
        .query_row([photo.0], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .optional()?;
    let (settings, has_edits, edited_at) =
        stored.ok_or_else(|| CatalogError::not_found(Entity::Photo, photo.0))?;
    let current_step = tx
        .prepare_cached("SELECT id FROM history WHERE photo_id = ?1 AND is_current = 1")?
        .query_row([photo.0], |row| row.get(0))
        .optional()?;
    let newest_step = tx
        .prepare_cached("SELECT MAX(id) FROM history WHERE photo_id = ?1")?
        .query_row([photo.0], |row| row.get(0))?;
    Ok(Point {
        settings,
        has_edits,
        edited_at,
        current_step,
        newest_step,
    })
}

/// A develop version no photo has had before: the next value of the
/// catalog's own counter, which never goes back.
pub(crate) fn next_develop_version(conn: &Connection) -> Result<i64> {
    let version = conn
        .prepare_cached(
            "UPDATE counters SET value = value + 1 WHERE name = 'develop_version' RETURNING value",
        )?
        .query_row([], |row| row.get(0))?;
    Ok(version)
}

/// Make `point` the photo's develop state, under a new `develop_version`.
fn apply_point(tx: &Transaction<'_>, photo: PhotoId, point: &Point) -> Result<()> {
    let version = next_develop_version(tx)?;
    let changed = tx
        .prepare_cached(
            "UPDATE photos SET develop = ?2, has_edits = ?3, edited_at = ?4, develop_version = ?5
             WHERE id = ?1",
        )?
        .execute(params![
            photo.0,
            point.settings,
            point.has_edits,
            point.edited_at,
            version
        ])?;
    if changed == 0 {
        return Err(CatalogError::not_found(Entity::Photo, photo.0));
    }
    // Clear before setting: two current steps at once would break the
    // one-current-step-per-photo index, even for an instant.
    tx.prepare_cached("UPDATE history SET is_current = 0 WHERE photo_id = ?1 AND is_current = 1")?
        .execute([photo.0])?;
    if let Some(step) = point.current_step {
        tx.prepare_cached("UPDATE history SET is_current = 1 WHERE id = ?1 AND photo_id = ?2")?
            .execute([step, photo.0])?;
    }
    Ok(())
}

/// The photo's steps recorded after `step`, oldest first.
fn steps_after(tx: &Transaction<'_>, photo: PhotoId, step: i64) -> Result<Vec<Step>> {
    let mut stmt = tx.prepare_cached(
        "SELECT id, label, settings, created_at FROM history WHERE photo_id = ?1 AND id > ?2 ORDER BY id",
    )?;
    let steps = stmt
        .query_map([photo.0, step], |row| {
            Ok(Step {
                id: row.get(0)?,
                label: row.get(1)?,
                settings: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(steps)
}

/// Delete the steps in `out` and put back the steps in `back`, under their
/// original ids. Neither is marked current; `apply_point` does that.
fn swap_steps(tx: &Transaction<'_>, photo: PhotoId, out: &[Step], back: &[Step]) -> Result<()> {
    for step in out {
        tx.prepare_cached("DELETE FROM history WHERE id = ?1 AND photo_id = ?2")?
            .execute([step.id, photo.0])?;
    }
    for step in back {
        tx.prepare_cached(
            "INSERT INTO history (id, photo_id, label, settings, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(params![step.id, photo.0, step.label, step.settings, step.created_at])?;
    }
    Ok(())
}
