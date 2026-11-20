//! Sidecar files: where they live, reading them, and replacing them whole.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use photo_core::meta::FileKind;

use crate::{ImageInfo, MAX_SIDECAR_BYTES, XmpData, XmpError, parse, to_string};

/// How many temporary names are tried before giving up on finding a free one.
const TEMPORARY_NAME_ATTEMPTS: u32 = 16;

/// How many times an update is merged afresh, when the sidecar keeps being
/// saved by someone else meanwhile, before it is given up.
const MERGE_ATTEMPTS: u32 = 4;

/// The sidecar that belongs to the image at `image_path`.
///
/// A RAW file's sidecar replaces the extension (`IMG_0001.ARW` has
/// `IMG_0001.xmp`), which is where other applications put it and look for
/// it. A raster file's sidecar is appended to the whole name
/// (`IMG_0001.jpg` has `IMG_0001.jpg.xmp`): the original is never written
/// into, and a JPEG beside a RAW file of the same name keeps a sidecar of
/// its own. Only the last extension counts, so `pano.final.CR3` has
/// `pano.final.xmp`.
pub fn sidecar_path(image_path: &Path, kind: FileKind) -> PathBuf {
    match kind {
        FileKind::Raw => image_path.with_extension("xmp"),
        FileKind::Raster => {
            let mut name = OsString::from(image_path);
            name.push(".xmp");
            PathBuf::from(name)
        }
    }
}

/// Reads the sidecar at `path` for an image described by `image`.
///
/// Returns `Ok(None)` when there is no file there. A file that exists but is
/// not a well-formed XMP packet in UTF-8 (an empty file included) is an
/// error, never silently "no metadata", and so is one larger than
/// [`MAX_SIDECAR_BYTES`], which is not read past that size.
pub fn read_sidecar(path: &Path, image: ImageInfo) -> Result<Option<XmpData>, XmpError> {
    match read_bytes(path)? {
        Some(bytes) => parse(text_of(&bytes)?, image).map(Some),
        None => Ok(None),
    }
}

/// Writes `data` to the sidecar at `path`, updating the file if it exists
/// (see [`to_string`] for what an update keeps) and creating it otherwise.
///
/// The file is replaced in one step: the new content goes to a temporary
/// file in the same folder, which is then renamed over the old one. Whatever
/// goes wrong (the existing sidecar cannot be parsed, the disk is full, the
/// folder is read-only) the previous sidecar is left exactly as it was, and
/// the temporary file is removed again. A sidecar that already says what
/// `data` says is not rewritten at all.
///
/// Another application may save the sidecar while the update is being made.
/// So just before the rename the sidecar is read again, and if it is no
/// longer exactly the bytes the update was merged into, the merge is made
/// again from what it holds now: what the other application saved is kept,
/// except the properties `data` sets, which take `data`'s values. When the
/// sidecar has changed again every time, a few times over, the update is
/// given up with [`XmpError::Conflict`] and the file is left as the other
/// application last saved it.
///
/// One window remains: a save that lands after that last check and before
/// the rename is replaced. No portable file system operation renames only
/// if the target is unchanged, so the window cannot be closed here; it is
/// as short as one read of the sidecar and one rename.
pub fn write_sidecar(path: &Path, data: &XmpData, image: ImageInfo) -> Result<(), XmpError> {
    write_sidecar_with(path, data, image, || {})
}

/// [`write_sidecar`], running `meanwhile` once each replacement is ready and
/// before the sidecar is checked again: the moment at which another
/// application's save would otherwise be overwritten.
fn write_sidecar_with(
    path: &Path,
    data: &XmpData,
    image: ImageInfo,
    mut meanwhile: impl FnMut(),
) -> Result<(), XmpError> {
    for _ in 0..MERGE_ATTEMPTS {
        let existing = read_bytes(path)?;
        let current = existing.as_deref().map(text_of).transpose()?;
        let text = to_string(data, image, current)?;
        if current == Some(text.as_str()) {
            return Ok(());
        }
        let temporary =
            write_beside(path, &text).map_err(|source| io_error("write", path, source))?;
        meanwhile();
        let unchanged = read_bytes(path).map(|now| now == existing);
        if !matches!(unchanged, Ok(true)) {
            // Best effort: what is worth reporting is why the update stops,
            // or nothing when it is merged again.
            let _ = fs::remove_file(&temporary);
            unchanged?;
            continue;
        }
        return fs::rename(&temporary, path).map_err(|source| {
            let _ = fs::remove_file(&temporary);
            io_error("write", path, source)
        });
    }
    Err(XmpError::Conflict {
        path: path.to_owned(),
    })
}

fn io_error(action: &'static str, path: &Path, source: io::Error) -> XmpError {
    XmpError::Io {
        action,
        path: path.to_owned(),
        source,
    }
}

