//! Keywords: a hierarchy of tags and their assignment to photos.

use photo_core::meta::{KeywordId, PhotoId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::catalog::Catalog;
use crate::codec::to_u64;
use crate::error::Result;
use crate::photos::{require_photo, require_photos};
use crate::text::natural_key;
use crate::tree::{KEYWORDS, clean_name};

/// A keyword and how many photos carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeywordInfo {
    /// The keyword's id.
    pub id: KeywordId,
    /// The keyword this one sits under, or `None` at the top level.
    pub parent: Option<KeywordId>,
    /// Its name, unique among its siblings ignoring ASCII case.
    pub name: String,
    /// Photos this keyword is assigned to directly. Photos that only carry
    /// one of its descendants are not counted here, although
    /// [`Source::Keyword`](crate::Source::Keyword) does show them.
    pub photo_count: u64,
}

const SELECT: &str = "SELECT k.id, k.parent_id, k.name,
            (SELECT COUNT(*) FROM photo_keywords pk WHERE pk.keyword_id = k.id)
     FROM keywords k";

fn keyword_from_row(row: &Row<'_>) -> rusqlite::Result<KeywordInfo> {
    Ok(KeywordInfo {
        id: KeywordId(row.get(0)?),
        parent: row.get::<_, Option<i64>>(1)?.map(KeywordId),
        name: row.get(2)?,
        photo_count: to_u64(row.get(3)?),
    })
}

fn in_name_order(mut keywords: Vec<KeywordInfo>) -> Vec<KeywordInfo> {
    keywords.sort_by_cached_key(|keyword| (natural_key(&keyword.name), keyword.id));
    keywords
}

