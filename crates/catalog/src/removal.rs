//! Removing photos from the catalog, undoably.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use photo_core::meta::PhotoId;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::catalog::Catalog;
use crate::error::{CatalogError, Result};
use crate::folders::ensure_folder;
use crate::membership::make_places;
use crate::paths;
use crate::photos::{master_at_path, require_photos, sync_copies};
use crate::rows::{RowSet, integer, text};
use crate::sql::id_list;

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
    /// What changed in the meantime is handled rather than failed on: a
    /// keyword or collection that has since been deleted is simply not
    /// re-attached; a folder that has since been removed is created again;
    /// and a virtual copy that was removed without its master follows the
    /// master to wherever its file is now. If a restored file's path has
    /// since been imported again, the call fails with
    /// [`CatalogError::DuplicatePath`] and restores nothing.
    pub fn restore_photos(&mut self, removed: &RemovedPhotos) -> Result<()> {
        self.write(|tx, _| {
            restore_rows(tx, &removed.photos)?;
            removed.history.insert(tx)?;
            removed.snapshots.insert(tx)?;
            surviving(
                tx,
                &removed.keywords,
                "keyword_id",
                "SELECT id FROM keywords",
            )?
            .insert(tx)?;
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

/// Re-insert photo rows, masters before the copies that refer to them.
fn restore_rows(tx: &Transaction<'_>, photos: &RowSet) -> Result<()> {
    let (master, folder, path) = (
        photos.column("master_id")?,
        photos.column("folder_id")?,
        photos.column("path")?,
    );
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
    let memberships = &removed.collections;
    let (collection_at, photo_at, position_at) = (
        memberships.column("collection_id")?,
        memberships.column("photo_id")?,
        memberships.column("position")?,
    );
    let mut memberships = surviving(
        tx,
        memberships,
        "collection_id",
        "SELECT id FROM collections",
    )?;
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

/// The link rows whose other end (`parent`) still exists.
fn surviving(
    conn: &Connection,
    links: &RowSet,
    parent: &str,
    existing: &'static str,
) -> Result<RowSet> {
    let column = links.column(parent)?;
    let mut stmt = conn.prepare_cached(existing)?;
    let alive: HashSet<i64> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut links = links.clone();
    links
        .rows_mut()
        .retain(|row| integer(&row[column]).is_some_and(|id| alive.contains(&id)));
    Ok(links)
}
