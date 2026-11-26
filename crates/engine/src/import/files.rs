//! The worker side of an import: reading what a file says about itself, and
//! copying it into place without ever overwriting anything.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use photo_catalog::NewPhoto;
use photo_core::time::DateTime;
use photo_xmp::XmpData;

use crate::error::EngineError;
use crate::sidecars::image_info;

/// Bytes copied between two checks for cancellation.
const COPY_CHUNK: usize = 1 << 20;
/// How many numbered names are tried before a copy gives up.
const NAME_ATTEMPTS: u32 = 10_000;

/// A file read for import.
pub(crate) struct ReadFile {
    /// The record to insert, filled from the file and its sidecar.
    pub(crate) photo: NewPhoto,
    /// Why the sidecar could not be read, if it could not.
    pub(crate) sidecar_problem: Option<String>,
    /// The format of the develop settings in the sidecar, when this build
    /// cannot read them: the photo then has the starting settings.
    pub(crate) unreadable_settings: Option<u32>,
}

/// Read a file's metadata and, if asked, its XMP sidecar.
pub(crate) fn read(path: &Path, read_sidecar: bool) -> Result<ReadFile, String> {
    let kind = photo_decode::file_kind(path).ok_or("not a supported image file")?;
    let stat = fs::metadata(path).map_err(|error| error.to_string())?;
    let metadata = photo_decode::read_metadata(path).map_err(|error| error.to_string())?;
    let mut photo = NewPhoto::new(path, kind);
    photo.file_size = stat.len();
    photo.file_mtime = stat.modified().map(unix_seconds).unwrap_or(0);
    photo.creator = metadata.artist.clone().unwrap_or_default();
    photo.copyright = metadata.copyright.clone().unwrap_or_default();
    photo.metadata = metadata;
    let (mut sidecar_problem, mut unreadable_settings) = (None, None);
    if read_sidecar {
        let info = image_info(kind, photo.metadata.width, photo.metadata.height);
        let sidecar = photo_xmp::sidecar_path(path, kind);
        match photo_xmp::read_sidecar(&sidecar, info) {
            Ok(Some(data)) => {
                unreadable_settings = data.unreadable_settings_format;
                apply_sidecar(&mut photo, data);
            }
            Ok(None) => {}
            Err(error) => sidecar_problem = Some(error.to_string()),
        }
    }
    Ok(ReadFile {
        photo,
        sidecar_problem,
        unreadable_settings,
    })
}

/// Lay what a sidecar says over a photo read from its file. Text the
/// sidecar does not give keeps what the file said.
fn apply_sidecar(photo: &mut NewPhoto, data: XmpData) {
    photo.rating = data.rating;
    photo.flag = data.flag;
    photo.label = data.label;
    photo.keywords = sidecar_keywords(&data);
    let texts = [
        (&mut photo.title, data.title),
        (&mut photo.caption, data.caption),
        (&mut photo.copyright, data.copyright),
        (&mut photo.creator, data.creators.join("; ")),
    ];
    for (field, text) in texts {
        if !text.is_empty() {
            *field = text;
        }
    }
    photo.develop = data.develop;
}

/// The keyword names a sidecar assigns: its flat list, or failing that the
/// last level of each hierarchical keyword.
pub(crate) fn sidecar_keywords(data: &XmpData) -> Vec<String> {
    if !data.keywords.is_empty() {
        return data.keywords.clone();
    }
    data.hierarchical_keywords
        .iter()
        .filter_map(|path| path.last().cloned())
        .collect()
}

