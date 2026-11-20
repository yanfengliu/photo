//! Removing photos from the catalog, undoably.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use photo_core::meta::PhotoId;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::catalog::Catalog;
use crate::develop::next_develop_version;
use crate::error::{CatalogError, Entity, Result};
use crate::folders::ensure_folder;
use crate::membership::make_places;
use crate::paths;
use crate::photos::{master_at_path, require_photos, sync_copies};
use crate::rows::{RowSet, integer, text};
use crate::sql::id_list;
use crate::tree::{COLLECTIONS, KEYWORDS, Tree};

const LISTED: &str = "id IN (SELECT value FROM json_each(?1))";
const OF_LISTED: &str = "photo_id IN (SELECT value FROM json_each(?1))";

/// Photos that were removed from the catalog, with everything that belonged
/// to them: their virtual copies, develop history, snapshots, keyword
/// assignments and collection memberships.
///
/// Returned by [`Catalog::remove_photos`]; hand it to
/// [`Catalog::restore_photos`] to put everything back under the same ids. It
/// only lives in memory, and only makes sense for the catalog it came from.
#[derive(Clone, Debug)]
pub struct RemovedPhotos {
    ids: Vec<PhotoId>,
    photos: RowSet,
    history: RowSet,
    snapshots: RowSet,
    keywords: RowSet,
    collections: RowSet,
    /// For each `(collection, photo)` membership, the photo's place in that
    /// collection's order (counting from 0) when it was removed.
    places: HashMap<(i64, i64), i64>,
}

impl RemovedPhotos {
    /// Every photo that was removed, in id order: the ones asked for and the
    /// virtual copies that went with their masters.
    pub fn photo_ids(&self) -> &[PhotoId] {
        &self.ids
    }
}

