//! Fixtures shared by the catalog's contract tests. Every catalog a test
//! opens is in memory or inside a `tempfile` directory.
#![allow(dead_code)] // each test binary uses its own subset

pub mod smart;

use photo_catalog::{Catalog, NewPhoto, PhotoQuery, PhotoSummary};
use photo_core::meta::{FileKind, PhotoId};
use photo_core::time::DateTime;

pub fn memory() -> Catalog {
    Catalog::open_in_memory().unwrap()
}

pub fn raster(path: &str) -> NewPhoto {
    NewPhoto::new(path, FileKind::Raster)
}

pub fn raw(path: &str) -> NewPhoto {
    NewPhoto::new(path, FileKind::Raw)
}

/// Noon on the given day.
pub fn date(year: i32, month: u8, day: u8) -> DateTime {
    DateTime::new(year, month, day, 12, 0, 0).unwrap()
}

/// Insert `photos` as one import run and return their ids in order.
pub fn import(catalog: &mut Catalog, photos: &[NewPhoto]) -> Vec<PhotoId> {
    let run = catalog.begin_import().unwrap();
    catalog.insert_photos(run, photos).unwrap()
}

/// Insert one raster photo per path.
pub fn import_paths(catalog: &mut Catalog, paths: &[&str]) -> Vec<PhotoId> {
    let photos: Vec<NewPhoto> = paths.iter().map(|path| raster(path)).collect();
    import(catalog, &photos)
}

/// The file names a query returns, in order.
pub fn names(catalog: &Catalog, query: &PhotoQuery) -> Vec<String> {
    catalog
        .query(query)
        .unwrap()
        .into_iter()
        .map(|photo| photo.file_name)
        .collect()
}

/// The ids a query returns, in order.
pub fn ids(photos: &[PhotoSummary]) -> Vec<PhotoId> {
    photos.iter().map(|photo| photo.id).collect()
}
