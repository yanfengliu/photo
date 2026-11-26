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
//! policy.
//!
//! "The same folder" is decided by the folder's identity on disk, not by
//! how its path is spelled: a folder the catalog reached through a symbolic
//! link, a junction or another spelling is the same folder. Folders that
//! are not there to ask (a drive that is away) are compared by their paths,
//! resolved as far as they exist and ignoring letter case. File names are
//! compared ignoring letter case, as on the file systems photos usually
//! live on.
//!
//! The guard looks when the export starts, and a path says nothing about a
//! folder that is away then: one the catalog reached through a link is not
//! recognised when its drive comes back before the file is written. So the
//! guard is not the last word. Each photo's export is also told which
//! originals of the catalog have the name its file is to have (see
//! `Engine::start_export`), and the export crate compares the file it is
//! about to replace with them by identity, at that moment. It refuses with
//! the sentence the guard uses.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use photo_catalog::{Catalog, CatalogError, PhotoQuery, Source};
use same_file::Handle;

/// A folder the export writes into, and the names of the catalog's
/// originals in it, in lower case.
#[derive(Debug)]
struct Destination {
    folder: PathBuf,
    key: PathBuf,
    originals: HashSet<String>,
}

/// The folders no export may go into, and the originals in every folder
/// the export writes to.
#[derive(Debug, Default)]
pub(crate) struct Guard {
    own: Vec<PathBuf>,
    destinations: Vec<Destination>,
}

