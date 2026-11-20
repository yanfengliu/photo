# Collections Feature Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add named photo collections with sidebar UI, context menus, and drag-and-drop in the Library view.

**Architecture:** New `collection.rs` module for data model and JSON persistence (`LOCALAPPDATA/photo/collections.json`). All collection UI lives in `main.rs`: sidebar in Library view, collection grid sub-view, context menu overlay via `stack` + `mouse_area` widgets, drag-and-drop via global mouse event tracking. `serde` + `serde_json` added for JSON serialization.

**Tech Stack:** Rust, iced 0.13 (`mouse_area`, `stack`, `Space` widgets), serde 1 + serde_json 1

---

### Task 1: Dependencies + collection.rs Module with Tests

**Files:**
- Modify: `Cargo.toml:7`
- Create: `src/collection.rs`
- Modify: `src/main.rs:1-7` (add `mod collection;`)

- [ ] **Step 1: Add serde and serde_json to Cargo.toml**

Add after the `natord` line in `Cargo.toml`:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

- [ ] **Step 2: Write tests for CollectionStore**

Create `src/collection.rs` with tests first (no implementation yet — just types and test stubs):

```rust
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub name: String,
    pub photos: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct CollectionStore {
    pub collections: Vec<Collection>,
}

impl CollectionStore {
    pub fn create(&mut self, _name: &str) { todo!() }
    pub fn rename(&mut self, _index: usize, _new_name: &str) { todo!() }
    pub fn delete(&mut self, _index: usize) { todo!() }
    pub fn add_photo(&mut self, _collection_index: usize, _path: &Path) { todo!() }
    pub fn remove_photo(&mut self, _collection_index: usize, _path: &Path) { todo!() }
    pub fn next_default_name(&self) -> String { todo!() }
    pub fn save_to(&self, _path: &Path) { todo!() }
    pub fn load_from(path: &Path) -> Self { todo!() }
}

pub fn collections_file_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|dir| Path::new(&dir).join("photo").join("collections.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_adds_collection_sorted() {
        let mut store = CollectionStore::default();
        store.create("Zebra");
        store.create("Alpha");
        assert_eq!(store.collections.len(), 2);
        assert_eq!(store.collections[0].name, "Alpha");
        assert_eq!(store.collections[1].name, "Zebra");
    }

    #[test]
    fn create_empty_photos() {
        let mut store = CollectionStore::default();
        store.create("Test");
        assert!(store.collections[0].photos.is_empty());
    }

    #[test]
    fn rename_resorts() {
        let mut store = CollectionStore::default();
        store.create("Alpha");
        store.create("Beta");
        // Rename "Alpha" (index 0) to "Zeta" — should move to end
        store.rename(0, "Zeta");
        assert_eq!(store.collections[0].name, "Beta");
        assert_eq!(store.collections[1].name, "Zeta");
    }

    #[test]
    fn rename_out_of_bounds_no_panic() {
        let mut store = CollectionStore::default();
        store.rename(99, "Nope");
        assert!(store.collections.is_empty());
    }

    #[test]
    fn delete_removes_collection() {
        let mut store = CollectionStore::default();
        store.create("A");
        store.create("B");
        store.create("C");
        store.delete(1); // delete "B"
        assert_eq!(store.collections.len(), 2);
        assert_eq!(store.collections[0].name, "A");
        assert_eq!(store.collections[1].name, "C");
    }

    #[test]
    fn delete_out_of_bounds_no_panic() {
        let mut store = CollectionStore::default();
        store.delete(0);
        assert!(store.collections.is_empty());
    }

    #[test]
    fn add_photo_no_duplicates() {
        let mut store = CollectionStore::default();
        store.create("Test");
        let path = PathBuf::from("/photo/a.jpg");
        store.add_photo(0, &path);
        store.add_photo(0, &path); // duplicate
        assert_eq!(store.collections[0].photos.len(), 1);
    }

    #[test]
    fn add_photo_out_of_bounds_no_panic() {
        let mut store = CollectionStore::default();
        store.add_photo(99, &PathBuf::from("/a.jpg"));
    }

    #[test]
    fn remove_photo_keeps_others() {
        let mut store = CollectionStore::default();
        store.create("Test");
        let a = PathBuf::from("/photo/a.jpg");
        let b = PathBuf::from("/photo/b.jpg");
        store.add_photo(0, &a);
        store.add_photo(0, &b);
        store.remove_photo(0, &a);
        assert_eq!(store.collections[0].photos, vec![b]);
    }

    #[test]
    fn remove_photo_not_present_no_panic() {
        let mut store = CollectionStore::default();
        store.create("Test");
        store.remove_photo(0, &PathBuf::from("/not_here.jpg"));
        assert!(store.collections[0].photos.is_empty());
    }

    #[test]
    fn next_default_name_increments() {
        let mut store = CollectionStore::default();
        assert_eq!(store.next_default_name(), "New Collection");
        store.create("New Collection");
        assert_eq!(store.next_default_name(), "New Collection 2");
        store.create("New Collection 2");
        assert_eq!(store.next_default_name(), "New Collection 3");
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("collections.json");

        let mut store = CollectionStore::default();
        store.create("Vacation");
        store.add_photo(0, &PathBuf::from("/photo/a.jpg"));
        store.add_photo(0, &PathBuf::from("/photo/b.png"));
        store.create("Work");
        store.add_photo(1, &PathBuf::from("/photo/c.jpg"));
        store.save_to(&file);

        let loaded = CollectionStore::load_from(&file);
        assert_eq!(loaded.collections.len(), 2);
        assert_eq!(loaded.collections[0].name, "Vacation");
        assert_eq!(loaded.collections[0].photos.len(), 2);
        assert_eq!(loaded.collections[1].name, "Work");
        assert_eq!(loaded.collections[1].photos.len(), 1);
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let store = CollectionStore::load_from(Path::new("/nonexistent/collections.json"));
        assert!(store.collections.is_empty());
    }

    #[test]
    fn load_corrupt_json_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("collections.json");
        std::fs::write(&file, "not valid json{{{").unwrap();
        let store = CollectionStore::load_from(&file);
        assert!(store.collections.is_empty());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib collection::tests -- --nocapture 2>&1 | tail -5`
Expected: FAIL — all tests panic with `todo!()`

- [ ] **Step 4: Implement CollectionStore methods**

Replace the `todo!()` stubs in `src/collection.rs` with real implementations:

