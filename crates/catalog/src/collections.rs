//! Collections: sets that hold collections, plain collections, smart
//! collections, and the built-in quick collection. Membership of photos is
//! in `membership`.

use std::fmt;

use photo_core::meta::CollectionId;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::catalog::Catalog;
use crate::error::{CatalogError, Entity, Result};
use crate::query::Source;
use crate::smart::SmartRules;
use crate::text::natural_key;
use crate::tree::{COLLECTIONS, clean_name};

/// What a collection is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CollectionKind {
    /// Holds other collections and sets, never photos directly.
    Set,
    /// Holds the photos put into it, in an order of the user's choosing.
    Regular,
    /// Holds whatever photos match its rules.
    Smart,
    /// The one built-in quick collection: behaves like a regular collection
    /// but cannot be renamed, moved or deleted.
    Quick,
}

impl CollectionKind {
    fn to_sql(self) -> i64 {
        match self {
            Self::Set => 0,
            Self::Regular => 1,
            Self::Smart => 2,
            Self::Quick => 3,
        }
    }

    fn from_sql(value: i64) -> Self {
        match value {
            0 => Self::Set,
            2 => Self::Smart,
            3 => Self::Quick,
            _ => Self::Regular,
        }
    }

    /// Whether photos can be put into a collection of this kind.
    pub(crate) fn holds_photos(self) -> bool {
        matches!(self, Self::Regular | Self::Quick)
    }
}

impl fmt::Display for CollectionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Set => "collection set",
            Self::Regular => "collection",
            Self::Smart => "smart collection",
            Self::Quick => "quick collection",
        })
    }
}

/// A collection as the Collections panel lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionInfo {
    /// The collection's id.
    pub id: CollectionId,
    /// The set this collection is in, or `None` at the top level.
    pub parent: Option<CollectionId>,
    /// Whether it is a set, a regular collection or a smart collection.
    pub kind: CollectionKind,
    /// Its name, unique among its siblings ignoring ASCII case.
    pub name: String,
    /// How many photos selecting it shows: members, matches of a smart
    /// collection, or for a set the distinct members of everything inside.
    pub photo_count: u64,
}

impl Catalog {
    /// The id of the built-in quick collection. Photos go in and out of it
    /// like any regular collection.
    pub fn quick_collection(&self) -> CollectionId {
        self.quick_collection
    }

