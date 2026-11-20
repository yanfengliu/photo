//! Keywords: assigning them to photos by name (one catalog write per
//! keyword rather than per photo), and editing the keyword tree itself.
//! All of it is undoable; a deleted keyword comes back with its
//! assignments.

use std::collections::{BTreeMap, BTreeSet};

use photo_catalog::{Catalog, CatalogError, Entity, KeywordInfo};
use photo_core::meta::{KeywordId, PhotoId};

use crate::commands::{Applied, Target};
use crate::engine::Engine;
use crate::error::EngineError;
use crate::undo::{Branch, Command, KeywordLink, TreeEdit};

/// Names trimmed, blanks dropped, and each name once (ignoring ASCII case,
/// as the catalog compares them).
fn clean<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    names
        .iter()
        .map(|name| name.as_ref().trim().to_string())
        .filter(|name| !name.is_empty() && seen.insert(name.to_ascii_lowercase()))
        .collect()
}

impl Engine {
    /// Give every target photo the named keywords, creating keywords that
    /// do not exist yet (at the top level). A name matches an existing
    /// keyword wherever it sits, ignoring ASCII case.
    pub fn add_keywords<S: AsRef<str>>(
        &mut self,
        target: Target,
        names: &[S],
    ) -> Result<(), EngineError> {
        let photos = self.resolve(&target);
        let names = clean(names);
        if photos.is_empty() || names.is_empty() {
            return Ok(());
        }
        let mut applied = Applied::default();
        if let Err(error) = assign_all(&mut self.catalog, &mut applied, &names, &photos) {
            applied.roll_back(&mut self.catalog);
            return Err(error.into());
        }
        let label = match names.as_slice() {
            [name] => format!("Add Keyword \"{name}\""),
            many => format!("Add {} Keywords", many.len()),
        };
        self.record_applied(label, applied);
        Ok(())
    }

    /// Take the named keywords off every target photo: every keyword of
    /// that name, wherever it sits in the hierarchy.
    pub fn remove_keywords<S: AsRef<str>>(
        &mut self,
        target: Target,
        names: &[S],
    ) -> Result<(), EngineError> {
        let photos = self.resolve(&target);
        let names = clean(names);
        if photos.is_empty() || names.is_empty() {
            return Ok(());
        }
        let keywords: Vec<KeywordId> = self
            .browser
            .keywords()
            .iter()
            .filter(|keyword| {
                names
                    .iter()
                    .any(|name| keyword.name.eq_ignore_ascii_case(name))
            })
            .map(|keyword| keyword.id)
            .collect();
        let mut applied = Applied::default();
        for keyword in keywords {
            match self.catalog.unassign_keyword(keyword, &photos) {
                Ok(removed) if removed.is_empty() => {}
                Ok(removed) => applied.push(Command::Keyword(KeywordLink {
                    keyword,
                    photos: removed,
                    linked: false,
                })),
                Err(error) => {
                    applied.roll_back(&mut self.catalog);
                    return Err(error.into());
                }
            }
        }
        let label = match names.as_slice() {
            [name] => format!("Remove Keyword \"{name}\""),
            many => format!("Remove {} Keywords", many.len()),
        };
        self.record_applied(label, applied);
        Ok(())
    }

    /// Make the named keywords exactly the keywords of every target photo.
    pub fn set_keywords<S: AsRef<str>>(
        &mut self,
        target: Target,
        names: &[S],
    ) -> Result<(), EngineError> {
        let photos = self.resolve(&target);
        if photos.is_empty() {
            return Ok(());
        }
        let names = clean(names);
        let mut applied = Applied::default();
        if let Err(error) = replace_all(&mut self.catalog, &mut applied, &names, &photos) {
            applied.roll_back(&mut self.catalog);
            return Err(error.into());
        }
        self.record_applied("Set Keywords".to_string(), applied);
        Ok(())
    }

    /// The keywords assigned to a photo, in name order.
    pub fn photo_keywords(&self, photo: PhotoId) -> Result<Vec<KeywordInfo>, EngineError> {
        Ok(self.catalog.photo_keywords(photo)?)
    }

    /// Create a keyword under `parent` (or at the top level).
    pub fn create_keyword(
        &mut self,
        name: &str,
        parent: Option<KeywordId>,
    ) -> Result<KeywordId, EngineError> {
        let id = self.catalog.create_keyword(name, parent)?;
        let edit = TreeEdit::Created(Branch::Present(id));
        self.record(
            format!("Create Keyword \"{}\"", name.trim()),
            Command::Keywords(edit),
        );
        Ok(id)
    }

