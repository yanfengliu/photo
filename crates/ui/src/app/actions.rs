//! What the user asked for, as values. While a frame is drawn every control
//! that is used queues an [`Action`]; once the frame is laid out the app
//! hands each to the engine, one engine call per action (see
//! [`Action::apply`]). Nothing here decides anything the engine owns.

use std::path::PathBuf;

use photo_core::meta::{CollectionId, ColorLabel, Flag, PhotoId, Rating};
use photo_engine::{
    Engine, EngineError, Filter, ImportRequest, Movement, SmartRules, Sort, Source, Target, TaskId,
    TextField,
};

/// One thing the user asked for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    SelectOnly(PhotoId),
    ToggleSelected(PhotoId),
    ExtendSelection(PhotoId),
    SelectAll,
    SelectNone,
    MoveActive { movement: Movement, extend: bool },
    /// Make a photo the active one and keep the rest of the selection.
    MakeActive(PhotoId),
    SetSource(Source),
    SetFilter(Filter),
    SetSort(Sort),
    SetRating(Target, Rating),
    IncreaseRating(Target),
    DecreaseRating(Target),
    SetFlag(Target, Flag),
    ToggleLabel(Target, ColorLabel),
    ToggleQuickCollection(Target),
    RotateLeft(Target),
    RotateRight(Target),
    CreateVirtualCopies(Target),
    RemoveFromCatalog(Target),
    SetText(Target, TextField, String),
    AddKeywords(Target, Vec<String>),
    RemoveKeywords(Target, Vec<String>),
    AddToCollection(CollectionId, Target),
    RemoveFromCollection(CollectionId, Target),
    CreateCollection {
        name: String,
        parent: Option<CollectionId>,
    },
    CreateCollectionSet {
        name: String,
        parent: Option<CollectionId>,
    },
    CreateSmartCollection {
        name: String,
        parent: Option<CollectionId>,
        rules: SmartRules,
    },
    SetSmartRules(CollectionId, SmartRules),
    RenameCollection(CollectionId, String),
    DeleteCollection(CollectionId),
    Undo,
    Redo,
    ScanFolder {
        root: PathBuf,
        recursive: bool,
    },
    StartImport(ImportRequest),
    CancelTask(TaskId),
}

impl Action {
    /// Hand the action to the engine.
    ///
    /// Each action is one engine call, with one exception the engine has no
    /// call for: making an already selected photo active without changing
    /// the selection takes it out and puts it back in (two toggles).
    pub(crate) fn apply(self, engine: &mut Engine) -> Result<(), EngineError> {
        match self {
            Self::SelectOnly(photo) => {
                engine.select_only(photo);
            }
            Self::ToggleSelected(photo) => {
                engine.toggle_selected(photo);
            }
            Self::ExtendSelection(photo) => {
                engine.extend_selection_to(photo);
            }
            Self::SelectAll => engine.select_all(),
            Self::SelectNone => engine.select_none(),
            Self::MoveActive { movement, extend } => {
                engine.move_active(movement, extend);
            }
            Self::MakeActive(photo) => make_active(engine, photo),
            Self::SetSource(source) => engine.set_source(source)?,
            Self::SetFilter(filter) => engine.set_filter(filter)?,
            Self::SetSort(sort) => engine.set_sort(sort)?,
            Self::SetRating(target, rating) => engine.set_rating(target, rating)?,
            Self::IncreaseRating(target) => engine.increase_rating(target)?,
            Self::DecreaseRating(target) => engine.decrease_rating(target)?,
            Self::SetFlag(target, flag) => engine.set_flag(target, flag)?,
            Self::ToggleLabel(target, label) => engine.toggle_label(target, label)?,
            Self::ToggleQuickCollection(target) => engine.toggle_quick_collection(target)?,
            Self::RotateLeft(target) => engine.rotate_left(target)?,
            Self::RotateRight(target) => engine.rotate_right(target)?,
            Self::CreateVirtualCopies(target) => {
                engine.create_virtual_copies(target)?;
            }
            Self::RemoveFromCatalog(target) => engine.remove_from_catalog(target)?,
            Self::SetText(target, field, text) => engine.set_text(target, field, &text)?,
            Self::AddKeywords(target, names) => engine.add_keywords(target, &names)?,
            Self::RemoveKeywords(target, names) => engine.remove_keywords(target, &names)?,
            Self::AddToCollection(collection, target) => {
                engine.add_to_collection(collection, target)?;
            }
            Self::RemoveFromCollection(collection, target) => {
                engine.remove_from_collection(collection, target)?;
            }
            Self::CreateCollection { name, parent } => {
                engine.create_collection(&name, parent)?;
            }
            Self::CreateCollectionSet { name, parent } => {
                engine.create_collection_set(&name, parent)?;
            }
            Self::CreateSmartCollection {
                name,
                parent,
                rules,
            } => {
                engine.create_smart_collection(&name, parent, &rules)?;
            }
            Self::SetSmartRules(id, rules) => engine.set_smart_rules(id, &rules)?,
            Self::RenameCollection(id, name) => engine.rename_collection(id, &name)?,
            Self::DeleteCollection(id) => engine.delete_collection(id)?,
            Self::Undo => {
                engine.undo()?;
            }
            Self::Redo => {
                engine.redo()?;
            }
            Self::ScanFolder { root, recursive } => {
                engine.scan_folder(&root, recursive);
            }
            Self::StartImport(request) => {
                engine.start_import(request)?;
            }
            Self::CancelTask(task) => {
                engine.cancel_task(task);
            }
        }
        Ok(())
    }

