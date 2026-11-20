//! Where Photo keeps its data: the single owner of storage-root discovery and
//! of the harness's sandbox override (`AGENTS.md`, "Invariants & boundaries";
//! design §3, principle 7).
//!
//! Run from a checkout of this repository, Photo keeps its stores in visible
//! directories directly under the repository root, so the data a bug is about
//! is one `ls` away on the machine that has the bug. Run from anywhere else,
//! it keeps them in `Photo/` under the user's Pictures folder. A harness
//! session points them all at its own sandbox before anything resolves a
//! path.
//!
//! Nothing else in the workspace decides a default location: every library
//! crate takes the paths [`Storage`] hands it.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The store holding the catalog file and its previews.
pub const CATALOG_DIR_NAME: &str = "catalog";

/// The store exports go to unless the user chooses another folder.
pub const EXPORT_DIR_NAME: &str = "edited";

/// The catalog file, inside [`CATALOG_DIR_NAME`].
pub const CATALOG_FILE_NAME: &str = "Photo Catalog.photocat";

/// The preview cache, a directory beside the catalog file.
pub const PREVIEWS_DIR_NAME: &str = "Photo Catalog.previews";

/// The folder under the user's Pictures directory that holds the stores when
/// Photo does not run from a checkout.
pub const USER_FOLDER_NAME: &str = "Photo";

static RUNTIME_STORAGE_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Points every store at `root` for the rest of the process. Only the first
/// call has an effect, and it reports whether it was that call. `main` makes
/// it when a harness session starts with sandboxed storage, before anything
/// resolves a path.
pub fn set_runtime_storage_root(root: PathBuf) -> bool {
    RUNTIME_STORAGE_ROOT.set(root).is_ok()
}

/// Whether `candidate` is a checkout of this repository: it has `.git` (a
/// directory, or the file a linked worktree has), `AGENTS.md`, `Cargo.toml`
/// and `src/main.rs`. Another Rust repository is not mistaken for this one.
pub fn is_repo_root(candidate: &Path) -> bool {
    candidate.join(".git").exists()
        && candidate.join("AGENTS.md").is_file()
        && candidate.join("Cargo.toml").is_file()
        && candidate.join("src").join("main.rs").is_file()
}

/// The checkout of this repository that contains `start`: the nearest of
/// `start` and its ancestors that [`is_repo_root`].
pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|candidate| is_repo_root(candidate))
        .map(Path::to_path_buf)
}

/// The checkout this process runs from, found from the executable's location
/// first (so `./target/release/photo` finds its own checkout whatever the
/// working directory is), then from the working directory.
pub fn repo_root() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| find_repo_root(exe.parent()?))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|dir| find_repo_root(&dir))
        })
}

/// Where the stores live for this process: the runtime override if one was
/// set, else the checkout, else `Photo/` under the user's Pictures folder.
/// `None` only when none of those exists.
pub fn storage_root() -> Option<PathBuf> {
    resolve_storage_root(
        RUNTIME_STORAGE_ROOT.get().cloned(),
        repo_root(),
        pictures_dir(),
    )
}

/// The rule [`storage_root`] applies, as a pure function of its inputs: an
/// override wins, then a checkout, then the Pictures folder.
pub fn resolve_storage_root(
    override_root: Option<PathBuf>,
    repo_root: Option<PathBuf>,
    pictures_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    override_root
        .or(repo_root)
        .or_else(|| pictures_dir.map(|pictures| pictures.join(USER_FOLDER_NAME)))
}

fn pictures_dir() -> Option<PathBuf> {
    directories::UserDirs::new().and_then(|dirs| dirs.picture_dir().map(Path::to_path_buf))
}

/// The location of every store under one storage root. Building it creates
/// nothing on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storage {
    /// The storage root itself.
    pub root: PathBuf,
    /// The catalog file.
    pub catalog: PathBuf,
    /// The preview cache directory, beside the catalog file.
    pub previews: PathBuf,
    /// The default export folder.
    pub exports: PathBuf,
}

impl Storage {
    /// The stores under `root`.
    pub fn under(root: &Path) -> Self {
        let catalog_dir = root.join(CATALOG_DIR_NAME);
        Self {
            root: root.to_path_buf(),
            catalog: catalog_dir.join(CATALOG_FILE_NAME),
            previews: catalog_dir.join(PREVIEWS_DIR_NAME),
            exports: root.join(EXPORT_DIR_NAME),
        }
    }

