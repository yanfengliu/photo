//! A string key-value store for application preferences that should travel
//! with the catalog.

use rusqlite::{OptionalExtension, params};

use crate::catalog::Catalog;
use crate::error::Result;

impl Catalog {
    /// The value stored under `key`, or `None` if it was never set.
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        let value = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(value)
    }

    /// Store `value` under `key`, replacing any earlier value.
    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<()> {
        self.write(|tx, _| {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }

    /// Forget `key`. Returns whether it was set.
    pub fn remove_setting(&mut self, key: &str) -> Result<bool> {
        self.write(|tx, _| Ok(tx.execute("DELETE FROM settings WHERE key = ?1", [key])? > 0))
    }
}
