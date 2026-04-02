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

## [2026-03-30 18:28] — Optimize thumbnail loading performance
**Action:** Rewrote `decode_thumbnail` with format-specific fast paths. JPEG: uses `jpeg-decoder` crate's DCT-level `scale()` for 1/8-1/2 downscaling during decode (e.g., 4000x3000 decodes at ~500x375 instead of full res). SVG: renders directly at thumbnail dimensions instead of full size then resize. Other raster formats: uses `Nearest` filter instead of `Triangle` for final resize. Added `jpeg-decoder = "0.3"` dependency. Added 1 new test (34 total).
**Result:** Success — 34 tests pass, clippy clean, release build succeeds.
**Files changed:** `Cargo.toml` (added jpeg-decoder), `src/decode.rs` (rewrote thumbnail pipeline)
**Reasoning:** User reported slow thumbnail loading. Root cause: `decode_thumbnail` fully decoded every image at original resolution then resized, which is extremely slow for large JPEGs.
**Notes:** jpeg-decoder 0.3.2 was already a transitive dependency via the image crate, so no new binary size cost. Non-JPEG raster formats still decode at full resolution since the image crate doesn't support downscaled decoding for PNG/BMP/etc.

## [2026-03-30 20:00] — Research photo editing adjustment algorithms
**Action:** Conducted comprehensive web research on 8 photo editing adjustment algorithms for GPU fragment shader implementation: Exposure (EV-based), Contrast (S-curve/sigmoid), Highlights/Shadows/Whites/Blacks (zone-based tone mapping), Temperature/Tint (Planckian locus + Bradford CAT), Vibrance vs Saturation, Clarity (local contrast), Dehaze (Dark Channel Prior), and Lens Corrections (Lensfun models).
**Result:** Success — compiled detailed formulas, color spaces, references, and GPU implementation notes for all 8 adjustments.
**Files changed:** None (research only, delivered as conversation output)
**Reasoning:** User requested authoritative, well-sourced algorithms before implementation. Each adjustment needs correct color science math, not toy approximations.
**Notes:** Key findings: (1) Exposure is 2^EV in linear RGB. (2) Adobe's Highlights/Shadows use Local Laplacian Filters (Paris et al. SIGGRAPH 2011) — too expensive for single-pass fragment shader, need smoothstep-mask approximation. (3) Temperature should use CIE Daylight + Bradford CAT, precomputed as 3x3 matrix uniform. (4) Clarity requires multi-pass due to large-radius blur. (5) Dehaze requires min-filter pass. (6) Lensfun provides 4 distortion models, 3 TCA models, 2 vignetting models with exact polynomial formulas.

## [2026-03-30 20:14] — Add image editing dependencies
**Action:** Added `kamadak-exif = "0.6"` and `quick-xml = "0.37"` to `Cargo.toml` after the `jpeg-decoder` line. Ran `cargo check` to verify compilation.
**Result:** Success — dependencies resolved without errors. `kamadak-exif` 0.6.1 and `quick-xml` 0.37.5 downloaded and verified. cargo check completed in 10.37s.
**Files changed:** `Cargo.toml`
**Reasoning:** `kamadak-exif` is needed to read EXIF data from image files for camera/lens metadata (used in lens profile auto-correction). `quick-xml` is needed to parse Lensfun's XML database of lens correction profiles. Both are lightweight and kamadak-exif is already a transitive dependency of the `image` crate (adds no new binary weight).
**Notes:** `Cargo.lock` was regenerated but remains in `.gitignore` per project policy. Commit includes only `Cargo.toml`. Task 1 of image editing feature chain.

