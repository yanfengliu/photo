//! Export: render a photo through the develop pipeline and write it to disk
//! as JPEG, PNG or TIFF, at the requested size and colour space, named by a
//! template, with an embedded ICC profile, metadata, optional output
//! sharpening and an optional text watermark. The only crate that writes
//! image files.
//!
//! [`export`] writes one photo; [`planned_path`] says, without writing,
//! where it would go. Both read [`ExportSettings`], which an export preset
//! stores.
//!
//! # How a picture is made
//!
//! The size comes from the photo's full-resolution output as
//! [`photo_develop::output_size`] gives it (the crop and rotation are
//! decided there, once), scaled by [`Sizing`]. The picture is rendered
//! directly at that size: one pipeline view of the whole output at the
//! target pixel count, never a full-size render shrunk afterwards. The
//! pipeline's resampler reads the pyramid level at or above the view's
//! scale and filters it with a kernel as wide as a view pixel, which is
//! already an antialiasing reduction; supersampling (rendering at twice the
//! size and averaging) was measured against it on a zone plate and the
//! test scene and gave more aliasing, not less, at four times the cost, so
//! it is not used.
//!
//! The view is rendered in horizontal strips of about two megapixels
//! (contract R1 makes them join exactly), output sharpening and the
//! watermark are applied per strip, and the strips go to the encoder as
//! they are made: JPEG and TIFF never hold more than one strip; PNG, whose
//! encoder takes the whole picture, holds the picture once at its output
//! depth (and the `image` encoder copies a 16-bit picture once more to put
//! its bytes in PNG order).
//!
//! # Safety of the originals
//!
//! An export never opens the original for writing. It writes a temporary
//! file in the destination folder and gives it its name only when it is
//! complete; on failure or cancellation the temporary file is removed. A
//! destination that is the original itself is never written: the original
//! is recognised by file identity taken when the export starts, so a hard
//! link, a name differing in case, or the original moved onto the
//! destination meanwhile all count. The overwrite policy refuses it, the
//! unique-name policy moves on to the next name and the skip policy skips
//! it; those two never replace any file, with or without hard links.
//!
//! See `docs/work/12_lightroom-workflow-restart/design.md` (§5.8, §5.9,
//! §8.4).

#![warn(missing_docs)]

mod encode;
mod font;
mod icc;
mod ifd;
mod item;
mod metadata;
mod naming;
mod place;
mod settings;
mod sharpen;
mod sizing;
mod strips;
mod watermark;
mod xmp;

use std::fs;
use std::path::PathBuf;

use photo_develop::{CancelToken, Source};

pub use item::{ExportAttributes, ExportError, ExportItem, ExportOutcome};
pub use settings::{
    Anchor, BitDepth, ExistingFile, ExportSettings, ExtensionCase, Folder, Format, Location,
    MetadataMode, MetadataSettings, Naming, OutputSharpening, Resize, Sizing, Watermark,
};

use crate::metadata::Metadata;
use crate::naming::{NamePlan, occupied};
use crate::place::{Original, Placed, TempFile};
use crate::sharpen::Sharpener;
use crate::strips::Producer;
use crate::watermark::WatermarkLayer;

/// Renders `item` and writes it as `settings` say.
///
/// Returns where the file went and its size, or that it was skipped because
/// a file with its name exists and the policy is
/// [`ExistingFile::Skip`]. The original is never modified, and a failed or
/// cancelled export leaves no file behind. `cancel` is checked between
/// strips and while encoding; once it is cancelled the result is
/// [`ExportError::Cancelled`].
pub fn export(
    item: &ExportItem,
    settings: &ExportSettings,
    cancel: &CancelToken,
) -> Result<ExportOutcome, ExportError> {
    // Captured first, so the original is recognised even if it is moved or
    // renamed while the export runs.
    let original = Original::capture(item.source_path);
    let plan = NamePlan::new(item, settings);
    let policy = settings.location.existing;
    let first = match policy {
        ExistingFile::UniqueName => plan.first_free(1),
        ExistingFile::Skip | ExistingFile::Overwrite => 1,
    };
    let target = plan.candidate(first);
    match policy {
        ExistingFile::Skip if occupied(&target) => {
            return Ok(ExportOutcome::Skipped { path: target });
        }
        ExistingFile::Overwrite
            if original
                .is(&target)
                .map_err(|error| ExportError::io(&target, error))? =>
        {
            return Err(ExportError::WouldOverwriteSource(target));
        }
        _ => {}
    }
    proceed(cancel)?;

    let decoded;
    let source = match item.source {
        Some(source) => source,
        None => {
            decoded = Source::new(photo_decode::decode(item.source_path)?.image);
            &decoded
        }
    };
    proceed(cancel)?;
    let output = photo_develop::output_size(source, item.settings);
    let size = sizing::target_size(output, &settings.sizing)?;
    sizing::check_format(settings.format, size)?;
    let metadata = Metadata::new(item, settings);
    let profile = icc::profile(settings.color_space);
    let watermark = settings
        .watermark
        .as_ref()
        .and_then(|watermark| WatermarkLayer::new(watermark, size, settings.color_space));
    let mut pixels = Producer::new(
        source,
        item.settings,
        settings.color_space,
        size,
        Sharpener::new(settings.sharpening),
        watermark,
        cancel,
    );

    fs::create_dir_all(&plan.folder).map_err(|error| ExportError::io(&plan.folder, error))?;
    let mut temporary = TempFile::create(&plan.folder)?;
    encode::write(
        settings.format,
        temporary.file(),
        &target,
        &mut pixels,
        &profile,
        &metadata,
        cancel,
    )?;
    proceed(cancel)?;
    match temporary.place(&plan, first, policy, &original)? {
        Placed::At(path) => Ok(ExportOutcome::Written {
            path,
            width: size.0,
            height: size.1,
        }),
        Placed::Skipped(path) => Ok(ExportOutcome::Skipped { path }),
    }
}

/// Where [`export`] would write `item` right now: the template's name in
/// the chosen folder, with the numeric suffix the unique-name policy would
/// add for the files that exist at this moment. Under the skip and
/// overwrite policies it is the plain name, which [`export`] then skips or
/// replaces (or refuses, when that file is the original). Nothing is
/// created, decoded or rendered.
pub fn planned_path(item: &ExportItem, settings: &ExportSettings) -> PathBuf {
    let plan = NamePlan::new(item, settings);
    match settings.location.existing {
        ExistingFile::UniqueName => plan.candidate(plan.first_free(1)),
        ExistingFile::Skip | ExistingFile::Overwrite => plan.candidate(1),
    }
}

fn proceed(cancel: &CancelToken) -> Result<(), ExportError> {
    if cancel.is_cancelled() {
        Err(ExportError::Cancelled)
    } else {
        Ok(())
    }
}