```rust
impl CollectionStore {
    pub fn create(&mut self, name: &str) {
        self.collections.push(Collection {
            name: name.to_string(),
            photos: Vec::new(),
        });
        self.sort();
    }

    pub fn rename(&mut self, index: usize, new_name: &str) {
        if let Some(c) = self.collections.get_mut(index) {
            c.name = new_name.to_string();
        }
        self.sort();
    }

    pub fn delete(&mut self, index: usize) {
        if index < self.collections.len() {
            self.collections.remove(index);
        }
    }

    pub fn add_photo(&mut self, collection_index: usize, path: &Path) {
        if let Some(c) = self.collections.get_mut(collection_index) {
            let pb = path.to_path_buf();
            if !c.photos.contains(&pb) {
                c.photos.push(pb);
            }
        }
    }

    pub fn remove_photo(&mut self, collection_index: usize, path: &Path) {
        if let Some(c) = self.collections.get_mut(collection_index) {
            c.photos.retain(|p| p != path);
        }
    }

    pub fn next_default_name(&self) -> String {
        let base = "New Collection";
        if !self.collections.iter().any(|c| c.name == base) {
            return base.to_string();
        }
        for i in 2.. {
            let name = format!("{base} {i}");
            if !self.collections.iter().any(|c| c.name == name) {
                return name;
            }
        }
        unreachable!()
    }

    pub fn save_to(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.collections) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn load_from(path: &Path) -> Self {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        let collections: Vec<Collection> = serde_json::from_str(&content).unwrap_or_default();
        CollectionStore { collections }
    }

    pub fn save(&self) {
        if let Some(path) = collections_file_path() {
            self.save_to(&path);
        }
    }

    pub fn load() -> Self {
        match collections_file_path() {
            Some(path) => Self::load_from(&path),
            None => Self::default(),
        }
    }

    fn sort(&mut self) {
        self.collections
            .sort_by(|a, b| natord::compare(&a.name, &b.name));
    }
}
```

- [ ] **Step 5: Add `mod collection;` to main.rs**

Add `mod collection;` after line 6 (`mod nav;`) in `src/main.rs`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: all tests pass (including existing tests)

- [ ] **Step 7: Build release**

Run: `cargo build --release 2>&1 | tail -1`
Expected: `Finished`

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/collection.rs src/main.rs
git commit -m "feat: add collection.rs module with data model, CRUD, and persistence"
```

---

### Task 2: App State Integration + Sidebar UI

**Files:**
- Modify: `src/main.rs` (imports, types, App struct, new(), library_view, view)

- [ ] **Step 1: Add new imports to main.rs**

Update the iced widget import (line 14-17) to add `mouse_area`, `stack`, `Space`:

```rust
use iced::widget::{
    button, column, container, horizontal_space, mouse_area, pick_list, row, scrollable, shader,
    slider, stack, text, text_input, Image, Space,
};
```

Add `mouse` and `Point` to the iced import (line 18-21):

```rust
use iced::{
    event, keyboard, mouse, window, Alignment, Background, Border, Color, Element, Length, Point,
    Size, Subscription, Task, Theme,
};
```

- [ ] **Step 2: Add context menu and drag types**

Add after the `SliderKind` enum (after line 82):

```rust
#[derive(Debug, Clone)]
enum ContextMenuKind {
    LibraryPhoto { photo_index: usize },
    CollectionPhoto { photo_index: usize },
    SidebarCollection { collection_index: usize },
}

#[derive(Debug, Clone)]
struct ContextMenu {
    position: [f32; 2],
    kind: ContextMenuKind,
}

struct DragState {
    photo_index: usize,
    start_pos: [f32; 2],
    current_pos: [f32; 2],
    active: bool,
}
```

- [ ] **Step 3: Add new fields to App struct**

Add these fields to the `App` struct (after line 118, the `lens_override_name` field):

```rust
    collection_store: collection::CollectionStore,
    active_collection: Option<usize>,
    context_menu: Option<ContextMenu>,
    drag_state: Option<DragState>,
    editing_collection_name: Option<usize>,
    collection_name_buf: String,
    hovered_thumbnail: Option<usize>,
    sidebar_hover_collection: Option<usize>,
    cursor_position: [f32; 2],
    last_collection_click: Option<(usize, Instant)>,
    /// When entering Detail from a collection, stores (collection_index, photo_index_within_collection).
    collection_nav: Option<(usize, usize)>,
```

- [ ] **Step 4: Initialize new fields in App::new()**

In `App::new()` (inside the `App { ... }` struct literal, after `lens_override_name: None,`), add:

```rust
            collection_store: collection::CollectionStore::load(),
            active_collection: None,
            context_menu: None,
            drag_state: None,
            editing_collection_name: None,
            collection_name_buf: String::new(),
            hovered_thumbnail: None,
            sidebar_hover_collection: None,
            cursor_position: [0.0, 0.0],
            last_collection_click: None,
            collection_nav: None,
```

- [ ] **Step 5: Add new Message variants**

Add these variants to the `Message` enum (after `LensProfileSelected(String)`):

```rust
    // Collections
    CreateCollection,
    CollectionNameChanged(String),
    CollectionNameSubmit,
    CollectionNameCancel,
    SidebarCollectionClicked(usize),
    SidebarCollectionRightClicked(usize),
    SidebarCollectionHovered(Option<usize>),
    ExitCollectionView,
    CollectionPhotoClicked(usize),
    CollectionPhotoRightClicked(usize),
    // Context menu
    DismissContextMenu,
    ContextMenuRename,
    ContextMenuDelete,
    AddPhotoToCollection(usize),
    RemovePhotoFromCollection,
    // Thumbnail hover (for right-click targeting)
    ThumbnailHovered(Option<usize>),
    // Right-click on library thumbnail
    LibraryPhotoRightClicked(usize),
    // Toggle photo membership in a collection (from library context menu)
    TogglePhotoInCollection(usize),
    // Back from detail to collection grid
    ExitCollectionDetail,
```

- [ ] **Step 6: Add stub handlers for new messages**

In `App::update()`, add a catch-all for the new messages at the end of the match (before the closing `}`). These will be implemented in later tasks:

```rust
            // -- Collection stubs (implemented in later tasks) --
            Message::CreateCollection
            | Message::CollectionNameChanged(_)
            | Message::CollectionNameSubmit
            | Message::CollectionNameCancel
            | Message::SidebarCollectionClicked(_)
            | Message::SidebarCollectionRightClicked(_)
            | Message::SidebarCollectionHovered(_)
            | Message::ExitCollectionView
            | Message::CollectionPhotoClicked(_)
            | Message::CollectionPhotoRightClicked(_)
            | Message::DismissContextMenu
            | Message::ContextMenuRename
            | Message::ContextMenuDelete
            | Message::AddPhotoToCollection(_)
            | Message::RemovePhotoFromCollection
            | Message::ThumbnailHovered(_)
            | Message::LibraryPhotoRightClicked(_)
            | Message::TogglePhotoInCollection(_)
            | Message::ExitCollectionDetail => Task::none(),