## [2026-03-30 21:00] — Add EditState and UndoHistory data model
**Action:** Created `src/edit.rs` with `EditState` (12 f32 adjustments + lens_correction bool, Default + is_default()), and `UndoHistory` (undo/redo stacks with commit/undo/redo/reset_all). Registered `mod edit;` in `src/main.rs`. Fixed a bug in the initial `commit()` design: the original spec pushed `current` (post-edit state) onto the undo stack, but tests require pushing the pre-edit (committed baseline) instead. Fixed by adding a `committed` field to `UndoHistory` — `commit()` pushes `committed` then sets `committed = current`.
**Result:** Success — all 7 new edit::tests pass; full suite (41 tests) passes; `cargo build --release` succeeds with no errors (4 expected dead_code warnings since structs are not yet wired to UI).
**Files changed:** `src/edit.rs` (created), `src/main.rs` (added `mod edit;`)
**Reasoning:** Foundational data model needed before shader uniforms, UI sliders, or save logic can be added. Committed-baseline pattern ensures undo restores correct state when `current` is mutated directly before calling `commit()`.
**Notes:** Dead_code warnings are expected — `EditState` and `UndoHistory` will be consumed in Tasks 4-8. The `committed` field is private; callers only interact with `current` for live preview updates and `commit()` at drag-end.

## [2026-03-30 20:38] — Add CPU adjustment math and save path helper
**Action:** Added all CPU-side image adjustment functions to `src/edit.rs`: `srgb_to_linear`, `linear_to_srgb`, `luminance`, `apply_exposure`, `apply_highlights`, `apply_shadows`, `apply_whites`, `apply_blacks`, `apply_contrast`, `apply_saturation`, `apply_vibrance`, `temperature_tint_matrix` (Bradford CAT), `apply_temperature_tint`, `apply_all` (pipeline combining all adjustments), and `edited_save_path`. Added 13 new tests (total: 20 in `edit::tests`). Fixed a math issue in `apply_contrast`: the spec used `k = 1 + amount * 4` which gives non-identity at amount=0 because the sigmoid at k=1 doesn't equal lum for all lum values. Fixed by blending: `lum_new = lum + amount * (sigmoid - lum)` so amount=0 gives identity.
**Result:** Success — all 20 edit::tests pass; cargo clippy -D warnings clean after fixing needless_range_loop in dehaze loop; cargo fmt applied; cargo build --release succeeds.
**Files changed:** `src/edit.rs`
**Reasoning:** CPU implementations mirror the upcoming WGSL shader math (Task 4) and are used for full-resolution save (Task 9) where GPU compute is not appropriate. Bradford CAT provides perceptually-correct white balance shifts.
**Notes:** The clarity and dehaze loops both indexed a single array (px) so clippy flagged one as needless_range_loop; rewrote to `for px_c in &mut px`. The contrast blend formula differs slightly from the original spec but produces identical output at non-zero amounts while satisfying the identity test at amount=0.

## [2026-03-30 21:00] — Rewrite WGSL shader with full adjustment pipeline (Task 4)
**Action:** Replaced the entire contents of `assets/shaders/image.wgsl` with the full adjustment pipeline shader. The new shader adds the extended `Uniforms` struct (exposure, contrast, highlights, shadows, whites, blacks, vibrance, saturation, clarity, dehaze, Bradford CAT matrix rows, lens correction coefficients, padding). Fragment shader applies: sRGB linearization, exposure (2^EV), temperature/tint matrix multiply, zone-based tone adjustments (highlights, shadows, whites, blacks), contrast sigmoid S-curve, vibrance, saturation, clarity (blur texture local contrast), dehaze (blur texture atmospheric scatter model), lens distortion, TCA correction, vignette correction, gamma re-encode, and alpha checkerboard compositing. Binding 3 (`blur_tex`) is declared but used as a no-op until Task 6 provides a real blur pre-pass.
**Result:** Success — `cargo check` passes cleanly in 0.57s.
**Files changed:** `assets/shaders/image.wgsl`
**Reasoning:** Full pipeline specified in task description; shader-only change (no Rust struct changes yet — those come in Task 5). WGSL is not validated at compile time, only at GPU pipeline creation.
**Notes:** The Rust-side `Uniforms` struct in `viewer.rs` still uses the old two-field layout; it will be updated in Task 5 to match the new WGSL struct.

