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
