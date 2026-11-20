//! Virtual copies: a second set of edits and attributes for the same file.

use photo_core::meta::PhotoId;
use rusqlite::types::Value;

use super::master_of;
use crate::catalog::Catalog;
use crate::error::Result;
use crate::rows::RowSet;

impl Catalog {
    /// Create a virtual copy of `source` and return its id.
    ///
    /// The copy shares the file of `source`'s master (a copy of a copy
    /// belongs to the same master) and starts as a duplicate of `source`:
    /// its current develop settings, rating, flag, label, text fields and
    /// keywords. From then on the two are independent. It gets an empty
    /// develop history, no snapshots, no collection memberships, and the
    /// name "Copy N" with N one more than the highest number among the
    /// master's present copies (names are labels, not identities: they are
    /// not required to be unique).
    pub fn create_virtual_copy(&mut self, source: PhotoId) -> Result<PhotoId> {
        self.write(|tx, now| {
            let master = master_of(tx, source)?;
            let mut row = RowSet::capture(tx, "photos", "id = ?1", [source.0])?;
            let (id, master_id, copy_name, added_at) = (
                row.column("id")?,
                row.column("master_id")?,
                row.column("copy_name")?,
                row.column("added_at")?,
            );

            let mut names =
                tx.prepare_cached("SELECT copy_name FROM photos WHERE master_id = ?1")?;
            let taken: Vec<String> = names
                .query_map([master], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;

            for values in row.rows_mut() {
                values[id] = Value::Null;
                values[master_id] = Value::Integer(master);
                values[copy_name] = Value::Text(next_copy_name(&taken));
                values[added_at] = Value::Integer(now);
            }
            row.insert(tx)?;
            let copy = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO photo_keywords (photo_id, keyword_id)
                 SELECT ?1, keyword_id FROM photo_keywords WHERE photo_id = ?2",
                [copy, source.0],
            )?;
            Ok(PhotoId(copy))
        })
    }
}

/// "Copy N" for the smallest N above every number in `taken`, the names of
/// the copies that exist now. Counting past the highest (rather than filling
/// gaps) keeps a new copy from taking the name of one removed from the
/// middle; a number is free again once no existing copy is at or above it.
fn next_copy_name(taken: &[String]) -> String {
    let highest = taken
        .iter()
        .filter_map(|name| name.strip_prefix("Copy ")?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("Copy {}", highest.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_names_count_up_past_the_highest_in_use() {
        let names = |taken: &[&str]| {
            next_copy_name(
                &taken
                    .iter()
                    .map(|name| name.to_string())
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(names(&[]), "Copy 1");
        assert_eq!(names(&["Copy 1", "Copy 2"]), "Copy 3");
        assert_eq!(names(&["Copy 3"]), "Copy 4");
        assert_eq!(names(&["B&W version", "Copy x"]), "Copy 1");
    }
}