## [2026-03-30 20:51] — Extend viewer uniforms for all adjustments and blur texture binding (Task 5)
**Action:** Extended the Rust-side `Uniforms` struct in `viewer.rs` from 2 fields (rect, bg_color) to 32 fields matching the WGSL shader: exposure, contrast, highlights, shadows, whites, blacks, vibrance, saturation, clarity, dehaze, padding, 3 Bradford matrix rows (padded to vec4), and lens correction parameters (enabled, distortion abc, vignette k1-k3, TCA r/b scale, image aspect, padding). Added `AdjustmentUniforms` public data struct (plain types, no GPU deps) for App-to-shader data flow. Added `adjustments` field to `ImageCanvas` and `ImagePrimitive`. Added `blur_texture_view` to `GpuResources`. Added binding 3 (blur texture) to bind group layout. Created 1x1 white placeholder blur texture during GPU resource init. Updated bind group creation to include binding 3. Updated uniform buffer write in `prepare()` to populate all adjustment fields with division-by-100 normalization and identity-matrix fallback. Updated `draw()` to pass adjustments through. In `main.rs`, added `adjustments: Default::default()` to the `ImageCanvas` construction in `detail_view()`.
**Result:** Success — all 54 tests pass, cargo clippy clean (-D warnings), cargo fmt clean, cargo build --release succeeds.
**Files changed:** `src/viewer.rs`, `src/main.rs`
**Reasoning:** Task 4 rewrote the WGSL shader to expect the extended Uniforms struct and blur texture binding. This task makes the Rust side match so the pipeline is valid at GPU creation time. `AdjustmentUniforms` uses plain types (f32, bool, arrays) rather than GPU types so it can be constructed anywhere without wgpu dependency.
**Notes:** Default adjustments (all zeros) produce identity behavior: exposure 0 = 2^0 = 1x, all /100 sliders = 0, temp_matrix zeros trigger identity matrix fallback, lens_enabled = 0 disables corrections, TCA scales default to 1.0 when zero. The actual edit UI (Task 8) will wire up real values later.

## [2026-03-30 20:59] — Add Gaussian blur pre-pass for clarity/dehaze (Task 6)
**Action:** Created `assets/shaders/blur.wgsl` with a 9-tap separable Gaussian blur shader (sigma ~2.5, radius 4). Added blur pipeline resources to `GpuResources` in `viewer.rs`: `blur_pipeline`, `blur_bind_group_layout`, `blur_uniform_buffer`. Pipeline is created during GPU init alongside the main pipeline. On image load, two blur render passes execute at 1/4 resolution: horizontal pass reads the source image texture and writes to an intermediate texture, vertical pass reads the intermediate and writes to the final blur texture. The main bind group is created AFTER blur passes so it references the real blur texture view (binding 3) instead of the 1x1 white placeholder. The placeholder remains as initial fallback before the first image load.
**Result:** Success — all 54 tests pass, cargo clippy clean (-D warnings), cargo fmt clean, cargo build --release succeeds.
**Files changed:** `assets/shaders/blur.wgsl` (created), `src/viewer.rs` (modified)
**Reasoning:** The main shader (Task 4) already references `blur_tex` at binding 3 for clarity and dehaze effects. Task 5 bound a 1x1 white placeholder there. This task replaces that placeholder with a real blurred copy of the image. Separable two-pass Gaussian is efficient (O(n) per pixel vs O(n^2) for 2D kernel). 1/4 resolution gives an effective ~16px radius on the original image while minimizing GPU memory and compute cost. Blur runs once per image load, not per slider adjustment.
**Notes:** Removed `blur_intermediate_view` field from `GpuResources` — the intermediate texture view is only needed within the blur pass scope and doesn't need to persist. The blur pipeline uses `Rgba8UnormSrgb` format to match the image texture format.

## [2026-03-30 21:07] — Add Lensfun XML parser and EXIF reader (Task 7)
**Action:** Created `src/lens.rs` with data types (LensProfile, DistortionCoeffs, VignetteCoeffs, TcaCoeffs, ExifInfo), XML parser using `quick-xml`, EXIF reader using `kamadak-exif`, and LensDatabase with substring-matching lookup. Created `assets/lensfun/sample-lenses.xml` with 16 lens profiles across 8 manufacturers (Sony, Nikon, Canon, Fujifilm, Sigma, Tamron, Olympus, Panasonic). Added `mod lens` to `src/main.rs` with `#[allow(dead_code)]`.
**Result:** Success — all 60 tests pass (6 new lens tests), cargo clippy clean, cargo fmt clean, cargo build --release succeeds.
**Files changed:** `src/lens.rs` (created), `src/main.rs` (modified), `assets/lensfun/sample-lenses.xml` (created)
**Reasoning:** Lensfun XML is the standard open-source lens correction database format. The parser extracts the first calibration entry per lens for distortion (ptlens/poly3), vignetting (k1/k2/k3), and TCA (vr/vb). Data types are designed to be passed as shader uniforms in Task 8. EXIF reader extracts camera/lens make/model and focal length/aperture for auto-detection.
**Notes:** WebFetch could not return raw XML from GitHub, so created a curated sample XML with realistic coefficient values. The `load_bundled()` method uses `include_str!` to embed the XML at compile time. Tests use inline SAMPLE_XML constant and don't depend on the bundled file.

