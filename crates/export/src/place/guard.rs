//! The files an export must never replace: the original photo and the
//! files its caller protects, recognised by file identity (device and
//! inode on Unix, volume serial number and file index on Windows) rather
//! than by name.

use std::io;
use std::path::{Path, PathBuf};

use same_file::Handle;

use crate::item::ExportError;

/// Why a file is guarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Guard {
    /// It is the original photo.
    Source,
    /// It is one of the item's protected files.
    Protected,
}

impl Guard {
    /// The refusal to replace the guarded file that is now at `path`.
    pub(crate) fn refusal(self, path: PathBuf) -> ExportError {
        match self {
            Self::Source => ExportError::WouldOverwriteSource(path),
            Self::Protected => ExportError::WouldOverwriteProtected(path),
        }
    }
}

/// One guarded file: the path it was named by and, when that path led to a
/// file as the export started, that file.
struct Kept {
    guard: Guard,
    path: PathBuf,
    at_start: Option<Handle>,
}

impl Kept {
    /// True when `file` is the file this path led to at the start, or the
    /// file it leads to now.
    fn is(&self, file: &Handle) -> bool {
        self.at_start.as_ref() == Some(file)
            || Handle::from_path(&self.path).is_ok_and(|now| now == *file)
    }
}

/// The original photo and the protected files of one export, as they were
/// when it started. Their handles stay open for the whole export, which
/// keeps the identities valid on Windows; they are opened for reading and
/// shared, so the files can still be read, moved or renamed meanwhile.
pub(crate) struct Guarded {
    files: Vec<Kept>,
}

impl Guarded {
    /// The files `source` and `protected` lead to now. A path that leads to
    /// no file yet (its folder is away, say) is looked at again whenever a
    /// file is compared.
    pub(crate) fn capture(source: &Path, protected: &[PathBuf]) -> Self {
        let kept = |guard, path: &Path| Kept {
            guard,
            path: path.to_path_buf(),
            at_start: Handle::from_path(path).ok(),
        };
        let mut files = vec![kept(Guard::Source, source)];
        files.extend(protected.iter().map(|path| kept(Guard::Protected, path)));
        Self { files }
    }

    /// Which guarded file `path` leads to, if any: the same file under
    /// whatever name it has by now, through a hard link, a symbolic link or
    /// a name that differs only in case. A name that leads to no file is
    /// guarded by nothing. An error means the file is there but its
    /// identity cannot be read, so nothing should replace it unverified.
    pub(crate) fn guards(&self, path: &Path) -> io::Result<Option<Guard>> {
        let file = match Handle::from_path(path) {
            Ok(file) => file,
            // Nothing has the name, or a symbolic link that leads nowhere.
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(self
            .files
            .iter()
            .find(|kept| kept.is(&file))
            .map(|kept| kept.guard))
    }
}
