//! The dialogs (design §8.4): Import; new collection, collection set and
//! smart collection; editing a smart collection's rules; rename; the
//! confirmations before deleting a collection or removing photos from the
//! catalog; and Develop's Copy Settings, Synchronize Settings, New Preset
//! and snapshot dialogs. At most one is open at a time, and while one is
//! open the keyboard shortcuts are off.

mod collections;
mod develop;
mod import;
mod import_grid;
mod smart;

use photo_core::meta::{CollectionId, PresetId, SnapshotId};
use photo_engine::{CollectionInfo, CollectionKind, Target};

use self::collections::{DeleteDialog, NameDialog, RemoveDialog};
use self::develop::{
    DeleteDialog as DeleteLookDialog, DevelopDialog, Doomed, GroupsDialog, GroupsPurpose,
    PresetDialog, SnapshotDialog, USER_GROUP,
};
use self::import::ImportDialog;
pub(crate) use self::import::ImportSettings;
use self::smart::SmartDialog;
use super::Cx;

/// What a name dialog makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NewKind {
    Collection,
    Set,
}

/// The dialog that is open.
pub(crate) enum Dialog {
    Import(Box<ImportDialog>),
    Name(NameDialog),
    Smart(Box<SmartDialog>),
    Delete(DeleteDialog),
    Remove(RemoveDialog),
    Develop(Box<DevelopDialog>),
}

/// Show the open dialog, if any, and act on its answer.
pub(crate) fn show(ctx: &egui::Context, cx: &mut Cx<'_>) {
    let Some(mut dialog) = cx.ui.dialog.take() else {
        return;
    };
    let open = match &mut dialog {
        Dialog::Import(dialog) => dialog.show(ctx, cx),
        Dialog::Name(dialog) => dialog.show(ctx, cx),
        Dialog::Smart(dialog) => dialog.show(ctx, cx),
        Dialog::Delete(dialog) => dialog.show(ctx, cx),
        Dialog::Remove(dialog) => dialog.show(ctx, cx),
        Dialog::Develop(dialog) => dialog.show(ctx, cx),
    };
    if open && cx.ui.dialog.is_none() {
        cx.ui.dialog = Some(dialog);
    }
}

/// Open the Import dialog with the settings it had last time.
pub(crate) fn open_import(cx: &mut Cx<'_>) {
    let dialog = ImportDialog::open(cx);
    cx.ui.dialog = Some(Dialog::Import(Box::new(dialog)));
}

/// Open the New Collection dialog, inside the set `parent`.
pub(crate) fn open_new_collection(cx: &mut Cx<'_>, parent: Option<CollectionId>) {
    open_new(cx, NewKind::Collection, parent);
}

/// Open the dialog that makes a collection or a set inside `parent`.
pub(crate) fn open_new(cx: &mut Cx<'_>, kind: NewKind, parent: Option<CollectionId>) {
    cx.ui.dialog = Some(Dialog::Name(NameDialog::new(kind, parent)));
}

/// Open the New Smart Collection dialog, inside the set `parent`.
pub(crate) fn open_new_smart(cx: &mut Cx<'_>, parent: Option<CollectionId>) {
    cx.ui.dialog = Some(Dialog::Smart(Box::new(SmartDialog::new(parent))));
}

/// Open the rule editor of the smart collection `id`.
pub(crate) fn open_edit_smart(cx: &mut Cx<'_>, id: CollectionId) {
    match cx.engine.smart_rules(id) {
        Ok(rules) => {
            let name = cx
                .engine
                .collections()
                .iter()
                .find(|collection| collection.id == id)
                .map(|collection| collection.name.clone())
                .unwrap_or_default();
            let dialog = SmartDialog::edit(id, name, &rules);
            cx.ui.dialog = Some(Dialog::Smart(Box::new(dialog)));
        }
        Err(error) => cx.say(format!("Could not read the rules: {error}")),
    }
}

/// Open the Rename dialog for `collection`.
pub(crate) fn open_rename(cx: &mut Cx<'_>, collection: &CollectionInfo) {
    cx.ui.dialog = Some(Dialog::Name(NameDialog::rename(collection)));
}

/// Ask before deleting `collection`.
pub(crate) fn open_delete(cx: &mut Cx<'_>, collection: &CollectionInfo) {
    cx.ui.dialog = Some(Dialog::Delete(DeleteDialog::new(collection)));
}

