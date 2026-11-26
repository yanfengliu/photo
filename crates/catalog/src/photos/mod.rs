//! Photo rows: what goes in at import and what comes back out.

mod attributes;
mod copies;
mod insert;

use std::collections::HashMap;
use std::path::PathBuf;

use photo_core::develop::DevelopSettings;
use photo_core::meta::{
    ColorLabel, FileKind, Flag, FolderId, GpsPosition, ImportId, PhotoId, PhotoMetadata, Rating,
};
use photo_core::time::DateTime;
use rusqlite::{Connection, OptionalExtension, Row, params};

pub use attributes::TextField;
pub(crate) use attributes::sync_copies;

use crate::catalog::Catalog;
use crate::codec::{
    date_from_sql, flag_from_sql, kind_from_sql, label_from_sql, rating_from_sql, to_i64, to_u32,
    to_u64, unix_seconds,
};
use crate::error::{CatalogError, Entity, Result};
use crate::paths;
use crate::sql::id_list;
use crate::text;

/// A photo about to be added to the catalog.
///
/// Only the path and kind are required; everything else has a neutral
/// starting value and can be filled in from the file's metadata, an XMP
/// sidecar or an import preset.
#[derive(Clone, Debug, PartialEq)]
pub struct NewPhoto {
    /// Where the file is. Must be valid UTF-8 and name a file inside a
    /// directory; that directory becomes the photo's folder. Stored as
    /// given, except that a Windows-shaped path is stored with `\` for
    /// every separator.
    pub path: PathBuf,
    /// Whether the file is a finished rendering or sensor data. Decides the
    /// default develop settings and so what counts as an edit.
    pub kind: FileKind,
    /// File extension without the dot. Stored in lower case.
    pub format: String,
    /// Size of the file in bytes.
    pub file_size: u64,
    /// Modification time of the file, in Unix seconds.
    pub file_mtime: i64,
    /// What was read from the file. Stored field for field.
    pub metadata: PhotoMetadata,
    /// Initial star rating.
    pub rating: Rating,
    /// Initial pick / reject flag.
    pub flag: Flag,
    /// Initial colour label.
    pub label: ColorLabel,
    /// Initial title; empty for none.
    pub title: String,
    /// Initial caption; empty for none.
    pub caption: String,
    /// Initial creator; empty for none. Not taken from `metadata.artist`
    /// unless the caller copies it here.
    pub creator: String,
    /// Initial copyright notice; empty for none. Not taken from
    /// `metadata.copyright` unless the caller copies it here.
    pub copyright: String,
    /// Keyword names, each resolved as
    /// [`Catalog::ensure_keyword`](crate::Catalog::ensure_keyword) does: the
    /// only keyword of that name (ignoring ASCII case), wherever it sits, or
    /// else the top-level keyword of that name, created if needed. Blank
    /// names are skipped.
    pub keywords: Vec<String>,
    /// Settings to start from, or `None` for the default of the kind.
    pub develop: Option<DevelopSettings>,
}

impl NewPhoto {
    /// A photo with the given path and kind, its format taken from the
    /// path's extension and everything else empty.
    pub fn new(path: impl Into<PathBuf>, kind: FileKind) -> Self {
        let path = path.into();
        let format = path.to_str().map(paths::extension).unwrap_or_default();
        Self {
            path,
            kind,
            format,
            file_size: 0,
            file_mtime: 0,
            metadata: PhotoMetadata::default(),
            rating: Rating::UNRATED,
            flag: Flag::Unflagged,
            label: ColorLabel::None,
            title: String::new(),
            caption: String::new(),
            creator: String::new(),
            copyright: String::new(),
            keywords: Vec::new(),
            develop: None,
        }
    }
}