    /// What to tell the user when the engine refuses the action, before the
    /// engine's own reason.
    pub(crate) fn failure(&self) -> &'static str {
        match self {
            Self::SetSource(_) => "Could not show that source",
            Self::SetFilter(_) => "Could not filter the photos",
            Self::SetSort(_) => "Could not sort the photos",
            Self::SetRating(..) | Self::IncreaseRating(_) | Self::DecreaseRating(_) => {
                "Could not change the rating"
            }
            Self::SetFlag(..) => "Could not change the flag",
            Self::ToggleLabel(..) => "Could not change the colour label",
            Self::ToggleQuickCollection(_) => "Could not change the quick collection",
            Self::RotateLeft(_) | Self::RotateRight(_) => "Could not rotate",
            Self::CreateVirtualCopies(_) => "Could not make a virtual copy",
            Self::RemoveFromCatalog(_) => "Could not remove the photos",
            Self::SetText(..) => "Could not change the metadata",
            Self::AddKeywords(..) | Self::RemoveKeywords(..) => "Could not change the keywords",
            Self::AddToCollection(..) => "Could not add to the collection",
            Self::RemoveFromCollection(..) => "Could not remove from the collection",
            Self::CreateCollection { .. }
            | Self::CreateCollectionSet { .. }
            | Self::CreateSmartCollection { .. } => "Could not create it",
            Self::SetSmartRules(..) => "Could not change the rules",
            Self::RenameCollection(..) => "Could not rename it",
            Self::DeleteCollection(_) => "Could not delete it",
            Self::Undo => "Could not undo",
            Self::Redo => "Could not redo",
            Self::StartImport(_) => "Could not start the import",
            Self::SelectOnly(_)
            | Self::ToggleSelected(_)
            | Self::ExtendSelection(_)
            | Self::SelectAll
            | Self::SelectNone
            | Self::MoveActive { .. }
            | Self::MakeActive(_)
            | Self::ScanFolder { .. }
            | Self::CancelTask(_) => "Could not do that",
        }
    }
}

/// Make `photo` the active photo, keeping every other selected photo
/// selected; a photo that was not selected joins the selection.
fn make_active(engine: &mut Engine, photo: PhotoId) {
    if engine.active_photo() == Some(photo) {
        return;
    }
    if engine.selection().is_selected(photo) {
        engine.toggle_selected(photo);
    }
    // Toggling a photo into the selection makes it the active one.
    engine.toggle_selected(photo);
}
