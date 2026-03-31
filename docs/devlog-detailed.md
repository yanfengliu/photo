# Detailed Devlog

## [2026-03-29 19:30] — Initial project scaffold and GPU image viewer
**Action:** Built full GPU-accelerated image viewer from scratch using Rust + iced 0.13 + wgpu. Created Cargo.toml, WGSL shader, and four source modules (main, viewer, decode, nav).
**Result:** Success — release build compiles and runs, window opens, dark theme renders.
**Files changed:** `Cargo.toml`, `src/main.rs`, `src/viewer.rs`, `src/decode.rs`, `src/nav.rs`, `assets/shaders/image.wgsl`
**Reasoning:** Rust + iced + wgpu chosen for zero-GC performance, native GPU control via custom `shader::Primitive`, and strong image format ecosystem (`image` crate + `resvg`).
**Notes:** iced 0.13 bundles wgpu 0.19 internally. Must use `iced::widget::shader::wgpu` re-export, not standalone `wgpu` crate.

## [2026-03-29 20:00] — Fix wgpu version mismatch and API differences
**Action:** Removed standalone `wgpu = "22"` dep, switched to iced's wgpu re-export. Fixed wgpu 0.19 API: `entry_point` is `&str` not `Option<&str>`, removed `compilation_options` and `cache` fields, specified all `RenderPassDescriptor` fields explicitly.
**Result:** Success — compiles cleanly.
**Files changed:** `Cargo.toml`, `src/viewer.rs`
**Reasoning:** iced re-exports its own wgpu; using a different version causes type mismatches in the `shader::Primitive` trait methods.
**Notes:** Also added explicit `tokio` dep (iced's tokio feature doesn't re-export it) and fixed Rust 1.94 lifetime elision warnings.

## [2026-03-29 20:15] — Add CLI argument support
**Action:** Modified `App::new()` to parse `std::env::args()` and load the first argument as an image path on startup.
**Result:** Success — `photo.exe path/to/image.jpg` opens the image directly.
**Files changed:** `src/main.rs`
**Reasoning:** Image viewers should accept a file path argument for shell integration and file associations.
**Notes:** None.

## [2026-03-29 20:20] — Add .gitignore
**Action:** Created `.gitignore` for `/target`, binaries, `Cargo.lock`.
**Result:** Success.
**Files changed:** `.gitignore`
**Reasoning:** Standard Rust binary project ignores.
**Notes:** User also added `*.jpg` to ignore test images.

## [2026-03-30 03:25] — Fix crash on large images exceeding GPU texture limit
**Action:** Added GPU-aware downscale in `viewer.rs` `prepare()`. Queries `device.limits().max_texture_dimension_2d` at texture upload time. If image exceeds limit, downscales with `image::imageops::resize(Triangle)` before creating the GPU texture.
**Result:** Success — `test.jpg` (9504px wide, GPU limit 8192) now loads without crash.
**Files changed:** `src/viewer.rs`
**Reasoning:** Hardcoding a cap in `decode.rs` can't work because the GPU limit isn't known until the shader's `prepare()` runs. The actual limit varies per GPU (8192 on this integrated GPU, 16384+ on discrete GPUs).
**Notes:** The downscale clone + resize happens once per oversized image. Original full-res pixels stay in memory for potential future tiling implementation.

## [2026-03-30 03:45] — Add test suite and extract testable math functions
**Action:** Added 24 unit tests across three modules. Extracted `compute_image_rect` and `zoom_at_cursor` from viewer.rs into public standalone functions for testability. Added `tempfile` dev-dependency for filesystem tests. Tests cover: decode (PNG, BMP, SVG, invalid file, nonexistent file, file size, RGBA format), nav (scanning, natural sort, next/prev cycling, empty dir, case-insensitive extensions, start position), viewer math (fit-to-window for square/wide/tall/mixed aspect ratios, zoom scaling, pan offset, zoom-at-center, zoom-at-corner, zoom-preserves-cursor-point, zoom clamping).
**Result:** Success — all 24 tests pass, release build succeeds.
**Files changed:** `Cargo.toml` (added tempfile dev-dep), `src/viewer.rs` (extracted functions, added tests), `src/decode.rs` (added tests), `src/nav.rs` (added tests), `src/main.rs` (use extracted zoom_at_cursor)
**Reasoning:** CLAUDE.md requires tests with every change. Extracting math into pure functions enables testing without GPU or iced runtime.
**Notes:** Used `1e-3` tolerance for f32 zoom math tests due to accumulated floating-point error in chained operations.

## [2026-03-30 04:00] — Hide console window on Windows
**Action:** Added `#![windows_subsystem = "windows"]` to top of `src/main.rs`.
**Result:** Success — app launches without a terminal window. 24 tests pass, release build succeeds.
**Files changed:** `src/main.rs`
**Reasoning:** Windows Rust binaries default to `console` subsystem, which spawns a visible terminal alongside the GUI window. The `windows` subsystem suppresses it.
**Notes:** None.

