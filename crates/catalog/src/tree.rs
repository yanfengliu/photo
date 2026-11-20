//! Rules shared by the two hierarchies in the catalog, keywords and
//! collections: names are unique among siblings, and nothing may become its
//! own ancestor.

use rusqlite::{Connection, params};

use crate::error::{CatalogError, Entity, Result};

/// A table whose rows form a tree through a `parent_id` column and carry a
/// `name`.
pub(crate) struct Tree {
    table: &'static str,
    entity: Entity,
}

pub(crate) const KEYWORDS: Tree = Tree {
    table: "keywords",
    entity: Entity::Keyword,
};

pub(crate) const COLLECTIONS: Tree = Tree {
    table: "collections",
    entity: Entity::Collection,
};

impl Tree {
    /// Fail unless a row with this id exists.
    pub(crate) fn require(&self, conn: &Connection, id: i64) -> Result<()> {
        let found = conn
            .prepare_cached(&format!("SELECT 1 FROM {} WHERE id = ?1", self.table))?
            .exists([id])?;
        if found {
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
