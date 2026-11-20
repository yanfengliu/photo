//! Folders: the directories that hold catalogued files.

use std::path::{Path, PathBuf};

use photo_core::meta::FolderId;
use rusqlite::{Connection, OptionalExtension};

use crate::catalog::Catalog;
use crate::codec::to_u64;
use crate::error::{CatalogError, Entity, Result};
use crate::paths;

/// A folder known to the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderInfo {
    /// The folder's id.
    pub id: FolderId,
    /// The directory, without a trailing separator (a root keeps its one).
    pub path: PathBuf,
    /// Photos whose file is directly in this folder, virtual copies
    /// included. Subfolders are separate entries.
    pub photo_count: u64,
}

impl Catalog {
    /// Every folder with its photo count, as a flat list ordered by path.
    /// The hierarchy is in the paths.
    pub fn folders(&self) -> Result<Vec<FolderInfo>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.id, f.path, (SELECT COUNT(*) FROM photos p WHERE p.folder_id = f.id)
             FROM folders f ORDER BY f.path",
        )?;
        let folders = stmt
            .query_map([], |row| {
                Ok(FolderInfo {
                    id: FolderId(row.get(0)?),
                    path: PathBuf::from(row.get::<_, String>(1)?),
                    photo_count: to_u64(row.get(2)?),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(folders)
    }

    /// The id of the folder for the directory `path`, adding the folder if
    /// the catalog does not know it yet. Inserting photos does this for
    /// their own directories; call it to list a directory that holds no
    /// photos itself, such as the root of an import.
    pub fn ensure_folder(&mut self, path: &Path) -> Result<FolderId> {
        let path = paths::text(path)?;
        let dir = paths::directory(&path);
        if dir.is_empty() {
            return Err(CatalogError::InvalidPath(String::new()));
        }
        self.write(|tx, _| ensure_folder(tx, dir).map(FolderId))
    }

    /// Forget an empty folder. Fails with [`CatalogError::FolderNotEmpty`]
    /// while photos are still catalogued directly in it.
    pub fn remove_folder(&mut self, id: FolderId) -> Result<()> {
        self.write(|tx, _| {
            folder_path(tx, id)?;
            if tx
                .prepare_cached("SELECT 1 FROM photos WHERE folder_id = ?1")?
                .exists([id.0])?
            {
                return Err(CatalogError::FolderNotEmpty(id));
            }
            tx.execute("DELETE FROM folders WHERE id = ?1", [id.0])?;
            Ok(())
        })
    }
}

/// The id of the folder stored under exactly `dir`, created if missing.
pub(crate) fn ensure_folder(conn: &Connection, dir: &str) -> Result<i64> {
    conn.prepare_cached("INSERT INTO folders (path) VALUES (?1) ON CONFLICT(path) DO NOTHING")?
        .execute([dir])?;
    let id = conn
        .prepare_cached("SELECT id FROM folders WHERE path = ?1")?
        .query_row([dir], |row| row.get(0))?;
    Ok(id)
}

/// The stored path of a folder. Fails if the folder does not exist.
pub(crate) fn folder_path(conn: &Connection, id: FolderId) -> Result<String> {
    conn.prepare_cached("SELECT path FROM folders WHERE id = ?1")?
        .query_row([id.0], |row| row.get(0))
        .optional()?
        .ok_or_else(|| CatalogError::not_found(Entity::Folder, id.0))
}