    /// Creates the directories that hold the catalog and the previews.
    /// The export folder is left to the first export, so an app that never
    /// exports leaves no empty folder behind.
    pub fn create_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.previews)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every directory this app persists into under the storage root, as
    /// `(constant name, value)`. A new store belongs in this list.
    fn repo_local_dir_names() -> Vec<(&'static str, &'static str)> {
        vec![
            ("CATALOG_DIR_NAME", CATALOG_DIR_NAME),
            ("EXPORT_DIR_NAME", EXPORT_DIR_NAME),
        ]
    }

    /// A store you cannot see is a store you cannot debug. When the app shows
    /// the wrong thing the first question is what is actually on disk, and
    /// the answer has to be one `ls` away in the checkout, not behind a
    /// leading dot or inside a per-user profile directory that differs
    /// between the machine reporting the bug and the machine debugging it.
    ///
    /// Asserting a path against the very constant that defines it proves
    /// nothing, so the shape of each value is what gets asserted.
    #[test]
    fn repo_local_stores_stay_visible_and_repo_relative() {
        for (constant, name) in repo_local_dir_names() {
            assert!(
                !name.is_empty(),
                "{constant} is empty, so its store would land on the root itself"
            );
            assert!(
                !name.starts_with('.'),
                "{constant} is {name:?}; a leading dot hides the store from an ordinary listing"
            );
            assert_eq!(
                Path::new(name).components().count(),
                1,
                "{constant} is {name:?}; a store is one directory directly under the root"
            );
            assert!(
                !Path::new(name).is_absolute(),
                "{constant} is {name:?}, an absolute path, which escapes the root and the harness sandbox with it"
            );
            assert!(
                !name.contains(".."),
                "{constant} is {name:?} and climbs out of the root"
            );
        }
        let mut names: Vec<_> = repo_local_dir_names().into_iter().map(|(_, n)| n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            repo_local_dir_names().len(),
            "two stores share a directory"
        );
    }

    /// The harness sandboxes a session by handing every store a root inside
    /// its run directory. That only sandboxes a store whose path is derived
    /// from the root it is given; one that reads the environment or the user
    /// profile writes into the real catalog during a sandboxed session.
    #[test]
    fn every_store_follows_the_root_it_is_given() {
        let sandbox = tempfile::tempdir().unwrap();
        let storage = Storage::under(sandbox.path());
        assert_eq!(storage.root, sandbox.path());
        for path in [&storage.catalog, &storage.previews, &storage.exports] {
            assert!(
                path.starts_with(sandbox.path()),
                "{} is outside the root {}",
                path.display(),
                sandbox.path().display()
            );
        }
        for (constant, name) in repo_local_dir_names() {
            let store = sandbox.path().join(name);
            let used = [&storage.catalog, &storage.previews, &storage.exports]
                .iter()
                .any(|path| path.starts_with(&store));
            assert!(used, "{constant} is listed but no store resolves into it");
        }
        assert_eq!(storage.catalog.parent(), storage.previews.parent());
    }

    #[test]
    fn creating_the_dirs_makes_the_catalog_and_preview_folders_only() {
        let sandbox = tempfile::tempdir().unwrap();
        let storage = Storage::under(sandbox.path());
        storage.create_dirs().unwrap();
        storage.create_dirs().unwrap();
        assert!(storage.previews.is_dir());
        assert!(
            !storage.catalog.exists(),
            "the catalog file is the catalog's to create"
        );
        assert!(!storage.exports.exists());
    }

    #[test]
    fn an_override_wins_then_the_checkout_then_the_pictures_folder() {
        let sandbox = PathBuf::from("/sandbox");
        let checkout = PathBuf::from("/checkout");
        let pictures = PathBuf::from("/home/someone/Pictures");
        assert_eq!(
            resolve_storage_root(
                Some(sandbox.clone()),
                Some(checkout.clone()),
                Some(pictures.clone())
            ),
            Some(sandbox)
        );
        assert_eq!(
            resolve_storage_root(None, Some(checkout.clone()), Some(pictures.clone())),
            Some(checkout)
        );
        assert_eq!(
            resolve_storage_root(None, None, Some(pictures.clone())),
            Some(pictures.join(USER_FOLDER_NAME))
        );
        assert_eq!(resolve_storage_root(None, None, None), None);
    }

    #[test]
    fn a_checkout_is_found_from_a_build_directory_inside_it() {
        let checkout = tempfile::tempdir().unwrap();
        let root = checkout.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join("AGENTS.md"), "# AGENTS.md\n").unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"photo\"\n").unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("main.rs"), "fn main() {}\n").unwrap();
        let build = root.join("target").join("release");
        std::fs::create_dir_all(&build).unwrap();
        assert_eq!(find_repo_root(&build), Some(root.to_path_buf()));
    }

    #[test]
    fn a_linked_worktree_counts_as_a_checkout() {
        let checkout = tempfile::tempdir().unwrap();
        let root = checkout.path();
        std::fs::write(root.join(".git"), "gitdir: /elsewhere/.git/worktrees/x\n").unwrap();
        std::fs::write(root.join("AGENTS.md"), "# AGENTS.md\n").unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"photo\"\n").unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("main.rs"), "fn main() {}\n").unwrap();
        assert!(is_repo_root(root));
    }

    #[test]
    fn another_rust_repository_is_not_mistaken_for_this_one() {
        let other = tempfile::tempdir().unwrap();
        let nested = other.path().join("target").join("debug");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            other.path().join("Cargo.toml"),
            "[package]\nname = \"not-photo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::create_dir(other.path().join(".git")).unwrap();
        assert_eq!(find_repo_root(&nested), None);
    }

    #[test]
    fn this_checkout_is_found_from_the_test_binary() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(repo_root(), Some(manifest_dir));
    }
}