/// Everything the catalog knows about one photo, apart from its develop
/// settings, keywords and collections, which have their own accessors.
#[derive(Clone, Debug, PartialEq)]
pub struct PhotoRecord {
    /// The photo's id. Never reused, even after the photo is removed.
    pub id: PhotoId,
    /// For a virtual copy, the photo that owns the file.
    pub master_id: Option<PhotoId>,
    /// For a virtual copy, its name ("Copy 1").
    pub copy_name: Option<String>,
    /// The folder that holds the file.
    pub folder_id: FolderId,
    /// The import run that added the photo (for a virtual copy, the run
    /// that added its master).
    pub import_id: ImportId,
    /// Where the file is. A virtual copy has the path of its master.
    pub path: PathBuf,
    /// The last component of `path`.
    pub file_name: String,
    /// Whether the file is a finished rendering or sensor data.
    pub kind: FileKind,
    /// File extension without the dot, in lower case.
    pub format: String,
    /// Size of the file in bytes, as given at import.
    pub file_size: u64,
    /// Modification time of the file at import, in Unix seconds.
    pub file_mtime: i64,
    /// True once the file was reported gone; cleared by relinking.
    pub missing: bool,
    /// What was read from the file at import, exactly as it was given.
    pub metadata: PhotoMetadata,
    /// Star rating.
    pub rating: Rating,
    /// Pick / reject flag.
    pub flag: Flag,
    /// Colour label.
    pub label: ColorLabel,
    /// Title; empty for none.
    pub title: String,
    /// Caption; empty for none.
    pub caption: String,
    /// Creator; empty for none.
    pub creator: String,
    /// Copyright notice; empty for none.
    pub copyright: String,
    /// With the id, identifies a rendering. A new one is drawn from a single
    /// counter for the whole catalog on every change to the develop settings
    /// (an undo, redo or history jump included) and when the photo is
    /// restored, so a photo's version only ever grows and never names two
    /// different looks.
    pub develop_version: u64,
    /// True when the develop settings differ from the default for the kind.
    pub has_edits: bool,
    /// When the photo was added, in Unix seconds.
    pub added_at: i64,
    /// When the develop settings last changed, in Unix seconds; `None` if
    /// they never have.
    pub edited_at: Option<i64>,
}

fn record_from_row(row: &Row<'_>) -> rusqlite::Result<PhotoRecord> {
    let capture_column = row.as_ref().column_index("capture_time")?;
    let latitude: Option<f64> = row.get("gps_latitude")?;
    let longitude: Option<f64> = row.get("gps_longitude")?;
    let gps = match (latitude, longitude) {
        (Some(latitude), Some(longitude)) => Some(GpsPosition {
            latitude,
            longitude,
            altitude_m: row.get("gps_altitude")?,
        }),
        _ => None,
    };
    let metadata = PhotoMetadata {
        width: to_u32(row.get("width")?),
        height: to_u32(row.get("height")?),
        capture_time: date_from_sql(capture_column, row.get(capture_column)?)?,
        camera_make: row.get("camera_make")?,
        camera_model: row.get("camera_model")?,
        lens: row.get("lens")?,
        focal_length_mm: row.get("focal_length")?,
        focal_length_35mm: row.get("focal_length_35")?,
        aperture: row.get("aperture")?,
        shutter_seconds: row.get("shutter")?,
        iso: row.get::<_, Option<i64>>("iso")?.map(to_u32),
        exposure_bias_ev: row.get("exposure_bias")?,
        flash_fired: row.get("flash")?,
        gps,
        artist: row.get("artist")?,
        copyright: row.get("file_copyright")?,
    };
    Ok(PhotoRecord {
        id: PhotoId(row.get("id")?),
        master_id: row.get::<_, Option<i64>>("master_id")?.map(PhotoId),
        copy_name: row.get("copy_name")?,
        folder_id: FolderId(row.get("folder_id")?),
        import_id: ImportId(row.get("import_id")?),
        path: PathBuf::from(row.get::<_, String>("path")?),
        file_name: row.get("file_name")?,
        kind: kind_from_sql(row.get("kind")?),
        format: row.get("format")?,
        file_size: to_u64(row.get("file_size")?),
        file_mtime: row.get("file_mtime")?,
        missing: row.get("missing")?,
        metadata,
        rating: rating_from_sql(row.get("rating")?),
        flag: flag_from_sql(row.get("flag")?),
        label: label_from_sql(row.get("label")?),
        title: row.get("title")?,
        caption: row.get("caption")?,
        creator: row.get("creator")?,
        copyright: row.get("copyright")?,
        develop_version: to_u64(row.get("develop_version")?),
        has_edits: row.get("has_edits")?,
        added_at: unix_seconds(row.get("added_at")?),
        edited_at: row.get::<_, Option<i64>>("edited_at")?.map(unix_seconds),
    })
}

impl Catalog {
    /// The record of one photo.
    pub fn photo(&self, id: PhotoId) -> Result<PhotoRecord> {
        self.conn
            .prepare_cached("SELECT * FROM photos WHERE id = ?1")?
            .query_row([id.0], record_from_row)
            .optional()?
            .ok_or_else(|| CatalogError::not_found(Entity::Photo, id.0))
    }

