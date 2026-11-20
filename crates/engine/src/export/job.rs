//! One photo's export, as a worker runs it, and the toast that sums up an
//! export.

use std::fs;
use std::path::PathBuf;

use photo_core::develop::DevelopSettings;
use photo_core::meta::{PhotoId, PhotoMetadata};
use photo_develop::{CancelToken, Source};
use photo_export::{ExportAttributes, ExportError, ExportItem, ExportOutcome, ExportSettings};

use super::guard::Guard;
use crate::sources::Sources;
use crate::stamp::Stamp;

/// One photo to export, with everything its job needs.
pub(super) struct Pending {
    pub(super) photo: PhotoId,
    pub(super) sequence: u32,
    pub(super) path: PathBuf,
    pub(super) settings: DevelopSettings,
    pub(super) metadata: PhotoMetadata,
    pub(super) attributes: ExportAttributes,
}

impl Pending {
    /// The export crate's view of the photo, rendered from `source` when
    /// it is given.
    pub(super) fn item<'a>(&'a self, source: Option<&'a Source>) -> ExportItem<'a> {
        ExportItem {
            source_path: &self.path,
            source,
            settings: &self.settings,
            metadata: &self.metadata,
            attributes: self.attributes.clone(),
            sequence: self.sequence,
        }
    }
}

/// Why a photo's job wrote nothing.
pub(super) enum Failure {
    /// The export was cancelled first.
    Cancelled,
    /// It failed, for this reason.
    Reason(String),
}

/// Export one photo on a worker, from its decoded source when one is in
/// memory, checking the guard before and after.
pub(super) fn export_one(
    pending: &Pending,
    settings: &ExportSettings,
    guard: &Guard,
    sources: &Sources,
    renders: &CancelToken,
) -> Result<ExportOutcome, Failure> {
    let source = Stamp::read(&pending.path)
        .ok()
        .and_then(|stamp| sources.cached(&pending.path, stamp));
    let item = pending.item(source.as_ref());
    let planned = photo_export::planned_path(&item, settings);
    if let Some(reason) = guard.refusal(&planned) {
        return Err(Failure::Reason(reason));
    }
    let outcome = photo_export::export(&item, settings, renders).map_err(|error| match error {
        ExportError::Cancelled => Failure::Cancelled,
        other => Failure::Reason(other.to_string()),
    })?;
    // Another export may have taken the planned name while this one
    // rendered, so the file went under the next free name: check that too.
    if let ExportOutcome::Written { path, .. } = &outcome
        && let Some(reason) = guard.refusal(path)
    {
        if let Err(error) = fs::remove_file(path) {
            log::error!(
                "cannot remove the refused export {}: {error}",
                path.display()
            );
        }
        return Err(Failure::Reason(reason));
    }
    Ok(outcome)
}

/// The toast at the end of an export.
pub(super) fn summary(
    written: usize,
    skipped: usize,
    failed: usize,
    total: usize,
    cancelled: bool,
) -> String {
    let photos = |count: usize| if count == 1 { "photo" } else { "photos" };
    if cancelled {
        return format!(
            "Export cancelled: {written} of {total} {} exported",
            photos(total)
        );
    }
    if written == total {
        return format!("Exported {written} {}", photos(written));
    }
    let mut message = format!("Exported {written} of {total} {}", photos(total));
    if failed > 0 {
        message.push_str(&format!(", {failed} failed"));
    }
    if skipped > 0 {
        message.push_str(&format!(", {skipped} skipped (a file of that name exists)"));
    }
    message
}

#[cfg(test)]
mod tests {
    use super::summary;

    #[test]
    fn the_summary_counts_what_was_written_and_what_was_not() {
        assert_eq!(summary(12, 0, 0, 12, false), "Exported 12 photos");
        assert_eq!(summary(1, 0, 0, 1, false), "Exported 1 photo");
        assert_eq!(
            summary(10, 0, 2, 12, false),
            "Exported 10 of 12 photos, 2 failed"
        );
        assert_eq!(
            summary(9, 1, 2, 12, false),
            "Exported 9 of 12 photos, 2 failed, 1 skipped (a file of that name exists)"
        );
        assert_eq!(
            summary(3, 0, 0, 5, true),
            "Export cancelled: 3 of 5 photos exported"
        );
    }
}
