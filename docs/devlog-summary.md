# Devlog Summary

## Current state
- GPU-accelerated image viewer built with Rust + iced 0.13 + wgpu 0.19
- Uses custom WGSL shader pipeline and `iced::widget::shader::Primitive`
- Image decoding via `image` crate and `resvg`
- CLI argument support: `photo.exe path/to/image.jpg`
- Dark theme UI, release build compiles and runs
- Large images auto-downscaled to GPU texture limit before upload
- 24 unit tests across three modules (decode, nav, viewer math), all passing

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
10. Added 24 unit tests covering decode (PNG, BMP, SVG, invalid file, nonexistent file, file size, RGBA format), nav (scanning, natural sort, next/prev cycling, empty dir, case-insensitive extensions, start position), viewer math (fit-to-window, zoom scaling, pan offset, zoom-at-center, zoom-at-corner, zoom-preserves-cursor-point, zoom clamping) — SUCCESS
11. Added `tempfile` dev-dependency for filesystem tests — SUCCESS

## Key decisions
- Use iced's wgpu re-export, not standalone wgpu crate (avoids type mismatches in `shader::Primitive` trait)
- GPU texture limit check done at upload time in `prepare()`, not at decode time (GPU not available during decode)
- Downscale clone + resize happens once per oversized image; original full-res pixels kept in memory for potential future tiling
- Math functions extracted as public standalone functions for testability without GPU or iced runtime
- Used `1e-3` tolerance for f32 zoom math tests due to floating-point error in chained operations
