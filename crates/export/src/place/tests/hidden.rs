//! Hidden names in a folder that other exporters write into too: each
//! exporter's are its own, so moving a file aside never replaces another
//! exporter's file (review F57).

use std::fs;

use super::{Folder, Kind, Staged, kinds, nothing_guarded};
use crate::place::Placed;
use crate::place::aside::EXCLUSIVE_ASIDE;
use crate::place::disk::Step;
use crate::settings::ExistingFile::Overwrite;

/// Review finding 6, as the report tells it: an exporter on another
/// machine with this process's id, which has made as many names as this
/// one, writes into the same shared folder. It makes its hidden name in
/// the instant this export makes its own, and moves a file of its own
/// there before this export moves the previous file aside. Their file is
/// not replaced, on every kind of file system: where the move aside is a
/// plain rename, only the names' random bits keep the two apart, and with a
/// process id and a count alone they would be the same name.
#[test]
fn an_exporter_with_this_process_id_and_count_keeps_its_hidden_file() {
    for kind in kinds() {
        let folder = Folder::new();
        folder.put("x.jpg", b"previous");
        let mut quiet = |_: usize, _: Step| {};
        let mut disk = Staged::new(kind, &mut quiet);
        disk.rival = Some(2);
        let result = folder.place_on(&mut disk, b"export", Overwrite, &nothing_guarded(&folder));
        let theirs = disk.rival_file.clone().expect("no hidden name was made");
        let context = format!("{kind:?}: {:?}", folder.names());
        assert_eq!(result.unwrap(), Placed::At(folder.at("x.jpg")), "{context}");
        assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{context}");
        assert_eq!(
            fs::read(&theirs).ok().as_deref(),
            Some(&b"theirs"[..]),
            "{context}: the other exporter's file was replaced"
        );
        let theirs_name = theirs.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(folder.hidden(), [theirs_name], "{context}");
    }
}

/// Where the move aside is the rename that cannot replace, even the very
/// name this export is about to use, taken by another exporter's file, is
/// refused, and another one used: the other exporter's file under it is
/// not replaced.
#[test]
fn a_hidden_name_another_exporter_has_is_not_replaced() {
    for kind in kinds() {
        if !(EXCLUSIVE_ASIDE && matches!(kind, Kind::Both | Kind::ExclusiveOnly)) {
            continue;
        }
        let folder = Folder::new();
        folder.put("x.jpg", b"previous");
        // Another exporter's file, moved aside under the very name this
        // export is about to use.
        let theirs = folder.put(".photo-export-7-00000000000000aa.previous", b"theirs");
        let mut quiet = |_: usize, _: Step| {};
        let mut disk = Staged::new(kind, &mut quiet);
        disk.hidden = vec![theirs.clone()];
        let result = folder.place_on(&mut disk, b"export", Overwrite, &nothing_guarded(&folder));
        assert_eq!(result.unwrap(), Placed::At(folder.at("x.jpg")), "{kind:?}");
        assert_eq!(folder.read("x.jpg").unwrap(), b"export", "{kind:?}");
        assert_eq!(
            fs::read(&theirs).ok().as_deref(),
            Some(&b"theirs"[..]),
            "{kind:?}: the other exporter's file was replaced"
        );
        assert_eq!(folder.hidden().len(), 1, "{kind:?}: {:?}", folder.names());
    }
}
