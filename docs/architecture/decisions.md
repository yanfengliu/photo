# Architectural Decisions

| Decision | Choice | Why | Date |
| --- | --- | --- | --- |
| GUI framework | iced 0.13 | Rust-native, wgpu integration via `shader::Program`, no GC | 2026-03-29 |
| GPU rendering | Custom wgpu shader via iced's `shader::Primitive` | Direct control over texture upload, zoom/pan uniforms, and render pass | 2026-03-29 |
| wgpu version | iced's bundled wgpu 0.19 re-export | Using standalone wgpu causes type mismatches with iced's shader traits | 2026-03-29 |
| SVG rendering | resvg | Rasterizes SVG to pixels on CPU, then uploads as texture so it shares the raster pipeline | 2026-03-29 |
| Thumbnail strategy | Decode raster/SVG images directly or prefer embedded RAW thumbnails and previews before resizing to 200px max | Keeps common-format thumbnails simple while making camera RAW thumbnails fast enough for library browsing | 2026-04-19 |
| Thumbnail display | iced `Image` widget with `Handle::from_rgba` | Pre-decoded pixel data avoids iced re-decoding and coexists with shader viewer | 2026-03-29 |
| Dual navigation | `library_index` (library mode) vs `DirNav` (directory mode) | Library browsing and direct file opening are independent use cases | 2026-03-29 |
| Async decoding | `tokio::task::spawn_blocking` | Keeps the UI responsive; iced's tokio feature provides the runtime | 2026-03-29 |
| GPU texture limit | Runtime query in `prepare()`, downscale if exceeded | The limit varies by GPU and cannot be hardcoded | 2026-03-30 |
| RAW decoding | `rawler` with thumbnail/preview-first library decoding and raw-pixel-first detail decoding with embedded-image fallback | Adds broad camera RAW support while keeping the viewer and thumbnail contracts unchanged | 2026-04-19 |
| RAW detail loading | Supersede the earlier raw-pixel-first Detail choice with staged RAW Detail loading: embedded preview first when available, then background full-resolution upgrade plus async EXIF | Improves time-to-first-image in Detail without giving up the higher-quality final RAW path | 2026-04-20 |
