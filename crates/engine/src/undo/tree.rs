//! Undoable edits of the two trees: collections and keywords.
//!
//! A deleted branch comes back from the catalog's restore data under the
//! same ids, with its names, places, kinds, rules, members and assignments,
//! so creating and deleting undo and redo exactly as long as the stack is
//! unwound in order.

use std::fmt::Debug;

use photo_catalog::{Catalog, CatalogError, DeletedCollections, DeletedKeywords, SmartRules};
use photo_core::meta::{CollectionId, KeywordId};

/// The operations a tree offers, for [`TreeEdit`].
pub(crate) trait Tree {
    /// The id of one node.
    type Id: Copy + Debug + PartialEq;
    /// What a deletion returns to restore with.
    type Deleted: Clone + Debug;

    fn delete(catalog: &mut Catalog, id: Self::Id) -> Result<Self::Deleted, CatalogError>;
    fn restore(catalog: &mut Catalog, deleted: &Self::Deleted) -> Result<(), CatalogError>;
    /// The node a deletion was asked for.
    fn root(deleted: &Self::Deleted) -> Option<Self::Id>;
    fn rename(catalog: &mut Catalog, id: Self::Id, name: &str) -> Result<(), CatalogError>;
    fn reparent(
        catalog: &mut Catalog,
        id: Self::Id,
        parent: Option<Self::Id>,
    ) -> Result<(), CatalogError>;
}

/// Collection sets, collections and smart collections.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Collections;

/// Keywords.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Keywords;

impl Tree for Collections {
    type Id = CollectionId;
    type Deleted = DeletedCollections;

    fn delete(catalog: &mut Catalog, id: CollectionId) -> Result<DeletedCollections, CatalogError> {
        catalog.delete_collection(id)
    }

    fn restore(catalog: &mut Catalog, deleted: &DeletedCollections) -> Result<(), CatalogError> {
        catalog.restore_collections(deleted)
    }

    fn root(deleted: &DeletedCollections) -> Option<CollectionId> {
        deleted.collection_ids().first().copied()
    }

    fn rename(catalog: &mut Catalog, id: CollectionId, name: &str) -> Result<(), CatalogError> {
        catalog.rename_collection(id, name)
    }

    fn reparent(
        catalog: &mut Catalog,
        id: CollectionId,
        parent: Option<CollectionId>,
    ) -> Result<(), CatalogError> {
        catalog.move_collection(id, parent)
    }
}

impl Tree for Keywords {
    type Id = KeywordId;
    type Deleted = DeletedKeywords;

    fn delete(catalog: &mut Catalog, id: KeywordId) -> Result<DeletedKeywords, CatalogError> {
        catalog.delete_keyword(id)
    }

    fn restore(catalog: &mut Catalog, deleted: &DeletedKeywords) -> Result<(), CatalogError> {
        catalog.restore_keywords(deleted)
    }

    fn root(deleted: &DeletedKeywords) -> Option<KeywordId> {
        deleted.keyword_ids().first().copied()
    }

    fn rename(catalog: &mut Catalog, id: KeywordId, name: &str) -> Result<(), CatalogError> {
        catalog.rename_keyword(id, name)
    }

    fn reparent(
        catalog: &mut Catalog,
        id: KeywordId,
        parent: Option<KeywordId>,
    ) -> Result<(), CatalogError> {
        catalog.move_keyword(id, parent)
    }
}

/// A branch that is in the catalog, or deleted with its restore data.
#[derive(Clone, Debug)]
pub(crate) enum Branch<T: Tree> {
    Present(T::Id),
    Deleted(Box<T::Deleted>),
}

impl<T: Tree> Branch<T> {
    fn delete(&mut self, catalog: &mut Catalog) -> Result<(), CatalogError> {
        if let Self::Present(id) = self {
            *self = Self::Deleted(Box::new(T::delete(catalog, *id)?));
        }
        Ok(())
    }

    fn restore(&mut self, catalog: &mut Catalog) -> Result<(), CatalogError> {
        if let Self::Deleted(deleted) = self {
            T::restore(catalog, deleted)?;
            if let Some(root) = T::root(deleted) {
                *self = Self::Present(root);
            }
        }
        Ok(())
    }
}

/// One undoable edit of a tree.
#[derive(Clone, Debug)]
pub(crate) enum TreeEdit<T: Tree> {
    /// A node was created: present after the edit.
    Created(Branch<T>),
    /// A branch was deleted: deleted after the edit.
    Deleted(Branch<T>),
    Renamed {
        id: T::Id,
        before: String,
        after: String,
    },
    Moved {
        id: T::Id,
        before: Option<T::Id>,
        after: Option<T::Id>,
    },
}

impl<T: Tree> TreeEdit<T> {
    pub(crate) fn undo(&mut self, catalog: &mut Catalog) -> Result<(), CatalogError> {
        match self {
            Self::Created(branch) => branch.delete(catalog),
            Self::Deleted(branch) => branch.restore(catalog),
            Self::Renamed { id, before, .. } => T::rename(catalog, *id, before),
            Self::Moved { id, before, .. } => T::reparent(catalog, *id, *before),
        }
    }

    pub(crate) fn redo(&mut self, catalog: &mut Catalog) -> Result<(), CatalogError> {
        match self {
            Self::Created(branch) => branch.restore(catalog),
            Self::Deleted(branch) => branch.delete(catalog),
            Self::Renamed { id, after, .. } => T::rename(catalog, *id, after),
            Self::Moved { id, after, .. } => T::reparent(catalog, *id, *after),
        }
    }
}

/// New rules for a smart collection.
#[derive(Clone, Debug)]
pub(crate) struct RulesEdit {
    pub(crate) collection: CollectionId,
    pub(crate) before: SmartRules,
    pub(crate) after: SmartRules,
}

impl RulesEdit {
    pub(crate) fn undo(&self, catalog: &mut Catalog) -> Result<(), CatalogError> {
        catalog.set_smart_rules(self.collection, &self.before)
    }

    pub(crate) fn redo(&self, catalog: &mut Catalog) -> Result<(), CatalogError> {
        catalog.set_smart_rules(self.collection, &self.after)
    }
}
