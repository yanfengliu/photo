//! Keywords: a hierarchy of tags and their assignment to photos.

use photo_core::meta::{KeywordId, PhotoId};
use rusqlite::{Connection, Row, params};

use crate::catalog::Catalog;
use crate::codec::to_u64;
use crate::error::Result;
use crate::photos::{require_photo, require_photos};
use crate::text::natural_key;
use crate::tree::{DeletedTree, KEYWORDS, clean_name};

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

/// Keywords that were deleted together: the one asked for and every keyword
/// beneath it, with the photos each was assigned to.
///
/// Returned by [`Catalog::delete_keyword`]; hand it to
/// [`Catalog::restore_keywords`] to put everything back under the same ids.
/// It only lives in memory, and only makes sense for the catalog it came
/// from.
#[derive(Clone, Debug)]
pub struct DeletedKeywords {
    ids: Vec<KeywordId>,
    deleted: DeletedTree,
}

impl DeletedKeywords {
    /// Every keyword that was deleted: the one asked for first, then each
    /// keyword after the keyword it sat under.
    pub fn keyword_ids(&self) -> &[KeywordId] {
        &self.ids
    }
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

    /// The keyword a bare name stands for, ignoring ASCII case: the only
    /// keyword of that name, wherever it sits; or else, when there are none
    /// or several, the top-level keyword of that name, created if there is
    /// none. A name shared by several nested keywords never picks one of
    /// them.
    pub fn ensure_keyword(&mut self, name: &str) -> Result<KeywordId> {
        self.write(|tx, _| resolve_keyword(tx, name).map(|(id, _)| KeywordId(id)))
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
    /// assignments. Photos are not affected otherwise. Everything deleted is
    /// returned, so that [`Catalog::restore_keywords`] can undo it.
    pub fn delete_keyword(&mut self, id: KeywordId) -> Result<DeletedKeywords> {
        self.write(|tx, _| {
            let deleted = KEYWORDS.delete_branch(tx, id.0)?;
            Ok(DeletedKeywords {
                ids: deleted.ids.iter().copied().map(KeywordId).collect(),
                deleted,
            })
        })
    }

    /// Put deleted keywords back exactly as they were: the same ids, names
    /// and places in the hierarchy, assigned to the same photos.
    ///
    /// Refused, changing nothing, unless that can be done exactly:
    /// [`CatalogError::AlreadyExists`] if they are back already;
    /// [`CatalogError::NotFound`] if the keyword they sat under, or a photo
    /// that carried one of them, is gone (put that back first, as an undo
    /// stack does); and [`CatalogError::NameTaken`] if another keyword
    /// there has taken the name.
    ///
    /// [`CatalogError::AlreadyExists`]: crate::CatalogError::AlreadyExists
    /// [`CatalogError::NotFound`]: crate::CatalogError::NotFound
    /// [`CatalogError::NameTaken`]: crate::CatalogError::NameTaken
    pub fn restore_keywords(&mut self, deleted: &DeletedKeywords) -> Result<()> {
        self.write(|tx, _| KEYWORDS.restore_branch(tx, &deleted.deleted))
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

    /// Make a photo's keywords exactly the named ones. Blank names are
    /// skipped and names count once, ignoring ASCII case.
    ///
    /// A name the photo already carries keeps the keyword (or keywords) it
    /// has, wherever they sit. Any other name resolves as in
    /// [`Catalog::ensure_keyword`]: to the only keyword of that name, or
    /// else to the top-level keyword of that name, created if needed. So a
    /// bare name never lands on one of several nested keywords that share
    /// it; [`Catalog::set_photo_keyword_ids`] picks one exactly.
    pub fn set_photo_keywords<S: AsRef<str>>(
        &mut self,
        photo: PhotoId,
        names: &[S],
    ) -> Result<KeywordsChange> {
        self.write(|tx, _| {
            require_photo(tx, photo)?;
            let current = keywords_of(tx, photo)?;
            let (mut wanted, mut created, mut seen) = (Vec::new(), Vec::new(), Vec::new());
            for name in names
                .iter()
                .map(|name| name.as_ref().trim())
                .filter(|name| !name.is_empty())
            {
                let folded = name.to_ascii_lowercase();
                if seen.contains(&folded) {
                    continue;
                }
                let carried: Vec<i64> = current
                    .iter()
                    .filter(|(_, carried)| carried.to_ascii_lowercase() == folded)
                    .map(|(id, _)| *id)
                    .collect();
                seen.push(folded);
                if carried.is_empty() {
                    let (id, new) = resolve_keyword(tx, name)?;
                    if new {
                        created.push(KeywordId(id));
                    }
                    wanted.push(id);
                } else {
                    wanted.extend(carried);
                }
            }
            let current: Vec<i64> = current.into_iter().map(|(id, _)| id).collect();
            let (removed, added) = replace_keywords(tx, photo, &current, &wanted)?;
            Ok(KeywordsChange {
                removed,
                added,
                created,
            })
        })
    }

    /// Make a photo's keywords exactly these, by id; an id given twice
    /// counts once. All or nothing: an unknown keyword or photo fails the
    /// whole call.
    pub fn set_photo_keyword_ids(
        &mut self,
        photo: PhotoId,
        keywords: &[KeywordId],
    ) -> Result<KeywordsChange> {
        self.write(|tx, _| {
            require_photo(tx, photo)?;
            let mut wanted = Vec::new();
            for keyword in keywords {
                KEYWORDS.require(tx, keyword.0)?;
                if !wanted.contains(&keyword.0) {
                    wanted.push(keyword.0);
                }
            }
            let current: Vec<i64> = keywords_of(tx, photo)?
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            let (removed, added) = replace_keywords(tx, photo, &current, &wanted)?;
            Ok(KeywordsChange {
                removed,
                added,
                created: Vec::new(),
            })
        })
    }

    /// Make the keywords of any number of photos exactly the given ones, by
    /// id, in one transaction: [`Catalog::set_photo_keyword_ids`] for each
    /// entry in order, so a photo listed twice ends with its last list.
    /// Returns one change per entry, in the same order. All or nothing: an
    /// unknown keyword or photo fails the whole call and changes nothing.
    ///
    /// Handing each photo's former ids back to this call undoes it exactly,
    /// again in one transaction.
    pub fn set_keyword_ids_of_photos(
        &mut self,
        lists: &[(PhotoId, Vec<KeywordId>)],
    ) -> Result<Vec<KeywordsChange>> {
        self.write(|tx, _| {
            let mut known = std::collections::HashSet::new();
            let mut changes = Vec::with_capacity(lists.len());
            for (photo, keywords) in lists {
                require_photo(tx, *photo)?;
                let mut wanted = Vec::new();
                for keyword in keywords {
                    if known.insert(keyword.0) {
                        KEYWORDS.require(tx, keyword.0)?;
                    }
                    if !wanted.contains(&keyword.0) {
                        wanted.push(keyword.0);
                    }
                }
                let current: Vec<i64> = keywords_of(tx, *photo)?
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
                let (removed, added) = replace_keywords(tx, *photo, &current, &wanted)?;
                changes.push(KeywordsChange {
                    removed,
                    added,
                    created: Vec::new(),
                });
            }
            Ok(changes)
        })
    }
}

/// What setting a photo's keywords changed, which is exactly what undoing it
/// needs: take `added` off the photo and put `removed` back (with
/// [`Catalog::unassign_keyword`] and [`Catalog::assign_keyword`]), then
/// delete the keywords in `created`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeywordsChange {
    /// Keywords the photo carried before and no longer does, in id order.
    pub removed: Vec<KeywordId>,
    /// Keywords the photo carries now and did not before, in the order they
    /// were named.
    pub added: Vec<KeywordId>,
    /// Keywords that did not exist and were created for a name; each is
    /// also in `added`.
    pub created: Vec<KeywordId>,
}

/// The keywords a photo carries, with their names, in id order.
fn keywords_of(conn: &Connection, photo: PhotoId) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT k.id, k.name FROM photo_keywords pk JOIN keywords k ON k.id = pk.keyword_id
         WHERE pk.photo_id = ?1 ORDER BY k.id",
    )?;
    let keywords = stmt
        .query_map([photo.0], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(keywords)
}