    /// Every collection except the quick collection, as a flat list in
    /// natural name order; the hierarchy is in each entry's `parent`.
    pub fn collections(&self) -> Result<Vec<CollectionInfo>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, parent_id, kind, name FROM collections WHERE kind <> 3")?;
        let rows = stmt.query_map([], |row| {
            Ok(CollectionInfo {
                id: CollectionId(row.get(0)?),
                parent: row.get::<_, Option<i64>>(1)?.map(CollectionId),
                kind: CollectionKind::from_sql(row.get(2)?),
                name: row.get(3)?,
                photo_count: 0,
            })
        })?;
        let mut collections: Vec<CollectionInfo> = rows.collect::<rusqlite::Result<_>>()?;
        for collection in &mut collections {
            collection.photo_count = match self.count(&Source::Collection(collection.id)) {
                Ok(count) => count,
                // One unreadable rule set must not take the whole panel down.
                Err(error @ (CatalogError::Corrupt { .. } | CatalogError::InvalidRule(_))) => {
                    log::warn!(
                        "smart collection {} cannot be evaluated: {error}",
                        collection.id
                    );
                    0
                }
                Err(error) => return Err(error),
            };
        }
        collections.sort_by_cached_key(|collection| (natural_key(&collection.name), collection.id));
        Ok(collections)
    }

    /// Create a collection set inside `parent` (or at the top level).
    pub fn create_collection_set(
        &mut self,
        name: &str,
        parent: Option<CollectionId>,
    ) -> Result<CollectionId> {
        self.create(CollectionKind::Set, name, parent, None)
    }

    /// Create an empty collection inside the set `parent` (or at the top
    /// level).
    pub fn create_collection(
        &mut self,
        name: &str,
        parent: Option<CollectionId>,
    ) -> Result<CollectionId> {
        self.create(CollectionKind::Regular, name, parent, None)
    }

    /// Create a smart collection inside the set `parent` (or at the top
    /// level). Rules that do not validate are refused.
    pub fn create_smart_collection(
        &mut self,
        name: &str,
        parent: Option<CollectionId>,
        rules: &SmartRules,
    ) -> Result<CollectionId> {
        self.create(
            CollectionKind::Smart,
            name,
            parent,
            Some(encode_rules(rules)?),
        )
    }

    fn create(
        &mut self,
        kind: CollectionKind,
        name: &str,
        parent: Option<CollectionId>,
        rules: Option<String>,
    ) -> Result<CollectionId> {
        let name = clean_name(name)?;
        let parent = parent.map(|parent| parent.0);
        self.write(|tx, now| {
            require_set(tx, parent)?;
            COLLECTIONS.require_free_name(tx, parent, name, None)?;
            tx.execute(
                "INSERT INTO collections (parent_id, kind, name, rules, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![parent, kind.to_sql(), name, rules, now],
            )?;
            Ok(CollectionId(tx.last_insert_rowid()))
        })
    }

    /// Rename a collection of any kind except the quick collection. Names
    /// are unique among siblings, ignoring ASCII case.
    pub fn rename_collection(&mut self, id: CollectionId, name: &str) -> Result<()> {
        let name = clean_name(name)?;
        self.write(|tx, _| {
            require_not_quick(tx, id)?;
            let parent = COLLECTIONS.parent(tx, id.0)?;
            COLLECTIONS.require_free_name(tx, parent, name, Some(id.0))?;
            tx.execute(
                "UPDATE collections SET name = ?2 WHERE id = ?1",
                params![id.0, name],
            )?;
            Ok(())
        })
    }

    /// Move a collection, with everything inside it, into the set `parent`
    /// (or to the top level).
    pub fn move_collection(
        &mut self,
        id: CollectionId,
        parent: Option<CollectionId>,
    ) -> Result<()> {
        let parent = parent.map(|parent| parent.0);
        self.write(|tx, _| {
            require_not_quick(tx, id)?;
            require_set(tx, parent)?;
            COLLECTIONS.require_no_cycle(tx, id.0, parent)?;
            let name: String = tx.query_row(
                "SELECT name FROM collections WHERE id = ?1",
                [id.0],
                |row| row.get(0),
            )?;
            COLLECTIONS.require_free_name(tx, parent, &name, Some(id.0))?;
            tx.execute(
                "UPDATE collections SET parent_id = ?2 WHERE id = ?1",
                params![id.0, parent],
            )?;
            Ok(())
        })
    }

    /// Delete a collection. Deleting a set deletes everything inside it.
    /// Memberships go with their collection; photos are never deleted.
    pub fn delete_collection(&mut self, id: CollectionId) -> Result<()> {
        self.write(|tx, _| {
            require_not_quick(tx, id)?;
            tx.execute("DELETE FROM collections WHERE id = ?1", [id.0])?;
            Ok(())
        })
    }

    /// The rules of a smart collection.
    pub fn smart_rules(&self, id: CollectionId) -> Result<SmartRules> {
        stored_rules(&self.conn, id)
    }

    /// Replace the rules of a smart collection. Rules that do not validate
    /// are refused and the old ones kept.
    pub fn set_smart_rules(&mut self, id: CollectionId, rules: &SmartRules) -> Result<()> {
        let rules = encode_rules(rules)?;
        self.write(|tx, _| {
            require_kind(tx, id, |kind| kind == CollectionKind::Smart)?;
            tx.execute(
                "UPDATE collections SET rules = ?2 WHERE id = ?1",
                params![id.0, rules],
            )?;
            Ok(())
        })
    }
}

fn encode_rules(rules: &SmartRules) -> Result<String> {
    rules.validate()?;
    serde_json::to_string(rules).map_err(|source| CatalogError::Unstorable {
        what: "smart collection rules",
        source,
    })
}

/// The kind of a collection. Fails if it does not exist.
pub(crate) fn collection_kind(conn: &Connection, id: CollectionId) -> Result<CollectionKind> {
    conn.prepare_cached("SELECT kind FROM collections WHERE id = ?1")?
        .query_row([id.0], |row| row.get(0).map(CollectionKind::from_sql))
        .optional()?
        .ok_or_else(|| CatalogError::not_found(Entity::Collection, id.0))
}

/// Fail unless the collection exists and its kind satisfies `allowed`.
pub(crate) fn require_kind(
    conn: &Connection,
    id: CollectionId,
    allowed: impl FnOnce(CollectionKind) -> bool,
) -> Result<()> {
    let kind = collection_kind(conn, id)?;
    if allowed(kind) {
        Ok(())
    } else {
        Err(CatalogError::CollectionKind { id, kind })
    }
}

fn require_not_quick(tx: &Transaction<'_>, id: CollectionId) -> Result<()> {
    require_kind(tx, id, |kind| kind != CollectionKind::Quick)
}

/// A parent, if given, must be an existing collection set.
fn require_set(tx: &Transaction<'_>, parent: Option<i64>) -> Result<()> {
    match parent {
        Some(parent) => require_kind(tx, CollectionId(parent), |kind| kind == CollectionKind::Set),
        None => Ok(()),
    }
}

/// The stored rules of a smart collection.
pub(crate) fn stored_rules(conn: &Connection, id: CollectionId) -> Result<SmartRules> {
    require_kind(conn, id, |kind| kind == CollectionKind::Smart)?;
    let json: String = conn
        .prepare_cached("SELECT rules FROM collections WHERE id = ?1")?
        .query_row([id.0], |row| row.get(0))?;
    serde_json::from_str(&json).map_err(|source| CatalogError::Corrupt {
        what: "smart collection rules",
        source,
    })
}
