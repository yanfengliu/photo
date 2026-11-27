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
//! (contract R1 makes them join without seams: within 1/255 of a single
//! render, and in practice bit for bit or one 16-bit level apart at a
//! strip's edge), output sharpening and the
//! watermark are applied per strip, and the strips go to the encoder as
//! they are made: JPEG and TIFF never hold more than one strip; PNG, whose
//! encoder takes the whole picture, holds the picture once at its output
//! depth (and the `image` encoder copies a 16-bit picture once more to put
//! its bytes in PNG order).
//!
//! # Safety of the originals
//!
//! An export never opens the original for writing. It writes a hidden
//! temporary file in the destination folder and gives it its name only
//! when it is complete; on failure or cancellation the temporary file is
//! removed.
//!
//! Guaranteed, whatever other programs do in the folder while the export
//! runs (the `place` module; its tests have another program act before
//! each step in turn, once per run, and pass on APFS and, with `TMPDIR` set
//! there, on FAT32 and exFAT volumes under macOS; nothing here has been run
//! on Windows):
//!
//! - **Skip and unique name** never replace or remove a file the export
//!   did not create. The name is taken by a hard link or an exclusive
//!   rename, which fail when the name is taken, so a reader never sees an
//!   unfinished file under the final name. On a file system with neither
//!   (measured: FAT32 and exFAT under macOS; some network shares may be
//!   alike) the name is created exclusively and the bytes are copied into
//!   it: still nothing is replaced, but every export is written twice
//!   there and a reader can see the file grow under its final name. The
//!   unique-name policy fails, rather than going on without end, when a
//!   hundred names in a row are each taken in the instant before it can
//!   have them. An identity that cannot be read while the copy is checked
//!   is an error at that name: never "skipped", never a reason to try the
//!   next name.
//! - **Overwrite** replaces what has the name, except the original photo
//!   and the files in [`ExportItem::protected`]. Those are recognised by
//!   file identity, as they were when the export started and as their
//!   paths lead when the file is replaced, so a hard link, a symbolic
//!   link, a name differing in case, a file moved onto the destination
//!   meanwhile and a folder that was away and has come back all count.
//!   The files are kept open and their identity is read afresh at every
//!   comparison, because on FAT and exFAT a file's identity changes over
//!   its life (with its first bytes, with a rename, with being rewritten).
//!   The check and the replacement are not two steps on a name: what has
//!   the name is moved aside in one rename, identified, and removed only
//!   once the export has the name, which it takes without replacing, as
//!   under the other policies. The export is refused with
//!   [`ExportError::WouldOverwriteSource`] or
//!   [`ExportError::WouldOverwriteProtected`]: before anything is rendered
//!   when the guarded file has the name as the export starts, and without
//!   moving it when it has the name as the export comes to place its file.
//!   Only a guarded file that takes the name in the instant after that
//!   last look is moved aside with it; it is then put back. What cannot be
//!   told apart is refused in the same way, the doubt going to the guarded
//!   file: a file at the name that cannot be opened to be identified; any
//!   file at all when the original or a protected path leads to a file
//!   that cannot be opened ([`ExportError::CannotIdentify`]; when that is
//!   so as the export starts, before anything is rendered or decoded, even
//!   if nothing has the name yet, since that file could be moved there); a
//!   comparison whose identities cannot be read; and a file moved aside
//!   from the very place a guarded path names (the same folder and the
//!   same name, ignoring case), which on a file system that tells such
//!   names apart can be another file. A path that leads to no file at all
//!   (an original on a drive that is away, rendered from memory) refuses
//!   nothing.
//! - **A failure loses nothing and hides nothing.** A failed, refused or
//!   cancelled export removes the files it created and nothing else (a
//!   destination folder it had to make stays). A file it had moved aside
//!   is put back without replacing anything: under its own name or, when
//!   that cannot be had (another program has taken it meanwhile, there is
//!   no room to copy it back where it has to be copied, the move fails),
//!   renamed to a visible name beside it (`name-kept.ext`, then
//!   `name-kept-2.ext` and so on). The error gives that path: the refusal
//!   itself for the original or a protected file, [`ExportError::Kept`]
//!   for any other. A file the export replaced and cannot remove is
//!   renamed beside it in the same way, and the log says where.
//! - **Hidden names are the export's own.** They are
//!   `.photo-export-<process>-<random>.<role>`, sixty-four random bits
//!   after the process id, so another exporter writing into the same
//!   folder, even one on another machine with the same process id, does
//!   not come to use them (short of a one in 2^64 chance for a pair of
//!   names). On Apple's systems the move aside also refuses
//!   a hidden name that is taken, and tries another; elsewhere, and on a
//!   volume without an exclusive rename, it is a plain rename, and the
//!   random bits alone keep the names apart.
//!
//! Not guaranteed:
//!
//! - Another program writing to this export's own hidden names is out of
//!   scope: a file put under such a name can be replaced or removed.
//! - Under the overwrite policy every other file with the export's name is
//!   replaced, one that another program put there a moment before
//!   included: only the original and the protected files are guarded.
//! - A protected path protects the file it leads to when the export starts
//!   and the file it leads to when the export identifies what has its
//!   name. A path that only begins to lead to that file later does not
//!   protect it: nothing then tells the file from any other. A path that
//!   reaches the file only through the export's own name (a symbolic link
//!   to that name, say) must lead there by the export's last look at the
//!   name, because moving the file aside breaks such a path.
//! - A comparison reads two identities one after the other. On exFAT,
//!   where rewriting a file changes its identity, a guarded file that
//!   another program moves onto the export's name and rewrites between
//!   those two reads can pass for another file.
//! - An original or protected file that takes the export's name in the
//!   instant between the export's last look at the name and its move is
//!   moved aside with it: until it is put back it is whole, but under a
//!   hidden name. A process that stops there, or between any other two
//!   steps (between moving a file aside and putting it back, say), leaves
//!   the hidden files that `place` lists step by step, and nothing picks
//!   them up later. It never leaves a file both gone and not yet replaced.
//! - A file that can neither go back under its name nor be renamed beside
//!   it (the folder can no longer be written, or the ninety-nine names
//!   tried beside it are all taken) stays whole under its hidden name,
//!   which the error then gives. On a file system with neither hard links
//!   nor exclusive renames the rename to a `-kept` name is a plain rename
//!   onto a name that nothing had a moment before: a file another program
//!   creates under that very name in that instant is replaced.
//! - Where the export is copied to its name, the copy is removed after a
//!   failure only once it is shown to be the export's. When no identity can
//!   be read at all (no descriptor left, say) the finished copy stays under
//!   the export's name, and the export still fails; when one cannot be read
//!   just after the copy was moved aside, the copy is put back like a file
//!   of somebody else's.
//! - On a file system with neither hard links nor exclusive renames, a
//!   file that is put back comes back as a new file with the same bytes,
//!   modification time and permissions (a symbolic link is made anew and
//!   leads where it led), so what another program writes to the old file
//!   through a handle it holds open is not in it. A file renamed beside its
//!   name is the file itself. On Windows without hard links (FAT, exFAT) a
//!   file that is put back or renamed beside its name has its attributes
//!   (read-only, hidden, system) cleared, which the exclusive rename used
//!   there does.
//! - Power loss, as opposed to this process stopping: an export is not
//!   synced to the disk before it takes its name (only a copy made where
//!   there is neither move is), so a file reported as written can be
//!   incomplete afterwards, and under the overwrite policy the file it
//!   replaced is gone by then.
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
mod template;
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
use crate::place::{Guarded, Placed, TempFile};
use crate::sharpen::Sharpener;
use crate::strips::Producer;
use crate::watermark::WatermarkLayer;

