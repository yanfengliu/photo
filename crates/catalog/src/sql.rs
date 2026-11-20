//! Statement assembly that cannot be injected into.

use rusqlite::types::Value;
use rusqlite::{ParamsFromIter, params_from_iter};

/// A SQL statement assembled from compile-time fragments and bound values.
///
/// Statement text can only be added as `&'static str`, so nothing typed by a
/// user can become part of the SQL; user values go through [`Sql::bind`] and
/// reach SQLite as parameters.
#[derive(Debug, Default)]
pub(crate) struct Sql {
    text: String,
    values: Vec<Value>,
}

impl Sql {
    /// Start a statement with `fragment`.
    pub(crate) fn new(fragment: &'static str) -> Self {
        Self {
            text: fragment.to_string(),
            values: Vec::new(),
        }
    }

    /// Append statement text.
    pub(crate) fn push(&mut self, fragment: &'static str) -> &mut Self {
        self.text.push_str(fragment);
        self
    }

    /// Append one placeholder and remember its value.
    pub(crate) fn bind(&mut self, value: impl Into<Value>) -> &mut Self {
        self.text.push('?');
        self.values.push(value.into());
        self
    }

    /// The statement text, with one `?` per bound value.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// The bound values, in placeholder order.
    pub(crate) fn params(&self) -> ParamsFromIter<std::slice::Iter<'_, Value>> {
        params_from_iter(self.values.iter())
    }
}

/// A JSON array of ids, for `json_each(?)`: one bound parameter carries any
/// number of ids, so no statement ever needs a placeholder per id.
pub(crate) fn id_list(ids: impl IntoIterator<Item = i64>) -> String {
    let mut json = String::from("[");
    for (index, id) in ids.into_iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str(&id.to_string());
    }
    json.push(']');
    json
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_and_values_stay_in_step() {
        let mut sql = Sql::new("SELECT 1 WHERE a = ");
        sql.bind(7_i64).push(" AND b = ").bind("x".to_string());
        assert_eq!(sql.text(), "SELECT 1 WHERE a = ? AND b = ?");
        assert_eq!(
            sql.values,
            [Value::Integer(7), Value::Text("x".to_string())]
        );
    }

    #[test]
    fn id_lists_are_json_arrays() {
        assert_eq!(id_list([]), "[]");
        assert_eq!(id_list([3, -1, 20]), "[3,-1,20]");
    }
}
