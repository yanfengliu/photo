# Devlog Summary

## Current state
- Rust + iced 0.13 + wgpu 0.19 image viewer/editor for Windows.
- GPU shader pipeline supports 12 real-time adjustments and Lensfun-based lens corrections.
- Library browsing, Detail viewing, collection management, common camera RAW viewing, and Detail-view rotation plus crop are in place.
- Library and collection thumbnail grids now reflow from the latest window width, including after resizing while Detail view is open.
- Docs now use the directory layout expected by AGENTS.md.
- 170 unit tests currently pass.

## Recent milestones
- Responsive library layout: track window resize state in app state, drive library and collection thumbnail columns from the current window width, share grid geometry constants between layout math and rendering, and cover both library and collection reflow after resizing in Detail.
- Detail-view crop support: freeform and square crop tools, crop overlay preview, pixel-snapped preview/save parity, crop-aware status dimensions, actual-size zoom preservation across crop changes, rotated crop save parity, and new viewer/app/save regression coverage.
- Detail-view pan affordance: fit-to-window images no longer drift on click, zoomed or off-center images still drag correctly, zoom-at-cursor respects clamp limits, and viewer event tests now cover drag start, drag move, and re-entry behavior.
- Rotation support: clockwise/counterclockwise Detail-view rotation, undo/redo/reset coverage, status-bar dimension updates, actual-size zoom parity across rotate/undo/reset, save/export rotation parity, and preview UV-direction coverage.
- RAW support: shared image-extension list, file-dialog coverage, thumbnail-first RAW embedded-image loading, raw-pixel-first detail decoding with embedded-image fallback, RAW-safe save copies, and synthetic DNG tests.
- Image editing stack: EditState, CPU save path, blur pre-pass, shader parity, and Lensfun integration.
- Collections stack: persistence, sidebar UI, context menus, collection grid, and drag-and-drop.
- Documentation stack: split architecture docs, dated detailed devlogs, compact summary, learning notes, debugging template, and review landing page.

## Keep it current
- Preserve only durable facts.
- Prefer short bullets over long chronology.
- Move deeper history into `docs/devlog/detailed/`.
