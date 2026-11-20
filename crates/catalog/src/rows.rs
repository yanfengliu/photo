//! Whole table rows, copied without naming their columns.
//!
//! Virtual copies and remove-then-restore both need to reproduce rows
//! exactly. Reading `SELECT *` into values and writing those values back
//! means a column added by a later migration is carried along automatically,
//! with no column list here to fall out of date.

use rusqlite::types::Value;
use rusqlite::{Connection, Params, params_from_iter};

use crate::error::Result;

/// Rows of one table, with the column names they were read under.
#[derive(Clone, Debug)]
pub(crate) struct RowSet {
    table: &'static str,
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}

impl RowSet {
    /// Read every row of `table` that satisfies `filter`. Both are
    /// compile-time text; values reach the statement through `params`.
    pub(crate) fn capture(
        conn: &Connection,
        table: &'static str,
        filter: &'static str,
        params: impl Params,
    ) -> Result<Self> {
        let mut stmt = conn.prepare_cached(&format!("SELECT * FROM {table} WHERE {filter}"))?;
        let columns: Vec<String> = stmt.column_names().map(str::to_string).collect();
        let mut rows = Vec::new();
        let mut cursor = stmt.query(params)?;
        while let Some(row) = cursor.next()? {
            let values: Vec<Value> = (0..columns.len())
                .map(|index| row.get(index))
                .collect::<rusqlite::Result<_>>()?;
            rows.push(values);
        }
        Ok(Self {
            table,
            columns,
            rows,
        })
    }

    /// Write every row back into the table it came from.
    pub(crate) fn insert(&self, conn: &Connection) -> Result<()> {
        if self.rows.is_empty() {
            return Ok(());
        }
        // The names were read from the schema of the opened file. Quoting
        // them as identifiers (doubling any quote inside) means a name is
        // only ever a name, whatever a tampered file calls its columns.
        let names: Vec<String> = self
            .columns
            .iter()
            .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
            .collect();
        let marks = vec!["?"; self.columns.len()];
        let statement = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.table,
            names.join(", "),
            marks.join(", ")
        );
        let mut stmt = conn.prepare_cached(&statement)?;
        for row in &self.rows {
            stmt.execute(params_from_iter(row.iter()))?;
        }
        Ok(())
    }

    /// Position of `column` in each row.
    pub(crate) fn column(&self, column: &str) -> Result<usize> {
        self.columns
            .iter()
            .position(|name| name == column)
            .ok_or_else(|| rusqlite::Error::InvalidColumnName(column.to_string()).into())
    }

    /// The captured rows, each with one value per column.
    pub(crate) fn rows(&self) -> &[Vec<Value>] {
        &self.rows
    }

    /// The captured rows, for adjusting values before they are written back.
    pub(crate) fn rows_mut(&mut self) -> &mut Vec<Vec<Value>> {
        &mut self.rows
    }
}

/// The integer in a captured value; `None` for `NULL` and other types.
pub(crate) fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Integer(number) => Some(*number),
        _ => None,
    }
}

/// The text in a captured value; `None` for `NULL` and other types.
pub(crate) fn text(value: &Value) -> Option<&str> {
    match value {
        Value::Text(text) => Some(text),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_reproduced_exactly_including_nulls_and_columns_added_later() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE things (id INTEGER PRIMARY KEY, name TEXT, weight REAL, data BLOB);
             ALTER TABLE things ADD COLUMN added_later TEXT DEFAULT 'later';
             INSERT INTO things (id, name, weight, data) VALUES (7, 'seven', 0.1, x'00ff'), (9, NULL, NULL, NULL);
             UPDATE things SET added_later = 'changed' WHERE id = 9;",
        )
        .unwrap();
        let dump = |conn: &Connection| -> Vec<Vec<Value>> {
            RowSet::capture(conn, "things", "1 ORDER BY id", [])
                .unwrap()
                .rows
        };
        let before = dump(&conn);

        let captured = RowSet::capture(&conn, "things", "id >= ?1", [7]).unwrap();
        conn.execute("DELETE FROM things", []).unwrap();
        captured.insert(&conn).unwrap();

        assert_eq!(dump(&conn), before);
        assert_eq!(captured.column("added_later").unwrap(), 4);
        assert!(captured.column("missing").is_err());
        assert_eq!(integer(&before[0][0]), Some(7));
        assert_eq!(text(&before[0][1]), Some("seven"));
        assert_eq!(text(&before[1][1]), None);
    }

    #[test]
    fn a_column_name_is_only_ever_a_name() {
        // A catalog file could be edited to give a column a name that reads
        // like SQL. It must still be treated as one identifier.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"CREATE TABLE things (id INTEGER PRIMARY KEY, "a"") VALUES (1); DROP TABLE things; --" TEXT);
               CREATE TABLE bystander (id INTEGER PRIMARY KEY);
               INSERT INTO things VALUES (1, 'kept');"#,
        )
        .unwrap();
        let captured = RowSet::capture(&conn, "things", "1", []).unwrap();
        assert_eq!(captured.rows().len(), 1);
        conn.execute("DELETE FROM things", []).unwrap();

        captured.insert(&conn).unwrap();

        let restored = RowSet::capture(&conn, "things", "1", []).unwrap();
        assert_eq!(restored.rows(), captured.rows());
        assert!(conn.prepare("SELECT 1 FROM bystander").is_ok());
    }
}