```

- [ ] **Step 7: Add cursor tracking to handle_event**

In `handle_event()`, add a new match arm before the `_ => Task::none()` catch-all:

```rust
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                self.cursor_position = [position.x, position.y];
                Task::none()
            }
```

- [ ] **Step 8: Add sidebar rendering method**

Add the `collection_sidebar` method to the `impl App` block (after `library_view`):

```rust
    fn collection_sidebar(&self) -> Element<'_, Message> {
        let header = row![
            container(text("COLLECTIONS").size(10).color(TEXT_DIM)).padding([5, 0]),
            horizontal_space(),
            button(text("+").size(14).color(TEXT_PRIMARY))
                .on_press(Message::CreateCollection)
                .padding([2, 8])
                .style(toolbar_button_style),
        ]
        .align_y(Alignment::Center);

        let mut list = column![].spacing(2);
        for (i, col) in self.collection_store.collections.iter().enumerate() {
            let entry: Element<'_, Message> = if self.editing_collection_name == Some(i) {
                text_input("Collection name", &self.collection_name_buf)
                    .on_input(Message::CollectionNameChanged)
                    .on_submit(Message::CollectionNameSubmit)
                    .size(12)
                    .width(Length::Fill)
                    .into()
            } else {
                let label = format!("{} ({})", col.name, col.photos.len());
                let is_drop_target = self
                    .drag_state
                    .as_ref()
                    .map_or(false, |d| d.active)
                    && self.sidebar_hover_collection == Some(i);
                let style_fn = if is_drop_target {
                    sidebar_item_drop_target_style
                } else {
                    sidebar_item_style
                };
                mouse_area(
                    button(text(label).size(12).color(TEXT_SECONDARY))
                        .on_press(Message::SidebarCollectionClicked(i))
                        .padding([4, 8])
                        .width(Length::Fill)
                        .style(style_fn),
                )
                .on_right_press(Message::SidebarCollectionRightClicked(i))
                .on_enter(Message::SidebarCollectionHovered(Some(i)))
                .on_exit(Message::SidebarCollectionHovered(None))
                .into()
            };
            list = list.push(entry);
        }

        container(
            column![header, scrollable(list).height(Length::Fill)]
                .spacing(6)
                .padding(8),
        )
        .width(180)
        .height(Length::Fill)
        .style(panel_container_style)
        .into()
    }
```

- [ ] **Step 9: Add sidebar styles**

Add these style functions after the existing style functions (after `invisible_button_style`):

```rust
fn sidebar_item_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Some(Background::Color(BG_BUTTON_HOVER)),
        _ => None,
    };
    button::Style {
        background: bg,
        text_color: TEXT_SECONDARY,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 3.0.into(),
        },
        shadow: Default::default(),
    }
}

fn sidebar_item_drop_target_style(_theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(Color::from_rgb(0.2, 0.3, 0.4))),
        text_color: TEXT_PRIMARY,
        border: Border {
            color: Color::from_rgb(0.3, 0.5, 0.7),
            width: 1.0,
            radius: 3.0.into(),
        },
        shadow: Default::default(),
    }
}
```

- [ ] **Step 10: Integrate sidebar into library_view**

Replace the thumbnail grid section of `library_view()`. Find the block starting at `let thumb_size: f32 = 150.0;` (around line 904) through the end of the function return. Replace the body of `library_view` (after the empty-library early return) with:

```rust
        let thumb_size: f32 = 150.0;
        let cols = 6;

        let entries: Vec<(usize, &LibraryEntry)> = self.library.iter().enumerate().collect();
        let mut grid = column![].spacing(8);

        for chunk in entries.chunks(cols) {
            let mut r = row![].spacing(8);
            for &(idx, entry) in chunk {
                r = r.push(self.thumbnail_card(entry, idx, thumb_size));
            }
            grid = grid.push(r);
        }

        let status_text = format!(
            "{} images  \u{2022}  Double-click to open",
            self.library.len()
        );
        let status = container(text(status_text).size(11).color(TEXT_DIM))
            .width(Length::Fill)
            .padding([6, 14]);

        let grid_area = column![
            scrollable(container(grid).padding(14).width(Length::Fill)).height(Length::Fill),
            container(status)
                .width(Length::Fill)
                .style(toolbar_container_style),
        ];

        let sidebar = self.collection_sidebar();
        let divider = container(Space::with_width(1))
            .height(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(DIVIDER)),
                ..Default::default()
            });

        container(row![sidebar, divider, container(grid_area).width(Length::Fill)])
            .style(dark_bg_style)
            .into()
```

- [ ] **Step 11: Build and verify**

Run: `cargo build --release 2>&1 | tail -1`
Expected: `Finished`

If `mouse_area` or `stack` imports fail, check iced 0.13 widget exports. `mouse_area` may need `use iced::widget::MouseArea;` instead. `stack` may need `use iced::widget::Stack;`. Fix import errors if any.

- [ ] **Step 12: Run all tests**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: all tests pass

- [ ] **Step 13: Commit**

```bash
git add src/main.rs
git commit -m "feat: add collection sidebar UI in library view with empty state"
```

---

### Task 3: Collection CRUD — Create + Inline Rename

**Files:**
- Modify: `src/main.rs` (update handlers)

- [ ] **Step 1: Implement CreateCollection handler**

Replace the `Message::CreateCollection` stub in the match arm with:

```rust
            Message::CreateCollection => {
                let name = self.collection_store.next_default_name();
                self.collection_store.create(&name);
                self.collection_store.save();
                // Find the new collection's index after sorting and enter rename mode
                let idx = self
                    .collection_store
                    .collections
                    .iter()
                    .position(|c| c.name == name)
                    .unwrap_or(0);
                self.editing_collection_name = Some(idx);
                self.collection_name_buf = name;
                Task::none()
            }
```

- [ ] **Step 2: Implement CollectionNameChanged handler**

Replace the stub:

```rust
            Message::CollectionNameChanged(s) => {
                self.collection_name_buf = s;
                Task::none()
            }
```

- [ ] **Step 3: Implement CollectionNameSubmit handler**

Replace the stub:

```rust
            Message::CollectionNameSubmit => {
                if let Some(idx) = self.editing_collection_name.take() {
                    let new_name = self.collection_name_buf.trim().to_string();
                    if !new_name.is_empty() {
                        self.collection_store.rename(idx, &new_name);
                    }
                    self.collection_store.save();
                    self.collection_name_buf.clear();
                }
                Task::none()
            }
```

- [ ] **Step 4: Implement CollectionNameCancel handler**

Replace the stub:

```rust
            Message::CollectionNameCancel => {
                self.editing_collection_name = None;
                self.collection_name_buf.clear();
                Task::none()
            }
