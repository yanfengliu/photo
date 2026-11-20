//! The undo and redo stacks, and the commands on them.
//!
//! Every command holds exactly what it needs to move the catalog between its
//! two states, per photo: the values each photo had before and after, the
//! develop changes the catalog returned, the restore data of a removal. The
//! stacks live in memory for the session only; they are never persisted.

use photo_catalog::{
    Catalog, CatalogError, CollectionEntry, DevelopChange, RemovedPhotos, TextField,
};
use photo_core::meta::{CollectionId, ColorLabel, Flag, KeywordId, PhotoId, Rating};

use crate::browse::{Attribute, Change};

/// How many commands the undo stack keeps; older ones are forgotten.
const LIMIT: usize = 200;

/// Per-photo values before and after a change. Only photos whose value
/// actually changed are listed.
#[derive(Clone, Debug)]
pub(crate) struct Values<T> {
    before: Vec<(PhotoId, T)>,
    after: Vec<(PhotoId, T)>,
}

impl<T: Clone + PartialEq> Values<T> {
    /// The change that gives each photo `next(current)`. `None` when no
    /// photo would change.
    pub(crate) fn new(current: Vec<(PhotoId, T)>, next: impl Fn(&T) -> T) -> Option<Self> {
        let (before, after): (Vec<_>, Vec<_>) = current
            .into_iter()
            .filter_map(|(photo, value)| {
                let changed = next(&value);
                (changed != value).then(|| ((photo, value), (photo, changed)))
            })
            .unzip();
        (!after.is_empty()).then_some(Self { before, after })
    }

    pub(crate) fn photos(&self) -> Vec<PhotoId> {
        self.after.iter().map(|(photo, _)| *photo).collect()
    }

    pub(crate) fn after(&self) -> &[(PhotoId, T)] {
        &self.after
    }
}

/// Keyword assignments that are either in place or not.
#[derive(Clone, Debug)]
pub(crate) struct KeywordLink {
    pub(crate) keyword: KeywordId,
    pub(crate) photos: Vec<PhotoId>,
    /// Whether the photos carry the keyword now.
    pub(crate) linked: bool,
}

/// Collection memberships that are either in place or not.
#[derive(Clone, Debug)]
pub(crate) struct Membership {
    pub(crate) collection: CollectionId,
    pub(crate) photos: Vec<PhotoId>,
    /// Where the photos sat in the collection when they last left it.
    pub(crate) entries: Vec<CollectionEntry>,
    /// Whether the photos are in the collection now.
    pub(crate) inside: bool,
}

/// Photos that exist now, or the data to bring them back.
#[derive(Clone, Debug)]
pub(crate) enum Presence {
    Present(Vec<PhotoId>),
    Removed(RemovedPhotos),
}

/// One undoable change to the catalog.
#[derive(Clone, Debug)]
pub(crate) enum Command {
    Ratings(Values<Rating>),
    Flags(Values<Flag>),
    Labels(Values<ColorLabel>),
    Text(TextField, Values<String>),
    Keyword(KeywordLink),
    Develop(Vec<DevelopChange>),
    /// Virtual copies: present after the command, removed when undone.
    Copies(Presence),
    /// Photos removed from the catalog: removed after the command.
    Removal(Presence),
    Membership(Membership),
    /// Several commands that undo and redo as one, in order.
    Batch(Vec<Command>),
}

impl Command {
    /// Take the catalog back to the state before this command.
    pub(crate) fn undo(&mut self, catalog: &mut Catalog) -> Result<(), CatalogError> {
        match self {
            Self::Ratings(values) => catalog.set_ratings(&values.before),
            Self::Flags(values) => catalog.set_flags(&values.before),
            Self::Labels(values) => catalog.set_labels(&values.before),
            Self::Text(field, values) => catalog.set_text(*field, &values.before),
            Self::Keyword(link) => toggle_keyword(catalog, link),
            Self::Develop(changes) => catalog.undo_develop(changes),
            Self::Copies(presence) => remove(catalog, presence),
            Self::Removal(presence) => restore(catalog, presence),
            Self::Membership(membership) => toggle_membership(catalog, membership),
            Self::Batch(commands) => commands
                .iter_mut()
                .rev()
                .try_for_each(|command| command.undo(catalog)),
        }
    }

    /// Bring the catalog back to the state this command left.
    pub(crate) fn redo(&mut self, catalog: &mut Catalog) -> Result<(), CatalogError> {
        match self {
            Self::Ratings(values) => catalog.set_ratings(&values.after),
            Self::Flags(values) => catalog.set_flags(&values.after),
            Self::Labels(values) => catalog.set_labels(&values.after),
            Self::Text(field, values) => catalog.set_text(*field, &values.after),
            Self::Keyword(link) => toggle_keyword(catalog, link),
            Self::Develop(changes) => catalog.redo_develop(changes),
            Self::Copies(presence) => restore(catalog, presence),
            Self::Removal(presence) => remove(catalog, presence),
            Self::Membership(membership) => toggle_membership(catalog, membership),
            Self::Batch(commands) => commands
                .iter_mut()
                .try_for_each(|command| command.redo(catalog)),
        }
    }

