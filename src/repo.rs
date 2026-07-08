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
// Runtime (harness-mode) override for the repo root. Set once by
// `harness::prepare_runtime` so harness sessions redirect the repo-local
// caches (`decoded-cache/`, `local-edits/`, `edited/`) into the run
// directory's sandbox. Unset in normal launches and in tests (the test
// override above takes precedence there).
static RUNTIME_PHOTO_REPO_ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Points repo-local cache discovery at `root` for the rest of the process.
/// Second calls are ignored (the first writer — always `main()` — wins).
pub(crate) fn set_runtime_photo_repo_root(root: PathBuf) {
    let _ = RUNTIME_PHOTO_REPO_ROOT.set(root);
}

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

    if let Some(root) = RUNTIME_PHOTO_REPO_ROOT.get() {
        return Some(root.clone());
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
    with_test_photo_repo_root_override(Some(repo_root.to_path_buf()), f)
}

/// Runs `f` with repo-root discovery overridden to "no repo root found".
#[cfg(test)]
pub(crate) fn with_test_photo_repo_root_absent<T>(f: impl FnOnce() -> T) -> T {
    with_test_photo_repo_root_override(None, f)
}

#[cfg(test)]
fn with_test_photo_repo_root_override<T>(repo_root: Option<PathBuf>, f: impl FnOnce() -> T) -> T {
    let _guard = TEST_PHOTO_REPO_ROOT_GUARD
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    clear_test_local_edit_thumbnail_hooks();
    let storage = TEST_PHOTO_REPO_ROOT_OVERRIDE.get_or_init(|| std::sync::Mutex::new(None));
    *storage
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(repo_root);
    let result = f();
    *storage
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    clear_test_local_edit_thumbnail_hooks();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_photo_repo_root_ignores_other_rust_repositories() {
        let repo_root = tempfile::tempdir().unwrap();
        let nested = repo_root.path().join("target").join("debug");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            repo_root.path().join("Cargo.toml"),
            "[package]\nname = \"not-photo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::create_dir(repo_root.path().join(".git")).unwrap();

        assert_eq!(find_photo_repo_root(&nested), None);
    }
}