/// Ask before removing the photos `target` stands for from the catalog.
pub(crate) fn open_remove_photos(cx: &mut Cx<'_>, target: Target) {
    let count = match &target {
        Target::Active => usize::from(cx.engine.active_photo().is_some()),
        Target::Selected => cx
            .engine
            .selection()
            .selected()
            .len()
            .max(usize::from(cx.engine.active_photo().is_some())),
        Target::Photos(photos) => photos.len(),
    };
    if count > 0 {
        cx.ui.dialog = Some(Dialog::Remove(RemoveDialog::new(target, count)));
    }
}

fn open_develop(cx: &mut Cx<'_>, dialog: DevelopDialog) {
    cx.ui.dialog = Some(Dialog::Develop(Box::new(dialog)));
}

/// Shift+Cmd+C: choose which settings of the active photo to copy.
pub(crate) fn open_copy_settings(cx: &mut Cx<'_>) {
    if cx.engine.active_photo().is_none() {
        cx.say("Select a photo to copy its settings.");
        return;
    }
    let groups = cx.ui.develop.copy_groups;
    open_develop(
        cx,
        DevelopDialog::Groups(GroupsDialog::new(GroupsPurpose::Copy, groups)),
    );
}

/// Choose which settings of the active photo the other selected photos
/// take.
pub(crate) fn open_sync(cx: &mut Cx<'_>) {
    let groups = cx.ui.develop.copy_groups;
    open_develop(
        cx,
        DevelopDialog::Groups(GroupsDialog::new(GroupsPurpose::Sync, groups)),
    );
}

/// Save settings of the open photo as a preset.
pub(crate) fn open_new_preset(cx: &mut Cx<'_>) {
    let purpose = GroupsPurpose::NewPreset {
        name: String::new(),
        group: USER_GROUP.to_owned(),
    };
    let groups = photo_core::develop::GroupSet::default_copy();
    open_develop(
        cx,
        DevelopDialog::Groups(GroupsDialog::new(purpose, groups)),
    );
}

/// Give a user preset a new name and group.
pub(crate) fn open_rename_preset(cx: &mut Cx<'_>, id: PresetId, group: &str, name: &str) {
    open_develop(
        cx,
        DevelopDialog::RenamePreset(PresetDialog::new(id, group, name)),
    );
}

/// Ask before deleting a user preset.
pub(crate) fn open_delete_preset(cx: &mut Cx<'_>, id: PresetId, name: &str) {
    open_develop(
        cx,
        DevelopDialog::Delete(DeleteLookDialog::new(Doomed::Preset(id), name)),
    );
}

/// Cmd+N in Develop: name a snapshot of the open photo.
pub(crate) fn open_new_snapshot(cx: &mut Cx<'_>) {
    let Some(photo) = cx.engine.develop().map(|session| session.photo()) else {
        cx.say("Open a photo in Develop to take a snapshot of it.");
        return;
    };
    let existing = cx.ui.develop.lists.snapshots.len();
    open_develop(
        cx,
        DevelopDialog::Snapshot(SnapshotDialog::new(photo, existing)),
    );
}

/// Give a snapshot of the open photo a new name.
pub(crate) fn open_rename_snapshot(cx: &mut Cx<'_>, id: SnapshotId, name: &str) {
    let Some(photo) = cx.engine.develop().map(|session| session.photo()) else {
        return;
    };
    open_develop(
        cx,
        DevelopDialog::Snapshot(SnapshotDialog::rename(photo, id, name)),
    );
}

/// Ask before deleting a snapshot.
pub(crate) fn open_delete_snapshot(cx: &mut Cx<'_>, id: SnapshotId, name: &str) {
    open_develop(
        cx,
        DevelopDialog::Delete(DeleteLookDialog::new(Doomed::Snapshot(id), name)),
    );
}

/// The sets a collection can be put in, with their names, for a parent
/// menu.
fn sets(cx: &Cx<'_>) -> Vec<(CollectionId, String)> {
    cx.engine
        .collections()
        .iter()
        .filter(|collection| collection.kind == CollectionKind::Set)
        .map(|collection| (collection.id, collection.name.clone()))
        .collect()
}

/// A menu of where to put a new collection: the top level or a set.
fn parent_menu(ui: &mut egui::Ui, cx: &Cx<'_>, parent: &mut Option<CollectionId>) {
    let sets = sets(cx);
    let shown = parent
        .and_then(|id| sets.iter().find(|(set, _)| *set == id))
        .map_or("(top level)", |(_, name)| name.as_str())
        .to_owned();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Inside").color(crate::theme::palette::TEXT_DIM));
        egui::ComboBox::from_id_salt("parent-set")
            .selected_text(shown)
            .width(240.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(parent, None, "(top level)");
                for (id, name) in &sets {
                    ui.selectable_value(parent, Some(*id), name.as_str());
                }
            })
            .response
            .accessible_name("Inside set");
    });
}
