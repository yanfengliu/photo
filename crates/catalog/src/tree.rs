//! Rules shared by the two hierarchies in the catalog, keywords and
//! collections: names are unique among siblings, nothing may become its own
//! ancestor, and deleting a branch can be undone exactly.

use std::collections::{BTreeSet, HashMap, HashSet};

use photo_core::meta::PhotoId;
use rusqlite::{Connection, params};

use crate::error::{CatalogError, Entity, Result};
use crate::photos::require_photo;
use crate::rows::{RowSet, integer, text};
use crate::sql::id_list;

/// A table whose rows form a tree through a `parent_id` column and carry a
/// `name`, and the table that links photos to its rows.
pub(crate) struct Tree {
    table: &'static str,
    entity: Entity,
    links: &'static str,
    /// The rows of `links` that belong to the rows listed in `?1`.
    links_of_listed: &'static str,
}

pub(crate) const KEYWORDS: Tree = Tree {
    table: "keywords",
    entity: Entity::Keyword,
    links: "photo_keywords",
    links_of_listed: "keyword_id IN (SELECT value FROM json_each(?1))",
};

pub(crate) const COLLECTIONS: Tree = Tree {
    table: "collections",
    entity: Entity::Collection,
    links: "collection_photos",
    links_of_listed: "collection_id IN (SELECT value FROM json_each(?1))",
};

const LISTED: &str = "id IN (SELECT value FROM json_each(?1))";

/// A row of a tree that was deleted with everything beneath it, and every
/// link from a photo to any of them, ready to be put back.
#[derive(Clone, Debug)]
pub(crate) struct DeletedTree {
    /// The rows' ids: the one that was deleted first, then each row after
    /// the row above it.
    pub(crate) ids: Vec<i64>,
    /// The rows, in the same order, so that each is put back after its
    /// parent.
    rows: RowSet,
    links: RowSet,
}

impl Tree {
    fn exists(&self, conn: &Connection, id: i64) -> Result<bool> {
        let found = conn
            .prepare_cached(&format!("SELECT 1 FROM {} WHERE id = ?1", self.table))?
            .exists([id])?;
        Ok(found)
    }

    /// Fail unless a row with this id exists.
    pub(crate) fn require(&self, conn: &Connection, id: i64) -> Result<()> {
        if self.exists(conn, id)? {
            Ok(())
        } else {
            Err(CatalogError::not_found(self.entity, id))
        }
    }

    /// Fail with [`CatalogError::NameTaken`] if a child of `parent` other
    /// than `except` already has this name (ignoring ASCII case).
    pub(crate) fn require_free_name(
        &self,
        conn: &Connection,
        parent: Option<i64>,
        name: &str,
        except: Option<i64>,
    ) -> Result<()> {
        let taken = conn
            .prepare_cached(&format!(
                "SELECT 1 FROM {} WHERE COALESCE(parent_id, 0) = COALESCE(?1, 0)
                 AND name = ?2 COLLATE NOCASE AND id IS NOT ?3",
                self.table
            ))?
            .exists(params![parent, name, except])?;
        if taken {
            Err(CatalogError::NameTaken(name.to_string()))
        } else {
            Ok(())
        }
    }