    /// Rename a keyword; its assignments stay.
    pub fn rename_keyword(&mut self, id: KeywordId, name: &str) -> Result<(), EngineError> {
        let before = self.known_keyword(id)?.name.clone();
        self.catalog.rename_keyword(id, name)?;
        let after = name.trim().to_string();
        let label = format!("Rename Keyword to \"{after}\"");
        self.record(
            label,
            Command::Keywords(TreeEdit::Renamed { id, before, after }),
        );
        Ok(())
    }

    /// Move a keyword, with everything beneath it, under `parent` (or to the
    /// top level).
    pub fn move_keyword(
        &mut self,
        id: KeywordId,
        parent: Option<KeywordId>,
    ) -> Result<(), EngineError> {
        let before = self.known_keyword(id)?.parent;
        self.catalog.move_keyword(id, parent)?;
        let edit = TreeEdit::Moved {
            id,
            before,
            after: parent,
        };
        self.record("Move Keyword".to_string(), Command::Keywords(edit));
        Ok(())
    }

    /// Delete a keyword, the keywords beneath it and all their assignments.
    /// Undo brings them back, assigned to the same photos.
    pub fn delete_keyword(&mut self, id: KeywordId) -> Result<(), EngineError> {
        let name = self.known_keyword(id)?.name.clone();
        let deleted = self.catalog.delete_keyword(id)?;
        let edit = TreeEdit::Deleted(Branch::Deleted(Box::new(deleted)));
        self.record(
            format!("Delete Keyword \"{name}\""),
            Command::Keywords(edit),
        );
        Ok(())
    }

    fn known_keyword(&self, id: KeywordId) -> Result<&KeywordInfo, EngineError> {
        self.browser
            .keywords()
            .iter()
            .find(|keyword| keyword.id == id)
            .ok_or_else(|| {
                CatalogError::NotFound {
                    entity: Entity::Keyword,
                    id: id.0,
                }
                .into()
            })
    }
}

/// Resolve each name to a keyword (creating it if needed) and assign it to
/// `photos`, recording only the photos that did not have it.
fn assign_all(
    catalog: &mut Catalog,
    applied: &mut Applied,
    names: &[String],
    photos: &[PhotoId],
) -> Result<(), CatalogError> {
    for name in names {
        let keyword = catalog.ensure_keyword(name)?;
        assign(catalog, applied, keyword, photos)?;
    }
    Ok(())
}

fn assign(
    catalog: &mut Catalog,
    applied: &mut Applied,
    keyword: KeywordId,
    photos: &[PhotoId],
) -> Result<(), CatalogError> {
    let added = catalog.assign_keyword(keyword, photos)?;
    if !added.is_empty() {
        applied.push(Command::Keyword(KeywordLink {
            keyword,
            photos: added,
            linked: true,
        }));
    }
    Ok(())
}

/// Give each photo exactly the named keywords: for every keyword involved,
/// one write that takes it off the photos that should not have it and one
/// that gives it to the photos that should.
pub(crate) fn replace_all(
    catalog: &mut Catalog,
    applied: &mut Applied,
    names: &[String],
    photos: &[PhotoId],
) -> Result<(), CatalogError> {
    let wanted = names
        .iter()
        .map(|name| catalog.ensure_keyword(name))
        .collect::<Result<BTreeSet<KeywordId>, _>>()?;
    let wanted_by_photo = photos
        .iter()
        .map(|photo| (*photo, wanted.clone()))
        .collect();
    replace_each(catalog, applied, wanted_by_photo)
}

/// Give each photo exactly its own set of keywords.
pub(crate) fn replace_each(
    catalog: &mut Catalog,
    applied: &mut Applied,
    wanted: BTreeMap<PhotoId, BTreeSet<KeywordId>>,
) -> Result<(), CatalogError> {
    let mut gain: BTreeMap<KeywordId, Vec<PhotoId>> = BTreeMap::new();
    let mut lose: BTreeMap<KeywordId, Vec<PhotoId>> = BTreeMap::new();
    for (photo, keywords) in &wanted {
        let current: BTreeSet<KeywordId> = catalog
            .photo_keywords(*photo)?
            .into_iter()
            .map(|keyword| keyword.id)
            .collect();
        for keyword in keywords.difference(&current) {
            gain.entry(*keyword).or_default().push(*photo);
        }
        for keyword in current.difference(keywords) {
            lose.entry(*keyword).or_default().push(*photo);
        }
    }
    for (keyword, photos) in lose {
        let removed = catalog.unassign_keyword(keyword, &photos)?;
        if !removed.is_empty() {
            applied.push(Command::Keyword(KeywordLink {
                keyword,
                photos: removed,
                linked: false,
            }));
        }
    }
    for (keyword, photos) in gain {
        assign(catalog, applied, keyword, &photos)?;
    }
    Ok(())
}
