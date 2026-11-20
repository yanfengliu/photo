//! Path handling that behaves the same on every host.
//!
//! The catalog stores paths as text and must reason about folders for paths
//! written on another platform (a catalog can travel between machines, and
//! the tests exercise Windows paths on any host), so separators are decided
//! by the shape of the path itself, never by `std::path` on the current OS.

use std::borrow::Cow;
use std::path::Path;

use crate::error::{CatalogError, Result};

/// The path as the text the catalog stores and compares, or the typed error
/// for paths that are not UTF-8.
///
/// Every path enters the catalog through this function, which gives each
/// file one spelling where the catalog can tell that two spellings are the
/// same file: in a Windows-shaped path `/` and `\` both separate components,
/// so `/` is rewritten to `\`. Nothing else is changed (letter case, `..` and
/// links are the caller's to resolve). A NUL character cannot be part of any
/// real path, and SQLite's text functions stop at one, so it is refused.
pub(crate) fn text(path: &Path) -> Result<Cow<'_, str>> {
    let text = path
        .to_str()
        .ok_or_else(|| CatalogError::NonUtf8Path(path.to_path_buf()))?;
    if text.contains('\0') {
        return Err(CatalogError::InvalidPath(text.to_string()));
    }
    if separators(text).len() > 1 && text.contains('/') {
        Ok(Cow::Owned(text.replace('/', "\\")))
    } else {
        Ok(Cow::Borrowed(text))
    }
}

/// A file path split into the directory that holds it and its name.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Split<'a> {
    pub(crate) dir: &'a str,
    pub(crate) file_name: &'a str,
}

/// Split a file path at its last separator. The directory of a file directly
/// under a root keeps the root's separator (`/`, `C:\`).
pub(crate) fn split(path: &str) -> Result<Split<'_>> {
    let invalid = || CatalogError::InvalidPath(path.to_string());
    let at = path.rfind(separators(path)).ok_or_else(invalid)?;
    let file_name = &path[at + 1..];
    let dir = directory(&path[..at + 1]);
    if file_name.is_empty() || dir.is_empty() {
        return Err(invalid());
    }
    Ok(Split { dir, file_name })
}

/// A directory path in the form the catalog stores: without trailing
/// separators, except that a root keeps exactly one.
pub(crate) fn directory(path: &str) -> &str {
    let seps = separators(path);
    let trimmed = path.trim_end_matches(seps);
    if trimmed.len() == path.len() {
        return path;
    }
    let is_root =
        trimmed.is_empty() || (trimmed.len() == 2 && starts_with_drive(trimmed.as_bytes()));
    if is_root {
        &path[..trimmed.len() + 1]
    } else {
        trimmed
    }
}

/// The prefix that marks a stored path as lying inside the stored folder
/// path `dir`: `dir` followed by a separator. Comparing against this (rather
/// than `dir` alone) is what keeps `/a/bc` from being mistaken for a child of
/// `/a/b`.
pub(crate) fn child_prefix(dir: &str) -> String {
    let seps = separators(dir);
    format!("{}{}", dir.trim_end_matches(seps), seps[0])
}

/// The lower-cased extension of the file a path names, without the dot;
/// empty when the name has none.
pub(crate) fn extension(path: &str) -> String {
    let name = split(path).map_or(path, |split| split.file_name);
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => extension.to_lowercase(),
        _ => String::new(),
    }
}

/// The separators that apply to `path`: both slashes for a Windows-shaped
/// path (drive letter or UNC prefix), otherwise only `/`, because a backslash
/// is an ordinary file-name character on POSIX systems.
fn separators(path: &str) -> &'static [char] {
    let bytes = path.as_bytes();
    let drive =
        starts_with_drive(bytes) && bytes.get(2).is_none_or(|byte| matches!(byte, b'\\' | b'/'));
    if drive || path.starts_with("\\\\") {
        &['\\', '/']
    } else {
        &['/']
    }
}