```

- [ ] **Step 5: Implement SidebarCollectionClicked (double-click detection)**

Replace the stub:

```rust
            Message::SidebarCollectionClicked(index) => {
                let now = Instant::now();
                let is_double_click = self
                    .last_collection_click
                    .map(|(prev_idx, prev_time)| {
                        prev_idx == index && now.duration_since(prev_time).as_millis() < 400
                    })
                    .unwrap_or(false);

                if is_double_click {
                    self.last_collection_click = None;
                    self.active_collection = Some(index);
                } else {
                    self.last_collection_click = Some((index, now));
                }
                Task::none()
            }
```

- [ ] **Step 6: Implement SidebarCollectionHovered**

Replace the stub:

```rust
            Message::SidebarCollectionHovered(idx) => {
                self.sidebar_hover_collection = idx;
                Task::none()
            }
```

- [ ] **Step 7: Implement ThumbnailHovered**

Replace the stub:

```rust
            Message::ThumbnailHovered(idx) => {
                self.hovered_thumbnail = idx;
                Task::none()
            }
```

- [ ] **Step 8: Handle Escape to cancel rename in handle_key**

In `handle_key()`, update the Escape arm (around line 643) to also cancel inline rename:

```rust
            Key::Named(Named::Escape) => {
                if self.context_menu.is_some() {
                    self.context_menu = None;
                } else if self.editing_collection_name.is_some() {
                    self.editing_collection_name = None;
                    self.collection_name_buf.clear();
                } else if self.active_collection.is_some() {
                    self.active_collection = None;
                } else if self.tab == Tab::Detail {
                    self.tab = Tab::Library;
                }
            }
```

- [ ] **Step 9: Build and verify**

Run: `cargo build --release 2>&1 | tail -1`
Expected: `Finished`

- [ ] **Step 10: Run all tests**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: all tests pass

- [ ] **Step 11: Commit**

```bash
git add src/main.rs
git commit -m "feat: collection create with inline rename and double-click to open"
```

---

### Task 4: Context Menu System + Sidebar Rename/Delete

**Files:**
- Modify: `src/main.rs` (context menu rendering, handlers, overlay in view)

- [ ] **Step 1: Add context menu rendering method**

Add this method to `impl App` (after `collection_sidebar`):

```rust
    fn context_menu_overlay(&self, menu: &ContextMenu) -> Element<'_, Message> {
        let items: Vec<Element<'_, Message>> = match &menu.kind {
            ContextMenuKind::SidebarCollection { .. } => {
                vec![
                    context_menu_item("Rename", Message::ContextMenuRename),
                    context_menu_item("Delete", Message::ContextMenuDelete),
                ]
            }
            ContextMenuKind::LibraryPhoto { photo_index } => {
                let photo_path = &self.library[*photo_index].path;
                self.collection_store
                    .collections
                    .iter()
                    .enumerate()
                    .map(|(i, col)| {
                        if col.photos.contains(photo_path) {
                            context_menu_item(
                                &format!("\u{2713} {}", col.name),
                                Message::RemovePhotoFromCollection,
                            )
                        } else {
                            context_menu_item(
                                &format!("Add to {}", col.name),
                                Message::AddPhotoToCollection(i),
                            )
                        }
                    })
                    .collect()
            }
            ContextMenuKind::CollectionPhoto { .. } => {
                let col_name = self
                    .active_collection
                    .and_then(|i| self.collection_store.collections.get(i))
                    .map(|c| c.name.as_str())
                    .unwrap_or("Collection");
                vec![context_menu_item(
                    &format!("Remove from {col_name}"),
                    Message::RemovePhotoFromCollection,
                )]
            }
        };

        let menu_content = container(column(items).spacing(2).padding(4))
            .style(context_menu_container_style)
            .width(Length::Shrink);

        // Clamp position so menu doesn't go off-screen (assume ~200px wide, ~30px per item)
        let x = menu.position[0].min(1000.0).max(0.0);
        let y = menu.position[1].min(700.0).max(0.0);

        let positioned = column![
            Space::with_height(y),
            row![Space::with_width(x), menu_content,]
        ];

        mouse_area(
            container(positioned)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(Message::DismissContextMenu)
        .into()
    }
```

- [ ] **Step 2: Add context_menu_item helper and style**

Add these free functions after the existing style functions:

```rust
fn context_menu_item(label: &str, msg: Message) -> Element<'_, Message> {
    button(text(label.to_string()).size(12).color(TEXT_PRIMARY))
        .on_press(msg)
        .padding([4, 12])
        .width(Length::Fill)
        .style(context_menu_button_style)
        .into()
}

fn context_menu_container_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb(0.2, 0.2, 0.2))),
        border: Border {
            color: Color::from_rgb(0.3, 0.3, 0.3),
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

fn context_menu_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Some(Background::Color(Color::from_rgb(0.3, 0.4, 0.55))),
        _ => None,
    };
    button::Style {
        background: bg,
        text_color: TEXT_PRIMARY,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 2.0.into(),
        },
        shadow: Default::default(),
    }
}
```

- [ ] **Step 3: Wire overlay into view()**

Replace the `view()` method:

```rust
    fn view(&self) -> Element<'_, Message> {
        let tab_bar = self.tab_bar();
        let content: Element<'_, Message> = match self.tab {
            Tab::Library => self.library_view(),
            Tab::Detail => self.detail_view(),
        };
        let main = column![tab_bar, content];

        let has_overlay = self.context_menu.is_some()
            || self.drag_state.as_ref().map_or(false, |d| d.active);

        if has_overlay {
            let mut layers: Vec<Element<'_, Message>> = vec![main.into()];
            if let Some(ref menu) = self.context_menu {
                layers.push(self.context_menu_overlay(menu));
            }
            if let Some(ref drag) = self.drag_state {
                if drag.active {
                    layers.push(self.drag_overlay(drag));
                }
            }
            stack(layers).into()
        } else {
            main.into()
        }
    }
```

- [ ] **Step 4: Add drag_overlay stub**

Add a placeholder method (implemented fully in Task 9):

```rust
    fn drag_overlay(&self, drag: &DragState) -> Element<'_, Message> {
        let _ = drag;
        Space::new(0, 0).into()
    }
```

- [ ] **Step 5: Implement SidebarCollectionRightClicked handler**

Replace the stub:

```rust
            Message::SidebarCollectionRightClicked(index) => {
                self.context_menu = Some(ContextMenu {
                    position: self.cursor_position,
                    kind: ContextMenuKind::SidebarCollection {
                        collection_index: index,
                    },
                });
                Task::none()
            }
```

- [ ] **Step 6: Implement DismissContextMenu handler**

Replace the stub:

```rust
            Message::DismissContextMenu => {
                self.context_menu = None;
                Task::none()
            }