impl Guard {
    /// The guard for an export into `destinations`. `own` are the folders
    /// the catalog keeps its files in. Every catalog folder that holds
    /// photos is asked for its identity once, which is one file-system
    /// call each.
    pub(crate) fn new(
        catalog: &Catalog,
        own: &[&Path],
        destinations: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, CatalogError> {
        let mut wanted: Vec<(Destination, Option<Handle>)> = destinations
            .into_iter()
            .map(|folder| {
                let handle = Handle::from_path(&folder).ok();
                let destination = Destination {
                    key: folder_key(&folder),
                    folder,
                    originals: HashSet::new(),
                };
                (destination, handle)
            })
            .collect();
        let folders = if wanted.is_empty() {
            Vec::new()
        } else {
            catalog.folders()?
        };
        for folder in folders.iter().filter(|folder| folder.photo_count > 0) {
            let handle = Handle::from_path(&folder.path).ok();
            let mut key = None;
            let mut names: Option<Vec<String>> = None;
            for (destination, there) in &mut wanted {
                let same = match (&handle, &*there) {
                    (Some(here), Some(there)) => here == there,
                    _ => *key.get_or_insert_with(|| folder_key(&folder.path)) == destination.key,
                };
                if !same {
                    continue;
                }
                if names.is_none() {
                    let photos = catalog.query(&PhotoQuery::from(Source::Folder {
                        id: folder.id,
                        include_subfolders: false,
                    }))?;
                    names = Some(
                        photos
                            .iter()
                            .map(|photo| photo.file_name.to_lowercase())
                            .collect(),
                    );
                }
                destination
                    .originals
                    .extend(names.iter().flatten().cloned());
            }
        }
        Ok(Self {
            own: own.iter().map(|folder| folder_key(folder)).collect(),
            destinations: wanted
                .into_iter()
                .map(|(destination, _)| destination)
                .collect(),
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
        let taken = self.destinations.iter().any(|destination| {
            destination.originals.contains(&name)
                && (destination.key == key
                    || same_file::is_same_file(folder, &destination.folder).unwrap_or(false))
        });
        taken.then(|| is_an_original(target))
    }
}

/// Why nothing is written over `original`, the original of a photo.
pub(super) fn is_an_original(original: &Path) -> String {
    format!(
        "{} is the original of a photo in the catalog, which an export never writes over",
        original.display()
    )
}

/// For each export, given as the file it is planned to write and the
/// photo's own file: the originals of the catalog it must not replace.
/// Those are the files the catalog knows under the planned file's name,
/// whatever the letter case and in whatever folder, other than the photo's
/// own. Read from the catalog in one query; no file is touched.
pub(super) fn originals_in_the_way(
    catalog: &Catalog,
    exports: &[(&Path, &Path)],
) -> Result<Vec<Vec<PathBuf>>, CatalogError> {
    let names: Vec<String> = exports
        .iter()
        .map(|(planned, _)| {
            let name = planned.file_name().unwrap_or_default();
            name.to_string_lossy().into_owned()
        })
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let named = catalog.files_named(&names)?;
    Ok(named
        .into_iter()
        .zip(exports)
        .map(|(originals, (_, own))| {
            originals
                .into_iter()
                .filter(|original| original != own)
                .collect()
        })
        .collect())
}

/// `path` with symbolic links resolved as far as it exists (the nearest
/// existing folder made canonical, the rest added back), in lower case,
/// for comparing folders that are not both there to be asked who they are.
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
    use photo_catalog::NewPhoto;
    use photo_core::meta::FileKind;

    use super::*;

    /// A catalog in memory holding one photo, recorded at `path`.
    fn catalog_with(path: &Path) -> Catalog {
        let mut catalog = Catalog::open_in_memory().unwrap();
        let run = catalog.begin_import().unwrap();
        catalog
            .insert_photos(run, &[NewPhoto::new(path, FileKind::Raster)])
            .unwrap();
        catalog
    }

    #[test]
    fn an_export_is_told_of_every_original_that_has_its_files_name_but_the_photos_own() {
        let mut catalog = Catalog::open_in_memory().unwrap();
        let run = catalog.begin_import().unwrap();
        let photos: Vec<NewPhoto> = [
            "/card/IMG_1.JPG",
            "/shots/img_1.jpg",
            "/shots/IMG_2.jpg",
            "/raw/IMG_1.CR2",
        ]
        .iter()
        .map(|path| NewPhoto::new(path, FileKind::Raster))
        .collect();
        catalog.insert_photos(run, &photos).unwrap();
        let exports = [
            // A RAW photo exported as a JPEG into another folder.
            (Path::new("/out/IMG_1.jpg"), Path::new("/raw/IMG_1.CR2")),
            // A JPEG exported under its own name.
            (Path::new("/out/img_1.jpg"), Path::new("/shots/img_1.jpg")),
            // A name the catalog does not hold.
            (Path::new("/out/IMG_3.jpg"), Path::new("/shots/IMG_2.jpg")),
        ];
        let told = originals_in_the_way(&catalog, &exports).unwrap();
        let paths = |paths: &[&str]| -> Vec<PathBuf> { paths.iter().map(PathBuf::from).collect() };
        assert_eq!(
            told,
            [
                paths(&["/card/IMG_1.JPG", "/shots/img_1.jpg"]),
                paths(&["/card/IMG_1.JPG"]),
                paths(&[]),
            ],
            "whatever the folder and the letter case, and never the photo's own file"
        );
    }

    #[test]
    fn folders_compare_through_letter_case_and_folders_not_made_yet() {
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

    #[cfg(unix)]
    #[test]
    fn an_original_recorded_through_a_linked_folder_is_found_in_the_real_one() {
        let dir = tempfile::tempdir().unwrap();
        let actual = dir.path().join("actual");
        std::fs::create_dir(&actual).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&actual, &link).unwrap();
        std::fs::write(actual.join("IMG_1.JPG"), b"the camera's own").unwrap();
        let catalog = catalog_with(&link.join("IMG_1.JPG"));

        let guard = Guard::new(&catalog, &[], [actual.clone()]).unwrap();
        assert!(guard.refusal(&actual.join("IMG_1.JPG")).is_some());
        assert!(
            guard.refusal(&actual.join("img_1.jpg")).is_some(),
            "whatever the case"
        );
        assert!(guard.refusal(&link.join("IMG_1.jpg")).is_some());
        assert!(guard.refusal(&actual.join("IMG_2.jpg")).is_none());
        assert!(guard.refusal(&dir.path().join("IMG_1.JPG")).is_none());

        // The other way round: recorded in the real folder, exported
        // through the link.
        let catalog = catalog_with(&actual.join("IMG_1.JPG"));
        let guard = Guard::new(&catalog, &[], [link.clone()]).unwrap();
        assert!(guard.refusal(&link.join("IMG_1.jpg")).is_some());
        assert!(guard.refusal(&actual.join("IMG_1.jpg")).is_some());
    }

    #[test]
    fn a_folder_that_is_away_is_compared_by_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let away = dir.path().join("Card").join("DCIM");
        let catalog = catalog_with(&away.join("IMG_1.JPG"));
        let guard = Guard::new(&catalog, &[], [away.clone()]).unwrap();
        assert!(guard.refusal(&away.join("img_1.jpg")).is_some());
        assert!(guard.refusal(&away.join("IMG_9.jpg")).is_none());
    }

    #[test]
    fn the_catalogs_own_folders_are_refused_with_what_is_inside_them() {
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("catalog");
        std::fs::create_dir(&own).unwrap();
        let guard = Guard {
            own: vec![folder_key(&own)],
            destinations: Vec::new(),
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
