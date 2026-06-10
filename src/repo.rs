//! Photo repo-root discovery and its test override plumbing.

#[cfg(test)]
use crate::local_edits::clear_test_local_edit_thumbnail_hooks;
use std::path::{Path, PathBuf};

#[cfg(test)]
pub(crate) static TEST_PHOTO_REPO_ROOT_OVERRIDE: std::sync::OnceLock<
    std::sync::Mutex<Option<Option<PathBuf>>>,
> = std::sync::OnceLock::new();
#[cfg(test)]
pub(crate) static TEST_PHOTO_REPO_ROOT_GUARD: std::sync::OnceLock<std::sync::Mutex<()>> =
    std::sync::OnceLock::new();
pub(crate) fn photo_repo_root() -> Option<PathBuf> {
    #[cfg(test)]
    {
        let override_root = TEST_PHOTO_REPO_ROOT_OVERRIDE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap()
            .clone();
        if let Some(repo_root) = override_root {
            return repo_root;
        }
    }

    std::env::current_exe()
        .ok()
        .and_then(|path| find_photo_repo_root(path.parent()?))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|dir| find_photo_repo_root(&dir))
        })
}

pub(crate) fn find_photo_repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|candidate| is_photo_repo_root(candidate))
        .map(Path::to_path_buf)
}

pub(crate) fn is_photo_repo_root(candidate: &Path) -> bool {
    candidate.join(".git").exists()
        && candidate.join("AGENTS.md").is_file()
        && candidate.join("Cargo.toml").is_file()
        && candidate.join("src").join("main.rs").is_file()
}

#[cfg(test)]
pub(crate) fn with_test_photo_repo_root<T>(repo_root: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = TEST_PHOTO_REPO_ROOT_GUARD
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    clear_test_local_edit_thumbnail_hooks();
    let storage = TEST_PHOTO_REPO_ROOT_OVERRIDE.get_or_init(|| std::sync::Mutex::new(None));
    *storage
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Some(repo_root.to_path_buf()));
    let result = f();
    *storage
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    clear_test_local_edit_thumbnail_hooks();
    result
}
