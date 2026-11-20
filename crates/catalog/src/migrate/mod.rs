//! Schema migrations, keyed on `PRAGMA user_version`.
//!
//! Guarantees: a database is only ever touched if it is blank or carries this
//! application's id; a schema newer than this build knows is refused before
//! anything is written; and all pending steps run in one transaction, so an
//! upgrade either completes or leaves the file exactly as it was.

use std::collections::HashMap;

use rusqlite::{Connection, TransactionBehavior};

use crate::error::{CatalogError, Result};
use crate::schema;

/// `PRAGMA application_id` of a Photo catalog ("PHOT").
pub(crate) const APPLICATION_ID: i32 = 0x5048_4F54;

/// One step: the SQL that takes the schema from `version - 1` to `version`.
pub(crate) struct Migration {
    pub(crate) version: u32,
    pub(crate) sql: &'static str,
}

/// Every step, in order. Append only; never edit a released step.
///
/// A step that rebuilds a table (create the new one, copy, drop the old,
/// rename) must also carry the table's `sqlite_sequence` row over, as the
/// rebuild in this module's tests does: otherwise the new table counts on
/// from its largest surviving row and hands out the ids of removed rows.
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: schema::V1,
}];

/// The newest schema version `migrations` produces.
pub(crate) const fn latest(migrations: &[Migration]) -> u32 {
    match migrations.last() {
        Some(step) => step.version,
        None => 0,
    }
}

/// What a database turned out to be.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Probe {
    /// No schema at all: safe to initialise.
    Blank,
    /// A catalog at this schema version (never newer than `supported`).
    Catalog(u32),
}

/// Identify the database behind `conn` without writing to it.
pub(crate) fn probe(conn: &Connection, supported: u32) -> Result<Probe> {
    let application_id: i64 = conn.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let user_version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if application_id == i64::from(APPLICATION_ID) {
        let found = u32::try_from(user_version).map_err(|_| CatalogError::NotACatalog)?;
        if found > supported {
            return Err(CatalogError::NewerSchema { found, supported });
        }
        return Ok(Probe::Catalog(found));
    }
    let objects: i64 =
        conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))?;
    if application_id == 0 && user_version == 0 && objects == 0 {
        Ok(Probe::Blank)
    } else {
        Err(CatalogError::NotACatalog)
    }
}

/// Bring the database up to the newest version in `migrations` and return
/// that version. A database that is already there is not written to at all.
///
/// Leaves foreign-key enforcement switched on, whatever the outcome.
pub(crate) fn migrate(conn: &mut Connection, migrations: &[Migration]) -> Result<u32> {
    let supported = latest(migrations);
    if probe(conn, supported)? == Probe::Catalog(supported) {
        return Ok(supported);
    }
    // A step that rebuilds a table drops the old one, and with foreign keys
    // enforced DROP TABLE first deletes the table's rows, cascading into
    // every table that refers to it: the upgrade would silently delete
    // history, keywords and memberships. So the steps run with enforcement
    // off (it can only be switched outside a transaction), and the result is
    // checked as a whole before it is committed.
    conn.pragma_update(None, "foreign_keys", false)?;
    let upgraded = upgrade(conn, migrations, supported);
    let enforced = conn.pragma_update(None, "foreign_keys", true);
    upgraded?;
    enforced?;
    Ok(supported)
}

fn upgrade(conn: &mut Connection, migrations: &[Migration], supported: u32) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = match probe(&tx, supported)? {
        Probe::Blank => {
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            0
        }
        Probe::Catalog(version) => version,
    };
    // Rows that already refer to nothing (left by another tool that wrote
    // with enforcement off) are not this upgrade's doing. Failing on them
    // would lock the catalog out of every later version, so only references
    // that the steps themselves break count against the upgrade. They are
    // counted, not just listed, because rows of a table without rowids all
    // look alike here.
    let mut already: HashMap<Dangling, usize> = HashMap::new();
    for row in dangling(&tx)? {
        *already.entry(row).or_default() += 1;
    }
    if !already.is_empty() {
        log::warn!("some catalog rows refer to rows that do not exist; they are left as they are");
    }
    for step in migrations.iter().filter(|step| step.version > current) {
        log::info!("migrating catalog schema to version {}", step.version);
        tx.execute_batch(step.sql)?;
        tx.pragma_update(None, "user_version", step.version)?;
    }
    for row in dangling(&tx)? {
        match already.get_mut(&row) {
            Some(left) if *left > 0 => *left -= 1,
            _ => {
                let (table, _, parent, _) = row;
                return Err(CatalogError::BrokenReferences { table, parent });
            }
        }
    }
    tx.commit()?;
    Ok(())
}

/// A row that refers to a row which does not exist: its table, its rowid
/// (none in a table without one), the table it points into, and which of
/// its table's foreign keys it breaks.
type Dangling = (String, Option<i64>, String, i64);

/// Every dangling reference in the database, whether or not foreign keys are
/// being enforced.
fn dangling(conn: &Connection) -> Result<Vec<Dangling>> {
    let mut found = Vec::new();
    conn.pragma_query(None, "foreign_key_check", |row| {
        found.push((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?));
        Ok(())
    })?;
    Ok(found)
}

#[cfg(test)]
mod tests;