    /// Fail with [`CatalogError::Cycle`] if putting `id` under `parent`
    /// would make it its own ancestor.
    pub(crate) fn require_no_cycle(
        &self,
        conn: &Connection,
        id: i64,
        parent: Option<i64>,
    ) -> Result<()> {
        let cycle = conn
            .prepare_cached(&format!(
                "WITH RECURSIVE up(id) AS (
                     SELECT ?1
                     UNION
                     SELECT t.parent_id FROM {} t JOIN up ON t.id = up.id WHERE t.parent_id IS NOT NULL
                 )
                 SELECT 1 FROM up WHERE id = ?2",
                self.table
            ))?
            .exists(params![parent, id])?;
        if cycle {
            Err(CatalogError::Cycle {
                entity: self.entity,
                id,
            })
        } else {
            Ok(())
        }
    }

    /// The parent of row `id`. Fails if the row does not exist.
    pub(crate) fn parent(&self, conn: &Connection, id: i64) -> Result<Option<i64>> {
        self.require(conn, id)?;
        let parent = conn
            .prepare_cached(&format!(
                "SELECT parent_id FROM {} WHERE id = ?1",
                self.table
            ))?
            .query_row([id], |row| row.get(0))?;
        Ok(parent)
    }

    /// Row `id` and every row beneath it, each after the row above it:
    /// breadth first, siblings in id order.
    fn branch(&self, conn: &Connection, id: i64) -> Result<Vec<i64>> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT id FROM {} WHERE parent_id = ?1 ORDER BY id",
            self.table
        ))?;
        let (mut ids, mut seen) = (vec![id], HashSet::from([id]));
        let mut next = 0;
        while let Some(parent) = ids.get(next).copied() {
            let children: Vec<i64> = stmt
                .query_map([parent], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            // `seen` keeps a damaged file whose rows form a loop from
            // keeping this busy for ever.
            ids.extend(children.into_iter().filter(|child| seen.insert(*child)));
            next += 1;
        }
        Ok(ids)
    }

    /// Delete row `id`, every row beneath it and every link from a photo to
    /// any of them, and return all of it.
    pub(crate) fn delete_branch(&self, conn: &Connection, id: i64) -> Result<DeletedTree> {
        self.require(conn, id)?;
        let ids = self.branch(conn, id)?;
        let listed = id_list(ids.iter().copied());
        let mut rows = RowSet::capture(conn, self.table, LISTED, [&listed])?;
        let rank: HashMap<i64, usize> = ids
            .iter()
            .enumerate()
            .map(|(rank, id)| (*id, rank))
            .collect();
        let id_at = rows.column("id")?;
        rows.rows_mut()
            .sort_by_key(|row| integer(&row[id_at]).and_then(|id| rank.get(&id).copied()));
        let links = RowSet::capture(conn, self.links, self.links_of_listed, [&listed])?;
        // Everything beneath goes with it, by the cascades on parent_id and
        // on the links.
        conn.prepare_cached(&format!("DELETE FROM {} WHERE id = ?1", self.table))?
            .execute([id])?;
        Ok(DeletedTree { ids, rows, links })
    }

    /// Put a deleted branch back under its own ids, with its links to
    /// photos. Refused, changing nothing, unless it can be put back exactly:
    /// [`CatalogError::AlreadyExists`] if it is back already,
    /// [`CatalogError::NotFound`] if the row it hung from or a linked photo
    /// is gone, and [`CatalogError::NameTaken`] if a sibling has taken its
    /// name.
    pub(crate) fn restore_branch(&self, conn: &Connection, deleted: &DeletedTree) -> Result<()> {
        if let Some(id) = first_that(&deleted.ids, |id| self.exists(conn, *id))? {
            return Err(CatalogError::AlreadyExists {
                entity: self.entity,
                id,
            });
        }
        // Rows beneath the first go back under rows that come back with
        // them; only the first goes back under a row that may have changed.
        if let Some(top) = deleted.rows.rows().first() {
            let parent = integer(&top[deleted.rows.column("parent_id")?]);
            if let Some(parent) = parent {
                self.require(conn, parent)?;
            }
            let name = text(&top[deleted.rows.column("name")?]).unwrap_or_default();
            self.require_free_name(conn, parent, name, None)?;
        }
        let photo_at = deleted.links.column("photo_id")?;
        let photos: BTreeSet<i64> = deleted
            .links
            .rows()
            .iter()
            .filter_map(|link| integer(&link[photo_at]))
            .collect();
        for photo in photos {
            require_photo(conn, PhotoId(photo))?;
        }
        deleted.rows.insert(conn)?;
        deleted.links.insert(conn)
    }
}

/// The first id for which `test` holds.
fn first_that(ids: &[i64], mut test: impl FnMut(&i64) -> Result<bool>) -> Result<Option<i64>> {
    for id in ids {
        if test(id)? {
            return Ok(Some(*id));
        }
    }
    Ok(None)
}

/// A name with surrounding white space removed; empty names are refused.
pub(crate) fn clean_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        Err(CatalogError::EmptyName)
    } else {
        Ok(name)
    }
}