impl Catalog {
    /// Remove photos from the catalog. The files on disk are not touched.
    ///
    /// Removing a master also removes its virtual copies. Everything that
    /// hangs off a removed photo goes with it, and all of it is returned so
    /// the removal can be undone. All or nothing: an id that is not in the
    /// catalog fails the whole call.
    pub fn remove_photos(&mut self, photos: &[PhotoId]) -> Result<RemovedPhotos> {
        self.write(|tx, _| {
            require_photos(tx, photos)?;
            let asked = id_list(photos.iter().map(|photo| photo.0));
            let mut stmt = tx.prepare_cached(
                "SELECT id FROM photos
                 WHERE id IN (SELECT value FROM json_each(?1)) OR master_id IN (SELECT value FROM json_each(?1))
                 ORDER BY id",
            )?;
            let ids: Vec<i64> = stmt.query_map([&asked], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
            let all = id_list(ids.iter().copied());

            let removed = RemovedPhotos {
                photos: RowSet::capture(tx, "photos", LISTED, [&all])?,
                history: RowSet::capture(tx, "history", OF_LISTED, [&all])?,
                snapshots: RowSet::capture(tx, "snapshots", OF_LISTED, [&all])?,
                keywords: RowSet::capture(tx, "photo_keywords", OF_LISTED, [&all])?,
                collections: RowSet::capture(tx, "collection_photos", OF_LISTED, [&all])?,
                places: places_in_collections(tx, &all)?,
                ids: ids.into_iter().map(PhotoId).collect(),
            };
            tx.execute("DELETE FROM photos WHERE id IN (SELECT value FROM json_each(?1))", [&all])?;
            Ok(removed)
        })
    }

    /// Put removed photos back exactly as they were: the same ids, the same
    /// attributes and develop state, history, snapshots, keywords and
    /// collection memberships (at their old places in each collection).
    ///
    /// Each photo comes back under a new develop version (see
    /// [`PhotoRecord::develop_version`](crate::PhotoRecord::develop_version)).
    /// A folder that has since been removed is created again, and a virtual
    /// copy that was removed without its master follows the master to
    /// wherever its file is now.
    ///
    /// Anything else that stands in the way fails the call, and nothing is
    /// restored: [`CatalogError::AlreadyExists`] if the photos are back
    /// already; [`CatalogError::DuplicatePath`] if a file's path has been
    /// imported again; and [`CatalogError::NotFound`] if a keyword or
    /// collection they belonged to has since been deleted, because the link
    /// could not come back with them. Put that back first
    /// ([`Catalog::restore_keywords`], [`Catalog::restore_collections`]),
    /// as an undo stack does.
    pub fn restore_photos(&mut self, removed: &RemovedPhotos) -> Result<()> {
        self.write(|tx, _| {
            restore_rows(tx, &removed.photos)?;
            removed.history.insert(tx)?;
            removed.snapshots.insert(tx)?;
            require_all(tx, &removed.keywords, "keyword_id", &KEYWORDS)?;
            removed.keywords.insert(tx)?;
            require_all(tx, &removed.collections, "collection_id", &COLLECTIONS)?;
            restore_memberships(tx, removed)
        })
    }
}

/// The place of each listed photo in every collection it is in, counted over
/// all the members of that collection.
fn places_in_collections(conn: &Connection, photos: &str) -> Result<HashMap<(i64, i64), i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT collection_id, photo_id, place FROM (
             SELECT collection_id, photo_id,
                    ROW_NUMBER() OVER (PARTITION BY collection_id ORDER BY position, photo_id) - 1 AS place
             FROM collection_photos
             WHERE collection_id IN (
                 SELECT collection_id FROM collection_photos WHERE photo_id IN (SELECT value FROM json_each(?1))
             )
         ) WHERE photo_id IN (SELECT value FROM json_each(?1))",
    )?;
    let places = stmt
        .query_map([photos], |row| {
            Ok(((row.get(0)?, row.get(1)?), row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(places)
}

/// Re-insert photo rows, masters before the copies that refer to them. Each
/// comes back under a new develop version: a version is never issued twice,
/// so one that was handed out after the removal (by an undo, say) cannot
/// now name a second look.
fn restore_rows(tx: &Transaction<'_>, photos: &RowSet) -> Result<()> {
    let (id, master, folder, path, version) = (
        photos.column("id")?,
        photos.column("master_id")?,
        photos.column("folder_id")?,
        photos.column("path")?,
        photos.column("develop_version")?,
    );
    let mut photos = photos.clone();
    for row in photos.rows_mut() {
        if let Some(photo) = integer(&row[id])
            && tx
                .prepare_cached("SELECT 1 FROM photos WHERE id = ?1")?
                .exists([photo])?
        {
            return Err(CatalogError::AlreadyExists {
                entity: Entity::Photo,
                id: photo,
            });
        }
        row[version] = Value::Integer(next_develop_version(tx)?);
    }
    let mut masters = photos.clone();
    masters.rows_mut().retain(|row| row[master] == Value::Null);
    for row in masters.rows_mut() {
        let file = text(&row[path]).unwrap_or_default().to_string();
        if master_at_path(tx, &file)?.is_some() {
            return Err(CatalogError::DuplicatePath(file));
        }
        let known = tx
            .prepare_cached("SELECT 1 FROM folders WHERE id = ?1")?
            .exists([integer(&row[folder])])?;
        if !known {
            row[folder] = Value::Integer(ensure_folder(tx, paths::split(&file)?.dir)?);
        }
    }
    masters.insert(tx)?;

    // A copy shares its master's file. The master is in the catalog by now
    // (just restored, or never removed), and if it was relinked or reported
    // missing while the copy was away, the copy must follow it.
    let mut copies = photos.clone();
    copies.rows_mut().retain(|row| row[master] != Value::Null);
    let mut owners = BTreeSet::new();
    for row in copies.rows_mut() {
        let owner = integer(&row[master]);
        let current: Option<i64> = tx
            .prepare_cached("SELECT folder_id FROM photos WHERE id = ?1")?
            .query_row([owner], |row| row.get(0))
            .optional()?;
        if let Some(current) = current {
            row[folder] = Value::Integer(current);
        }
        owners.extend(owner);
    }
    copies.insert(tx)?;
    owners
        .into_iter()
        .try_for_each(|owner| sync_copies(tx, owner))
}

/// Re-insert collection memberships, each at the place it had in its
/// collection's order.
fn restore_memberships(tx: &Transaction<'_>, removed: &RemovedPhotos) -> Result<()> {
    let mut memberships = removed.collections.clone();
    let (collection_at, photo_at, position_at) = (
        memberships.column("collection_id")?,
        memberships.column("photo_id")?,
        memberships.column("position")?,
    );
    let membership =
        |row: &[Value]| Some((integer(&row[collection_at])?, integer(&row[photo_at])?));

    let mut returning: BTreeMap<i64, Vec<(i64, i64)>> = BTreeMap::new();
    for (collection, photo) in memberships.rows().iter().filter_map(|row| membership(row)) {
        // A membership with no recorded place goes to the end.
        let place = removed
            .places
            .get(&(collection, photo))
            .copied()
            .unwrap_or(i64::MAX);
        returning
            .entry(collection)
            .or_default()
            .push((photo, place));
    }
    let mut positions = HashMap::new();
    for (collection, places) in returning {
        for (photo, position) in make_places(tx, collection, places)? {
            positions.insert((collection, photo), position);
        }
    }
    for row in memberships.rows_mut() {
        if let Some(position) = membership(row).and_then(|key| positions.get(&key)) {
            row[position_at] = Value::Integer(*position);
        }
    }
    memberships.insert(tx)
}

/// Fail with [`CatalogError::NotFound`] unless the keyword or collection
/// that every one of `links` points at (in `column`) still exists.
fn require_all(conn: &Connection, links: &RowSet, column: &str, tree: &Tree) -> Result<()> {
    let at = links.column(column)?;
    let targets: BTreeSet<i64> = links
        .rows()
        .iter()
        .filter_map(|row| integer(&row[at]))
        .collect();
    targets
        .into_iter()
        .try_for_each(|id| tree.require(conn, id))
}