```

- [ ] **Step 7: Implement ContextMenuRename handler**

Replace the stub:

```rust
            Message::ContextMenuRename => {
                if let Some(ContextMenu {
                    kind: ContextMenuKind::SidebarCollection { collection_index },
                    ..
                }) = &self.context_menu
                {
                    let idx = *collection_index;
                    if let Some(col) = self.collection_store.collections.get(idx) {
                        self.collection_name_buf = col.name.clone();
                        self.editing_collection_name = Some(idx);
                    }
                }
                self.context_menu = None;
                Task::none()
            }
```

- [ ] **Step 8: Implement ContextMenuDelete handler**

Replace the stub:

```rust
            Message::ContextMenuDelete => {
                if let Some(ContextMenu {
                    kind: ContextMenuKind::SidebarCollection { collection_index },
                    ..
                }) = &self.context_menu
                {
                    let idx = *collection_index;
                    self.collection_store.delete(idx);
                    self.collection_store.save();
                    // If we were viewing this collection, exit
                    if self.active_collection == Some(idx) {
                        self.active_collection = None;
                    } else if let Some(active) = self.active_collection {
                        if active > idx {
                            self.active_collection = Some(active - 1);
                        }
                    }
                }
                self.context_menu = None;
                Task::none()
            }
```

- [ ] **Step 9: Add left mouse button handler in handle_event (placeholder for drag)**

In `handle_event()`, add before the `_ => Task::none()` arm. This is a placeholder — drag logic is added in Task 8:

```rust
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                Task::none()
            }

            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                Task::none()
            }
```

- [ ] **Step 10: Build and verify**

Run: `cargo build --release 2>&1 | tail -1`
Expected: `Finished`

- [ ] **Step 11: Run all tests**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: all tests pass

- [ ] **Step 12: Commit**

```bash
git add src/main.rs
git commit -m "feat: context menu system with sidebar rename and delete"
```

---

### Task 5: Collection Grid View

**Files:**
- Modify: `src/main.rs` (collection grid view, tab bar adjustment, library_view routing)

- [ ] **Step 1: Add collection_grid_view method**

Add this method after `collection_sidebar`:

```rust
    fn collection_grid_view(&self, collection_index: usize) -> Element<'_, Message> {
        let collection = &self.collection_store.collections[collection_index];

        let back_btn = button(text("\u{2190}").size(16).color(TEXT_PRIMARY))
            .on_press(Message::ExitCollectionView)
            .padding([4, 12])
            .style(toolbar_button_style);

        let title = text(format!("{} ({})", collection.name, collection.photos.len()))
            .size(14)
            .color(TEXT_PRIMARY);

        let top_bar = container(
            row![back_btn, container(title).padding([0, 8])]
                .spacing(6)
                .align_y(Alignment::Center),
        )
        .padding([6, 10])
        .width(Length::Fill)
        .style(toolbar_container_style);

        let thumb_size: f32 = 150.0;
        let cols = 6;
        let mut grid = column![].spacing(8);

        let photo_entries: Vec<(usize, &PathBuf)> =
            collection.photos.iter().enumerate().collect();

        for chunk in photo_entries.chunks(cols) {
            let mut r = row![].spacing(8);
            for &(photo_idx, photo_path) in chunk {
                let lib_entry = self.library.iter().find(|e| &e.path == photo_path);
                let filename = photo_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                let thumb: Element<'_, Message> =
                    if let Some(Some(ref handle)) = lib_entry.map(|e| &e.thumbnail_handle) {
                        container(
                            Image::new(handle.clone())
                                .width(thumb_size)
                                .height(thumb_size),
                        )
                        .width(thumb_size)
                        .height(thumb_size)
                        .center_x(Length::Shrink)
                        .center_y(Length::Shrink)
                        .into()
                    } else {
                        container(text("...").size(24).color(TEXT_DIM))
                            .width(thumb_size)
                            .height(thumb_size)
                            .center_x(Length::Shrink)
                            .center_y(Length::Shrink)
                            .into()
                    };

                let label = container(text(filename).size(10).color(TEXT_SECONDARY))
                    .width(thumb_size);

                let card = button(column![thumb, label].spacing(4).width(thumb_size))
                    .on_press(Message::CollectionPhotoClicked(photo_idx))
                    .padding(6)
                    .style(card_button_style);

                let card_with_menu = mouse_area(card)
                    .on_right_press(Message::CollectionPhotoRightClicked(photo_idx))
                    .into();

                r = r.push(card_with_menu);
            }
            grid = grid.push(r);
        }

        let status_text = format!("{} photos", collection.photos.len());
        let status = container(text(status_text).size(11).color(TEXT_DIM))
            .width(Length::Fill)
            .padding([6, 14]);

        container(column![
            top_bar,
            scrollable(container(grid).padding(14).width(Length::Fill)).height(Length::Fill),
            container(status)
                .width(Length::Fill)
                .style(toolbar_container_style),
        ])
        .style(dark_bg_style)
        .into()
    }
```

- [ ] **Step 2: Route library_view to collection grid when active**

At the top of `library_view()` (before the empty-library check), add:

```rust
        if let Some(col_idx) = self.active_collection {
            if col_idx < self.collection_store.collections.len() {
                return self.collection_grid_view(col_idx);
            } else {
                // Collection was deleted, reset
                // (can't mutate self here, but this is a view — just show library)
            }
        }
```

- [ ] **Step 3: Implement ExitCollectionView handler**

Replace the stub:

```rust
            Message::ExitCollectionView => {
                self.active_collection = None;
                Task::none()
            }
```

- [ ] **Step 4: Implement CollectionPhotoClicked (double-click to open detail)**

Replace the stub:

```rust
            Message::CollectionPhotoClicked(photo_index) => {
                let now = Instant::now();
                let is_double_click = self
                    .last_thumb_click
                    .map(|(prev_idx, prev_time)| {
                        prev_idx == photo_index && now.duration_since(prev_time).as_millis() < 400
                    })
                    .unwrap_or(false);

                if is_double_click {
                    self.last_thumb_click = None;
                    if let Some(col_idx) = self.active_collection {
                        if let Some(col) = self.collection_store.collections.get(col_idx) {
                            if let Some(photo_path) = col.photos.get(photo_index) {
                                self.collection_nav = Some((col_idx, photo_index));
                                self.library_index = None;
                                self.tab = Tab::Detail;
                                let path = photo_path.clone();
                                self.current_image_path = Some(path.clone());
                                return self.start_load(path);
                            }
                        }
                    }
                } else {
                    self.last_thumb_click = Some((photo_index, now));
                }
                Task::none()
            }
```

- [ ] **Step 5: Implement CollectionPhotoRightClicked**

Replace the stub:

```rust
            Message::CollectionPhotoRightClicked(photo_index) => {
                self.context_menu = Some(ContextMenu {
                    position: self.cursor_position,
                    kind: ContextMenuKind::CollectionPhoto { photo_index },
                });
                Task::none()
            }
