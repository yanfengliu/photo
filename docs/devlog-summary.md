# Devlog Summary

## Current state
- GPU-accelerated image viewer built with Rust + iced 0.13 + wgpu 0.19
- Uses custom WGSL shader pipeline and `iced::widget::shader::Primitive`
- Image decoding via `image` crate and `resvg`
- CLI argument support: `photo.exe path/to/image.jpg`
- Dark theme UI, release build compiles and runs
- Large images auto-downscaled to GPU texture limit before upload
- Console window hidden on Windows via `#![windows_subsystem = "windows"]`
- Tab-based UI: Library tab (scrollable thumbnail grid) and Detail tab (GPU shader viewer)
- Folder picker and file picker buttons load images into library
- Thumbnails loaded asynchronously at 200px max dimension
- Clicking a thumbnail opens it in Detail view; arrow key navigation works from library
- 31 unit tests across three modules (decode, nav, viewer math), all passing

## Actions log
1. Built initial project scaffold: Cargo.toml, WGSL shader, four source modules (main, viewer, decode, nav) — SUCCESS
2. Fixed wgpu version mismatch: removed standalone `wgpu = "22"`, switched to iced's wgpu re-export — SUCCESS
3. Fixed wgpu 0.19 API differences: `entry_point` as `&str`, removed `compilation_options`/`cache` fields, explicit `RenderPassDescriptor` fields — SUCCESS
4. Added explicit `tokio` dependency (iced's tokio feature does not re-export it) — SUCCESS
5. Fixed Rust 1.94 lifetime elision warnings — SUCCESS
6. Added CLI argument support: parse `std::env::args()` in `App::new()` — SUCCESS
7. Created `.gitignore` for `/target`, binaries, `Cargo.lock`, `*.jpg` — SUCCESS
8. Fixed crash on images exceeding GPU texture limit: query `device.limits().max_texture_dimension_2d` in `prepare()`, downscale with `image::imageops::resize(Triangle)` before GPU upload — SUCCESS
9. Extracted `compute_image_rect` and `zoom_at_cursor` from viewer.rs into public standalone functions — SUCCESS
10. Added 24 unit tests covering decode, nav, and viewer math — SUCCESS
11. Added `tempfile` dev-dependency for filesystem tests — SUCCESS
12. Added `#![windows_subsystem = "windows"]` to hide console window on Windows — SUCCESS
13. Added Library/Detail tab UI: scrollable thumbnail grid, folder/file pickers, async thumbnail loading, click-to-open detail, dual navigation modes — SUCCESS
14. Added `decode_thumbnail` function in decode.rs, made `is_image_file` public in nav.rs — SUCCESS
15. Added 7 new tests (31 total): thumbnail decode tests and library UI tests — SUCCESS

## Image editing feature (feat/image-editing branch)
16. Added kamadak-exif and quick-xml dependencies for EXIF/lens data — SUCCESS
17. Added EditState (12 adjustments) and UndoHistory (undo/redo/reset) in src/edit.rs — SUCCESS
18. Added CPU adjustment math (exposure, contrast, highlights, shadows, whites, blacks, vibrance, saturation, clarity, dehaze, temperature/tint Bradford CAT, lens corrections) and edited_save_path — SUCCESS
19. Rewrote WGSL shader with full adjustment pipeline (32-field Uniforms, blur texture binding 3, lens corrections) — SUCCESS
20. Extended Rust-side Uniforms struct to match shader, added AdjustmentUniforms, blur placeholder texture, wired adjustments through pipeline — SUCCESS
21. Added Gaussian blur pre-pass: blur.wgsl shader, blur pipeline in viewer.rs, two-pass separable blur at 1/4 resolution on image load — SUCCESS
22. Added Lensfun XML parser (src/lens.rs): data types, quick-xml parser, kamadak-exif EXIF reader, LensDatabase with substring lookup, 16 bundled lens profiles — SUCCESS
23. Wired edit panel UI (Tasks 8+9): sidebar with 12 sliders in 4 sections, undo/redo/save keybinds, lens correction toggle, CPU save, EXIF auto-detection, inline text editing — SUCCESS

## Key decisions
- Use iced's wgpu re-export, not standalone wgpu crate (avoids type mismatches in `shader::Primitive` trait)
- GPU texture limit check done at upload time in `prepare()`, not at decode time (GPU not available during decode)
- Downscale clone + resize happens once per oversized image; original full-res pixels kept in memory for potential future tiling
- Math functions extracted as public standalone functions for testability without GPU or iced runtime
- Used `1e-3` tolerance for f32 zoom math tests due to floating-point error in chained operations
- Thumbnails decode full image then resize; optimization for large files deferred
- Grid uses fixed 6 columns; tab highlighting uses Unicode bullet character
- Two navigation modes: library-based (entering from library) and directory-based (CLI/drag-drop/Ctrl+O)
- AdjustmentUniforms uses plain types (f32, bool, arrays) so it can be constructed without wgpu dependency
- Slider values divided by 100 in uniform write (sliders are -100..+100, shader expects -1..+1)
- Zero temp_matrix triggers identity matrix fallback; zero TCA scales default to 1.0

## Bug fixes
24. Fixed WGSL shader crash: renamed `smooth` to `smooth_step` (reserved keyword in WGSL), unrolled blur loop (wgpu 0.19 naga forbids dynamic array indexing) — SUCCESS
