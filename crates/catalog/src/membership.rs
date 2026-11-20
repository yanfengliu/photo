//! Which photos are in which collection, and in what order.
//!
//! Every member has a position, and a collection's own order is ascending
//! position. The numbers themselves mean nothing outside this module: adding
//! appends, a new order renumbers from 1, and a removed photo is remembered
//! by its *place* in the order (first, second, ...), which is what lets it
//! return between the same neighbours whatever happened to the numbers.

use std::collections::HashSet;

use photo_core::meta::{CollectionId, PhotoId};
use rusqlite::{Connection, Transaction, params};

use crate::catalog::Catalog;
use crate::codec::{to_i64, to_u64};
use crate::collections::{CollectionKind, require_kind};
use crate::error::{CatalogError, Result};
use crate::photos::{require_photo, require_photos};
use crate::sql::id_list;

/// A photo's place in a collection, as returned when it is removed so that
/// it can be put back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CollectionEntry {
    /// The photo that was in the collection.
    pub photo: PhotoId,
    /// Its place in the collection's own order just before it was removed,
    /// counting from 0.
    pub index: u64,
}

fn require_holds_photos(tx: &Transaction<'_>, collection: CollectionId) -> Result<()> {
    require_kind(tx, collection, CollectionKind::holds_photos)
}

impl Catalog {
    /// Add photos to the end of a collection, in the order given. Returns
    /// the photos that were not in it before, which is exactly what an undo
    /// has to remove; photos already in the collection keep their place.
    ///
    /// Only regular collections and the quick collection hold photos. All
    /// or nothing: an unknown photo fails the whole call.
    pub fn add_to_collection(
        &mut self,
        collection: CollectionId,
        photos: &[PhotoId],
    ) -> Result<Vec<PhotoId>> {
        self.write(|tx, _| {
            require_holds_photos(tx, collection)?;
            require_photos(tx, photos)?;
            let mut next: i64 = tx.query_row(
                "SELECT COALESCE(MAX(position), 0) + 1 FROM collection_photos WHERE collection_id = ?1",
                [collection.0],
                |row| row.get(0),
            )?;
            let mut insert = tx.prepare_cached(
                "INSERT OR IGNORE INTO collection_photos (collection_id, photo_id, position) VALUES (?1, ?2, ?3)",
            )?;
            let mut added = Vec::new();
            for photo in photos {
                if insert.execute([collection.0, photo.0, next])? > 0 {
                    added.push(*photo);
                    next += 1;
                }
            }
            Ok(added)
        })
    }