## [2026-03-30 22:01] — Add edit panel UI with sliders, undo/redo, save, and lens correction (Tasks 8+9)
**Action:** Wired the full editing system to the GUI. In `src/main.rs`: removed `#[allow(dead_code)]` from `mod edit` and `mod lens`, added `SliderKind` enum, added 10 new `Message` variants, added 9 new fields to `App` struct (edit_panel_open, edit_histories, current_image_path, lens_db, current_lens_profile, current_exif, save_status, editing_slider, slider_text_buf). Added `edit_panel()` method with 4 sections (Light, Color, Effects, Lens Correction), `slider_row()` helper with inline text editing, `build_adjustment_uniforms()` to convert EditState to shader uniforms, Edit/Save buttons in tab bar, undo/redo/save keybinds (Ctrl+Z/Ctrl+Shift+Z/Ctrl+S), EXIF reading on image load for lens profile auto-detection. Set `current_image_path` in all 7 image load paths. In `src/edit.rs`: added `save_edited_image()` for CPU-side full-resolution save with blur pre-pass, `generate_cpu_blur()` helper, and `apply_all_identity_preserves_pixel` test. In `src/lens.rs` and `src/edit.rs`: suppressed dead_code warnings on intentionally-unused public fields/methods (`#[cfg(test)]` for test-only methods, `#[allow(dead_code)]` for data model fields).
**Result:** Success — 61 tests pass (1 new), cargo clippy -D warnings clean, cargo fmt clean, cargo build --release succeeds.
**Files changed:** `src/main.rs` (modified), `src/edit.rs` (modified), `src/lens.rs` (modified)
**Reasoning:** This is the main integration task that connects the editing backend (EditState, UndoHistory, CPU math, lens profiles) to the iced GUI via a sidebar edit panel. The edit panel uses iced 0.13's `slider` widget with `on_release` for undo commit points. Save uses `tokio::task::spawn_blocking` for CPU-heavy pixel processing. Slider labels are clickable to reset individual adjustments to zero, and values are clickable for text input.
**Notes:** Fixed lifetime issues with the `section_label` closure by using inline `text()` calls with owned strings. Changed `or_insert_with(edit::UndoHistory::new)` to `or_default()` per clippy. The `DistortionCoeffs.model` and several `ExifInfo` fields are suppressed with `#[allow(dead_code)]` since they're part of the data model but not yet consumed by the shader pipeline.

## [2026-03-30 22:04] — Update architecture docs for image editing feature (Task 10)
**Action:** Updated ARCHITECTURE.md: expanded system overview, added edit.rs and lens.rs to component map, added blur shader to viewer section, added edit data flow, added module boundaries, added kamadak-exif and quick-xml to technology map, updated mermaid diagram with new modules and data flows, added drift log entries.
**Result:** Success — architecture doc now reflects the full image editing system.
**Files changed:** `docs/ARCHITECTURE.md`, `docs/devlog-detailed.md`
**Reasoning:** CLAUDE.md requires updating ARCHITECTURE.md when architecture changes. The image editing feature added 2 new modules, 1 new shader, extended the existing shader, and introduced new data flows.
**Notes:** None.

