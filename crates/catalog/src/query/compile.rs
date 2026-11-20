//! Turning a source, filter and sort into SQL.
//!
//! Everything pushed as statement text is a string literal in this file (or
//! in `smart`); every value a caller supplied is bound as a parameter.

use photo_core::meta::FolderId;

use super::{Comparison, CopyFilter, Facet, Filter, Sort, SortDirection, SortKey, Source};
use crate::catalog::Catalog;
use crate::codec::{flag_to_sql, kind_to_sql, label_to_sql, rating_to_sql};
use crate::collections::{CollectionKind, collection_kind, stored_rules};
use crate::error::Result;
use crate::folders::folder_path;
use crate::paths;
use crate::smart;
use crate::sql::Sql;
use crate::text::fold;
use crate::tree::KEYWORDS;

/// The text the free-text filter searches, apart from keywords. The line
/// breaks keep a word from matching across two fields.
const SEARCHED_TEXT: &str =
    "p.file_name || char(10) || p.title || char(10) || p.caption || char(10)
    || COALESCE(p.camera, '') || char(10) || COALESCE(p.lens, '')";

impl Catalog {
    /// Append ` FROM ... WHERE ...` selecting the photos of `source` that
    /// pass `filter`, with `photos` aliased as `p`. Returns whether the
    /// collection's own order is available (as `cp.position`).
    pub(super) fn push_selection(
        &self,
        sql: &mut Sql,
        source: &Source,
        filter: &Filter,
    ) -> Result<bool> {
        let source = match *source {
            Source::QuickCollection => Source::Collection(self.quick_collection),
            other => other,
        };
        let collection = match source {
            Source::Collection(id) => Some((id, collection_kind(&self.conn, id)?)),
            _ => None,
        };
        sql.push(" FROM photos p");
        let mut custom_order = false;
        if let Some((id, CollectionKind::Regular | CollectionKind::Quick)) = collection {
            sql.push(" JOIN collection_photos cp ON cp.photo_id = p.id AND cp.collection_id = ")
                .bind(id.0);
            custom_order = true;
        }
        sql.push(" WHERE 1");
        match source {
            Source::AllPhotos | Source::QuickCollection | Source::Collection(_) => {}
            Source::Folder {
                id,
                include_subfolders,
            } => self.push_folder(sql, id, include_subfolders)?,
            Source::PreviousImport => {
                sql.push(" AND p.import_id = (SELECT MAX(import_id) FROM photos)");
            }
            Source::Keyword(id) => {
                KEYWORDS.require(&self.conn, id.0)?;
                sql.push(
                    " AND p.id IN (SELECT tagged.photo_id FROM photo_keywords tagged WHERE tagged.keyword_id IN (
                        WITH RECURSIVE below(id) AS (SELECT ",
                )
                .bind(id.0)
                .push(" UNION SELECT k.id FROM keywords k JOIN below ON k.parent_id = below.id) SELECT id FROM below))");
            }
        }
        if let Some((id, CollectionKind::Set)) = collection {
            sql.push(
                " AND p.id IN (SELECT member.photo_id FROM collection_photos member WHERE member.collection_id IN (
                    WITH RECURSIVE inside(id) AS (SELECT ",
            )
            .bind(id.0)
            .push(
                " UNION SELECT c.id FROM collections c JOIN inside ON c.parent_id = inside.id)
                  SELECT id FROM inside))",
            );
        }
        // SQLite tests the conditions of a row in the order they are written,
        // so the plain column comparisons go first and the ones that fold
        // text (a function call per row) only run for rows that are left.
        push_attribute_filter(sql, filter);
        if let Some((id, CollectionKind::Smart)) = collection {
            smart::push_rules(sql, &stored_rules(&self.conn, id)?)?;
        }
        push_text_filter(sql, &filter.text);
        Ok(custom_order)
    }

    fn push_folder(&self, sql: &mut Sql, id: FolderId, include_subfolders: bool) -> Result<()> {
        let path = folder_path(&self.conn, id)?;
        if !include_subfolders {
            sql.push(" AND p.folder_id = ").bind(id.0);
            return Ok(());
        }
        // A subfolder is a folder whose path starts with this folder's path
        // *and a separator*. Compared with substr rather than LIKE, so `%`
        // and `_` in a name are ordinary characters and case is respected.
        let prefix = paths::child_prefix(&path);
        let length = i64::try_from(prefix.chars().count()).unwrap_or(i64::MAX);
        sql.push(" AND p.folder_id IN (SELECT f.id FROM folders f WHERE f.id = ")
            .bind(id.0)
            .push(" OR substr(f.path, 1, ")
            .bind(length)
            .push(") = ")
            .bind(prefix)
            .push(")");
        Ok(())
    }
}