fn starts_with_drive(bytes: &[u8]) -> bool {
    matches!(bytes, [letter, b':', ..] if letter.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(path: &str) -> (&str, &str) {
        let split = split(path).unwrap();
        (split.dir, split.file_name)
    }

    #[test]
    fn posix_paths_split_at_the_last_slash() {
        assert_eq!(
            parts("/photos/2024/IMG_1.jpg"),
            ("/photos/2024", "IMG_1.jpg")
        );
        assert_eq!(parts("/IMG_1.jpg"), ("/", "IMG_1.jpg"));
        assert_eq!(parts("relative/dir/a.png"), ("relative/dir", "a.png"));
        assert_eq!(parts("/写真/夏 2024/海.jpg"), ("/写真/夏 2024", "海.jpg"));
        assert_eq!(parts("日本/a.jpg"), ("日本", "a.jpg"));
    }

    #[test]
    fn a_backslash_is_part_of_the_name_on_posix_paths() {
        assert_eq!(parts("/photos/odd\\name.jpg"), ("/photos", "odd\\name.jpg"));
    }

    #[test]
    fn windows_paths_split_at_either_separator() {
        assert_eq!(
            parts("C:\\Photos\\2024\\IMG_1.jpg"),
            ("C:\\Photos\\2024", "IMG_1.jpg")
        );
        assert_eq!(parts("C:\\IMG_1.jpg"), ("C:\\", "IMG_1.jpg"));
        assert_eq!(parts("D:/Photos/a.jpg"), ("D:/Photos", "a.jpg"));
        assert_eq!(
            parts("C:\\Photos/mixed\\a.jpg"),
            ("C:\\Photos/mixed", "a.jpg")
        );
        assert_eq!(parts("\\\\nas\\share\\a.jpg"), ("\\\\nas\\share", "a.jpg"));
    }

    #[test]
    fn paths_without_a_directory_or_a_name_are_rejected() {
        for path in ["", "IMG_1.jpg", "/photos/", "C:\\Photos\\", "/"] {
            assert!(
                matches!(split(path), Err(CatalogError::InvalidPath(_))),
                "{path:?}"
            );
        }
    }

    #[test]
    fn directories_lose_trailing_separators_but_roots_keep_one() {
        assert_eq!(directory("/photos/2024/"), "/photos/2024");
        assert_eq!(directory("/photos/2024"), "/photos/2024");
        assert_eq!(directory("/"), "/");
        assert_eq!(directory("///"), "/");
        assert_eq!(directory("C:\\"), "C:\\");
        assert_eq!(directory("C:\\Photos\\"), "C:\\Photos");
        assert_eq!(directory("D:/"), "D:/");
    }

    #[test]
    fn a_child_prefix_ends_in_exactly_one_separator() {
        assert_eq!(child_prefix("/a/b"), "/a/b/");
        assert_eq!(child_prefix("/"), "/");
        assert_eq!(child_prefix("C:\\a"), "C:\\a\\");
        assert_eq!(child_prefix("C:\\"), "C:\\");
        assert_eq!(child_prefix("\\\\nas\\share"), "\\\\nas\\share\\");
    }

    #[test]
    fn windows_shaped_paths_get_one_spelling_and_other_paths_are_left_alone() {
        let stored = |path: &str| text(Path::new(path)).unwrap().into_owned();
        assert_eq!(stored("C:/Photos/2024/a.jpg"), "C:\\Photos\\2024\\a.jpg");
        assert_eq!(
            stored("C:\\Photos/mixed\\a.jpg"),
            "C:\\Photos\\mixed\\a.jpg"
        );
        assert_eq!(stored("d:/"), "d:\\");
        assert_eq!(
            stored("\\\\nas\\share/dir/a.jpg"),
            "\\\\nas\\share\\dir\\a.jpg"
        );
        // Not Windows-shaped: a backslash is just a character here, and a
        // drive-like first component without a separator is not a drive.
        for same in [
            "/photos/a.jpg",
            "/photos/odd\\name.jpg",
            "relative/a.jpg",
            "C:relative/a.jpg",
        ] {
            assert_eq!(stored(same), same);
        }
        assert!(matches!(
            text(Path::new("/photos/a.jpg")).unwrap(),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn extensions_are_lower_cased_and_come_from_the_file_name_only() {
        assert_eq!(extension("/photos/IMG_1.JPG"), "jpg");
        assert_eq!(extension("C:\\Photos.old\\scan"), "");
        assert_eq!(extension("/photos/archive.tar.GZ"), "gz");
        assert_eq!(extension("/photos/.hidden"), "");
        assert_eq!(extension("bare.ARW"), "arw");
    }

    #[test]
    fn non_utf8_paths_are_a_typed_error() {
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            let path = Path::new(OsStr::from_bytes(b"/photos/\xff.jpg"));
            assert!(matches!(text(path), Err(CatalogError::NonUtf8Path(_))));
        }
        assert_eq!(text(Path::new("/photos/ok.jpg")).unwrap(), "/photos/ok.jpg");
    }

    #[test]
    fn a_path_with_a_nul_character_is_refused() {
        let path = Path::new("/photos/a\0b.jpg");
        assert!(matches!(text(path), Err(CatalogError::InvalidPath(_))));
    }
}