```

- [ ] **Step 6: Implement RemovePhotoFromCollection handler**

Replace the stub:

```rust
            Message::RemovePhotoFromCollection => {
                match &self.context_menu {
                    Some(ContextMenu {
                        kind: ContextMenuKind::CollectionPhoto { photo_index },
                        ..
                    }) => {
                        let photo_index = *photo_index;
                        if let Some(col_idx) = self.active_collection {
                            if let Some(col) = self.collection_store.collections.get(col_idx) {
                                if let Some(path) = col.photos.get(photo_index).cloned() {
                                    self.collection_store.remove_photo(col_idx, &path);
                                    self.collection_store.save();
                                }
                            }
                        }
                    }
                    Some(ContextMenu {
                        kind: ContextMenuKind::LibraryPhoto { photo_index },
                        ..
                    }) => {
                        // Remove from whichever collection has a checkmark — find it
                        let photo_index = *photo_index;
                        if let Some(entry) = self.library.get(photo_index) {
                            let path = entry.path.clone();
                            // Find the collection that contains this photo and was clicked
                            // This case is for the checkmark toggle in library context menu
                            for (i, col) in
                                self.collection_store.collections.iter().enumerate()
                            {
                                if col.photos.contains(&path) {
                                    self.collection_store.remove_photo(i, &path);
                                    break;
                                }
                            }
                            self.collection_store.save();
                        }
                    }
                    _ => {}
                }
                self.context_menu = None;
                Task::none()
            }
```

- [ ] **Step 7: Build and verify**

Run: `cargo build --release 2>&1 | tail -1`
Expected: `Finished`

- [ ] **Step 8: Run all tests**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: all tests pass

- [ ] **Step 9: Commit**

```bash
git add src/main.rs
git commit -m "feat: collection grid view with double-click to detail and right-click remove"
```

---

### Task 6: Detail Navigation from Collection

**Files:**
- Modify: `src/main.rs` (handle_key arrow keys, back button, SwitchTab)

- [ ] **Step 1: Update arrow key navigation for collection mode**

In `handle_key()`, update the ArrowRight/Space arm (around line 650). Replace the entire arm body with:

```rust
            Key::Named(Named::ArrowRight) | Key::Named(Named::Space) => {
                if self.tab == Tab::Detail {
                    if let Some((col_idx, ref mut photo_idx)) = self.collection_nav {
                        if let Some(col) = self.collection_store.collections.get(col_idx) {
                            if !col.photos.is_empty() {
                                *photo_idx = (*photo_idx + 1) % col.photos.len();
                                let path = col.photos[*photo_idx].clone();
                                self.current_image_path = Some(path.clone());
                                return self.start_load(path);
                            }
                        }
                    } else if let Some(ref mut lib_idx) = self.library_index {
                        if !self.library.is_empty() {
                            *lib_idx = (*lib_idx + 1) % self.library.len();
                            let path = self.library[*lib_idx].path.clone();
                            self.current_image_path = Some(path.clone());
                            return self.start_load(path);
                        }
                    } else if let Some(nav) = &mut self.nav {
                        if let Some(p) = nav.next() {
                            self.current_image_path = Some(p.clone());
                            return self.start_load(p);
                        }
                    }
                }
            }
```

- [ ] **Step 2: Update ArrowLeft/Backspace navigation similarly**

Replace the ArrowLeft/Backspace arm body:

```rust
            Key::Named(Named::ArrowLeft) | Key::Named(Named::Backspace) => {
                if self.tab == Tab::Detail {
                    if let Some((col_idx, ref mut photo_idx)) = self.collection_nav {
                        if let Some(col) = self.collection_store.collections.get(col_idx) {
                            if !col.photos.is_empty() {
                                *photo_idx = if *photo_idx == 0 {
                                    col.photos.len() - 1
                                } else {
                                    *photo_idx - 1
                                };
                                let path = col.photos[*photo_idx].clone();
                                self.current_image_path = Some(path.clone());
                                return self.start_load(path);
                            }
                        }
                    } else if let Some(ref mut lib_idx) = self.library_index {
                        if !self.library.is_empty() {
                            *lib_idx = if *lib_idx == 0 {
                                self.library.len() - 1
                            } else {
                                *lib_idx - 1
                            };
                            let path = self.library[*lib_idx].path.clone();
                            self.current_image_path = Some(path.clone());
                            return self.start_load(path);
                        }
                    } else if let Some(nav) = &mut self.nav {
                        if let Some(p) = nav.prev() {
                            self.current_image_path = Some(p.clone());
                            return self.start_load(p);
                        }
                    }
                }
            }
```

- [ ] **Step 3: Update Escape to return to collection view when navigating from collection**

Update the Escape arm in `handle_key()`:

```rust
            Key::Named(Named::Escape) => {
                if self.context_menu.is_some() {
                    self.context_menu = None;
                } else if self.editing_collection_name.is_some() {
                    self.editing_collection_name = None;
                    self.collection_name_buf.clear();
                } else if self.tab == Tab::Detail && self.collection_nav.is_some() {
                    // Return to collection grid view
                    self.tab = Tab::Library;
                    self.collection_nav = None;
                } else if self.active_collection.is_some() {
                    self.active_collection = None;
                } else if self.tab == Tab::Detail {
                    self.tab = Tab::Library;
                }
            }
```

- [ ] **Step 4: Update the back button in tab_bar for collection nav**

In `tab_bar()`, update the Detail arm's back button `on_press`. Replace:

```rust
                let back_btn =
                    button(text("\u{2190}").size(16).color(TEXT_PRIMARY))
                        .on_press(Message::SwitchTab(Tab::Library))
                        .padding([4, 12])
                        .style(toolbar_button_style);
```

With:

```rust
                let back_msg = if self.collection_nav.is_some() {
                    Message::ExitCollectionDetail
                } else {
                    Message::SwitchTab(Tab::Library)
                };
                let back_btn = button(text("\u{2190}").size(16).color(TEXT_PRIMARY))
                    .on_press(back_msg)
                    .padding([4, 12])
                    .style(toolbar_button_style);
```

- [ ] **Step 5: Add ExitCollectionDetail message variant and handler**

Add to Message enum:

```rust
    ExitCollectionDetail,
```

Add handler in `update()`:

```rust
            Message::ExitCollectionDetail => {
                self.tab = Tab::Library;
                // active_collection is still set, so we return to collection grid
                self.collection_nav = None;
                Task::none()
            }
