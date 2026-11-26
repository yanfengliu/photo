//! What the user asked for, as values. While a frame is drawn every control
//! that is used queues an [`Action`]; once the frame is laid out the app
//! hands each to the engine, one engine call per action (see
//! [`Action::apply`]). Nothing here decides anything the engine owns.

use std::path::PathBuf;

use photo_core::meta::{CollectionId, ColorLabel, Flag, KeywordId, PhotoId, Rating};
use photo_engine::{
    Engine, EngineError, ExportSettings, Filter, ImportRequest, Movement, SmartRules, Sort, Source,
    Target, TaskId, TextField,
};

use super::develop::actions::DevelopAction;

/// One thing the user asked for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// A Develop or Quick Develop request.
    Develop(DevelopAction),
    SelectOnly(PhotoId),
    ToggleSelected(PhotoId),
    ExtendSelection(PhotoId),
    SelectAll,
    SelectNone,
    MoveActive {
        movement: Movement,
        extend: bool,
    },
    /// Make a photo the active one and keep the rest of the selection.
    MakeActive(PhotoId),
    SetSource(Source),
    SetFilter(Filter),
    SetSort(Sort),
    SetRating(Target, Rating),
    IncreaseRating(Target),
    DecreaseRating(Target),
    SetFlag(Target, Flag),
    SetLabel(Target, ColorLabel),
    ToggleLabel(Target, ColorLabel),
    ToggleQuickCollection(Target),
    RotateLeft(Target),
    RotateRight(Target),
    CreateVirtualCopies(Target),
    RemoveFromCatalog(Target),
    SetText(Target, TextField, String),
    /// Keywords by name, made if they do not exist.
    AddKeywords(Target, Vec<String>),
    /// Keywords by id: exact where names are shared.
    AddKeywordIds(Target, Vec<KeywordId>),
    RemoveKeywordIds(Target, Vec<KeywordId>),
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
    /// Export the target's photos as the settings say.
    StartExport(Target, Box<ExportSettings>),
    SaveExportPreset(String, Box<ExportSettings>),
    DeleteExportPreset(String),
    CancelTask(TaskId),
}

impl Action {
    /// Hand the action to the engine, one engine call per action.
    pub(crate) fn apply(self, engine: &mut Engine) -> Result<(), EngineError> {
        match self {
            Self::Develop(action) => action.apply(engine)?,
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
            Self::SetLabel(target, label) => engine.set_label(target, label)?,
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
            Self::AddKeywordIds(target, ids) => engine.add_keyword_ids(target, &ids)?,
            Self::RemoveKeywordIds(target, ids) => engine.remove_keyword_ids(target, &ids)?,
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
            Self::StartExport(target, settings) => {
                engine.start_export(target, *settings)?;
            }
            Self::SaveExportPreset(name, settings) => {
                engine.save_export_preset(&name, &settings)?;
            }
            Self::DeleteExportPreset(name) => {
                engine.delete_export_preset(&name)?;
            }
            Self::CancelTask(task) => {
                engine.cancel_task(task);
            }
        }
        Ok(())
    }

    /// Whether what the open Develop tool has changed is recorded as its
    /// own step before the action. It is before one that changes the open
    /// photo's settings from outside the tools (see
    /// `DevelopAction::edits_outside_tools`; undo and redo do too, they put
    /// the catalog's settings back in the session), and before one that
    /// takes the photo as it is saved, so that it takes what is shown: a
    /// virtual copy, an export.
    pub(crate) fn ends_tool_step(&self) -> bool {
        match self {
            Self::Develop(action) => action.edits_outside_tools(),
            Self::Undo | Self::Redo => true,
            Self::CreateVirtualCopies(_) | Self::StartExport(..) => true,
            _ => false,
        }
    }

    /// What to tell the user when the engine refuses the action, before the
    /// engine's own reason.
    pub(crate) fn failure(&self) -> &'static str {
        match self {
            Self::Develop(action) => action.failure(),
            Self::SetSource(_) => "Could not show that source",
            Self::SetFilter(_) => "Could not filter the photos",
            Self::SetSort(_) => "Could not sort the photos",
            Self::SetRating(..) | Self::IncreaseRating(_) | Self::DecreaseRating(_) => {
                "Could not change the rating"
            }
            Self::SetFlag(..) => "Could not change the flag",
            Self::SetLabel(..) | Self::ToggleLabel(..) => "Could not change the colour label",
            Self::ToggleQuickCollection(_) => "Could not change the quick collection",
            Self::RotateLeft(_) | Self::RotateRight(_) => "Could not rotate",
            Self::CreateVirtualCopies(_) => "Could not make a virtual copy",
            Self::RemoveFromCatalog(_) => "Could not remove the photos",
            Self::SetText(..) => "Could not change the metadata",
            Self::AddKeywords(..) | Self::AddKeywordIds(..) | Self::RemoveKeywordIds(..) => {
                "Could not change the keywords"
            }
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
            Self::StartExport(..) => "Could not start the export",
            Self::SaveExportPreset(..) => "Could not save the export preset",
            Self::DeleteExportPreset(_) => "Could not delete the export preset",
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
/// selected; a photo that was not selected joins the selection (toggling
/// it in makes it the active one).
fn make_active(engine: &mut Engine, photo: PhotoId) {
    if engine.selection().is_selected(photo) {
        engine.make_active(photo);
    } else {
        engine.toggle_selected(photo);
    }
}
