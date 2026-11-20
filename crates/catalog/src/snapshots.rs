//! Snapshots: named copies of one photo's develop settings.

use photo_core::develop::DevelopSettings;
use photo_core::meta::{PhotoId, SnapshotId};
use rusqlite::params;

use crate::catalog::Catalog;
use crate::codec::unix_seconds;
use crate::error::{CatalogError, Entity, Result};
use crate::photos::require_photo;
use crate::stored::{SETTINGS_WRITTEN, decode_settings, encode_settings};
use crate::tree::clean_name;

/// A named copy of a photo's develop settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// The snapshot's id.
    pub id: SnapshotId,
    /// The photo it belongs to.
    pub photo: PhotoId,
    /// Its name; snapshots of one photo may share a name.
    pub name: String,
    /// When the snapshot was taken, in Unix seconds.
    pub created_at: i64,
    /// The settings it was given, exactly.
    pub settings: DevelopSettings,
}

impl Catalog {
    /// Save `settings` as a named snapshot of `photo`. The photo itself is
    /// not changed, and several snapshots may share a name.
    pub fn create_snapshot(
        &mut self,
        photo: PhotoId,
        name: &str,
        settings: &DevelopSettings,
    ) -> Result<SnapshotId> {
        let name = clean_name(name)?;
        let json = encode_settings(settings)?;
        self.write(|tx, now| {
            require_photo(tx, photo)?;
            tx.execute(
                "INSERT INTO snapshots (photo_id, name, settings, settings_format, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![photo.0, name, json, SETTINGS_WRITTEN, now],
            )?;
            Ok(SnapshotId(tx.last_insert_rowid()))
        })
    }

    /// Rename a snapshot.
    pub fn rename_snapshot(&mut self, id: SnapshotId, name: &str) -> Result<()> {
        let name = clean_name(name)?;
        self.write(|tx, _| {
            let changed = tx.execute(
                "UPDATE snapshots SET name = ?2 WHERE id = ?1",
                params![id.0, name],
            )?;
            found(changed, id)
        })
    }

    /// Delete a snapshot.
    pub fn delete_snapshot(&mut self, id: SnapshotId) -> Result<()> {
        self.write(|tx, _| {
            found(
                tx.execute("DELETE FROM snapshots WHERE id = ?1", [id.0])?,
                id,
            )
        })
    }

    /// The photo's snapshots, oldest first.
    ///
    /// A snapshot stored in a form this build cannot read (written by a
    /// newer version, or damaged) is not listed, so that it cannot take the
    /// others down with it. It is left in the file exactly as it is.
    pub fn snapshots(&self, photo: PhotoId) -> Result<Vec<Snapshot>> {
        require_photo(&self.conn, photo)?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, name, created_at, settings, settings_format FROM snapshots
             WHERE photo_id = ?1 ORDER BY id",
        )?;
        let mut rows = stmt.query([photo.0])?;
        let mut snapshots = Vec::new();
        while let Some(row) = rows.next()? {
            let id = SnapshotId(row.get(0)?);
            match decode_settings(&row.get::<_, String>(3)?, row.get(4)?) {
                Ok(settings) => snapshots.push(Snapshot {
                    id,
                    photo,
                    name: row.get(1)?,
                    created_at: unix_seconds(row.get(2)?),
                    settings,
                }),
                Err(error) => log::warn!("snapshot {id} is not listed: {error}"),
            }
        }
        Ok(snapshots)
    }
}

fn found(changed: usize, id: SnapshotId) -> Result<()> {
    if changed == 0 {
        Err(CatalogError::not_found(Entity::Snapshot, id.0))
    } else {
        Ok(())
    }
}