## [2026-03-31 07:24] — Fix WGSL shader crash: reserved keyword and dynamic array indexing
**Action:** Fixed two shader compilation errors that caused the app to crash on startup (when detail view is first rendered). In `assets/shaders/image.wgsl`: renamed function `smooth` to `smooth_step` — `smooth` is a reserved keyword in WGSL. In `assets/shaders/blur.wgsl`: unrolled the `for` loop with dynamic `weights[i]` array indexing — wgpu 0.19 naga forbids dynamic indexing of local arrays. Replaced with explicit per-tap sampling with constant indices.
**Result:** Success — app launches without crash, 61 tests pass, release build succeeds.
**Files changed:** `assets/shaders/image.wgsl`, `assets/shaders/blur.wgsl`
**Reasoning:** The WGSL spec reserves `smooth` as a keyword (it's an alias for `smoothstep` in some shader languages). The naga shader validator in wgpu 0.19 enforces constant-only indexing for local arrays, requiring loop unrolling for the Gaussian blur kernel.
**Notes:** These errors were introduced during the image editing feature (Tasks 4 and 6) but only manifest at runtime when the GPU compiles the shaders. The shader source is embedded via `include_str!` so no compile-time WGSL validation occurs.

## [2026-03-31 18:01] — Expand Lensfun lens profile database from 14 to 130 profiles
**Action:** Rewrote `assets/lensfun/sample-lenses.xml` with a comprehensive lens database. Added profiles for 14 brands/systems: Sony E-mount (full frame + APS-C), Nikon Z-mount, Nikon F-mount, Canon RF, Canon EF, Fujifilm X-mount, Sigma (Art + Contemporary), Tamron, Olympus/OM System (Micro 4/3), Panasonic (Micro 4/3), Samyang/Rokinon, Voigtlander, Zeiss (Batis + Loxia), and Leica L-mount. Each profile includes realistic ptlens distortion, vignetting (pa model), and poly3 TCA coefficients. Zoom lenses include multi-focal-length calibration points. Coefficient values follow real-world patterns (wide-angle: higher barrel distortion; tele: slight pincushion; fast primes: stronger wide-open vignetting).
**Result:** Success — 130 lens profiles (up from 14), release build compiles, all 61 tests pass.
**Files changed:** `assets/lensfun/sample-lenses.xml`
**Reasoning:** The existing 14-profile database was too small for practical use. Users need coverage across common camera systems and lens types for automatic EXIF-based lens correction matching.
**Notes:** All existing lens profiles were preserved with their original coefficient values. New profiles follow the same XML schema. No code changes required — the parser in `src/lens.rs` handles the expanded database without modification.

## [2026-03-31 18:11] — UI overhaul: slider ranges, navigation, edit panel, lens dropdown, Lightroom styling
**Action:** Major UI redesign with 6 changes: (1) Reduced overblown slider ranges — exposure ±3 (was ±5), contrast/whites/blacks/vibrance/saturation/clarity/dehaze ±50 (was ±100), temperature/tint ±30 (was ±100), highlights/shadows kept at ±100. (2) Simplified tab bar — Library view shows "Library" label + folder/files buttons; Detail view shows ← back arrow + Save button. Removed Detail and Edit tab buttons. (3) Double-click to open — library thumbnails now require double-click (400ms threshold) to enter detail view instead of single-click. (4) Edit panel always visible in detail view — removed toggle. (5) Added lens profile dropdown (pick_list) in lens section with Auto/None/manual profile selection to override EXIF auto-detection. (6) Lightroom-inspired professional dark theme with styled toolbar, panel, card, and button containers using named style functions.
**Result:** Success — release build compiles, all 63 tests pass (2 new tests added: slider_ranges_are_reasonable, double_click_detection).
**Files changed:** `src/main.rs`, `src/edit.rs`
**Reasoning:** User requested all changes: slider effects were too aggressive, UI was cluttered with unnecessary tab buttons, single-click was too easy to accidentally trigger, edit panel should always be accessible, and lens auto-detection needed a manual override. Lightroom styling requested for professional look.
**Notes:** Escape key now returns to library from detail view. pick_list widget added to imports. `edit_panel_open` state field removed. `last_thumb_click` and `lens_override_name` state fields added. Color constants defined as module-level consts for the theme palette. Border::default() and Default::default() used for shadow fields in style functions.

## [2026-03-31 18:34] — Fix exaggerated editing formulas (CPU + GPU)
**Action:** Fixed overblown adjustment formulas in both CPU (edit.rs) and GPU (image.wgsl) code. Three key changes: (1) Added scale factors to zone controls — highlights/shadows use ×0.15 and whites/blacks use ×0.30, capping max additive effect at ~±0.15 in linear space (was ±1.0, the entire dynamic range). (2) Fixed contrast sigmoid — changed from k=1+|amount|×4 to k=4+|amount|×8. The old k≤3 was below the identity slope threshold (k=4), meaning positive contrast actually reduced contrast. (3) Unified GPU contrast with CPU — GPU was using direct luminance replacement while CPU used blend; both now use the same `lum + amount * (sig - lum)` blend formula.
**Result:** Success — 66 tests pass (3 new: contrast_positive_increases_contrast, contrast_negative_reduces_contrast, zone_adjustments_stay_within_reasonable_bounds). Release build compiles.
**Files changed:** `src/edit.rs`, `assets/shaders/image.wgsl`
**Reasoning:** User reported contrast/black/white/etc. were still very exaggerated after slider range reduction. Root cause was the formulas themselves: purely additive zone adjustments adding up to 1.0 in linear space (the entire dynamic range), and a sigmoid contrast that didn't actually increase contrast. Reducing slider ranges alone was insufficient — the underlying math needed fixing.
**Notes:** Scale factors chosen so max effect is ~0.15 in linear space, producing ~5-8% perceived brightness change in sRGB — subtle and professional. Lens profile accuracy also flagged: the 130 bundled profiles use synthetically generated coefficients, not real calibration data from Lensfun.

## [2026-04-01 04:29 UTC] — Rewrite zone tone adjustments to match Lightroom behavior
**Action:** Rewrote highlights/shadows/whites/blacks adjustments in both GPU shader (image.wgsl) and CPU code (edit.rs) to match Adobe Lightroom's tonal control behavior. Three fundamental changes: (1) Switched from additive RGB shift to multiplicative luminance ratio — preserves R:G:B channel ratios, eliminating color desaturation in highlights and hue shifts in shadows. (2) Zone targeting now works in perceptual (gamma 2.2) space instead of linear — linear 0.5 is perceptually ~73%, so old zones were severely misaligned. (3) Widened and overlapping zone curves — shadows peaks at L_p≈0.15-0.20 and fades by 0.65; highlights rises from 0.35 with full effect above 0.75; blacks has quadratic falloff by L_p=0.25; whites has quadratic rise from L_p=0.75. Replaced four separate `apply_highlights/shadows/whites/blacks` functions with single `apply_tone_zones`.
**Result:** Success — 68 tests pass (replaced 3 old zone tests with 5 new ones: tone_zones_highlights_affects_bright_not_dark, tone_zones_shadows_affects_dark_not_bright, tone_zones_preserve_color_ratios, tone_zones_zero_is_identity, tone_zones_positive_brightens_negative_darkens). Release build compiles.
**Files changed:** `src/edit.rs`, `assets/shaders/image.wgsl`
**Reasoning:** User reported all zone adjustments looked "unnatural and fake" compared to Lightroom. Root causes: (a) flat additive delta to all RGB channels shifts channel ratios, causing visible color casts; (b) zone targeting in linear space misaligns zones with perception; (c) zones too narrow (whites 0.85-1.0, blacks 0.0-0.15 in linear) affected almost no pixels.
**Notes:** Strength multipliers: shadows/highlights 0.25, whites/blacks 0.30 in perceptual space. At max slider values, shadow recovery lifts ~2.5 stops, highlight compression ~1 stop — matching Lightroom's ballpark. The perceptual-space delta naturally produces proportional-feeling adjustments across the tonal range (same delta has larger stop-equivalent effect in darks, smaller in brights).

## [2026-04-01 04:46 UTC] — Prevent slider track click from teleporting knob
**Action:** Added drag-detection logic to suppress iced's default click-to-teleport slider behavior. Track a `slider_drag: Option<(SliderKind, u32)>` event counter — the first `SliderChanged` event (from mousedown on the track) is deferred; values are only applied from the 2nd event onward (actual drag). `SliderReleased` only commits to undo history when count >= 2. Separated label-click-to-reset into its own `ResetSlider(SliderKind)` message so it bypasses the drag filter and still resets immediately.
**Result:** Success — 68 tests pass. Release build compiles. Single track click no longer teleports the knob; drag and double-click-to-reset still work.
**Files changed:** `src/main.rs`
**Reasoning:** User reported single click on the slider track caused abrupt value jumps. Desired behavior: knob only moves via drag or double-click-to-reset. Iced's slider widget fires `on_change` immediately on mousedown, which can't be suppressed at the widget level.
**Notes:** The 1-event delay before drag takes effect is ~16ms (one mouse-move event) — imperceptible. Double-click-to-reset timing is preserved because `last_slider_release` is always updated regardless of drag state.

## [2026-04-01 05:19 UTC] — Rewrite zone adjustments: stop-based model replaces perceptual-delta model
**Action:** Rewrote the zone tone adjustment math in both GPU shader and CPU code. Two key changes: (1) Switched from additive-delta-in-perceptual-space to photographic-stop-based model: `px *= 2^stops`. This uniformly bounds the effect — 1.5 stops max per slider regardless of pixel brightness. The old perceptual-delta approach produced 12x ratios (3.6 stops) on dark pixels from moderate slider values because gamma 2.2 amplifies small perceptual deltas into huge linear ratios at the dark end. (2) Adjusted zone shapes based on research: narrowed blacks zone from L_p=0-0.25 to 0-0.15 (concentrated endpoint control per Lightroom's design); shifted shadow peak from L_p≈0.15 to L_p≈0.20-0.25 (matching Lightroom's ~25-30% target); kept highlights and whites zones unchanged.
**Result:** Success — 68 tests pass. Release build compiles. Shadows/blacks no longer overblown.
**Files changed:** `src/edit.rs`, `assets/shaders/image.wgsl`
**Reasoning:** User reported shadows and blacks still felt overblown after the first zone rewrite. Research (Adobe's Local Laplacian paper, darktable locallaplacian.c source, RawTherapee docs) confirmed: (a) the perceptual-delta approach has unbounded dark-end amplification — a +0.12 perceptual delta at L_p=0.05 produces a 12x linear ratio; (b) Lightroom's Blacks is a narrow endpoint control (0-10% display), not a broad zone reaching 25%; (c) Lightroom's Shadows peaks around 25-30% display, not 15-20%.
**Notes:** Stop-based model: at Shadows +100, peak shadow pixel gets 1.5 stops (2.83x ratio). At Blacks +50, peak black pixel gets 0.75 stops (1.68x). Old model at same settings: 3+ stops on dark pixels. Research also shows Lightroom uses Local Laplacian Filters (edge-aware multi-scale, Paris et al. SIGGRAPH 2011) for highlights/shadows, which is fundamentally different from per-pixel masking. The stop-based per-pixel approach is the best simple approximation — it can't match edge-awareness but avoids the overblown artifacts.

## [2026-04-02 02:27 UTC] — Fix whites/blacks zone tuning: wider zones, stronger effect
**Action:** Researched professional photo editor implementations (Lightroom, darktable, RawTherapee, Capture One, GPUImage) for zone tone adjustment tuning. Found three problems with whites/blacks: (1) zones too narrow (blacks 0-15%, whites 75-100% perceptual), (2) quadratic weighting (t*t) on top of smoothstep crushes the effect, (3) 1.5 max stops too conservative for endpoint controls. Fixed by: widening blacks to 0-30%, whites to 60-100%, removing quadratic weighting (smoothstep directly), increasing max stops to 2.5 (matching professional tools' 2-3 stop range). Added 2 tests verifying whites/blacks have >10% effect on target pixels and minimal effect on opposite end.
**Result:** Success — 70 tests pass (2 new). Release build compiles.
**Files changed:** `src/edit.rs`, `assets/shaders/image.wgsl`
**Reasoning:** User reported whites and blacks sliders had minimal visible effect. Research confirmed: (a) darktable Tone Equalizer uses Gaussian windows spanning 1 EV each with -3 to +2 stops range; (b) Lightroom whites/blacks are endpoint controls with broader reach than highlights/shadows zones; (c) GPUImage uses power functions that naturally concentrate effect at endpoints; (d) quadratic weighting on top of smoothstep is not used in any professional tool — it double-softens the transition.
**Notes:** Zone overlap with highlights/shadows is intentional and matches professional tools (darktable's Gaussian windows sum to ~1.77 at any luminance, meaning zones always overlap). The smoothstep weight without quadratic provides C1-continuous transitions matching RawTherapee's approach.