```

- [ ] **Step 6: Update status_bar for collection navigation context**

In `status_bar()`, update the position display section. Add a collection nav branch before the `library_index` check:

```rust
            let name = if let Some((col_idx, _)) = self.collection_nav {
                self.collection_store
                    .collections
                    .get(col_idx)
                    .and_then(|c| {
                        self.current_image_path.as_ref().and_then(|p| {
                            p.file_name().and_then(|n| n.to_str()).map(|s| s.to_string())
                        })
                    })
                    .unwrap_or_default()
            } else if let Some(idx) = self.library_index {
```

And for position:

```rust
            let pos = if let Some((col_idx, photo_idx)) = self.collection_nav {
                let total = self
                    .collection_store
                    .collections
                    .get(col_idx)
                    .map(|c| c.photos.len())
                    .unwrap_or(0);
                format!("  {}/{}", photo_idx + 1, total)
            } else if let Some(idx) = self.library_index {
```

- [ ] **Step 7: Build and verify**

Run: `cargo build --release 2>&1 | tail -1`
Expected: `Finished`

- [ ] **Step 8: Run all tests**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: all tests pass

- [ ] **Step 9: Commit**

```bash
git add src/main.rs
git commit -m "feat: detail view navigation within collection with arrow keys"
```

---

### Task 7: Library Photo Context Menu

**Files:**
- Modify: `src/main.rs` (thumbnail_card wrapping, handler)

- [ ] **Step 1: Wrap library thumbnail_card in mouse_area for right-click and hover**

Update `thumbnail_card()`. Replace the final return expression (the `button(column![...])` block):

```rust
        let card = button(column![thumb, label].spacing(4).width(thumb_size))
            .on_press(Message::LibraryItemClicked(index))
            .padding(6)
            .style(card_button_style);

        mouse_area(card)
            .on_right_press(Message::LibraryPhotoRightClicked(index))
            .on_enter(Message::ThumbnailHovered(Some(index)))
            .on_exit(Message::ThumbnailHovered(None))
            .into()
```

- [ ] **Step 2: Implement LibraryPhotoRightClicked handler**

Replace the stub:

```rust
            Message::LibraryPhotoRightClicked(index) => {
                if self.collection_store.collections.is_empty() {
                    return Task::none();
                }
                self.context_menu = Some(ContextMenu {
                    position: self.cursor_position,
                    kind: ContextMenuKind::LibraryPhoto { photo_index: index },
                });
                Task::none()
            }
```

- [ ] **Step 3: Implement AddPhotoToCollection handler**

Replace the stub:

```rust
            Message::AddPhotoToCollection(collection_index) => {
                if let Some(ContextMenu {
                    kind: ContextMenuKind::LibraryPhoto { photo_index },
                    ..
                }) = &self.context_menu
                {
                    if let Some(entry) = self.library.get(*photo_index) {
                        self.collection_store
                            .add_photo(collection_index, &entry.path);
                        self.collection_store.save();
                    }
                }
                self.context_menu = None;
                Task::none()
            }
```

- [ ] **Step 4: Fix RemovePhotoFromCollection for library context menu**

The `RemovePhotoFromCollection` handler from Task 5 Step 6 handles the `LibraryPhoto` case by finding the first collection containing the photo. However, the context menu shows a checkmark for each collection that contains the photo — clicking should toggle that specific collection. We need to track which collection the user clicked.

Update the `LibraryPhoto` context menu rendering in `context_menu_overlay()` to use `AddPhotoToCollection` for both add and remove (toggle behavior):

In `context_menu_overlay()`, replace the `LibraryPhoto` arm:

```rust
            ContextMenuKind::LibraryPhoto { photo_index } => {
                let photo_path = &self.library[*photo_index].path;
                self.collection_store
                    .collections
                    .iter()
                    .enumerate()
                    .map(|(i, col)| {
                        if col.photos.contains(photo_path) {
                            context_menu_item(
                                &format!("\u{2713} {}", col.name),
                                Message::TogglePhotoInCollection(i),
                            )
                        } else {
                            context_menu_item(
                                &format!("Add to {}", col.name),
                                Message::AddPhotoToCollection(i),
                            )
                        }
                    })
                    .collect()
            }
```

Add new message variant:

```rust
    TogglePhotoInCollection(usize),
```

Add handler:

```rust
            Message::TogglePhotoInCollection(collection_index) => {
                if let Some(ContextMenu {
                    kind: ContextMenuKind::LibraryPhoto { photo_index },
                    ..
                }) = &self.context_menu
                {
                    if let Some(entry) = self.library.get(*photo_index) {
                        let path = entry.path.clone();
                        if self
                            .collection_store
                            .collections
                            .get(collection_index)
                            .map_or(false, |c| c.photos.contains(&path))
                        {
                            self.collection_store.remove_photo(collection_index, &path);
                        } else {
                            self.collection_store.add_photo(collection_index, &path);
                        }
                        self.collection_store.save();
                    }
                }
                self.context_menu = None;
                Task::none()
            }
```

- [ ] **Step 5: Build and verify**

Run: `cargo build --release 2>&1 | tail -1`
Expected: `Finished`

- [ ] **Step 6: Run all tests**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "feat: right-click on library photos to add/remove from collections"
```

---

### Task 8: Drag and Drop

**Files:**
- Modify: `src/main.rs` (drag state machine, visual overlay, drop handling)

- [ ] **Step 1: Start potential drag on thumbnail press**

In the `LibraryItemClicked` handler, add drag initialization at the very beginning of the handler (before the double-click check):

```rust
            Message::LibraryItemClicked(index) => {
                // Start potential drag
                self.drag_state = Some(DragState {
                    photo_index: index,
                    start_pos: self.cursor_position,
                    current_pos: self.cursor_position,
                    active: false,
                });

                // Existing double-click detection follows unchanged...
```

- [ ] **Step 2: Update CursorMoved handler for drag tracking**

Update the `CursorMoved` handler in `handle_event()`:

```rust
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                self.cursor_position = [position.x, position.y];
                if let Some(ref mut drag) = self.drag_state {
                    drag.current_pos = [position.x, position.y];
                    if !drag.active {
                        let dx = drag.current_pos[0] - drag.start_pos[0];
                        let dy = drag.current_pos[1] - drag.start_pos[1];
                        if (dx * dx + dy * dy).sqrt() > 5.0 {
                            drag.active = true;
                        }
                    }
                }
                Task::none()
            }
```

- [ ] **Step 3: Handle mouse release for drag drop**

Update the `ButtonPressed(Left)` handler and add a `ButtonReleased(Left)` handler in `handle_event()`.

Replace the existing `ButtonPressed(Left)` arm:

```rust
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if self.context_menu.is_some() {
                    // Don't dismiss here — let the overlay mouse_area handle it
                    // (this fires for ALL left clicks including on menu items)
                }
                Task::none()
            }
```

Add a new arm for `ButtonReleased(Left)`:

```rust
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if let Some(drag) = self.drag_state.take() {
                    if drag.active {
                        if let Some(col_idx) = self.sidebar_hover_collection {
                            if let Some(entry) = self.library.get(drag.photo_index) {
                                self.collection_store
                                    .add_photo(col_idx, &entry.path);
                                self.collection_store.save();
                            }
                        }
                        // Cancel the click that started this drag
                        self.last_thumb_click = None;
                    }
                }
                Task::none()
            }
```

- [ ] **Step 4: Implement drag_overlay with visual indicator**

Replace the stub `drag_overlay` method:

```rust
    fn drag_overlay(&self, drag: &DragState) -> Element<'_, Message> {
        let label = self
            .library
            .get(drag.photo_index)
            .map(|e| e.filename.clone())
            .unwrap_or_default();

        let thumb: Element<'_, Message> = if let Some(Some(ref handle)) = self
            .library
            .get(drag.photo_index)
            .map(|e| &e.thumbnail_handle)
        {
            container(Image::new(handle.clone()).width(60).height(60))
                .width(60)
                .height(60)
                .center_x(Length::Shrink)
                .center_y(Length::Shrink)
                .into()
        } else {
            text(&label).size(11).color(TEXT_PRIMARY).into()
        };

        let drag_widget =
            container(column![thumb, text(label).size(10).color(TEXT_SECONDARY)].spacing(2))
                .padding(4)
                .style(|_theme: &Theme| container::Style {
                    background: Some(Background::Color(Color {
                        r: 0.15,
                        g: 0.15,
                        b: 0.15,
                        a: 0.85,
                    })),
                    border: Border {
                        color: Color::from_rgb(0.3, 0.5, 0.7),
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..Default::default()
                });

        // Offset so the drag widget appears below-right of cursor
        let x = drag.current_pos[0] + 10.0;
        let y = drag.current_pos[1] + 10.0;

        container(column![
            Space::with_height(y),
            row![Space::with_width(x), drag_widget,]
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
```

- [ ] **Step 5: Build and verify**

Run: `cargo build --release 2>&1 | tail -1`
Expected: `Finished`

- [ ] **Step 6: Run all tests**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "feat: drag and drop photos from library grid to collection sidebar"
```

---

### Task 9: Final Polish + Integration Tests

**Files:**
- Modify: `src/main.rs` (tests, edge cases)

- [ ] **Step 1: Add collection-related tests to main.rs**

Add these tests in the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn collection_store_integration() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("collections.json");

        let mut store = collection::CollectionStore::default();
        store.create("Photos");
        store.create("Archive");
        store.add_photo(0, &PathBuf::from("/test/a.jpg"));
        store.save_to(&file);

        let loaded = collection::CollectionStore::load_from(&file);
        assert_eq!(loaded.collections.len(), 2);
        assert_eq!(loaded.collections[0].name, "Archive"); // sorted
        assert_eq!(loaded.collections[1].name, "Photos");
        assert_eq!(loaded.collections[1].photos.len(), 1);
    }

    #[test]
    fn context_menu_kinds_are_distinct() {
        let lib = ContextMenuKind::LibraryPhoto { photo_index: 0 };
        let col = ContextMenuKind::CollectionPhoto { photo_index: 0 };
        let side = ContextMenuKind::SidebarCollection {
            collection_index: 0,
        };
        // Just verify they construct without issue
        let _ = format!("{:?}", lib);
        let _ = format!("{:?}", col);
        let _ = format!("{:?}", side);
    }

    #[test]
    fn drag_state_activation_threshold() {
        let drag = DragState {
            photo_index: 0,
            start_pos: [100.0, 100.0],
            current_pos: [103.0, 104.0],
            active: false,
        };
        let dx = drag.current_pos[0] - drag.start_pos[0];
        let dy = drag.current_pos[1] - drag.start_pos[1];
        let dist = (dx * dx + dy * dy).sqrt();
        assert!(dist == 5.0);
        // Exactly 5.0 should NOT activate (threshold is > 5.0)
        assert!(!(dist > 5.0));

        let drag2 = DragState {
            photo_index: 0,
            start_pos: [100.0, 100.0],
            current_pos: [104.0, 104.0],
            active: false,
        };
        let dx2 = drag2.current_pos[0] - drag2.start_pos[0];
        let dy2 = drag2.current_pos[1] - drag2.start_pos[1];
        let dist2 = (dx2 * dx2 + dy2 * dy2).sqrt();
        assert!(dist2 > 5.0); // This should activate
    }
```

- [ ] **Step 2: Run all tests**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 3: Run cargo clippy (if available)**

Run: `cargo clippy --all-targets 2>&1 | tail -10`
Expected: no errors (warnings are OK but fix if trivial)

- [ ] **Step 4: Build release**

Run: `cargo build --release 2>&1 | tail -1`
Expected: `Finished`

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "test: add collection integration tests and drag threshold test"
```

- [ ] **Step 6: Update ARCHITECTURE.md**

Add to the Component Map section, after the Navigation subsection:

```markdown
### Collections
- **collection store** (`src/collection.rs`) — Named photo collections with JSON persistence. Owns: `Collection`, `CollectionStore`, `collections_file_path()`. CRUD operations with automatic alphabetical sorting.
```

Add to the Data Flow section:

```markdown
### Collection Flow
1. Collections loaded from `LOCALAPPDATA/photo/collections.json` on startup.
2. User creates/renames/deletes collections via sidebar UI.
3. Photos added via drag-and-drop (library → sidebar) or right-click context menu.
4. Adding stores a `PathBuf` reference — no file copying.
5. Removing a photo from a collection does not delete the file.
6. `CollectionStore::save()` writes JSON after every mutation.
7. Double-clicking a collection enters collection grid view (sub-view of Library tab).
8. Opening a photo from collection grid enters Detail view with collection-scoped navigation.
```

Add to Boundaries:

```markdown
- Only `collection.rs` handles collection serialization/persistence.
- `main.rs` owns all collection UI (sidebar, grid view, context menus, drag-and-drop).
```

Add to Technology Map:

```markdown
| JSON persistence | serde + serde_json | 1.x | Collection data serialization |
```

Add to Drift Log:

```markdown
| 2026-04-03 | Added collections system (collection.rs, sidebar UI, context menus, drag-drop) | Named photo collections with JSON persistence, context menu overlay, drag-and-drop | agent |
| 2026-04-03 | Added serde and serde_json dependencies | JSON serialization for collection persistence | agent |
```

- [ ] **Step 7: Commit architecture update**

```bash
git add docs/ARCHITECTURE.md
git commit -m "docs: update architecture for collections feature"
```

- [ ] **Step 8: Update devlog**

Append to the latest file in `docs/devlog/detailed/` and update `docs/devlog/summary.md` with the collections feature work.

- [ ] **Step 9: Commit devlog**

```bash
git add docs/devlog/detailed/*.md docs/devlog/summary.md
git commit -m "docs: update devlogs for collections feature"
```
