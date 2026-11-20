//! The Library's dialogs (design §8.4): Import; new collection, collection
//! set and smart collection; editing a smart collection's rules; rename;
//! and the confirmations before deleting a collection or removing photos
//! from the catalog. At most one is open at a time, and while one is open
//! the Library's shortcuts are off.

mod collections;
mod import;
mod import_grid;
mod smart;

use photo_core::meta::CollectionId;
use photo_engine::{CollectionInfo, CollectionKind, Target};

use self::collections::{DeleteDialog, NameDialog, RemoveDialog};
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
