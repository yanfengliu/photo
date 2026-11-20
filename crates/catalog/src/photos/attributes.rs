//! Changing what a photo is marked with, and where its file is.
//!
//! The setters take one value per photo so that a caller can undo a change
//! by handing back the values it read beforehand.

use std::path::Path;

use photo_core::meta::{ColorLabel, Flag, PhotoId, Rating};
use rusqlite::types::Value;
use rusqlite::{Connection, Transaction, params};

use super::{master_at_path, master_of};
use crate::catalog::Catalog;
use crate::codec::{flag_to_sql, label_to_sql, rating_to_sql};
use crate::error::{CatalogError, Entity, Result};
use crate::folders::ensure_folder;
use crate::paths;
use crate::text::natural_key;

/// The editable text fields of a photo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextField {
    /// A short name for the picture.
    Title,
    /// A description of the picture.
    Caption,
    /// Who made the picture.
    Creator,
    /// The copyright notice.
    Copyright,
}

impl TextField {
    fn update(self) -> &'static str {
        match self {
            Self::Title => "UPDATE photos SET title = ?2 WHERE id = ?1",
            Self::Caption => "UPDATE photos SET caption = ?2 WHERE id = ?1",
            Self::Creator => "UPDATE photos SET creator = ?2 WHERE id = ?1",
            Self::Copyright => "UPDATE photos SET copyright = ?2 WHERE id = ?1",
        }
    }
}

impl Catalog {
    /// Set the star rating of each listed photo. All or nothing: an id that
    /// is not in the catalog fails the whole call.
    pub fn set_ratings(&mut self, ratings: &[(PhotoId, Rating)]) -> Result<()> {
        let values = ratings
            .iter()
            .map(|(photo, rating)| (*photo, rating_to_sql(*rating).into()));
        self.update_each("UPDATE photos SET rating = ?2 WHERE id = ?1", values)
    }

    /// Set the pick / reject flag of each listed photo. All or nothing.
    pub fn set_flags(&mut self, flags: &[(PhotoId, Flag)]) -> Result<()> {
        let values = flags
            .iter()
            .map(|(photo, flag)| (*photo, flag_to_sql(*flag).into()));
        self.update_each("UPDATE photos SET flag = ?2 WHERE id = ?1", values)
    }

    /// Set the colour label of each listed photo. All or nothing.
    pub fn set_labels(&mut self, labels: &[(PhotoId, ColorLabel)]) -> Result<()> {
        let values = labels
            .iter()
            .map(|(photo, label)| (*photo, label_to_sql(*label).into()));
        self.update_each("UPDATE photos SET label = ?2 WHERE id = ?1", values)
    }

    /// Set one text field of each listed photo. All or nothing.
    pub fn set_text<S: AsRef<str>>(
        &mut self,
        field: TextField,
        texts: &[(PhotoId, S)],
    ) -> Result<()> {
        let values = texts
            .iter()
            .map(|(photo, text)| (*photo, text.as_ref().to_string().into()));
        self.update_each(field.update(), values)
    }

    /// Record whether the files of the listed photos could be found. The
    /// state belongs to the file, so it is shared by a master and all its
    /// virtual copies whichever of them is listed.
    pub fn set_missing(&mut self, photos: &[PhotoId], missing: bool) -> Result<()> {
        self.write(|tx, _| {
            for photo in photos {
                let master = master_of(tx, *photo)?;
                tx.prepare_cached("UPDATE photos SET missing = ?2 WHERE id = ?1")?
                    .execute(params![master, missing])?;
                sync_copies(tx, master)?;
            }
            Ok(())
        })
    }

    /// Point a photo at the place its file now lives and clear its missing
    /// state. The master and all its virtual copies move together, into the
    /// folder for the new directory (created if needed).
    ///
    /// Fails with [`CatalogError::DuplicatePath`] if another photo already
    /// owns the new path.
    pub fn relink_photo(&mut self, photo: PhotoId, new_path: &Path) -> Result<()> {
        let path = paths::text(new_path)?;
        let split = paths::split(&path)?;
        self.write(|tx, _| {
            let master = master_of(tx, photo)?;
            if master_at_path(tx, &path)?.is_some_and(|owner| owner != master) {
                return Err(CatalogError::DuplicatePath(path.to_string()));
            }
            let folder = ensure_folder(tx, split.dir)?;
            tx.execute(
                "UPDATE photos SET path = ?2, folder_id = ?3, file_name = ?4, file_name_key = ?5, missing = 0
                 WHERE id = ?1",
                params![master, path, folder, split.file_name, natural_key(split.file_name)],
            )?;
            sync_copies(tx, master)
        })
    }

    fn update_each(
        &mut self,
        statement: &'static str,
        values: impl Iterator<Item = (PhotoId, Value)>,
    ) -> Result<()> {
        self.write(|tx, _| {
            values
                .into_iter()
                .try_for_each(|(photo, value)| update_one(tx, statement, photo, value))
        })
    }
}

/// Give every virtual copy of `master` its master's file location again.
///
/// A virtual copy carries its own copy of the file columns. The ones listed
/// here are those that can change after import (by relinking, or when the
/// file is reported missing or found); this statement is the single place
/// that says so, and everything that changes them on a master ends with it.
pub(crate) fn sync_copies(conn: &Connection, master: i64) -> Result<()> {
    conn.prepare_cached(
        "UPDATE photos SET (path, folder_id, file_name, file_name_key, missing) =
            (SELECT path, folder_id, file_name, file_name_key, missing FROM photos source WHERE source.id = ?1)
         WHERE master_id = ?1",
    )?
    .execute([master])?;
    Ok(())
}

fn update_one(
    tx: &Transaction<'_>,
    statement: &'static str,
    photo: PhotoId,
    value: Value,
) -> Result<()> {
    let changed = tx
        .prepare_cached(statement)?
        .execute(params![photo.0, value])?;
    if changed == 0 {
        return Err(CatalogError::not_found(Entity::Photo, photo.0));
    }
    Ok(())
}
