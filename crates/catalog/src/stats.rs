//! Row counts, for diagnostics and for checking that nothing is left behind.

use crate::catalog::Catalog;
use crate::codec::to_u64;
use crate::error::Result;

/// How many rows each part of the catalog holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CatalogStats {
    /// Photos, virtual copies included.
    pub photos: u64,
    /// Folders, including any that hold no photos at the moment.
    pub folders: u64,
    /// Collections of every kind, the built-in quick collection included.
    pub collections: u64,
    /// Photo-in-collection memberships.
    pub collection_photos: u64,
    /// Keywords, whether or not any photo carries them.
    pub keywords: u64,
    /// Keyword-on-photo assignments.
    pub photo_keywords: u64,
    /// Develop history steps, over all photos.
    pub history: u64,
    /// Snapshots, over all photos.
    pub snapshots: u64,
    /// Develop presets.
    pub presets: u64,
    /// Import runs.
    pub imports: u64,
}

impl Catalog {
    /// Count the rows of every table. After any deletion these numbers show
    /// directly that no dependent row was orphaned.
    pub fn stats(&self) -> Result<CatalogStats> {
        let stats = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM photos), (SELECT COUNT(*) FROM folders),
                    (SELECT COUNT(*) FROM collections), (SELECT COUNT(*) FROM collection_photos),
                    (SELECT COUNT(*) FROM keywords), (SELECT COUNT(*) FROM photo_keywords),
                    (SELECT COUNT(*) FROM history), (SELECT COUNT(*) FROM snapshots),
                    (SELECT COUNT(*) FROM presets), (SELECT COUNT(*) FROM imports)",
            [],
            |row| {
                Ok(CatalogStats {
                    photos: to_u64(row.get(0)?),
                    folders: to_u64(row.get(1)?),
                    collections: to_u64(row.get(2)?),
                    collection_photos: to_u64(row.get(3)?),
                    keywords: to_u64(row.get(4)?),
                    photo_keywords: to_u64(row.get(5)?),
                    history: to_u64(row.get(6)?),
                    snapshots: to_u64(row.get(7)?),
                    presets: to_u64(row.get(8)?),
                    imports: to_u64(row.get(9)?),
                })
            },
        )?;
        Ok(stats)
    }
}