    /// Take photos out of a collection. Returns the entries that were
    /// actually removed, each with its place in the collection's order, in
    /// that order; this is exactly what
    /// [`Catalog::restore_collection_entries`] needs to undo it. Photos that
    /// were not in the collection are ignored.
    pub fn remove_from_collection(
        &mut self,
        collection: CollectionId,
        photos: &[PhotoId],
    ) -> Result<Vec<CollectionEntry>> {
        self.write(|tx, _| {
            require_holds_photos(tx, collection)?;
            let listed = id_list(photos.iter().map(|photo| photo.0));
            // Places are counted over the whole collection, then narrowed to
            // the photos that are leaving.
            let mut stmt = tx.prepare_cached(
                "SELECT photo_id, place FROM (
                     SELECT photo_id, ROW_NUMBER() OVER (ORDER BY position, photo_id) - 1 AS place
                     FROM collection_photos WHERE collection_id = ?1
                 ) WHERE photo_id IN (SELECT value FROM json_each(?2)) ORDER BY place",
            )?;
            let removed = stmt
                .query_map(params![collection.0, listed], |row| {
                    Ok(CollectionEntry {
                        photo: PhotoId(row.get(0)?),
                        index: to_u64(row.get(1)?),
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            tx.execute(
                "DELETE FROM collection_photos
                 WHERE collection_id = ?1 AND photo_id IN (SELECT value FROM json_each(?2))",
                params![collection.0, listed],
            )?;
            Ok(removed)
        })
    }

    /// Put removed entries back at the places they had in the collection's
    /// order. Undone in the reverse order of the removals (and of any
    /// reordering in between), this restores the earlier order exactly;
    /// otherwise each entry goes back in at its old place in line. Entries
    /// whose photo is in the collection again already are left where they
    /// are.
    pub fn restore_collection_entries(
        &mut self,
        collection: CollectionId,
        entries: &[CollectionEntry],
    ) -> Result<()> {
        self.write(|tx, _| {
            require_holds_photos(tx, collection)?;
            let photos: Vec<PhotoId> = entries.iter().map(|entry| entry.photo).collect();
            require_photos(tx, &photos)?;
            let places = entries.iter().map(|entry| (entry.photo.0, to_i64(entry.index))).collect();
            let mut insert = tx.prepare_cached(
                "INSERT INTO collection_photos (collection_id, photo_id, position) VALUES (?1, ?2, ?3)",
            )?;
            for (photo, position) in make_places(tx, collection.0, places)? {
                insert.execute([collection.0, photo, position])?;
            }
            Ok(())
        })
    }

    /// Give a collection a new order. `order` must list exactly the
    /// collection's members, each once; otherwise the call fails with
    /// [`CatalogError::OrderMismatch`] and the order is unchanged.
    pub fn set_collection_order(
        &mut self,
        collection: CollectionId,
        order: &[PhotoId],
    ) -> Result<()> {
        self.write(|tx, _| {
            require_holds_photos(tx, collection)?;
            let members: HashSet<PhotoId> = members(tx, collection)?.into_iter().collect();
            let listed: HashSet<PhotoId> = order.iter().copied().collect();
            if listed.len() != order.len() || listed != members {
                return Err(CatalogError::OrderMismatch(collection));
            }
            let mut update = tx.prepare_cached(
                "UPDATE collection_photos SET position = ?3 WHERE collection_id = ?1 AND photo_id = ?2",
            )?;
            for (photo, position) in order.iter().zip(1_i64..) {
                update.execute(params![collection.0, photo.0, position])?;
            }
            Ok(())
        })
    }

    /// The members of a regular collection or the quick collection, in the
    /// collection's own order.
    pub fn collection_photos(&self, collection: CollectionId) -> Result<Vec<PhotoId>> {
        require_kind(&self.conn, collection, CollectionKind::holds_photos)?;
        members(&self.conn, collection)
    }

    /// The collections (the quick collection included) a photo has been put
    /// into, by id. Smart collections are not listed: a photo matches those,
    /// it is not put into them.
    pub fn photo_collections(&self, photo: PhotoId) -> Result<Vec<CollectionId>> {
        require_photo(&self.conn, photo)?;
        let mut stmt = self
            .conn
            .prepare_cached("SELECT collection_id FROM collection_photos WHERE photo_id = ?1 ORDER BY collection_id")?;
        let collections = stmt
            .query_map([photo.0], |row| row.get(0).map(CollectionId))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(collections)
    }
}

/// Make room for photos that are returning to a collection, and say where
/// each goes.
///
/// `places` pairs a photo with the place (counting from 0) it should take in
/// the collection's order. The result pairs each photo that is not already a
/// member with the position to insert it at; members that are in the way
/// have been moved along. Handled lowest place first, so photos that left
/// together come back in the order they were in.
pub(crate) fn make_places(
    conn: &Connection,
    collection: i64,
    mut places: Vec<(i64, i64)>,
) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT photo_id, position FROM collection_photos WHERE collection_id = ?1 ORDER BY position, photo_id",
    )?;
    let members: Vec<(i64, i64)> = stmt
        .query_map([collection], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut seen: HashSet<i64> = members.iter().map(|(photo, _)| *photo).collect();
    places.sort_by_key(|(photo, place)| (*place, *photo));
    places.retain(|(photo, _)| seen.insert(*photo));

    let mut update = conn.prepare_cached(
        "UPDATE collection_photos SET position = ?3 WHERE collection_id = ?1 AND photo_id = ?2",
    )?;
    let mut placed = Vec::with_capacity(places.len());
    let (mut returning, mut staying) = (
        places.into_iter().peekable(),
        members.into_iter().peekable(),
    );
    // Walk the new order from the front, giving each photo a position above
    // the one before it. A member keeps its position whenever that is still
    // high enough, so nothing is rewritten that does not have to be.
    let (mut place, mut last) = (0_i64, 0_i64);
    loop {
        let returns_here = match (returning.peek(), staying.peek()) {
            (Some((_, wanted)), Some(_)) => *wanted <= place,
            (Some(_), None) => true,
            (None, Some((_, position))) if *position <= last => false,
            // Nobody left to put back, and everyone after this is in order.
            (None, _) => break,
        };
        if returns_here {
            if let Some((photo, _)) = returning.next() {
                last += 1;
                placed.push((photo, last));
            }
        } else if let Some((photo, position)) = staying.next() {
            if position > last {
                last = position;
            } else {
                last += 1;
                update.execute([collection, photo, last])?;
            }
        }
        place += 1;
    }
    Ok(placed)
}

fn members(conn: &Connection, collection: CollectionId) -> Result<Vec<PhotoId>> {
    let mut stmt = conn.prepare_cached(
        "SELECT photo_id FROM collection_photos WHERE collection_id = ?1 ORDER BY position, photo_id",
    )?;
    let photos = stmt
        .query_map([collection.0], |row| row.get(0).map(PhotoId))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(photos)
}