## [2026-03-29 21:00] — Add Library/Detail tab UI with thumbnail grid
**Action:** Major UI redesign: added tab-based navigation with Library and Detail tabs. Library tab shows a scrollable grid of image thumbnails. Detail tab retains the existing GPU shader viewer. Added folder picker and file picker buttons to load images into the library. Thumbnails are loaded asynchronously at 200px max dimension. Clicking a thumbnail opens it in Detail view. Arrow key navigation works through library entries when viewing from library.
**Result:** Success — 31 tests pass (7 new), release build compiles cleanly with no warnings.
**Files changed:** `Cargo.toml` (added `"image"` feature to iced), `src/main.rs` (major rewrite: tab state, library entries, new messages, tab bar view, library grid view, thumbnail loading, dual navigation modes), `src/decode.rs` (added `decode_thumbnail` function, removed unused `decode_raster_raw`, added 3 thumbnail tests), `src/nav.rs` (made `is_image_file` public for reuse)
**Reasoning:** User requested a library tab with grid view and a detail tab with full image view. Used iced's built-in `Image` widget (via `"image"` feature) for thumbnails with `Handle::from_rgba` for pre-decoded pixel data. Kept existing shader viewer as the Detail tab. Two navigation modes: library-based (when entering from library) and directory-based (when opening via CLI/drag-drop/Ctrl+O).
**Notes:** Thumbnails decode the full image then resize — could be optimized for large files later. Grid uses fixed 6 columns. Tab highlighting uses Unicode bullet character as a simple indicator. The `image` crate version from iced's dependency coexists with the project's `image = "0.24"` without conflict.

## [2026-03-29 21:30] — Populate ARCHITECTURE.md and fix CLAUDE.md directories
**Action:** Replaced the template ARCHITECTURE.md with real project architecture: system overview, component map (4 modules + shader), data flows (image loading, thumbnail loading, navigation), boundaries/ownership rules, 9 key architectural decisions, technology map, ASCII diagram, and drift log. Updated CLAUDE.md key directories to remove nonexistent `/agents` and `/skills`, added `/assets`.
**Result:** Success — docs now accurately reflect the codebase.
**Files changed:** `docs/ARCHITECTURE.md` (full rewrite from template), `CLAUDE.md` (key directories section)
**Reasoning:** CLAUDE.md requires reading and updating ARCHITECTURE.md for structural changes. The template was never populated. The Library/Detail tab addition was a structural change that warranted the initial population.
**Notes:** None.

## [2026-03-30 18:00] — Add development and release instructions to README.md
**Action:** Expanded README.md from a single title line to full documentation: project description, features, prerequisites, development instructions (build, run, test, lint, format), release workflow, and project structure. Also ran `cargo fmt` to fix pre-existing formatting issues across 4 source files.
**Result:** Success — all 31 tests pass, clippy clean, formatting clean, release build succeeds.
**Files changed:** `README.md` (expanded), `src/main.rs` (fmt), `src/decode.rs` (fmt), `src/nav.rs` (fmt), `src/viewer.rs` (fmt)
**Reasoning:** User requested development and release instructions in README.md.
**Notes:** Formatting issues were pre-existing from prior sessions. Fixed them to satisfy CLAUDE.md's "all linter tests pass" requirement.

## [2026-03-30 18:05] — Replace Unicode symbols with ASCII in UI text
**Action:** Replaced all Unicode escape sequences in main.rs UI strings with ASCII equivalents: `│` (U+2502) → `|`, `×` (U+00D7) → `x`, `←→` (U+2190/2192) → "Arrow keys", `●` (U+25CF) → `*`, `…` (U+2026) → `...`, `⌛` (U+231B) → `...`, `—` (U+2014) → `-`.
**Result:** Success — all symbols now render correctly with iced's default font. 31 tests pass, clippy clean.
**Files changed:** `src/main.rs`
**Reasoning:** iced's default font lacks glyphs for box-drawing, bullet, arrow, and hourglass characters, causing them to render as missing-glyph placeholders in the status bar and tab bar.
**Notes:** If a broader Unicode font is added later, these could be reverted to the original symbols.

## [2026-03-30 18:11] — Persist library entries across sessions
**Action:** Added library persistence: file paths saved to `%LOCALAPPDATA%\photo\library.txt` (one path per line). On startup, saved paths are loaded, dead paths filtered out, and thumbnails reloaded. Save triggers on folder pick and file pick. Added `library_file_path()`, `save_library()`, `load_library()` free functions. Added 2 new tests (33 total).
**Result:** Success — 33 tests pass, clippy clean, release build succeeds.
**Files changed:** `src/main.rs`
**Reasoning:** User reported library not remembering loaded files between sessions. The library was purely in-memory with no persistence.
**Notes:** Uses plain text format (no serde dependency). Deleted files are silently filtered out on load.
