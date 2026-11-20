//! What the export crate cannot know: which files are the originals of the
//! catalog's photos, and which folders are the catalog's own.
//!
//! The export crate never replaces the original it renders, but under the
//! overwrite policy it would replace any other file of the planned name:
//! exporting `IMG_1.CR2` beside itself as `IMG_1.jpg` would replace the
//! camera's own JPEG of a RAW+JPEG pair. And a numbered name could land on
//! the path of a photo whose file is missing, which would then be taken
//! for that photo. So an export is refused when its file would have the
//! name of an original of the catalog in the same folder, whatever the
//! policy. Names are compared ignoring letter case, as on the file systems
//! photos usually live on (Windows and macOS), and folders after following
//! symbolic links (among the catalog's folders of the destination's name).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use photo_catalog::{Catalog, CatalogError, PhotoQuery, Source};

/// The folders no export may go into, and the names of the originals in
/// every folder the export writes to.
#[derive(Debug, Default)]
pub(crate) struct Guard {
    own: Vec<PathBuf>,
    originals: HashMap<PathBuf, HashSet<String>>,
}

impl Guard {
    /// The guard for an export into `destinations`. `own` are the folders
    /// the catalog keeps its files in.
    pub(crate) fn new(
        catalog: &Catalog,
        own: &[&Path],
        destinations: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, CatalogError> {
        let wanted: HashSet<PathBuf> = destinations
            .into_iter()
            .map(|folder| folder_key(&folder))
            .collect();
        // Only catalog folders named like a destination can be one; the
        // others are not looked up on disk, which keeps a catalog of many
        // folders quick to check.
        let leaves: HashSet<String> = wanted.iter().filter_map(|key| leaf(key)).collect();
        let mut originals: HashMap<PathBuf, HashSet<String>> = HashMap::new();
        for folder in catalog.folders()? {
            // A root (a drive) has no name, and is always looked at.
            let named = leaf(&folder.path).is_none_or(|name| leaves.contains(&name));
            if folder.photo_count == 0 || !named {
                continue;
            }
            let key = folder_key(&folder.path);
            if !wanted.contains(&key) {
                continue;
            }
            let photos = catalog.query(&PhotoQuery::from(Source::Folder {
                id: folder.id,
                include_subfolders: false,
            }))?;
            originals
                .entry(key)
                .or_default()
                .extend(photos.iter().map(|photo| photo.file_name.to_lowercase()));
        }
        Ok(Self {
            own: own.iter().map(|folder| folder_key(folder)).collect(),
            originals,
        })
    }

    /// Why nothing may be written at `target`, if it may not.
    pub(crate) fn refusal(&self, target: &Path) -> Option<String> {
        let folder = target.parent().unwrap_or(Path::new(""));
        let key = folder_key(folder);
        if self.own.iter().any(|own| key.starts_with(own)) {
            return Some(format!(
                "{} is inside the catalog's own folders, where an export never goes",
                target.display()
            ));
        }
        let name = target.file_name()?.to_string_lossy().to_lowercase();
        let taken = self
            .originals
            .get(&key)
            .is_some_and(|names| names.contains(&name));
        taken.then(|| {
            format!(
                "{} is the original of a photo in the catalog, which an export never writes over",
                target.display()
            )
        })
    }
}

/// The last part of a folder's path, in lower case.
fn leaf(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
}

/// `path` with symbolic links resolved as far as it exists (the nearest
/// existing folder made canonical, the rest added back), in lower case,
/// for comparing folders.
fn folder_key(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut rest = Vec::new();
    let resolved = loop {
        if let Ok(canonical) = existing.canonicalize() {
            break rest
                .iter()
                .rev()
                .fold(canonical, |path, part| path.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => break path.to_path_buf(),
        }
    };
    PathBuf::from(resolved.to_string_lossy().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_compare_through_links_and_letter_case_and_folders_not_made_yet() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("Shots");
        std::fs::create_dir(&real).unwrap();
        assert_eq!(
            folder_key(&real),
            folder_key(&dir.path().join("Shots").join("."))
        );
        let upper = PathBuf::from(real.to_string_lossy().to_uppercase());
        assert_eq!(folder_key(&upper), folder_key(&real));
        let unborn = real.join("web").join("large");
        assert!(folder_key(&unborn).starts_with(folder_key(&real)));
    }

    #[test]
    fn the_catalogs_own_folders_are_refused_with_what_is_inside_them() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("catalog");
        std::fs::create_dir(&own).unwrap();
        let guard = Guard {
            own: vec![folder_key(&own)],
            originals: HashMap::new(),
        };
        assert!(guard.refusal(&own.join("a.jpg")).is_some());
        assert!(guard.refusal(&own.join("new").join("a.jpg")).is_some());
        assert!(
            guard
                .refusal(&dir.path().join("catalog2").join("a.jpg"))
                .is_none()
        );
        assert!(guard.refusal(&dir.path().join("a.jpg")).is_none());
    }
}