    /// What applying, undoing or redoing this command changes.
    pub(crate) fn changes(&self, out: &mut Vec<Change>) {
        let attribute = |attribute, photos| Change::Attribute(attribute, photos);
        match self {
            Self::Ratings(values) => out.push(attribute(Attribute::Rating, values.photos())),
            Self::Flags(values) => out.push(attribute(Attribute::Flag, values.photos())),
            Self::Labels(values) => out.push(attribute(Attribute::Label, values.photos())),
            Self::Text(field, values) => {
                out.push(attribute(Attribute::Text(*field), values.photos()));
            }
            Self::Keyword(link) => out.push(attribute(Attribute::Keywords, link.photos.clone())),
            Self::Develop(changes) => {
                let photos = changes.iter().map(DevelopChange::photo).collect();
                out.push(attribute(Attribute::Develop, photos));
            }
            Self::Copies(_) | Self::Removal(_) => out.push(Change::Photos),
            Self::Membership(membership) => out.push(Change::Membership(
                membership.collection,
                membership.photos.clone(),
            )),
            Self::Batch(commands) => commands.iter().for_each(|command| command.changes(out)),
        }
    }
}

fn toggle_keyword(catalog: &mut Catalog, link: &mut KeywordLink) -> Result<(), CatalogError> {
    if link.linked {
        catalog.unassign_keyword(link.keyword, &link.photos)?;
    } else {
        catalog.assign_keyword(link.keyword, &link.photos)?;
    }
    link.linked = !link.linked;
    Ok(())
}

fn toggle_membership(catalog: &mut Catalog, membership: &mut Membership) -> Result<(), CatalogError> {
    if membership.inside {
        membership.entries =
            catalog.remove_from_collection(membership.collection, &membership.photos)?;
    } else {
        catalog.restore_collection_entries(membership.collection, &membership.entries)?;
    }
    membership.inside = !membership.inside;
    Ok(())
}

/// Remove present photos, keeping what it takes to restore them.
fn remove(catalog: &mut Catalog, presence: &mut Presence) -> Result<(), CatalogError> {
    if let Presence::Present(photos) = presence {
        *presence = Presence::Removed(catalog.remove_photos(photos)?);
    }
    Ok(())
}

/// Restore removed photos under their old ids.
fn restore(catalog: &mut Catalog, presence: &mut Presence) -> Result<(), CatalogError> {
    if let Presence::Removed(removed) = presence {
        catalog.restore_photos(removed)?;
        *presence = Presence::Present(removed.photo_ids().to_vec());
    }
    Ok(())
}

/// A command with the label the UI shows for it.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub(crate) label: String,
    pub(crate) command: Command,
}

/// The session's undo and redo stacks.
#[derive(Debug, Default)]
pub(crate) struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}

impl History {
    /// Record a command just applied. Anything that could be redone is
    /// forgotten, as the new command replaces it.
    pub(crate) fn record(&mut self, label: String, command: Command) {
        self.redo.clear();
        self.push_undo(Entry { label, command });
    }

    fn push_undo(&mut self, entry: Entry) {
        if self.undo.len() == LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(entry);
    }

    pub(crate) fn pop_undo(&mut self) -> Option<Entry> {
        self.undo.pop()
    }

    pub(crate) fn pop_redo(&mut self) -> Option<Entry> {
        self.redo.pop()
    }

    /// File a command that was just undone, so it can be redone.
    pub(crate) fn undone(&mut self, entry: Entry) {
        self.redo.push(entry);
    }

    /// File a command that was just redone, keeping the rest of the redo
    /// stack.
    pub(crate) fn redone(&mut self, entry: Entry) {
        self.push_undo(entry);
    }

    pub(crate) fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|entry| entry.label.as_str())
    }

    pub(crate) fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|entry| entry.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_list_only_the_photos_that_change() {
        let current = vec![
            (PhotoId(1), Rating::new(5)),
            (PhotoId(2), Rating::new(3)),
        ];
        let values = Values::new(current, |_| Rating::new(5)).unwrap();
        assert_eq!(values.before, [(PhotoId(2), Rating::new(3))]);
        assert_eq!(values.after(), [(PhotoId(2), Rating::new(5))]);
        assert!(Values::new(vec![(PhotoId(1), Rating::new(5))], |_| Rating::new(5)).is_none());
    }

    #[test]
    fn a_new_command_clears_the_redo_stack_and_the_stack_is_bounded() {
        let mut history = History::default();
        let command = || Command::Batch(Vec::new());
        for index in 0..LIMIT + 5 {
            history.record(format!("step {index}"), command());
        }
        assert_eq!(history.undo.len(), LIMIT);
        assert_eq!(history.undo[0].label, "step 5");
        let entry = history.pop_undo().unwrap();
        history.undone(entry);
        assert_eq!(history.redo_label(), Some(format!("step {}", LIMIT + 4).as_str()));
        history.record("new".into(), command());
        assert_eq!(history.redo_label(), None);
        assert_eq!(history.undo_label(), Some("new"));
    }
}
