//! What a source file looked like when something was derived from it.

use std::fs;
use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// A file's size and modification time, read by the task that then reads
/// its pixels. Whatever was derived from those pixels records the stamp; if
/// either number differs later, the file changed and the derived thing no
/// longer describes it.
///
/// Read before the pixels, never after: if the file changes in between,
/// the recorded stamp is the older one, so the next check finds a
/// difference and derives again. The other order could pair new numbers
/// with old pixels for good.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Stamp {
    pub(crate) bytes: u64,
    pub(crate) modified_secs: i64,
    pub(crate) modified_nanos: u32,
}

impl Stamp {
    /// The stamp of the file at `path` as it is now.
    pub(crate) fn read(path: &Path) -> io::Result<Self> {
        fs::metadata(path).map(|metadata| Self::of(&metadata))
    }

    pub(crate) fn of(metadata: &fs::Metadata) -> Self {
        let (modified_secs, modified_nanos) = match metadata.modified() {
            Ok(time) => match time.duration_since(UNIX_EPOCH) {
                Ok(after) => (
                    i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
                    after.subsec_nanos(),
                ),
                Err(before) => (
                    -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
                    before.duration().subsec_nanos(),
                ),
            },
            Err(_) => (0, 0),
        };
        Self {
            bytes: metadata.len(),
            modified_secs,
            modified_nanos,
        }
    }
}
