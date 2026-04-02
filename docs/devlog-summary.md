# Devlog Summary

## Current state
- GPU-accelerated image viewer/editor built with Rust + iced 0.13 + wgpu 0.19
- Custom WGSL shader pipeline with 12 real-time adjustments + lens corrections
- Library view (thumbnail grid, double-click to open) and Detail view (GPU viewer + always-on edit panel)
- Lightroom-inspired dark professional theme with styled containers and buttons
- Lens profile dropdown (pick_list) for manual override of EXIF auto-detection
- 130 bundled Lensfun lens profiles across 14 brands/systems
- 63 unit tests across five modules, all passing
- Release build compiles and runs on Windows

## Actions log
1. Built initial project scaffold: Cargo.toml, WGSL shader, four source modules — SUCCESS
2. Fixed wgpu version mismatch: switched to iced's wgpu re-export — SUCCESS
3. Fixed wgpu 0.19 API differences — SUCCESS
4. Added explicit `tokio` dependency — SUCCESS
5. Fixed Rust 1.94 lifetime elision warnings — SUCCESS
6. Added CLI argument support — SUCCESS
7. Created `.gitignore` — SUCCESS
8. Fixed crash on images exceeding GPU texture limit — SUCCESS
9. Extracted `compute_image_rect` and `zoom_at_cursor` as public functions — SUCCESS
10. Added 24 unit tests (decode, nav, viewer math) — SUCCESS
11. Added `tempfile` dev-dependency — SUCCESS
12. Added `#![windows_subsystem = "windows"]` — SUCCESS
13. Added Library/Detail tab UI with thumbnail grid — SUCCESS
14. Added `decode_thumbnail` and `is_image_file` — SUCCESS
15. Added 7 new tests (31 total) — SUCCESS

## Image editing feature
16. Added kamadak-exif and quick-xml dependencies — SUCCESS
17. Added EditState (12 adjustments) and UndoHistory — SUCCESS
18. Added CPU adjustment math — SUCCESS
19. Rewrote WGSL shader with full adjustment pipeline — SUCCESS
20. Extended Rust-side Uniforms, added AdjustmentUniforms — SUCCESS
21. Added Gaussian blur pre-pass — SUCCESS
22. Added Lensfun XML parser (src/lens.rs) — SUCCESS
23. Wired edit panel UI with 12 sliders — SUCCESS

## Bug fixes
24. Fixed WGSL shader crash: reserved keyword + dynamic indexing — SUCCESS

## Data expansion
25. Expanded Lensfun lens profiles from 14 to 130 (14 brands) — SUCCESS

## UI overhaul
26. Reduced slider ranges: exposure ±3, contrast/whites/blacks/vibrance/saturation/clarity/dehaze ±50, temp/tint ±30, highlights/shadows ±100 — SUCCESS
27. Simplified tab bar: Library label + back arrow navigation — SUCCESS
28. Double-click to enter detail view (400ms threshold) — SUCCESS
29. Edit panel always visible in detail view — SUCCESS
30. Added lens profile dropdown (pick_list) with Auto/None/manual selection — SUCCESS
31. Applied Lightroom-inspired professional dark theme (styled containers, buttons, color palette) — SUCCESS
32. Added Escape key to return to library from detail — SUCCESS

## Formula fixes
33. Fixed zone adjustment scaling: highlights/shadows ×0.15, whites/blacks ×0.30 (was ×1.0) — SUCCESS
34. Fixed contrast sigmoid: k=4+|amount|×8 (was k=1+amount×4, which was below identity threshold) — SUCCESS
35. Unified GPU contrast with CPU: both use blend formula `lum + amount * (sig - lum)` — SUCCESS
36. Rewrote zone tone adjustments (Lightroom-style): multiplicative luminance ratio (not additive), perceptual-space zone targeting, wider overlapping zones — SUCCESS
38. Rewrote zone adjustments again: stop-based model (px *= 2^stops, max 1.5 stops), narrowed blacks zone to 0-15% perceptual, shifted shadow peak to 20-25% — SUCCESS
37. Prevented slider track click from teleporting knob; only drag and double-click-to-reset work — SUCCESS
39. Fixed whites/blacks zones: widened (blacks 0-30%, whites 60-100%), removed quadratic weighting, increased to 2.5 max stops — SUCCESS

## Key decisions
- Use iced's wgpu re-export, not standalone wgpu crate
- GPU texture limit check at upload time in `prepare()`
- Math functions as public standalone for testability
- Two navigation modes: library-based vs directory-based
- AdjustmentUniforms uses plain types (no wgpu dependency)
- Slider values divided by 100 in uniform write
- Double-click detection via timestamp comparison (Instant)
- Named style functions for Lightroom theme (toolbar_button_style, etc.)
- pick_list with String values for lens profile selection