/// Take the photo from the keywords in `current` to those in `wanted`,
/// touching only the links that differ, and say which went and which came.
fn replace_keywords(
    conn: &Connection,
    photo: PhotoId,
    current: &[i64],
    wanted: &[i64],
) -> Result<(Vec<KeywordId>, Vec<KeywordId>)> {
    let mut delete =
        conn.prepare_cached("DELETE FROM photo_keywords WHERE photo_id = ?1 AND keyword_id = ?2")?;
    let mut removed = Vec::new();
    for keyword in current.iter().filter(|keyword| !wanted.contains(keyword)) {
        delete.execute([photo.0, *keyword])?;
        removed.push(KeywordId(*keyword));
    }
    let mut insert =
        conn.prepare_cached("INSERT INTO photo_keywords (photo_id, keyword_id) VALUES (?1, ?2)")?;
    let mut added = Vec::new();
    for keyword in wanted.iter().filter(|keyword| !current.contains(keyword)) {
        insert.execute([photo.0, *keyword])?;
        added.push(KeywordId(*keyword));
    }
    Ok((removed, added))
}

/// The keyword a bare name stands for: the only keyword of that name
/// (ignoring ASCII case), or else the top-level keyword of that name,
/// created if there is none. The flag says whether it was created.
pub(crate) fn resolve_keyword(conn: &Connection, name: &str) -> Result<(i64, bool)> {
    let name = clean_name(name)?;
    let mut stmt =
        conn.prepare_cached("SELECT id, parent_id FROM keywords WHERE name = ?1 COLLATE NOCASE")?;
    let named: Vec<(i64, Option<i64>)> = stmt
        .query_map([name], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if let [(only, _)] = named.as_slice() {
        return Ok((*only, false));
    }
    if let Some((top, _)) = named.iter().find(|(_, parent)| parent.is_none()) {
        return Ok((*top, false));
    }
    conn.prepare_cached("INSERT INTO keywords (parent_id, name) VALUES (NULL, ?1)")?
        .execute([name])?;
    Ok((conn.last_insert_rowid(), true))
}
