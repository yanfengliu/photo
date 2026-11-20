//! The small dialogs: naming a new collection or set, renaming one, and
//! the confirmations before deleting a collection or removing photos from
//! the catalog.

use egui::{RichText, TextEdit};
use photo_core::meta::CollectionId;
use photo_engine::{CollectionInfo, CollectionKind, Target};

use super::{NewKind, parent_menu};
use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::text;
use crate::theme::{palette, space};
use crate::widgets::{Dialog, DialogOutcome};

/// Naming something: a new collection or set, or a new name for one.
pub(crate) struct NameDialog {
    purpose: Purpose,
    name: String,
    parent: Option<CollectionId>,
    /// Put the keyboard in the name field when the dialog first shows.
    focus: bool,
}

enum Purpose {
    New(NewKind),
    Rename {
        id: CollectionId,
        kind: CollectionKind,
        before: String,
    },
}

impl NameDialog {
    /// Name a new collection or set, to go inside `parent`.
    pub(crate) fn new(kind: NewKind, parent: Option<CollectionId>) -> Self {
        Self {
            purpose: Purpose::New(kind),
            name: String::new(),
            parent,
            focus: true,
        }
    }

    /// Give `collection` a new name.
    pub(crate) fn rename(collection: &CollectionInfo) -> Self {
        Self {
            purpose: Purpose::Rename {
                id: collection.id,
                kind: collection.kind,
                before: collection.name.clone(),
            },
            name: collection.name.clone(),
            parent: collection.parent,
            focus: true,
        }
    }

    /// Show the dialog; returns whether it stays open.
    pub(crate) fn show(&mut self, ctx: &egui::Context, cx: &mut Cx<'_>) -> bool {
        let (title, field, confirm) = match &self.purpose {
            Purpose::New(NewKind::Collection) => ("New Collection", "Collection name", "Create"),
            Purpose::New(NewKind::Set) => ("New Collection Set", "Collection set name", "Create"),
            Purpose::Rename { kind, .. } => (
                match kind {
                    CollectionKind::Set => "Rename Collection Set",
                    CollectionKind::Smart => "Rename Smart Collection",
                    CollectionKind::Regular | CollectionKind::Quick => "Rename Collection",
                },
                "New name",
                "Rename",
            ),
        };
        let trimmed = self.name.trim().to_owned();
        let changed = match &self.purpose {
            Purpose::Rename { before, .. } => trimmed != *before,
            Purpose::New(_) => true,
        };
        let response = Dialog::new(title)
            .confirm_label(confirm)
            .can_confirm(!trimmed.is_empty() && changed)
            .show(ctx, |ui| {
                ui.label(RichText::new("Name").color(palette::TEXT_DIM));
                let response = ui
                    .add(
                        TextEdit::singleline(&mut self.name)
                            .id_salt("collection-name")
                            .desired_width(f32::INFINITY),
                    )
                    .accessible_name(field);
                if std::mem::take(&mut self.focus) {
                    response.request_focus();
                }
                if let Purpose::New(_) = self.purpose {
                    ui.add_space(space::MD);
                    parent_menu(ui, cx, &mut self.parent);
                }
            });
        match response.outcome {
            DialogOutcome::Open => true,
            DialogOutcome::Cancelled => false,
            DialogOutcome::Confirmed => {
                let name = self.name.trim().to_owned();
                cx.act(match &self.purpose {
                    Purpose::New(NewKind::Collection) => Action::CreateCollection {
                        name,
                        parent: self.parent,
                    },
                    Purpose::New(NewKind::Set) => Action::CreateCollectionSet {
                        name,
                        parent: self.parent,
                    },
                    Purpose::Rename { id, .. } => Action::RenameCollection(*id, name),
                });
                false
            }
        }
    }
}

/// Asking before a collection is deleted.
pub(crate) struct DeleteDialog {
    id: CollectionId,
    kind: CollectionKind,
    name: String,
}

impl DeleteDialog {
    /// Ask about deleting `collection`.
    pub(crate) fn new(collection: &CollectionInfo) -> Self {
        Self {
            id: collection.id,
            kind: collection.kind,
            name: collection.name.clone(),
        }
    }

    /// Show the dialog; returns whether it stays open.
    pub(crate) fn show(&mut self, ctx: &egui::Context, cx: &mut Cx<'_>) -> bool {
        let kind = text::kind_name(self.kind);
        let title = format!("Delete {kind}");
        let what = if self.kind == CollectionKind::Set {
            format!(
                "Delete the collection set \"{}\" and everything inside it?",
                self.name
            )
        } else {
            format!("Delete the {} \"{}\"?", kind.to_lowercase(), self.name)
        };
        let response = Dialog::new(&title).confirm_label("Delete").show(ctx, |ui| {
            ui.label(RichText::new(what).color(palette::TEXT_STRONG));
            ui.add_space(space::SM);
            ui.label(
                RichText::new("The photos in it stay in the catalog. You can undo this.")
                    .color(palette::TEXT_DIM),
            );
        });
        match response.outcome {
            DialogOutcome::Open => true,
            DialogOutcome::Cancelled => false,
            DialogOutcome::Confirmed => {
                cx.act(Action::DeleteCollection(self.id));
                false
            }
        }
    }
}

/// Asking before photos are removed from the catalog.
pub(crate) struct RemoveDialog {
    target: Target,
    count: usize,
}

impl RemoveDialog {
    /// Ask about removing the `count` photos `target` stands for.
    pub(crate) fn new(target: Target, count: usize) -> Self {
        Self { target, count }
    }

    /// Show the dialog; returns whether it stays open.
    pub(crate) fn show(&mut self, ctx: &egui::Context, cx: &mut Cx<'_>) -> bool {
        let photos = text::count(self.count, "photo", "photos");
        let response = Dialog::new("Remove Photos")
            .confirm_label("Remove")
            .show(ctx, |ui| {
                let question = format!("Remove {photos} from the catalog?");
                ui.label(RichText::new(question).color(palette::TEXT_STRONG));
                ui.add_space(space::SM);
                ui.label(
                    RichText::new(
                        "The files on disk are not touched: only the catalog forgets them, with \
                         their ratings, keywords and edits. You can undo this.",
                    )
                    .color(palette::TEXT_DIM),
                );
            });
        match response.outcome {
            DialogOutcome::Open => true,
            DialogOutcome::Cancelled => false,
            DialogOutcome::Confirmed => {
                cx.act(Action::RemoveFromCatalog(self.target.clone()));
                false
            }
        }
    }
}