pub(crate) fn unix_seconds(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

/// A folder layout by date, such as `YYYY/YYYY-MM-DD`: `YYYY`, `YY`, `MM`
/// and `DD` stand for the date's year, two-digit year, month and day,
/// everything else is literal, and `/` separates folders.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DatePattern(String);

impl DatePattern {
    /// Check a pattern. Refused: an empty pattern or folder name, `.` and
    /// `..`, a leading or doubled `/`, and the characters `\`, `:` and NUL.
    pub fn new(pattern: &str) -> Result<Self, EngineError> {
        let refuse = |reason| EngineError::InvalidPattern {
            pattern: pattern.to_string(),
            reason,
        };
        if pattern.contains(['\\', ':', '\0']) {
            return Err(refuse("it may not contain \\, : or NUL"));
        }
        if pattern
            .split('/')
            .any(|part| part.trim().is_empty() || part == "." || part == "..")
        {
            return Err(refuse("every folder name must be a plain, non-empty name"));
        }
        Ok(Self(pattern.to_string()))
    }

    /// The pattern as given.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The relative folder for `date`.
    pub fn folder_for(&self, date: DateTime) -> PathBuf {
        let mut text = String::new();
        let mut rest = self.0.as_str();
        while !rest.is_empty() {
            let (expansion, length) = if rest.starts_with("YYYY") {
                (format!("{:04}", date.year), 4)
            } else if rest.starts_with("YY") {
                (format!("{:02}", date.year.rem_euclid(100)), 2)
            } else if rest.starts_with("MM") {
                (format!("{:02}", date.month), 2)
            } else if rest.starts_with("DD") {
                (format!("{:02}", date.day), 2)
            } else {
                let next = rest.chars().next().map_or(1, char::len_utf8);
                (rest[..next].to_string(), next)
            };
            text.push_str(&expansion);
            rest = &rest[length..];
        }
        text.split('/').collect()
    }
}

/// A file copied into place.
pub(crate) struct Copied {
    pub(crate) path: PathBuf,
    pub(crate) bytes: u64,
    pub(crate) modified: i64,
}

/// Why a copy did not happen.
pub(crate) enum CopyError {
    /// It was cancelled; nothing was left behind.
    Cancelled,
    /// It failed for this reason; nothing was left behind.
    Failed(String),
}

/// Copy `source` into `folder` (created if needed) under its own name, or
/// with `-1`, `-2`, ... before the extension when that name is taken.
///
/// An existing file is never overwritten, and a copy that fails or is
/// cancelled leaves no file behind: the data goes to a hidden temporary
/// file first, a free name is then claimed by creating it (which fails if
/// it exists), and the finished copy is renamed over the claim.
pub(crate) fn copy_into(
    source: &Path,
    folder: &Path,
    should_stop: impl Fn() -> bool,
) -> Result<Copied, CopyError> {
    let failed = |error: io::Error| CopyError::Failed(error.to_string());
    let name = source
        .file_name()
        .ok_or_else(|| CopyError::Failed("the source has no file name".to_string()))?;
    fs::create_dir_all(folder).map_err(failed)?;
    let temporary = folder.join(format!(
        ".photo-import-{}-{}.tmp",
        std::process::id(),
        next_temporary()
    ));
    let copied = copy_data(source, &temporary, &should_stop)
        .and_then(|()| claim_and_publish(&temporary, folder, Path::new(name)).map_err(failed));
    if copied.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    let path = copied?;
    let stat = fs::metadata(&path).map_err(failed)?;
    Ok(Copied {
        modified: stat.modified().map(unix_seconds).unwrap_or(0),
        bytes: stat.len(),
        path,
    })
}

fn next_temporary() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Copy the bytes, flush them to disk and carry the modification time over.
fn copy_data(
    source: &Path,
    target: &Path,
    should_stop: &impl Fn() -> bool,
) -> Result<(), CopyError> {
    let failed = |error: io::Error| CopyError::Failed(error.to_string());
    let mut input = File::open(source).map_err(failed)?;
    let modified = input.metadata().and_then(|stat| stat.modified()).ok();
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(failed)?;
    let mut buffer = vec![0u8; COPY_CHUNK];
    loop {
        if should_stop() {
            return Err(CopyError::Cancelled);
        }
        let read = input.read(&mut buffer).map_err(failed)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).map_err(failed)?;
    }
    if let Some(modified) = modified
        && let Err(error) = output.set_modified(modified)
    {
        log::warn!(
            "cannot keep the modification time of {}: {error}",
            source.display()
        );
    }
    output.sync_all().map_err(failed)
}

