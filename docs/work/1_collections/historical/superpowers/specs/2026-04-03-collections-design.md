# Collections Feature Design

## Overview

Add photo collections to the Library view. Collections are named groups of photo references (no file copying). Users can create, rename, delete collections, and add/remove photos via drag-and-drop or right-click context menu.

## Data Model & Persistence

### Types (`src/collection.rs`)

```rust
Collection {
    name: String,
    photos: Vec<PathBuf>,  // ordered references, no duplicates
}

CollectionStore {
    collections: Vec<Collection>,  // sorted by name
}
```

### Operations on `CollectionStore`

- `create(name: &str)` — adds empty collection, re-sorts
- `rename(index: usize, new_name: &str)` — renames, re-sorts
- `delete(index: usize)` — removes collection
- `add_photo(collection_index: usize, path: &PathBuf)` — adds if not already present
- `remove_photo(collection_index: usize, path: &PathBuf)` — removes reference only
- `load()` / `save()` — JSON to/from `LOCALAPPDATA/photo/collections.json`

### JSON Format

```json
[
  { "name": "Vacation", "photos": ["C:/photos/a.jpg", "C:/photos/b.png"] },
  { "name": "Work", "photos": ["C:/photos/c.jpg"] }
]
```

### Dependencies

- `serde = { version = "1", features = ["derive"] }` — serialization derives
- `serde_json = "1"` — JSON read/write

## UI: Collection Sidebar (Library View Only)

The sidebar appears on the left side of the Library view, ~180px wide, with a divider separating it from the thumbnail grid. Not visible in collection grid view or detail view.

### Layout (top to bottom)

1. **Header row:** "COLLECTIONS" section label + "+" button to create
2. **Scrollable list** of collection names, sorted alphabetically
3. Each entry shows name and photo count (e.g. "Vacation (12)")

### Interactions

- **Single click** — visual highlight only
- **Double-click** — enters that collection's grid view
- **Right-click** — context menu with "Rename" and "Delete"
- **"+" button** — creates "New Collection" (auto-incrementing suffix if needed) and immediately enters inline rename mode on it

### Renaming

Only triggered via right-click context menu or on initial creation (never via double-click, to avoid confusion with "open collection"). A text_input replaces the label. Enter commits, Escape cancels. List re-sorts after rename.

### Deleting

Right-click → "Delete" removes the collection. Photos are not affected.

## UI: Collection Grid View

When the user double-clicks a collection in the sidebar, the Library view is replaced with a collection grid view.

### Layout

- **Top bar:** Left arrow button + collection name as title + photo count
- **Main area:** Thumbnail grid of the collection's photos (same card style as Library grid)
- **No sidebar** — the collection sidebar is not visible

### Interactions

- **Left arrow button / Escape** — returns to Library view
- **Double-click thumbnail** — opens Detail view. Arrow keys cycle within this collection's photos only. Back button returns to this collection grid view (not Library).
- **Right-click thumbnail** — context menu with "Remove from [Collection Name]" (does not delete the file)

## UI: Right-Click Context Menu

A custom overlay popup rendered as a positioned container on top of the main view.

### Implementation

- App state: `context_menu: Option<ContextMenu>` with position, target index, and menu kind
- Cursor tracking: each thumbnail fires a hover message on mouse enter, storing the hovered index. Right-click in the global event handler uses this index.
- Rendered as a positioned `container` with `column` of clickable rows, drawn on top of the main view
- Dismissed by clicking outside or pressing Escape

### Library Grid Right-Click Menu

- Lists all collection names: "Add to Vacation", "Add to Work", etc.
- If photo is already in a collection, show "Remove from Vacation" (or a checkmark)
- Clicking adds/removes and closes the menu

### Collection Grid Right-Click Menu

- Single option: "Remove from [Collection Name]"
- Clicking removes the photo reference and closes the menu

### Sidebar Right-Click Menu

- Options: "Rename", "Delete"
- Clicking triggers the action and closes the menu

## UI: Drag and Drop

Drag a thumbnail from the Library grid and drop it onto a collection in the sidebar to add it.

### Implementation

- **Mouse down** on a thumbnail stores the index and start position
- **Mouse move** with button held, after ~5px distance threshold, enters drag mode
- **During drag:** render a semi-transparent thumbnail (or filename text if thumbnail not loaded) following the cursor as an overlay
- **Mouse up over sidebar collection:** fires "add to collection" action
- **Mouse up elsewhere:** cancels the drag

### State

- `drag_state: Option<DragState>` with fields: `photo_index`, `start_pos`, `current_pos`, `active: bool` (becomes true after threshold)
- `sidebar_hover_collection: Option<usize>` — which sidebar row the cursor is over, used as drop target and for visual highlight during drag

## App State Changes (main.rs)

### New Fields

- `collection_store: CollectionStore` — loaded on startup
- `active_collection: Option<usize>` — which collection's grid is being viewed (None = library view)
- `context_menu: Option<ContextMenu>` — overlay popup state
- `drag_state: Option<DragState>` — drag-and-drop tracking
- `editing_collection_name: Option<usize>` — inline rename state + text buffer
- `hovered_thumbnail: Option<usize>` — for right-click targeting
- `sidebar_hover_collection: Option<usize>` — for drop targeting

### View Routing

The existing `Tab::Library` branches on `active_collection`:
- `None` → library grid with sidebar
- `Some(idx)` → collection grid view

No new `Tab` variant needed.

### Detail View Navigation

When entering Detail from a collection, store `(collection_index, photo_index_within_collection)` so arrow keys cycle within the collection. Pressing Escape or the back arrow in Detail returns to the collection grid view (not Library). The existing `library_index` continues to handle the library-wide navigation case (entering Detail from Library returns to Library).

## Module Boundaries

- **`collection.rs`** — owns `Collection`, `CollectionStore`, serialization, load/save. No UI knowledge.
- **`main.rs`** — owns all collection UI: sidebar, collection grid, context menu overlay, drag state, messages. Coordinates between `CollectionStore` and the view.

## Non-Goals

- No nested collections or hierarchy
- No smart/auto collections (e.g. by date or tag)
- No bulk operations (select multiple photos and add at once)
- No collection reordering (always sorted by name)