/// The file's bytes, or `None` when there is no such file. Nothing past
/// [`MAX_SIDECAR_BYTES`] is read.
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, XmpError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("read", path, error)),
    };
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_SIDECAR_BYTES).unwrap_or(u64::MAX);
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read", path, error))?;
    if bytes.len() > MAX_SIDECAR_BYTES {
        return Err(XmpError::TooLarge);
    }
    Ok(Some(bytes))
}

/// The bytes of a sidecar as text, which they must be in UTF-8.
fn text_of(bytes: &[u8]) -> Result<&str, XmpError> {
    std::str::from_utf8(bytes).map_err(|error| XmpError::Malformed {
        offset: error.valid_up_to() as u64,
        message: "the file is not UTF-8".to_owned(),
    })
}

/// Puts `contents` in a new temporary file beside `path`, on disk, and
/// returns the file's path. The file is removed again if that fails.
fn write_beside(path: &Path, contents: &str) -> io::Result<PathBuf> {
    let (temporary, mut file) = create_beside(path)?;
    let written = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all());
    // Closed before any rename: Windows will not rename an open file.
    drop(file);
    match written {
        Ok(()) => Ok(temporary),
        Err(error) => {
            // Best effort: the error worth reporting is the one that got us here.
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

/// Creates a new, empty file next to `path` under a name nothing else uses:
/// one that is short whatever the sidecar is called, and told apart by this
/// process and a running count.
fn create_beside(path: &Path) -> io::Result<(PathBuf, File)> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    if path.file_name().is_none() {
        let reason = "the sidecar path does not name a file";
        return Err(io::Error::new(io::ErrorKind::InvalidInput, reason));
    }
    let mut last_error = None;
    for _ in 0..TEMPORARY_NAME_ATTEMPTS {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!(".photo-sidecar-{}-{unique}.tmp", std::process::id());
        let temporary = path.with_file_name(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last_error = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("no temporary file name was tried")))
}

#[cfg(test)]
mod tests {
    use photo_core::meta::Rating;

    use super::*;

    const RAW: ImageInfo = ImageInfo::new(FileKind::Raw);

    fn rated(stars: u8) -> XmpData {
        XmpData {
            rating: Rating::new(stars),
            ..XmpData::default()
        }
    }

    /// `text` as another application saves it: with a note of its own.
    fn noted(text: &str, note: &str) -> String {
        let description = r#"<rdf:Description rdf:about="""#;
        let noted = format!(r#"{description} xmlns:acme="urn:acme" acme:Note="{note}""#);
        assert!(text.contains(description), "{text}");
        text.replacen(description, &noted, 1)
    }

    fn entries(directory: &Path) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn a_save_made_while_writing_is_merged_rather_than_overwritten() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("IMG_0001.xmp");
        write_sidecar(&path, &rated(1), RAW).unwrap();

        let mut moments = 0;
        let written = write_sidecar_with(&path, &rated(4), RAW, || {
            if moments == 0 {
                let theirs = noted(&fs::read_to_string(&path).unwrap(), "theirs");
                fs::write(&path, theirs).unwrap();
            }
            moments += 1;
        });
        written.unwrap();
        assert_eq!(moments, 2, "the save was found, and the merge made again");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#"acme:Note="theirs""#), "{text}");
        assert_eq!(read_sidecar(&path, RAW).unwrap(), Some(rated(4)));
        assert_eq!(entries(folder.path()), [path]);
    }

    #[test]
    fn a_sidecar_created_while_writing_is_merged_too() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("IMG_0001.xmp");
        let theirs = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:acme="urn:acme" acme:Note="theirs"/></rdf:RDF></x:xmpmeta>"#;
        let mut moments = 0;
        let written = write_sidecar_with(&path, &rated(4), RAW, || {
            if moments == 0 {
                fs::write(&path, theirs).unwrap();
            }
            moments += 1;
        });
        written.unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#"acme:Note="theirs""#), "{text}");
        assert_eq!(read_sidecar(&path, RAW).unwrap(), Some(rated(4)));
        assert_eq!(entries(folder.path()), [path]);
    }

    #[test]
    fn a_sidecar_that_keeps_changing_is_given_up_on_and_left_as_last_saved() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("IMG_0001.xmp");
        write_sidecar(&path, &rated(1), RAW).unwrap();
        let original = fs::read_to_string(&path).unwrap();

        let mut saves = 0;
        let mut last = String::new();
        let written = write_sidecar_with(&path, &rated(4), RAW, || {
            saves += 1;
            last = noted(&original, &format!("save {saves}"));
            fs::write(&path, &last).unwrap();
        });
        let error = written.unwrap_err();
        assert!(
            matches!(&error, XmpError::Conflict { path: reported } if *reported == path),
            "{error:?}"
        );
        assert!(error.to_string().contains("kept changing"), "{error}");
        assert_eq!(saves, MERGE_ATTEMPTS);
        assert_eq!(fs::read_to_string(&path).unwrap(), last);
        assert_eq!(entries(folder.path()), [path]);
    }
}
