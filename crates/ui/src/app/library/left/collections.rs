//! The Collections panel: sets, collections and smart collections as a
//! tree, each with its photo count. Clicking one shows its photos; photos
//! dragged from the grid onto a collection go into it; the panel's create
//! menu and each row's context menu make, rename and delete them.

use std::collections::{HashMap, HashSet};

use egui::{RichText, Ui};
use photo_core::meta::CollectionId;
use photo_engine::{CollectionInfo, CollectionKind, Source, Target};

use super::DraggedPhotos;
use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::dialogs::{self, NewKind};
use crate::app::text;
use crate::theme::{palette, space};
use crate::widgets::{Icon, IconButton, ListRow};

/// The collections in display order, each with its depth: a set's
/// contents follow it, siblings in name order. A collection whose set is
/// not listed shows at the top level; a cycle is cut where it closes.
pub(super) fn tree(collections: &[CollectionInfo]) -> Vec<(usize, usize)> {
    let known: HashSet<CollectionId> = collections.iter().map(|c| c.id).collect();
    let mut children: HashMap<Option<CollectionId>, Vec<usize>> = HashMap::new();
    for (index, collection) in collections.iter().enumerate() {
        let parent = collection.parent.filter(|parent| known.contains(parent));
        children.entry(parent).or_default().push(index);
    }
    let mut order = Vec::with_capacity(collections.len());
    let mut seen = HashSet::new();
    let mut stack: Vec<(usize, usize)> = children
        .get(&None)
        .map(|roots| roots.iter().rev().map(|index| (*index, 0)).collect())
        .unwrap_or_default();
    while let Some((index, depth)) = stack.pop() {
        if !seen.insert(index) {
            continue;
        }
        order.push((index, depth));
        if let Some(inside) = children.get(&Some(collections[index].id)) {
            stack.extend(inside.iter().rev().map(|child| (*child, depth + 1)));
        }
    }
    order
}

/// Show the panel.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    create_menu(ui, cx);
    let collections = cx.engine.collections().to_vec();
    if collections.is_empty() {
        ui.label(
            RichText::new("Make a collection, then drag photos onto it.")
                .color(palette::TEXT_DIM),
        );
        return;
    }
    let current = *cx.engine.source();
    let mut hidden_below: Option<usize> = None;
    for (index, depth) in tree(&collections) {
        if let Some(closed) = hidden_below {
            if depth > closed {
                continue;
            }
            hidden_below = None;
        }
        let collection = &collections[index];
        let open = !cx.ui.left.closed_sets.contains(&collection.id);
        if collection.kind == CollectionKind::Set && !open {
            hidden_below = Some(depth);
        }
        row(ui, cx, collection, depth, open, current);
    }
}

/// The "+" button and the menu of what it creates.
fn create_menu(ui: &mut Ui, cx: &mut Cx<'_>) {
    let response = ui
        .horizontal(|ui| {
            let button = ui.add(IconButton::new(Icon::Plus, "Create collection").side(20.0));
            ui.label(RichText::new("New").color(palette::TEXT_DIM));
            button
        })
        .inner;
    egui::Popup::menu(&response).show(|ui| new_items(ui, cx, None));
    ui.add_space(space::XS);
}

/// The three ways to make something, inside the set `parent` or at the
/// top level.
fn new_items(ui: &mut Ui, cx: &mut Cx<'_>, parent: Option<CollectionId>) {
    let inside = if parent.is_some() { " Inside" } else { "" };
    if ui.button(format!("New Collection{inside}…")).clicked() {
        dialogs::open_new_collection(cx, parent);
    }
    if ui.button(format!("New Collection Set{inside}…")).clicked() {
        dialogs::open_new(cx, NewKind::Set, parent);
    }
    if ui.button(format!("New Smart Collection{inside}…")).clicked() {
        dialogs::open_new_smart(cx, parent);
    }
}

/// One collection's row, its menu, and what is dropped on it.
fn row(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    collection: &CollectionInfo,
    depth: usize,
    open: bool,
    current: Source,
) {
    let icon = match collection.kind {
        CollectionKind::Set => Icon::Folder,
        CollectionKind::Smart => Icon::SmartCollection,
        CollectionKind::Regular | CollectionKind::Quick => Icon::Collection,
    };
    let name = format!("{} {}", text::kind_name(collection.kind), collection.name);
    let takes_photos = collection.kind == CollectionKind::Regular;
    let mut widget = ListRow::new(&name, &collection.name)
        .icon(icon)
        .depth(depth)
        .count(collection.photo_count)
        .selected(current == Source::Collection(collection.id))
        .accepts_drop(takes_photos);
    if collection.kind == CollectionKind::Set {
        widget = widget.expanded(open);
    }
    let row = widget.show(ui);
    if takes_photos && let Some(dragged) = row.response.dnd_release_payload::<DraggedPhotos>() {
        let target = Target::Photos(dragged.0.clone());
        cx.act(Action::AddToCollection(collection.id, target));
    } else if row.toggled {
        if open {
            cx.ui.left.closed_sets.insert(collection.id);
        } else {
            cx.ui.left.closed_sets.remove(&collection.id);
        }
    } else if row.clicked {
        cx.act(Action::SetSource(Source::Collection(collection.id)));
    }
    egui::Popup::context_menu(&row.response).show(|ui| {
        if collection.kind == CollectionKind::Set {
            new_items(ui, cx, Some(collection.id));
            ui.separator();
        }
        if collection.kind == CollectionKind::Smart && ui.button("Edit Smart Collection…").clicked()
        {
            dialogs::open_edit_smart(cx, collection.id);
        }
        if ui.button("Rename…").clicked() {
            dialogs::open_rename(cx, collection);
        }
        if ui.button("Delete…").clicked() {
            dialogs::open_delete(cx, collection);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection(id: i64, parent: Option<i64>, kind: CollectionKind, name: &str) -> CollectionInfo {
        CollectionInfo {
            id: CollectionId(id),
            parent: parent.map(CollectionId),
            kind,
            name: name.to_owned(),
            photo_count: 0,
        }
    }

    #[test]
    fn a_set_is_followed_by_what_it_holds() {
        let collections = [
            collection(1, None, CollectionKind::Regular, "Best"),
            collection(2, None, CollectionKind::Set, "Trips"),
            collection(3, Some(2), CollectionKind::Regular, "Iceland"),
            collection(4, Some(2), CollectionKind::Smart, "Iceland five stars"),
            collection(5, Some(99), CollectionKind::Regular, "Orphan"),
        ];
        let order: Vec<(&str, usize)> = tree(&collections)
            .into_iter()
            .map(|(index, depth)| (collections[index].name.as_str(), depth))
            .collect();
        assert_eq!(
            order,
            [
                ("Best", 0),
                ("Trips", 0),
                ("Iceland", 1),
                ("Iceland five stars", 1),
                ("Orphan", 0),
            ]
        );
    }

    #[test]
    fn a_cycle_cannot_hang_the_tree() {
        let collections = [
            collection(1, Some(2), CollectionKind::Set, "A"),
            collection(2, Some(1), CollectionKind::Set, "B"),
        ];
        // Neither is at the top level, so neither is shown, and nothing loops.
        assert!(tree(&collections).is_empty());
    }
}
