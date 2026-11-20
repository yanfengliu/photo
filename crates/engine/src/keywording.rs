//! Keywords: assigning them to photos by name, and editing the keyword tree
//! itself. All of it is undoable exactly: a keyword an action created is
//! deleted again by its undo (and restored under its id by the redo), and a
//! deleted keyword comes back with its assignments.
//!
//! Adding and removing a keyword, by name or by id, is one catalog write per
//! keyword, whatever the number of photos. Setting a photo's whole list
//! goes through `Catalog::set_photo_keywords`, which resolves each name
//! against the keywords the photo already carries, one write per photo.

use std::collections::{BTreeSet, HashMap, HashSet};

use photo_catalog::{Catalog, CatalogError, Entity, KeywordInfo};
use photo_core::meta::{KeywordId, PhotoId};

use crate::commands::{Applied, Target};
use crate::engine::Engine;
use crate::error::EngineError;
use crate::undo::{Branch, Command, KeywordLink, KeywordList, TreeEdit};

/// A keyword some of a set of photos carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeywordUse {
    /// The keyword, with its count over the whole catalog.
    pub keyword: KeywordInfo,
    /// How many of the photos carry it directly.
    pub photos: usize,
}

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
        let lists = photos.iter().map(|photo| (*photo, names.clone())).collect();
        let mut applied = Applied::default();
        if let Err(error) = set_lists(&mut self.catalog, &mut applied, lists) {
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

    /// The keywords the target photos carry directly, each with how many
    /// of them carry it, in name order: what the Keywording panel lists for
    /// a selection, where a keyword that only some carry shows as partial.
    /// One catalog read per photo.
    pub fn keywords_of(&self, target: Target) -> Result<Vec<KeywordUse>, EngineError> {
        let mut uses: HashMap<KeywordId, KeywordUse> = HashMap::new();
        for photo in self.resolve(&target) {
            for keyword in self.catalog.photo_keywords(photo)? {
                uses.entry(keyword.id)
                    .or_insert(KeywordUse { keyword, photos: 0 })
                    .photos += 1;
            }
        }
        let order: HashMap<KeywordId, usize> = self
            .browser
            .keywords()
            .iter()
            .enumerate()
            .map(|(place, keyword)| (keyword.id, place))
            .collect();
        let mut listed: Vec<KeywordUse> = uses.into_values().collect();
        listed.sort_by_key(|used| (order.get(&used.keyword.id).copied(), used.keyword.id));
        Ok(listed)
    }

    /// Give every target photo these keywords, chosen by id: exact where
    /// a name is shared by keywords in different places. An unknown keyword
    /// is refused, changing nothing.
    pub fn add_keyword_ids(
        &mut self,
        target: Target,
        keywords: &[KeywordId],
    ) -> Result<(), EngineError> {
        self.link_keywords(&target, keywords, true)
    }

    /// Take these keywords, chosen by id, off every target photo. An
    /// unknown keyword is refused, changing nothing.
    pub fn remove_keyword_ids(
        &mut self,
        target: Target,
        keywords: &[KeywordId],
    ) -> Result<(), EngineError> {
        self.link_keywords(&target, keywords, false)
    }

    /// Assign (`link`) or unassign each keyword over the target, one
    /// catalog write per keyword, recording only the photos that changed.
    fn link_keywords(
        &mut self,
        target: &Target,
        keywords: &[KeywordId],
        link: bool,
    ) -> Result<(), EngineError> {
        let photos = self.resolve(target);
        let mut seen = HashSet::new();
        let keywords: Vec<KeywordId> = keywords
            .iter()
            .copied()
            .filter(|keyword| seen.insert(*keyword))
            .collect();
        if photos.is_empty() || keywords.is_empty() {
            return Ok(());
        }
        let names = keywords
            .iter()
            .map(|keyword| self.known_keyword(*keyword).map(|known| known.name.clone()))
            .collect::<Result<Vec<String>, EngineError>>()?;
        let mut applied = Applied::default();
        for keyword in &keywords {
            let changed = if link {
                self.catalog.assign_keyword(*keyword, &photos)
            } else {
                self.catalog.unassign_keyword(*keyword, &photos)
            };
            match changed {
                Ok(changed) if changed.is_empty() => {}
                Ok(changed) => applied.push(Command::Keyword(KeywordLink {
                    keyword: *keyword,
                    photos: changed,
                    linked: link,
                })),
                Err(error) => {
                    applied.roll_back(&mut self.catalog);
                    return Err(error.into());
                }
            }
        }
        let verb = if link { "Add" } else { "Remove" };
        let label = match names.as_slice() {
            [name] => format!("{verb} Keyword \"{name}\""),
            many => format!("{verb} {} Keywords", many.len()),
        };
        self.record_applied(label, applied);
        Ok(())
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
/// `photos`, recording only the photos that did not have it, and recording
/// each keyword that had to be created before its assignment.
fn assign_all(
    catalog: &mut Catalog,
    applied: &mut Applied,
    names: &[String],
    photos: &[PhotoId],
) -> Result<(), CatalogError> {
    let mut known: HashSet<KeywordId> = catalog.keywords()?.into_iter().map(|k| k.id).collect();
    for name in names {
        let keyword = catalog.ensure_keyword(name)?;
        if known.insert(keyword) {
            applied.push(Command::Keywords(TreeEdit::Created(Branch::Present(
                keyword,
            ))));
        }
        let added = catalog.assign_keyword(keyword, photos)?;
        if !added.is_empty() {
            applied.push(Command::Keyword(KeywordLink {
                keyword,
                photos: added,
                linked: true,
            }));
        }
    }
    Ok(())
}

/// Give each photo exactly its list of keyword names, through
/// `Catalog::set_photo_keywords`: a name the photo already carries keeps
/// that keyword, any other resolves to the only keyword of the name or a
/// new top-level one. Each keyword created is recorded before the list
/// that uses it, so undoing deletes it after the list is put back.
pub(crate) fn set_lists(
    catalog: &mut Catalog,
    applied: &mut Applied,
    lists: Vec<(PhotoId, Vec<String>)>,
) -> Result<(), CatalogError> {
    for (photo, names) in lists {
        let before: Vec<KeywordId> = catalog
            .photo_keywords(photo)?
            .into_iter()
            .map(|keyword| keyword.id)
            .collect();
        let change = catalog.set_photo_keywords(photo, &names)?;
        for created in &change.created {
            applied.push(Command::Keywords(TreeEdit::Created(Branch::Present(
                *created,
            ))));
        }
        if change.added.is_empty() && change.removed.is_empty() {
            continue;
        }
        let after = before
            .iter()
            .filter(|keyword| !change.removed.contains(keyword))
            .chain(&change.added)
            .copied()
            .collect();
        applied.push(Command::KeywordList(KeywordList {
            photo,
            before,
            after,
        }));
    }
    Ok(())
}
