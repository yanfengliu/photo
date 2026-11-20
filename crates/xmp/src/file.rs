//! Sidecar files: where they live, reading them, and replacing them whole.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use photo_core::meta::FileKind;

use crate::{ImageInfo, XmpData, XmpError, parse, to_string};

/// How many temporary names are tried before giving up on finding a free one.
const TEMPORARY_NAME_ATTEMPTS: u32 = 16;

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
/// error, never silently "no metadata".
pub fn read_sidecar(path: &Path, image: ImageInfo) -> Result<Option<XmpData>, XmpError> {
    match read_text(path)? {
        Some(text) => parse(&text, image).map(Some),
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
pub fn write_sidecar(path: &Path, data: &XmpData, image: ImageInfo) -> Result<(), XmpError> {
    let existing = read_text(path)?;
    let text = to_string(data, image, existing.as_deref())?;
    if existing.as_deref() == Some(text.as_str()) {
        return Ok(());
    }
    replace(path, &text).map_err(|source| io_error("write", path, source))
}

fn io_error(action: &'static str, path: &Path, source: io::Error) -> XmpError {
    XmpError::Io {
        action,
        path: path.to_owned(),
        source,
    }
}

/// The file's text, or `None` when there is no such file.
fn read_text(path: &Path) -> Result<Option<String>, XmpError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("read", path, error)),
    };
    match String::from_utf8(bytes) {
        Ok(text) => Ok(Some(text)),
        Err(error) => Err(XmpError::Malformed {
            offset: error.utf8_error().valid_up_to() as u64,
            message: "the file is not UTF-8".to_owned(),
        }),
    }
}

/// Puts `contents` at `path` through a temporary file beside it.
fn replace(path: &Path, contents: &str) -> io::Result<()> {
    let (temporary, mut file) = create_beside(path)?;
    let written = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all());
    // Closed before the rename: Windows will not rename an open file.
    drop(file);
    let replaced = written.and_then(|()| fs::rename(&temporary, path));
    if replaced.is_err() {
        // Best effort: the error worth reporting is the one that got us here.
        let _ = fs::remove_file(&temporary);
    }
    replaced
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