/// Claim the first free name for `name` in `folder` and move the finished
/// copy onto it.
fn claim_and_publish(temporary: &Path, folder: &Path, name: &Path) -> io::Result<PathBuf> {
    for attempt in 0..NAME_ATTEMPTS {
        let target = folder.join(numbered(name, attempt));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(claim) => drop(claim),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
        return match fs::rename(temporary, &target) {
            Ok(()) => Ok(target),
            Err(error) => {
                let _ = fs::remove_file(&target);
                Err(error)
            }
        };
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free file name for the copy",
    ))
}

/// `name` itself for attempt 0, else `stem-N.ext`.
fn numbered(name: &Path, attempt: u32) -> PathBuf {
    if attempt == 0 {
        return name.to_path_buf();
    }
    let stem = name
        .file_stem()
        .map(|stem| stem.to_string_lossy())
        .unwrap_or_default();
    match name.extension() {
        Some(extension) => {
            PathBuf::from(format!("{stem}-{attempt}.{}", extension.to_string_lossy()))
        }
        None => PathBuf::from(format!("{stem}-{attempt}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_patterns_expand_their_tokens_and_keep_everything_else() {
        let date = DateTime::new(2024, 3, 5, 7, 8, 9).unwrap();
        let pattern = DatePattern::new("YYYY/YYYY-MM-DD").unwrap();
        assert_eq!(
            pattern.folder_for(date),
            Path::new("2024").join("2024-03-05")
        );
        let pattern = DatePattern::new("Shoots YY/MM_DD").unwrap();
        assert_eq!(
            pattern.folder_for(date),
            Path::new("Shoots 24").join("03_05")
        );
        assert_eq!(pattern.as_str(), "Shoots YY/MM_DD");
    }

    #[test]
    fn unsafe_date_patterns_are_refused() {
        for pattern in [
            "", "/YYYY", "YYYY//MM", "YYYY/", "../YYYY", "C:YYYY", "a\\b", ".",
        ] {
            assert!(DatePattern::new(pattern).is_err(), "{pattern:?}");
        }
    }

    #[test]
    fn a_copy_never_overwrites_and_numbers_its_name_instead() {
        let source_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();
        let source = source_dir.path().join("IMG_1.jpg");
        fs::write(&source, b"new picture").unwrap();
        fs::write(target_dir.path().join("IMG_1.jpg"), b"existing").unwrap();

        let first = copy_into(&source, target_dir.path(), || false)
            .ok()
            .unwrap();
        assert_eq!(first.path, target_dir.path().join("IMG_1-1.jpg"));
        let second = copy_into(&source, target_dir.path(), || false)
            .ok()
            .unwrap();
        assert_eq!(second.path, target_dir.path().join("IMG_1-2.jpg"));
        assert_eq!(
            fs::read(target_dir.path().join("IMG_1.jpg")).unwrap(),
            b"existing"
        );
        assert_eq!(fs::read(&first.path).unwrap(), b"new picture");
        assert_eq!(first.bytes, 11);
        let names = fs::read_dir(target_dir.path()).unwrap().count();
        assert_eq!(names, 3, "no temporary file is left behind");
    }

    #[test]
    fn a_cancelled_or_failed_copy_leaves_nothing_behind() {
        let source_dir = tempfile::tempdir().unwrap();
        let target_dir = tempfile::tempdir().unwrap();
        let source = source_dir.path().join("IMG_2.jpg");
        fs::write(&source, vec![7u8; 3 * COPY_CHUNK]).unwrap();
        assert!(matches!(
            copy_into(&source, target_dir.path(), || true),
            Err(CopyError::Cancelled)
        ));
        let missing = source_dir.path().join("absent.jpg");
        assert!(matches!(
            copy_into(&missing, target_dir.path(), || false),
            Err(CopyError::Failed(_))
        ));
        assert_eq!(fs::read_dir(target_dir.path()).unwrap().count(), 0);
    }
}
