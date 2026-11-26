//! The overwrite policy: the export replaces what has its name, but never
//! the original photo or a protected file, whatever another program does
//! in the meantime.
//!
//! Looking at the name and then renaming onto it would be two steps on a
//! name, and the file could be exchanged between them. Instead:
//!
//! 1. The export tries to take the name without replacing ([`move_new`]).
//!    When nothing has the name, that is all.
//! 2. Otherwise what has the name is moved aside under a hidden name in the
//!    same folder. That is one rename, so what is aside is exactly what had
//!    the name.
//! 3. What is aside is identified. If it is the original or a protected
//!    file it is put back and the export is refused. Otherwise it stays
//!    aside and the export tries step 1 again; something new that has
//!    appeared at the name meanwhile is unverified, so it is moved aside
//!    and identified too, [`ROUNDS`] times at most.
//! 4. Only once the export has the name is what was moved aside removed.
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
use super::moves::{folder_of, hidden_name, move_new, put_back};
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

/// Steps 1 to 3 of the module documentation. Files moved aside and found
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
        // A folder is nothing an export replaces; better not to move it.
        if fs::symlink_metadata(name).is_ok_and(|meta| meta.is_dir()) {
            return Err(failed(io::Error::new(
                io::ErrorKind::IsADirectory,
                "a folder has the name of the export",
            )));
        }
        disk.reached(Step::Look);
        let seen = guarded.look();
        let hidden = hidden_name(folder_of(name), "previous");
        disk.reached(Step::SetAside);
        match fs::rename(name, &hidden) {
            Ok(()) => {}
            // It left by itself; the name may be free now.
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(failed(error)),
        }
        disk.reached(Step::Identify);
        match guarded.identify(&hidden, &seen, name) {
            Ok(None) => aside.push(hidden),
            Ok(Some(guard)) => return Err(guard.refusal(put_back(disk, &hidden, name))),
            Err(error) => return Err(ExportError::io(&put_back(disk, &hidden, name), error)),
        }
    }
    if move_new(disk, finished, name).map_err(failed)? {
        return Ok(());
    }
    Err(failed(io::Error::other(
        "other files kept taking the name of the export",
    )))
}
