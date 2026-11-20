//! Collections: putting photos in and taking them out, and editing the
//! collection tree itself. All of it is undoable; a deleted collection
//! comes back with its members in their order.

use photo_catalog::{CatalogError, CollectionInfo, Entity, SmartRules};
use photo_core::meta::{CollectionId, PhotoId};

use crate::commands::Target;
use crate::engine::Engine;
use crate::error::EngineError;
use crate::undo::{Branch, Command, Membership, RulesEdit, TreeEdit};

impl Engine {
    /// The id of the built-in quick collection.
    pub fn quick_collection(&self) -> CollectionId {
        self.catalog.quick_collection()
    }

    /// Put every target photo into a regular collection (or the quick
    /// collection), after its present members.
    pub fn add_to_collection(
        &mut self,
        collection: CollectionId,
        target: Target,
    ) -> Result<(), EngineError> {
        let photos = self.resolve(&target);
        if photos.is_empty() {
            return Ok(());
        }
        let added = self.catalog.add_to_collection(collection, &photos)?;
        let label = format!("Add to {}", self.collection_name(collection));
        if added.is_empty() {
            self.toasts.info(label);
            return Ok(());
        }
        let membership = Membership {
            collection,
            photos: added,
            entries: Vec::new(),
            inside: true,
        };
        self.record(label, Command::Membership(membership));
        Ok(())
    }

    /// Take every target photo out of a regular collection (or the quick
    /// collection). Undo puts them back where they were in its order.
    pub fn remove_from_collection(
        &mut self,
        collection: CollectionId,
        target: Target,
    ) -> Result<(), EngineError> {
        let photos = self.resolve(&target);
        if photos.is_empty() {
            return Ok(());
        }
        let entries = self.catalog.remove_from_collection(collection, &photos)?;
        let label = format!("Remove from {}", self.collection_name(collection));
        if entries.is_empty() {
            self.toasts.info(label);
            return Ok(());
        }
        let membership = Membership {
            collection,
            photos: entries.iter().map(|entry| entry.photo).collect(),
            entries,
            inside: false,
        };
        self.record(label, Command::Membership(membership));
        Ok(())
    }

    /// Take the target photos out of the quick collection if they are all
    /// in it; otherwise put the ones that are not into it.
    pub fn toggle_quick_collection(&mut self, target: Target) -> Result<(), EngineError> {
        let photos = self.resolve(&target);
        if photos.is_empty() {
            return Ok(());
        }
        let quick = self.quick_collection();
        let all_in = self
            .catalog
            .summaries(&photos)?
            .iter()
            .all(|summary| summary.in_quick_collection);
        let target = Target::Photos(photos);
        if all_in {
            self.remove_from_collection(quick, target)
        } else {
            self.add_to_collection(quick, target)
        }
    }

    /// The collections (the quick collection included) a photo has been put
    /// into.
    pub fn photo_collections(&self, photo: PhotoId) -> Result<Vec<CollectionId>, EngineError> {
        Ok(self.catalog.photo_collections(photo)?)
    }

    /// Create a collection set inside `parent` (or at the top level).
    pub fn create_collection_set(
        &mut self,
        name: &str,
        parent: Option<CollectionId>,
    ) -> Result<CollectionId, EngineError> {
        let id = self.catalog.create_collection_set(name, parent)?;
        self.record_created(format!("Create Collection Set \"{}\"", name.trim()), id);
        Ok(id)
    }

    /// Create an empty collection inside the set `parent` (or at the top
    /// level).
    pub fn create_collection(
        &mut self,
        name: &str,
        parent: Option<CollectionId>,
    ) -> Result<CollectionId, EngineError> {
        let id = self.catalog.create_collection(name, parent)?;
        self.record_created(format!("Create Collection \"{}\"", name.trim()), id);
        Ok(id)
    }

    /// Create a smart collection inside the set `parent` (or at the top
    /// level); rules that do not validate are refused.
    pub fn create_smart_collection(
        &mut self,
        name: &str,
        parent: Option<CollectionId>,
        rules: &SmartRules,
    ) -> Result<CollectionId, EngineError> {
        let id = self.catalog.create_smart_collection(name, parent, rules)?;
        self.record_created(format!("Create Smart Collection \"{}\"", name.trim()), id);
        Ok(id)
    }

    fn record_created(&mut self, label: String, id: CollectionId) {
        let edit = TreeEdit::Created(Branch::Present(id));
        self.record(label, Command::Collections(edit));
    }

    /// The rules of a smart collection.
    pub fn smart_rules(&self, id: CollectionId) -> Result<SmartRules, EngineError> {
        Ok(self.catalog.smart_rules(id)?)
    }

    /// Replace the rules of a smart collection.
    pub fn set_smart_rules(
        &mut self,
        id: CollectionId,
        rules: &SmartRules,
    ) -> Result<(), EngineError> {
        let before = self.catalog.smart_rules(id)?;
        self.catalog.set_smart_rules(id, rules)?;
        let edit = RulesEdit {
            collection: id,
            before,
            after: rules.clone(),
        };
        self.record(
            "Edit Smart Collection".to_string(),
            Command::SmartRules(edit),
        );
        Ok(())
    }

    /// Rename a collection, collection set or smart collection.
    pub fn rename_collection(&mut self, id: CollectionId, name: &str) -> Result<(), EngineError> {
        let before = self.known_collection(id)?.name.clone();
        self.catalog.rename_collection(id, name)?;
        let after = name.trim().to_string();
        let label = format!("Rename Collection to \"{after}\"");
        self.record(
            label,
            Command::Collections(TreeEdit::Renamed { id, before, after }),
        );
        Ok(())
    }

    /// Move a collection, with everything inside it, into the set `parent`
    /// (or to the top level).
    pub fn move_collection(
        &mut self,
        id: CollectionId,
        parent: Option<CollectionId>,
    ) -> Result<(), EngineError> {
        let before = self.known_collection(id)?.parent;
        self.catalog.move_collection(id, parent)?;
        let edit = TreeEdit::Moved {
            id,
            before,
            after: parent,
        };
        self.record("Move Collection".to_string(), Command::Collections(edit));
        Ok(())
    }

    /// Delete a collection (a set with everything inside it). Photos are not
    /// affected, and undo brings everything back as it was. When the
    /// current source was inside it, the list falls back to all photos.
    pub fn delete_collection(&mut self, id: CollectionId) -> Result<(), EngineError> {
        let name = self.collection_name(id);
        let deleted = self.catalog.delete_collection(id)?;
        let edit = TreeEdit::Deleted(Branch::Deleted(Box::new(deleted)));
        self.record(format!("Delete {name}"), Command::Collections(edit));
        Ok(())
    }

    fn known_collection(&self, id: CollectionId) -> Result<&CollectionInfo, EngineError> {
        self.browser
            .collections()
            .iter()
            .find(|collection| collection.id == id)
            .ok_or_else(|| {
                CatalogError::NotFound {
                    entity: Entity::Collection,
                    id: id.0,
                }
                .into()
            })
    }

    fn collection_name(&self, id: CollectionId) -> String {
        if id == self.catalog.quick_collection() {
            return "Quick Collection".to_string();
        }
        self.browser
            .collections()
            .iter()
            .find(|collection| collection.id == id)
            .map_or_else(
                || "Collection".to_string(),
                |collection| format!("\"{}\"", collection.name),
            )
    }
}
