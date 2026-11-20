//! The Folders panel: the folders that hold catalogued files, as a tree
//! built from their paths, each with the number of photos in it and the
//! folders beneath. Clicking a folder shows its photos and its subfolders'.

use egui::{RichText, Ui};
use photo_engine::{FolderInfo, Source};

use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::text;
use crate::theme::palette;
use crate::widgets::{Icon, ListRow};

/// One folder in the tree.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Node {
    /// Where the folder is in the engine's list.
    pub(super) index: usize,
    /// How many folders up the tree its nearest listed ancestor is.
    pub(super) depth: usize,
    /// Its photos and those of every folder beneath it.
    pub(super) total: u64,
    /// Whether a listed folder is beneath it.
    pub(super) has_children: bool,
}

/// The folders as a tree in display order: each folder after the nearest
/// listed folder its path lies in.
pub(super) fn tree(folders: &[FolderInfo]) -> Vec<Node> {
    let mut order: Vec<usize> = (0..folders.len()).collect();
    // Path order compares component by component, so a folder comes right
    // before everything inside it.
    order.sort_by(|a, b| folders[*a].path.cmp(&folders[*b].path));
    let mut nodes: Vec<Node> = Vec::with_capacity(folders.len());
    let mut ancestors: Vec<usize> = Vec::new();
    for index in order {
        let path = &folders[index].path;
        while let Some(&top) = ancestors.last() {
            if path.starts_with(&folders[nodes[top].index].path) {
                break;
            }
            ancestors.pop();
        }
        if let Some(&parent) = ancestors.last() {
            nodes[parent].has_children = true;
        }
        nodes.push(Node {
            index,
            depth: ancestors.len(),
            total: folders[index].photo_count,
            has_children: false,
        });
        ancestors.push(nodes.len() - 1);
    }
    // Totals, from the deepest folders up.
    for at in (0..nodes.len()).rev() {
        let depth = nodes[at].depth;
        if depth == 0 {
            continue;
        }
        if let Some(parent) = (0..at).rev().find(|before| nodes[*before].depth < depth) {
            nodes[parent].total += nodes[at].total;
        }
    }
    nodes
}

/// Show the folder tree.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let folders = cx.engine.folders().to_vec();
    if folders.is_empty() {
        ui.label(RichText::new("Folders appear here once photos are imported.").color(palette::TEXT_DIM));
        return;
    }
    let current = *cx.engine.source();
    let nodes = tree(&folders);
    let mut hidden_below: Option<usize> = None;
    for node in &nodes {
        if let Some(depth) = hidden_below {
            if node.depth > depth {
                continue;
            }
            hidden_below = None;
        }
        let folder = &folders[node.index];
        let label = text::file_name(&folder.path);
        let name = format!("Folder {label}");
        let open = !cx.ui.left.closed_folders.contains(&folder.id);
        let mut row = ListRow::new(&name, &label)
            .icon(Icon::Folder)
            .depth(node.depth)
            .count(node.total)
            .selected(matches!(current, Source::Folder { id, .. } if id == folder.id));
        if node.has_children {
            row = row.expanded(open);
        }
        let row = row.show(ui);
        // The full path, for folders whose names repeat.
        let path = folder.path.display().to_string();
        row.response.clone().on_hover_text(path);
        if row.toggled {
            if open {
                cx.ui.left.closed_folders.insert(folder.id);
            } else {
                cx.ui.left.closed_folders.remove(&folder.id);
            }
        } else if row.clicked {
            cx.act(Action::SetSource(Source::Folder {
                id: folder.id,
                include_subfolders: true,
            }));
        }
        if node.has_children && !open {
            hidden_below = Some(node.depth);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use photo_core::meta::FolderId;

    use super::*;

    fn folder(id: i64, path: &str, photo_count: u64) -> FolderInfo {
        FolderInfo {
            id: FolderId(id),
            path: PathBuf::from(path),
            photo_count,
        }
    }

    #[test]
    fn folders_nest_by_path_and_count_what_is_beneath_them() {
        let folders = [
            folder(1, "/photos/2024 trip", 4),
            folder(2, "/photos/2024", 1),
            folder(3, "/photos/2024/03", 2),
            folder(4, "/photos/2024/03/raw", 5),
            folder(5, "/elsewhere", 7),
        ];
        let nodes = tree(&folders);
        let shown: Vec<(&str, usize, u64)> = nodes
            .iter()
            .map(|node| {
                let path = folders[node.index].path.to_str().unwrap_or_default();
                (path, node.depth, node.total)
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("/elsewhere", 0, 7),
                ("/photos/2024", 0, 8),
                ("/photos/2024/03", 1, 7),
                ("/photos/2024/03/raw", 2, 5),
                ("/photos/2024 trip", 0, 4),
            ]
        );
        assert!(nodes[1].has_children && nodes[2].has_children && !nodes[3].has_children);
    }
}
