//! Import runs: every photo is added as part of one, which is what
//! "Previous Import" shows.

use photo_core::meta::ImportId;
use rusqlite::{Connection, OptionalExtension, Row};

use crate::catalog::Catalog;
use crate::codec::{to_u64, unix_seconds};
use crate::error::{CatalogError, Entity, Result};

/// One import run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRun {
    /// The run's id. Later runs have larger ids.
    pub id: ImportId,
    /// When the run began, in Unix seconds.
    pub started_at: i64,
    /// Photos added by this run that are still in the catalog, virtual copies
    /// of them included.
    pub photo_count: u64,
}

fn run_from_row(row: &Row<'_>) -> rusqlite::Result<ImportRun> {
    Ok(ImportRun {
        id: ImportId(row.get(0)?),
        started_at: unix_seconds(row.get(1)?),
        photo_count: to_u64(row.get(2)?),
    })
}

impl Catalog {
    /// Start an import run. Photos inserted with the returned id belong to
    /// it.
    ///
    /// A run only becomes the "Previous Import" once it has added a photo,
    /// so a run that ends up adding nothing (everything was a duplicate, the
    /// batch failed, the user cancelled) does not hide the last real one.
    pub fn begin_import(&mut self) -> Result<ImportId> {
        self.write(|tx, now| {
            tx.execute("INSERT INTO imports (started_at) VALUES (?1)", [now])?;
            Ok(ImportId(tx.last_insert_rowid()))
        })
    }

    /// Every import run, newest first, including runs that added nothing.
    pub fn imports(&self) -> Result<Vec<ImportRun>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT i.id, i.started_at, (SELECT COUNT(*) FROM photos p WHERE p.import_id = i.id)
             FROM imports i ORDER BY i.id DESC",
        )?;
        let runs = stmt
            .query_map([], run_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(runs)
    }

    /// The run [`Source::PreviousImport`](crate::Source::PreviousImport)
    /// shows: the newest import run that has photos in the catalog, if there
    /// is one.
    pub fn latest_import(&self) -> Result<Option<ImportRun>> {
        let run = self
            .conn
            .prepare_cached(
                "SELECT i.id, i.started_at, (SELECT COUNT(*) FROM photos p WHERE p.import_id = i.id)
                 FROM imports i WHERE i.id = (SELECT MAX(import_id) FROM photos)",
            )?
            .query_row([], run_from_row)
            .optional()?;
        Ok(run)
    }
}

/// Fail with a typed error unless the import run exists.
pub(crate) fn require_import(conn: &Connection, import: ImportId) -> Result<()> {
    let exists = conn
        .prepare_cached("SELECT 1 FROM imports WHERE id = ?1")?
        .exists([import.0])?;
    if exists {
        Ok(())
    } else {
        Err(CatalogError::not_found(Entity::Import, import.0))
    }
}
