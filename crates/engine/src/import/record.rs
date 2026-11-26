//! Handing a batch of imported photos to the catalog.

use photo_catalog::{CatalogError, NewPhoto};
use photo_core::meta::PhotoId;

/// What became of a batch.
pub(super) struct Recorded<'a> {
    /// The photos that went in, in the order of the batch.
    pub(super) ids: Vec<PhotoId>,
    /// The photos the catalog refused, each with the reason. They held
    /// none of the others back.
    pub(super) refused: Vec<(&'a NewPhoto, CatalogError)>,
    /// The photos that did not go in because the catalog as a whole could
    /// not be written, and why. They were not tried one by one.
    pub(super) stranded: Option<(&'a [NewPhoto], CatalogError)>,
}

/// Hand `batch` to the catalog: `all` at once and, when that is refused,
/// `one` by one, so that one bad photo does not sink the others.
///
/// A catalog that cannot be written at all (locked by another connection,
/// write-protected, failing; see [`CatalogError::is_catalog_wide`]) is not
/// asked again: trying photo by photo isolates a bad photo, and here there
/// is none to isolate, only the same wait and the same failure once more
/// for every photo.
pub(super) fn record<'a, C>(
    catalog: &mut C,
    batch: &'a [NewPhoto],
    all: impl FnOnce(&mut C, &[NewPhoto]) -> Result<Vec<PhotoId>, CatalogError>,
    mut one: impl FnMut(&mut C, &NewPhoto) -> Result<PhotoId, CatalogError>,
) -> Recorded<'a> {
    let mut recorded = Recorded {
        ids: Vec::new(),
        refused: Vec::new(),
        stranded: None,
    };
    match all(catalog, batch) {
        Ok(ids) => recorded.ids = ids,
        Err(error) if error.is_catalog_wide() => recorded.stranded = Some((batch, error)),
        Err(_) => {
            for (index, photo) in batch.iter().enumerate() {
                match one(catalog, photo) {
                    Ok(id) => recorded.ids.push(id),
                    Err(error) if error.is_catalog_wide() => {
                        recorded.stranded = Some((&batch[index..], error));
                        break;
                    }
                    Err(error) => recorded.refused.push((photo, error)),
                }
            }
        }
    }
    recorded
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use photo_catalog::Catalog;
    use photo_core::meta::{FileKind, ImportId};

    use super::*;

    /// A catalog, and how often it was asked to take a batch and to take
    /// one photo.
    struct Asked {
        catalog: Catalog,
        batches: usize,
        singles: usize,
    }

    impl Asked {
        fn of(catalog: Catalog) -> (Self, ImportId) {
            let mut asked = Self {
                catalog,
                batches: 0,
                singles: 0,
            };
            let import = asked.catalog.begin_import().unwrap();
            (asked, import)
        }
    }

    fn all(
        import: ImportId,
    ) -> impl FnOnce(&mut Asked, &[NewPhoto]) -> Result<Vec<PhotoId>, CatalogError> {
        move |asked, photos| {
            asked.batches += 1;
            asked.catalog.insert_photos(import, photos)
        }
    }

    fn one(import: ImportId) -> impl FnMut(&mut Asked, &NewPhoto) -> Result<PhotoId, CatalogError> {
        move |asked, photo| {
            asked.singles += 1;
            asked.catalog.insert_photo(import, photo)
        }
    }

    fn photos(names: &[&str]) -> Vec<NewPhoto> {
        names
            .iter()
            .map(|name| NewPhoto::new(format!("/r/{name}"), FileKind::Raster))
            .collect()
    }

    /// A catalog in a file, which does not wait for a lock, and a second
    /// connection to the same file.
    fn catalog_and_another_connection() -> (tempfile::TempDir, Catalog, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.photocat");
        let catalog = Catalog::open(&path).unwrap();
        catalog.set_busy_timeout(Duration::ZERO).unwrap();
        let other = rusqlite::Connection::open(&path).unwrap();
        (dir, catalog, other)
    }

    #[test]
    fn a_catalog_that_cannot_be_written_is_asked_once_and_not_again_for_every_photo() {
        let (_dir, catalog, other) = catalog_and_another_connection();
        let (mut asked, import) = Asked::of(catalog);
        // The other connection is in the middle of a change, and stays
        // there.
        other.execute_batch("BEGIN IMMEDIATE").unwrap();
        let batch = photos(&["a.jpg", "b.jpg", "c.jpg"]);
        let recorded = record(&mut asked, &batch, all(import), one(import));
        assert_eq!(
            (asked.batches, asked.singles),
            (1, 0),
            "the batch, and not each of its photos after it"
        );
        assert!(recorded.ids.is_empty() && recorded.refused.is_empty());
        let (stranded, error) = recorded.stranded.expect("the whole batch is left");
        assert_eq!(stranded.len(), 3);
        assert!(error.is_catalog_wide(), "{error}");
    }

    #[test]
    fn one_photo_the_catalog_refuses_does_not_sink_the_others() {
        let (mut asked, import) = Asked::of(Catalog::open_in_memory().unwrap());
        asked
            .catalog
            .insert_photos(import, &photos(&["b.jpg"]))
            .unwrap();
        let batch = photos(&["a.jpg", "b.jpg", "c.jpg"]);
        let recorded = record(&mut asked, &batch, all(import), one(import));
        assert_eq!((asked.batches, asked.singles), (1, 3));
        assert_eq!(recorded.ids.len(), 2, "the two the catalog can take");
        let [(refused, error)] = recorded.refused.as_slice() else {
            panic!("{} photos were refused", recorded.refused.len());
        };
        assert_eq!(refused.path, batch[1].path);
        assert!(matches!(error, CatalogError::DuplicatePath(_)), "{error}");
        assert!(recorded.stranded.is_none());
    }

    #[test]
    fn a_catalog_that_stops_being_writable_part_way_is_not_asked_for_the_rest() {
        let (_dir, catalog, other) = catalog_and_another_connection();
        let (mut asked, import) = Asked::of(catalog);
        asked
            .catalog
            .insert_photos(import, &photos(&["b.jpg"]))
            .unwrap();
        let batch = photos(&["a.jpg", "b.jpg", "c.jpg", "d.jpg"]);
        let mut insert = one(import);
        // The batch is refused for the photo already there. While the
        // photos are tried one by one, the other connection starts a
        // change after the second and stays in it.
        let one_then_locked = |asked: &mut Asked, photo: &NewPhoto| {
            let result = insert(asked, photo);
            if asked.singles == 2 {
                other.execute_batch("BEGIN IMMEDIATE").unwrap();
            }
            result
        };
        let recorded = record(&mut asked, &batch, all(import), one_then_locked);
        assert_eq!(
            (asked.batches, asked.singles),
            (1, 3),
            "the fourth photo is not tried once the third met the lock"
        );
        assert_eq!(recorded.ids.len(), 1);
        assert_eq!(recorded.refused.len(), 1);
        let (stranded, error) = recorded.stranded.expect("the rest is left");
        assert_eq!(stranded.len(), 2);
        assert_eq!(stranded[0].path, batch[2].path);
        assert!(error.is_catalog_wide(), "{error}");
    }
}