impl Catalog {
    /// Every keyword, as a flat list in natural name order; the hierarchy is
    /// in each entry's `parent`.
    pub fn keywords(&self) -> Result<Vec<KeywordInfo>> {
        let mut stmt = self.conn.prepare_cached(SELECT)?;
        let keywords = stmt
            .query_map([], keyword_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(in_name_order(keywords))
    }

    /// The keywords assigned to a photo, in natural name order.
    pub fn photo_keywords(&self, photo: PhotoId) -> Result<Vec<KeywordInfo>> {
        require_photo(&self.conn, photo)?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT k.id, k.parent_id, k.name,
                    (SELECT COUNT(*) FROM photo_keywords pk WHERE pk.keyword_id = k.id)
             FROM keywords k JOIN photo_keywords mine ON mine.keyword_id = k.id
             WHERE mine.photo_id = ?1",
        )?;
        let keywords = stmt
            .query_map([photo.0], keyword_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(in_name_order(keywords))
    }

    /// Create a keyword under `parent` (or at the top level). The name is
    /// trimmed and must be unique among its siblings, ignoring ASCII case.
    pub fn create_keyword(&mut self, name: &str, parent: Option<KeywordId>) -> Result<KeywordId> {
        let name = clean_name(name)?;
        let parent = parent.map(|parent| parent.0);
        self.write(|tx, _| {
            if let Some(parent) = parent {
                KEYWORDS.require(tx, parent)?;
            }
            KEYWORDS.require_free_name(tx, parent, name, None)?;
            tx.execute(
                "INSERT INTO keywords (parent_id, name) VALUES (?1, ?2)",
                params![parent, name],
            )?;
            Ok(KeywordId(tx.last_insert_rowid()))
        })
    }

    /// The keyword called `name`, wherever it sits in the hierarchy (the
    /// oldest one if several share the name, ignoring ASCII case); if there
    /// is none, a new top-level keyword is created.
    pub fn ensure_keyword(&mut self, name: &str) -> Result<KeywordId> {
        self.write(|tx, _| ensure_keyword(tx, name).map(KeywordId))
    }

    /// Rename a keyword, keeping its assignments.
    pub fn rename_keyword(&mut self, id: KeywordId, name: &str) -> Result<()> {
        let name = clean_name(name)?;
        self.write(|tx, _| {
            let parent = KEYWORDS.parent(tx, id.0)?;
            KEYWORDS.require_free_name(tx, parent, name, Some(id.0))?;
            tx.execute(
                "UPDATE keywords SET name = ?2 WHERE id = ?1",
                params![id.0, name],
            )?;
            Ok(())
        })
    }

    /// Move a keyword, with everything beneath it, under `parent` (or to the
    /// top level).
    pub fn move_keyword(&mut self, id: KeywordId, parent: Option<KeywordId>) -> Result<()> {
        let parent = parent.map(|parent| parent.0);
        self.write(|tx, _| {
            KEYWORDS.require(tx, id.0)?;
            if let Some(parent) = parent {
                KEYWORDS.require(tx, parent)?;
            }
            KEYWORDS.require_no_cycle(tx, id.0, parent)?;
            let name: String =
                tx.query_row("SELECT name FROM keywords WHERE id = ?1", [id.0], |row| {
                    row.get(0)
                })?;
            KEYWORDS.require_free_name(tx, parent, &name, Some(id.0))?;
            tx.execute(
                "UPDATE keywords SET parent_id = ?2 WHERE id = ?1",
                params![id.0, parent],
            )?;
            Ok(())
        })
    }

    /// Delete a keyword, every keyword beneath it, and all their
    /// assignments. Photos are not affected otherwise.
    pub fn delete_keyword(&mut self, id: KeywordId) -> Result<()> {
        self.write(|tx, _| {
            KEYWORDS.require(tx, id.0)?;
            tx.execute("DELETE FROM keywords WHERE id = ?1", [id.0])?;
            Ok(())
        })
    }

    /// Assign a keyword to photos. Returns the photos that did not have it
    /// before, which is exactly what an undo has to unassign. All or
    /// nothing: an unknown photo fails the whole call.
    pub fn assign_keyword(
        &mut self,
        keyword: KeywordId,
        photos: &[PhotoId],
    ) -> Result<Vec<PhotoId>> {
        self.write(|tx, _| {
            KEYWORDS.require(tx, keyword.0)?;
            require_photos(tx, photos)?;
            let mut insert = tx.prepare_cached(
                "INSERT OR IGNORE INTO photo_keywords (photo_id, keyword_id) VALUES (?1, ?2)",
            )?;
            let mut added = Vec::new();
            for photo in photos {
                if insert.execute([photo.0, keyword.0])? > 0 {
                    added.push(*photo);
                }
            }
            Ok(added)
        })
    }

    /// Take a keyword off photos. Returns the photos that actually had it,
    /// which is exactly what an undo has to assign again.
    pub fn unassign_keyword(
        &mut self,
        keyword: KeywordId,
        photos: &[PhotoId],
    ) -> Result<Vec<PhotoId>> {
        self.write(|tx, _| {
            KEYWORDS.require(tx, keyword.0)?;
            let mut delete = tx.prepare_cached(
                "DELETE FROM photo_keywords WHERE photo_id = ?1 AND keyword_id = ?2",
            )?;
            let mut removed = Vec::new();
            for photo in photos {
                if delete.execute([photo.0, keyword.0])? > 0 {
                    removed.push(*photo);
                }
            }
            Ok(removed)
        })
    }

    /// Make a photo's keywords exactly the named ones and return their ids
    /// in the order given. Each name resolves as in
    /// [`Catalog::ensure_keyword`], creating keywords that do not exist yet;
    /// blank names are skipped and repeated names count once.
    pub fn set_photo_keywords<S: AsRef<str>>(
        &mut self,
        photo: PhotoId,
        names: &[S],
    ) -> Result<Vec<KeywordId>> {
        self.write(|tx, _| {
            require_photo(tx, photo)?;
            let mut keywords: Vec<i64> = Vec::new();
            for name in names
                .iter()
                .map(|name| name.as_ref().trim())
                .filter(|name| !name.is_empty())
            {
                let keyword = ensure_keyword(tx, name)?;
                if !keywords.contains(&keyword) {
                    keywords.push(keyword);
                }
            }
            tx.execute("DELETE FROM photo_keywords WHERE photo_id = ?1", [photo.0])?;
            let mut insert = tx.prepare_cached(
                "INSERT INTO photo_keywords (photo_id, keyword_id) VALUES (?1, ?2)",
            )?;
            for keyword in &keywords {
                insert.execute([photo.0, *keyword])?;
            }
            Ok(keywords.into_iter().map(KeywordId).collect())
        })
    }
}

/// Find a keyword by name anywhere in the hierarchy, or create it at the top
/// level.
pub(crate) fn ensure_keyword(conn: &Connection, name: &str) -> Result<i64> {
    let name = clean_name(name)?;
    let existing = conn
        .prepare_cached(
            "SELECT id FROM keywords WHERE name = ?1 COLLATE NOCASE ORDER BY id LIMIT 1",
        )?
        .query_row([name], |row| row.get(0))
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    conn.prepare_cached("INSERT INTO keywords (parent_id, name) VALUES (NULL, ?1)")?
        .execute([name])?;
    Ok(conn.last_insert_rowid())
}