/// Renders `item` and writes it as `settings` say.
///
/// Returns where the file went and its size, or that it was skipped because
/// a file with its name exists and the policy is
/// [`ExistingFile::Skip`]. The original is never modified, neither it nor
/// a [protected](ExportItem::protected) file is ever replaced, and a
/// failed or cancelled export leaves no file behind (see the crate
/// documentation for exactly what is guaranteed). `cancel` is checked between
/// strips and while encoding; once it is cancelled the result is
/// [`ExportError::Cancelled`].
pub fn export(
    item: &ExportItem,
    settings: &ExportSettings,
    cancel: &CancelToken,
) -> Result<ExportOutcome, ExportError> {
    // Captured first, so the original and the protected files are
    // recognised even if they are moved or renamed while the export runs.
    let guarded = Guarded::capture(item.source_path, item.protected);
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
        ExistingFile::Overwrite => {
            // Refused before anything is rendered: an original or a
            // protected file that cannot be told from what has the name,
            // or that has it.
            guarded.readable().map_err(|doubt| doubt.error(&target))?;
            if let Some(guard) = guarded
                .guards(&target)
                .map_err(|doubt| doubt.error(&target))?
            {
                return Err(guard.refusal(target));
            }
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
    match temporary.place(&plan, first, policy, &guarded)? {
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
