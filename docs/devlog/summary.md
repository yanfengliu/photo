# Devlog Summary

## Current state
- Rust + iced 0.13 + wgpu 0.19 image viewer/editor for Windows.
- GPU shader pipeline supports 12 real-time adjustments and Lensfun-based lens corrections.
- Library browsing, Detail viewing, collection management, common camera RAW viewing, and Detail-view crop plus icon-based rotation controls are in place.
- Detail save-as-copy now follows the visible crop state, becomes a safe no-op while a new image is still loading, and collection actions fail closed if their destination collection disappears.
- Detail-view shader uniforms now explicitly match the WGSL layout, so first renders no longer hit the crop-uniform `wgpu` validation panic that previously killed the app on open.
- Detail view now skips the blur pre-pass on first open unless clarity or dehaze is active, which reduces unnecessary GPU work in the release click-to-open path.
- Library and collection thumbnail grids now reflow from the latest window width, including after resizing while Detail view is open.
- Docs now use the directory layout expected by AGENTS.md.
- 195 unit tests currently pass.

## Recent milestones
- Detail-open crash fix: reproduce the first-render `wgpu` panic locally, trace it to a Rust/WGSL uniform-buffer layout mismatch around crop fields, add an explicit layout regression test, and restore both debug and release builds so they stay alive when opened directly into `test.jpg` and persisted `.ARW` files.
- Release click crash follow-up: make the viewer blur pre-pass lazy so first-open Detail renders only build blur resources when clarity or dehaze is active, reset blur bindings back to the placeholder on image changes, add lazy-blur state-transition coverage, and record the remaining manual-release-verification risk in the debugging note.
- Crash hardening follow-up: add/toggle photo-in-collection actions now verify their destination collection still exists, save-as-copy uses the visible crop state, loading-time save attempts no-op safely, and regression coverage locks down save status plus collection/save edge cases.
- Detail-view rotation controls: replace worded rotate actions with icon buttons that keep compact `-90°` and `+90°` cues inside the button, and add widget-tree regression coverage for the edit-panel structure around the rotation controls.
- Responsive library layout: track window resize state in app state, drive library and collection thumbnail columns from the current window width, share grid geometry constants between layout math and rendering, and cover both library and collection reflow after resizing in Detail.
- Detail-view crop support: freeform and square crop tools, crop overlay preview, pixel-snapped preview/save parity, crop-aware status dimensions, actual-size zoom preservation across crop changes, rotated crop save parity, and new viewer/app/save regression coverage.
- Detail-view pan affordance: fit-to-window images no longer drift on click, zoomed or off-center images still drag correctly, zoom-at-cursor respects clamp limits, and viewer event tests now cover drag start, drag move, and re-entry behavior.
- Rotation support: clockwise/counterclockwise Detail-view rotation, undo/redo/reset coverage, status-bar dimension updates, actual-size zoom parity across rotate/undo/reset, save rotation parity, and preview UV-direction coverage.
- RAW support: shared image-extension list, file-dialog coverage, thumbnail-first RAW embedded-image loading, raw-pixel-first detail decoding with embedded-image fallback, RAW-safe save copies, and synthetic DNG tests.
- Image editing stack: EditState, CPU save path, blur pre-pass, shader parity, and Lensfun integration.
- Collections stack: persistence, sidebar UI, context menus, collection grid, and drag-and-drop.
- Documentation stack: split architecture docs, dated detailed devlogs, compact summary, learning notes, debugging template, and review landing page.

## Keep it current
- Preserve only durable facts.
- Prefer short bullets over long chronology.
- Move deeper history into `docs/devlog/detailed/`.
