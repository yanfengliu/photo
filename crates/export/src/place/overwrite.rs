//! The overwrite policy: the export replaces what has its name, but never
//! the original photo or a protected file, whatever another program does
//! in the meantime.
//!
//! Looking at the name and then renaming onto it would be two steps on a
//! name, and the file could be exchanged between them. Instead:
//!
//! 1. The export tries to take the name without replacing ([`move_new`]).
//!    When nothing has the name, that is all.
//! 2. Otherwise the export looks at what has the name. A folder, the
//!    original, a protected file or a file that cannot be identified is
//!    refused where it is, without being moved. This look decides nothing
//!    about what is replaced: it only spares what is plainly not
//!    replaceable the move of the next step.
//! 3. What has the name is moved aside under a hidden name in the same
//!    folder. That is one rename, so what is aside is exactly what had the
//!    name, whether or not it is what step 2 looked at.
//! 4. What is aside is identified. If it is the original or a protected
//!    file (it took the name after the look of step 2) it is put back and
//!    the export is refused. Otherwise it stays aside and the export tries
//!    step 1 again; something new that has appeared at the name meanwhile
//!    is unverified, so it is looked at, moved aside and identified too,
//!    [`ROUNDS`] times at most.
//! 5. Only once the export has the name is what was moved aside removed.
//!    If the export fails instead, everything moved aside is put back.
//!
//! A file is put back without replacing: under its own name when that is
//! still free, otherwise under a visible name beside it, which the error
//! then gives.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::disk::{Disk, Step};
use super::guard::Guarded;
use super::moves::{move_new, put_back, set_aside};
use crate::item::ExportError;

/// How many times something that has the export's name is moved aside and
/// identified before the export gives up.
pub(crate) const ROUNDS: u32 = 4;

/// Gives the finished file at `finished` the name `name`, replacing what
/// has it unless that is a `guarded` file.
///
/// On success the export has the name and whatever had it is gone. On
/// failure nothing is lost: every file that was moved aside is back under
/// `name` or under a visible name beside it, and `finished` is where it
/// was. A refusal carries the path the guarded file now has.
pub(crate) fn replace(
    disk: &mut dyn Disk,
    finished: &Path,
    name: &Path,
    guarded: &Guarded,
) -> Result<(), ExportError> {
    let mut aside = Vec::new();
    let result = take_over(disk, finished, name, guarded, &mut aside);
    match &result {
        Ok(()) => {
            disk.reached(Step::Discard);
            for previous in &aside {
                if let Err(error) = fs::remove_file(previous) {
                    log::warn!("cannot remove the replaced {}: {error}", previous.display());
                }
            }
        }
        Err(_) => {
            // Latest first: it had the name last.
            for previous in aside.iter().rev() {
                let now = put_back(disk, previous, name);
                if now != name {
                    log::warn!("what was {} is now {}", name.display(), now.display());
                }
            }
        }
    }
    result
}

/// Steps 1 to 4 of the module documentation. Files moved aside and found
/// replaceable are pushed onto `aside`; the caller removes them or puts
/// them back.
fn take_over(
    disk: &mut dyn Disk,
    finished: &Path,
    name: &Path,
    guarded: &Guarded,
    aside: &mut Vec<PathBuf>,
) -> Result<(), ExportError> {
    let failed = |error: io::Error| ExportError::io(name, error);
    for _ in 0..ROUNDS {
        if move_new(disk, finished, name).map_err(failed)? {
            return Ok(());
        }
        disk.reached(Step::Look);
        // What can be seen not to be replaceable is refused where it is: a
        // folder, the original, a protected file, a file that cannot be
        // identified. Moving it aside would only put it at risk. The move
        // below is for what takes the name after this look.
        if fs::symlink_metadata(name).is_ok_and(|meta| meta.is_dir()) {
            return Err(failed(io::Error::new(
                io::ErrorKind::IsADirectory,
                "a folder has the name of the export",
            )));
        }
        if let Some(guard) = guarded.guards(name).map_err(|doubt| doubt.error(name))? {
            return Err(guard.refusal(name.to_path_buf()));
        }
        let seen = guarded.look().map_err(|doubt| doubt.error(name))?;
        let Some(hidden) = set_aside(disk, name, "previous").map_err(failed)? else {
            // It left by itself; the name may be free now.
            continue;
        };
        disk.reached(Step::Identify);
        match guarded.identify(&hidden, &seen, name) {
            Ok(None) => aside.push(hidden),
            Ok(Some(guard)) => return Err(guard.refusal(put_back(disk, &hidden, name))),
            Err(doubt) => return Err(doubt.error(&put_back(disk, &hidden, name))),
        }
    }
    if move_new(disk, finished, name).map_err(failed)? {
        return Ok(());
    }
    Err(failed(io::Error::other(
        "other files kept taking the name of the export",
    )))
}
