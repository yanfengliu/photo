//! Adding photos to the catalog.

use std::collections::HashMap;

use photo_core::meta::{ImportId, PhotoId, PhotoMetadata};
use rusqlite::{Transaction, named_params};

use super::{NewPhoto, master_at_path};
use crate::catalog::Catalog;
use crate::codec::{flag_to_sql, kind_to_sql, label_to_sql, rating_to_sql, to_i64};
use crate::develop::encode_settings;
use crate::error::{CatalogError, Result};
use crate::folders::ensure_folder;
use crate::imports::require_import;
use crate::keywords::ensure_keyword;
use crate::paths;
use crate::text::natural_key;

const INSERT: &str = "INSERT INTO photos (
        folder_id, import_id, path, file_name, file_name_key, kind, format, file_size, file_mtime, added_at,
        width, height, capture_time, camera_make, camera_model, camera, lens,
        focal_length, focal_length_35, aperture, shutter, iso, exposure_bias, flash,
        gps_latitude, gps_longitude, gps_altitude, artist, file_copyright,
        rating, flag, label, title, caption, creator, copyright, develop, has_edits
    ) VALUES (
        :folder_id, :import_id, :path, :file_name, :file_name_key, :kind, :format, :file_size, :file_mtime, :added_at,
        :width, :height, :capture_time, :camera_make, :camera_model, :camera, :lens,
        :focal_length, :focal_length_35, :aperture, :shutter, :iso, :exposure_bias, :flash,
        :gps_latitude, :gps_longitude, :gps_altitude, :artist, :file_copyright,
        :rating, :flag, :label, :title, :caption, :creator, :copyright, :develop, :has_edits
    )";

impl Catalog {
    /// Add one photo as part of the import run `import`.
    ///
    /// See [`Catalog::insert_photos`] for the rules.
    pub fn insert_photo(&mut self, import: ImportId, photo: &NewPhoto) -> Result<PhotoId> {
        let ids = self.insert_photos(import, std::slice::from_ref(photo))?;
        Ok(ids[0])
    }

    /// Add a batch of photos as part of the import run `import`, returning
    /// their ids in the same order.
    ///
    /// The batch is one transaction: if any photo is rejected, none is added
    /// and neither are the folders or keywords the batch would have created.
    /// Each photo's folder is the directory that holds its file and is
    /// created when first needed. A path that is already in the catalog (or
    /// appears twice in the batch) fails with
    /// [`CatalogError::DuplicatePath`]; a path that is not UTF-8 with
    /// [`CatalogError::NonUtf8Path`].
    ///
    /// Paths are stored as given, except that in a Windows-shaped path
    /// (drive letter or `\\server` prefix) every `/` becomes `\`, so that
    /// both spellings of one file are one photo.
    pub fn insert_photos(&mut self, import: ImportId, photos: &[NewPhoto]) -> Result<Vec<PhotoId>> {
        self.write(|tx, now| {
            require_import(tx, import)?;
            let mut folders: HashMap<String, i64> = HashMap::new();
            let mut ids = Vec::with_capacity(photos.len());
            for photo in photos {
                let path = paths::text(&photo.path)?;
                let split = paths::split(&path)?;
                if master_at_path(tx, &path)?.is_some() {
                    return Err(CatalogError::DuplicatePath(path.to_string()));
                }
                let folder = match folders.get(split.dir) {
                    Some(folder) => *folder,
                    None => {
                        let folder = ensure_folder(tx, split.dir)?;
                        folders.insert(split.dir.to_string(), folder);
                        folder
                    }
                };
                let id = insert_row(tx, photo, &path, split.file_name, folder, import, now)?;
                link_keywords(tx, id, &photo.keywords)?;
                ids.push(PhotoId(id));
            }
            Ok(ids)
        })
    }
}

fn insert_row(
    tx: &Transaction<'_>,
    photo: &NewPhoto,
    path: &str,
    file_name: &str,
    folder: i64,
    import: ImportId,
    now: i64,
) -> Result<i64> {
    // Destructured without `..` on purpose: a field added to PhotoMetadata
    // must be given a column here before this crate compiles again.
    let PhotoMetadata {
        width,
        height,
        capture_time,
        camera_make,
        camera_model,
        lens,
        focal_length_mm,
        focal_length_35mm,
        aperture,
        shutter_seconds,
        iso,
        exposure_bias_ev,
        flash_fired,
        gps,
        artist,
        copyright: file_copyright,
    } = &photo.metadata;
    let develop = photo.develop.as_ref().map(encode_settings).transpose()?;
    let has_edits = photo
        .develop
        .as_ref()
        .is_some_and(|settings| !settings.is_default_for(photo.kind));

    tx.prepare_cached(INSERT)?.execute(named_params! {
        ":folder_id": folder,
        ":import_id": import.0,
        ":path": path,
        ":file_name": file_name,
        ":file_name_key": natural_key(file_name),
        ":kind": kind_to_sql(photo.kind),
        ":format": photo.format.to_lowercase(),
        ":file_size": to_i64(photo.file_size),
        ":file_mtime": photo.file_mtime,
        ":added_at": now,
        ":width": width,
        ":height": height,
        ":capture_time": capture_time.map(|time| time.to_string()),
        ":camera_make": camera_make,
        ":camera_model": camera_model,
        ":camera": photo.metadata.camera_name(),
        ":lens": lens,
        ":focal_length": focal_length_mm,
        ":focal_length_35": focal_length_35mm,
        ":aperture": aperture,
        ":shutter": shutter_seconds,
        ":iso": iso,
        ":exposure_bias": exposure_bias_ev,
        ":flash": flash_fired,
        ":gps_latitude": gps.map(|position| position.latitude),
        ":gps_longitude": gps.map(|position| position.longitude),
        ":gps_altitude": gps.and_then(|position| position.altitude_m),
        ":artist": artist,
        ":file_copyright": file_copyright,
        ":rating": rating_to_sql(photo.rating),
        ":flag": flag_to_sql(photo.flag),
        ":label": label_to_sql(photo.label),
        ":title": photo.title,
        ":caption": photo.caption,
        ":creator": photo.creator,
        ":copyright": photo.copyright,
        ":develop": develop,
        ":has_edits": has_edits,
    })?;
    Ok(tx.last_insert_rowid())
}

fn link_keywords(tx: &Transaction<'_>, photo: i64, names: &[String]) -> Result<()> {
    for name in names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
    {
        let keyword = ensure_keyword(tx, name)?;
        tx.prepare_cached(
            "INSERT OR IGNORE INTO photo_keywords (photo_id, keyword_id) VALUES (?1, ?2)",
        )?
        .execute([photo, keyword])?;
    }
    Ok(())
}