    /// The records of several photos in one query, one per id in the order
    /// given. Ids that are not in the catalog are skipped.
    pub fn photos(&self, ids: &[PhotoId]) -> Result<Vec<PhotoRecord>> {
        let wanted = id_list(ids.iter().map(|id| id.0));
        let mut stmt = self.conn.prepare_cached(
            "SELECT p.* FROM json_each(?1) AS wanted JOIN photos p ON p.id = wanted.value
             ORDER BY wanted.key",
        )?;
        let records = stmt
            .query_map([wanted], record_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(records)
    }

    /// The photo that owns the file at exactly this path, if it is in the
    /// catalog. Virtual copies are never returned. The comparison is exact
    /// (letter case included), except that either separator may be used in
    /// a Windows-shaped path.
    pub fn find_by_path(&self, path: impl AsRef<std::path::Path>) -> Result<Option<PhotoId>> {
        let path = paths::text(path.as_ref())?;
        Ok(master_at_path(&self.conn, &path)?.map(PhotoId))
    }

    /// Photos that are probably the same picture as a file about to be
    /// imported: same file name, same size and same capture time (two files
    /// with no capture time count as the same time). Virtual copies are not
    /// listed. Ordered by id.
    pub fn find_suspected_duplicates(
        &self,
        file_name: &str,
        file_size: u64,
        capture_time: Option<DateTime>,
    ) -> Result<Vec<PhotoId>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id FROM photos
             WHERE file_name = ?1 AND file_size = ?2 AND capture_time IS ?3 AND master_id IS NULL
             ORDER BY id",
        )?;
        let capture_time = capture_time.map(|time| time.to_string());
        let ids = stmt
            .query_map(params![file_name, to_i64(file_size), capture_time], |row| {
                row.get(0).map(PhotoId)
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// For each of `names`, in order, the paths of the catalogued files
    /// that have that name, whatever the letter case: `IMG_1.JPG` and
    /// `img_1.jpg` are one name on the file systems photos usually live
    /// on. Virtual copies have no file of their own and are not listed.
    /// The files of one name are in the order they were added. One query
    /// however many names; it reads the name of every photo.
    pub fn files_named(&self, names: &[&str]) -> Result<Vec<Vec<PathBuf>>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let folded: Vec<String> = names.iter().map(|name| text::fold(name)).collect();
        let wanted = serde_json::Value::from(folded.clone()).to_string();
        let mut stmt = self.conn.prepare_cached(
            "SELECT file_name, path FROM photos
             WHERE master_id IS NULL
               AND photo_fold(file_name) IN (SELECT value FROM json_each(?1))
             ORDER BY id",
        )?;
        let rows = stmt.query_map([wanted], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut found: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for row in rows {
            let (name, path) = row?;
            found
                .entry(text::fold(&name))
                .or_default()
                .push(PathBuf::from(path));
        }
        Ok(folded
            .iter()
            .map(|name| found.get(name).cloned().unwrap_or_default())
            .collect())
    }
}

/// The id of the master photo stored at exactly `path`.
pub(crate) fn master_at_path(conn: &Connection, path: &str) -> Result<Option<i64>> {
    let id = conn
        .prepare_cached("SELECT id FROM photos WHERE path = ?1 AND master_id IS NULL")?
        .query_row([path], |row| row.get(0))
        .optional()?;
    Ok(id)
}

/// The id of the photo that owns `photo`'s file: itself, or its master if it
/// is a virtual copy. Fails if `photo` is not in the catalog.
pub(crate) fn master_of(conn: &Connection, photo: PhotoId) -> Result<i64> {
    conn.prepare_cached("SELECT COALESCE(master_id, id) FROM photos WHERE id = ?1")?
        .query_row([photo.0], |row| row.get(0))
        .optional()?
        .ok_or_else(|| CatalogError::not_found(Entity::Photo, photo.0))
}

/// Fail with a typed error unless `photo` is in the catalog.
pub(crate) fn require_photo(conn: &Connection, photo: PhotoId) -> Result<()> {
    master_of(conn, photo).map(|_| ())
}

/// Fail with a typed error naming the first of `photos` that is not in the
/// catalog. One query, however many ids.
pub(crate) fn require_photos(conn: &Connection, photos: &[PhotoId]) -> Result<()> {
    let unknown: Option<i64> = conn
        .prepare_cached(
            "SELECT value FROM json_each(?1) WHERE value NOT IN (SELECT id FROM photos) LIMIT 1",
        )?
        .query_row([id_list(photos.iter().map(|photo| photo.0))], |row| {
            row.get(0)
        })
        .optional()?;
    match unknown {
        Some(id) => Err(CatalogError::not_found(Entity::Photo, id)),
        None => Ok(()),
    }
}

/// The kind of file behind `photo`. Fails if `photo` is not in the catalog.
pub(crate) fn kind_of(conn: &Connection, photo: PhotoId) -> Result<FileKind> {
    conn.prepare_cached("SELECT kind FROM photos WHERE id = ?1")?
        .query_row([photo.0], |row| row.get(0).map(kind_from_sql))
        .optional()?
        .ok_or_else(|| CatalogError::not_found(Entity::Photo, photo.0))
}