/// Append one condition per distinct word of `text`: the word occurs in the
/// photo's searched text or in the name of one of its keywords.
fn push_text_filter(sql: &mut Sql, text: &str) {
    let mut words: Vec<String> = Vec::new();
    for word in text.split_whitespace().map(fold) {
        if !words.contains(&word) && words.len() < Filter::MAX_WORDS {
            words.push(word);
        }
    }
    for word in words {
        sql.push(" AND (instr(photo_fold(")
            .push(SEARCHED_TEXT)
            .push("), ")
            .bind(word.clone());
        sql.push(
            ") > 0 OR p.id IN (SELECT tagged.photo_id FROM photo_keywords tagged WHERE tagged.keyword_id IN (
                SELECT k.id FROM keywords k WHERE instr(photo_fold(k.name), ",
        )
        .bind(word)
        .push(") > 0)))");
    }
}

/// Append `opening` (which ends in `IN (`), a subquery over `values` and the
/// closing bracket. The values travel as one JSON array in one bound
/// parameter, so a filter may name any number of them.
fn push_any_of<V: Into<serde_json::Value>>(
    sql: &mut Sql,
    opening: &'static str,
    values: impl Iterator<Item = V>,
) {
    let list = serde_json::Value::Array(values.map(Into::into).collect());
    sql.push(opening)
        .push("SELECT value FROM json_each(")
        .bind(list.to_string())
        .push("))");
}

/// Append the conditions of every filter field except the free text.
fn push_attribute_filter(sql: &mut Sql, filter: &Filter) {
    if !filter.flags.is_empty() {
        let flags = filter.flags.iter().map(|flag| flag_to_sql(*flag));
        push_any_of(sql, " AND p.flag IN (", flags);
    }
    if let Some(rating) = filter.rating {
        sql.push(match rating.comparison {
            Comparison::AtLeast => " AND p.rating >= ",
            Comparison::AtMost => " AND p.rating <= ",
            Comparison::Exactly => " AND p.rating = ",
        })
        .bind(rating_to_sql(rating.rating));
    }
    if !filter.labels.is_empty() {
        let labels = filter.labels.iter().map(|label| label_to_sql(*label));
        push_any_of(sql, " AND p.label IN (", labels);
    }
    match filter.copies {
        Some(CopyFilter::Masters) => {
            sql.push(" AND p.master_id IS NULL");
        }
        Some(CopyFilter::VirtualCopies) => {
            sql.push(" AND p.master_id IS NOT NULL");
        }
        None => {}
    }
    if let Some(kind) = filter.kind {
        sql.push(" AND p.kind = ").bind(kind_to_sql(kind));
    }
    if let Some(edited) = filter.edited {
        sql.push(" AND p.has_edits = ").bind(edited);
    }
    if !filter.cameras.is_empty() {
        let cameras = filter.cameras.iter().cloned();
        push_any_of(sql, " AND COALESCE(p.camera, '') IN (", cameras);
    }
    if !filter.lenses.is_empty() {
        let lenses = filter.lenses.iter().cloned();
        push_any_of(sql, " AND COALESCE(p.lens, '') IN (", lenses);
    }
    if let Some(range) = filter.capture {
        sql.push(" AND p.capture_time IS NOT NULL");
        if let Some(from) = range.from {
            sql.push(" AND p.capture_time >= ").bind(from.to_string());
        }
        if let Some(to) = range.to {
            sql.push(" AND p.capture_time <= ").bind(to.to_string());
        }
    }
    if let Some(range) = filter.iso {
        sql.push(" AND p.iso IS NOT NULL");
        if let Some(min) = range.min {
            sql.push(" AND p.iso >= ").bind(i64::from(min));
        }
        if let Some(max) = range.max {
            sql.push(" AND p.iso <= ").bind(i64::from(max));
        }
    }
}

/// Append the `ORDER BY` for `sort`. Every key in the chain runs in the same
/// direction, which is what makes descending the mirror image of ascending.
pub(super) fn push_order(sql: &mut Sql, sort: Sort, custom_order: bool) {
    let keys: &[&'static str] = match sort.key {
        SortKey::CaptureTime => &["p.capture_time", "p.file_name_key", "p.file_name", "p.id"],
        SortKey::FileName => &["p.file_name_key", "p.capture_time", "p.file_name", "p.id"],
        SortKey::Rating => &[
            "p.rating",
            "p.capture_time",
            "p.file_name_key",
            "p.file_name",
            "p.id",
        ],
        SortKey::Pick => &[
            "p.flag",
            "p.capture_time",
            "p.file_name_key",
            "p.file_name",
            "p.id",
        ],
        SortKey::Label => &[
            "p.label",
            "p.capture_time",
            "p.file_name_key",
            "p.file_name",
            "p.id",
        ],
        SortKey::EditTime => &[
            "p.edited_at",
            "p.capture_time",
            "p.file_name_key",
            "p.file_name",
            "p.id",
        ],
        SortKey::FileType => &[
            "p.format",
            "p.capture_time",
            "p.file_name_key",
            "p.file_name",
            "p.id",
        ],
        SortKey::Custom if custom_order => &["cp.position", "p.id"],
        SortKey::Added | SortKey::Custom => &["p.id"],
    };
    let direction = match sort.direction {
        SortDirection::Ascending => " ASC",
        SortDirection::Descending => " DESC",
    };
    sql.push(" ORDER BY ");
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            sql.push(", ");
        }
        sql.push(key).push(direction);
    }
}

/// For a facet: the expression listed as its value, and the expression rows
/// are grouped and ordered by.
pub(super) fn facet_columns(facet: Facet) -> (&'static str, &'static str) {
    match facet {
        Facet::Camera => ("COALESCE(p.camera, '')", "COALESCE(p.camera, '')"),
        Facet::Lens => ("COALESCE(p.lens, '')", "COALESCE(p.lens, '')"),
        Facet::Iso => ("COALESCE(CAST(p.iso AS TEXT), '')", "p.iso"),
        Facet::CaptureDate => (
            "COALESCE(substr(p.capture_time, 1, 10), '')",
            "substr(p.capture_time, 1, 10)",
        ),
    }
}
