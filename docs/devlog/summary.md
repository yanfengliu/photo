# Devlog Summary

## Current state
- Rust + iced 0.13 + wgpu 0.19 image viewer/editor for Windows.
- GPU shader pipeline supports 12 real-time adjustments and Lensfun-based lens corrections.
- Library browsing, Detail viewing, collection management, common camera RAW viewing, and Detail-view crop plus reliably rendered icon-based rotation controls are in place.
- Repeated RAW and SVG Detail loads can reuse a persisted decoded-image cache in the repo-local `decoded-cache` directory with normalized path keys, explicit contract versioning, same-size rewrite validation when metadata changes, failed-write cleanup, and bounded retention, imported RAW and SVG files now start warming that persisted cache in the background right away, and same-session repeat Detail opens can reuse validated full images immediately without blanking while EXIF follow-up continues, including when the user leaves Detail and reopens the same library item; the in-memory cache now also keeps the two most recently viewed full Detail images hot even when large images overflow the byte budget.
- Per-image Detail edits now persist locally across restarts in `%LOCALAPPDATA%/photo/edits.json`, including rotation and crop, while save-as-copy still exports the visible edited result and remains a safe no-op while a new image is still loading; collection actions also fail closed if their destination collection disappears.
- RAW Detail view now uses an explicit staged-load state: non-RAW files still go straight to full decode, RAW files show an embedded preview first when available, only the still-current request launches the heavier follow-up work, and EXIF/lens autodetection no longer sits on the UI completion path.
- Detail-view shader uniforms now explicitly match the WGSL layout, so first renders no longer hit the crop-uniform `wgpu` validation panic that previously killed the app on open.
- Detail view now skips the blur pre-pass on first open unless clarity or dehaze is active, which reduces unnecessary GPU work in the release click-to-open path.
- Library and collection thumbnail grids now reflow from the latest window width, including after resizing while Detail view is open.
- Docs now use the directory layout expected by AGENTS.md.
- 241 unit tests currently pass.

## Recent milestones
- Detail-load speedup: add embedded RAW preview decoding, choose RAW vs. non-RAW load plans up front, show RAW previews immediately in Detail while only the still-current request launches the heavier follow-up work, centralize that staged lifecycle in `DetailLoadState`, move EXIF loading off the UI completion path, keep the preview-to-full upgrade on the user's existing zoom/pan, and guard save until the full image plus required auto lens metadata are ready.
- Local edit persistence: save non-default per-image `EditState` entries into `%LOCALAPPDATA%/photo/edits.json`, restore them on startup as the committed baseline for each image, keep rotation and crop in that persisted state, prune default or missing-file entries on write/load, and cover round-trip plus normalization behavior with regression tests.
- Same-session full-image cache: reuse already loaded full Detail images immediately on repeat opens, keep preview-only RAW states out of the cache, reopen the current image from Library by reusing the already displayed full image behind a quick metadata check, validate ordinary cache hits against current source state under a deny-write read handle, keep the two most recently viewed full Detail images hot even when large images overrun the byte budget, bound the rest of the cache with LRU-style entry/byte caps, and cover stale-state reset, library-return reopen behavior, same-size rewrites, and eviction behavior with regression tests.
- Import-time persisted-cache warming: newly imported RAW and SVG files now enqueue repo-local decoded-cache warming immediately, run one warm at a time to avoid hammering the machine on bulk imports, keep later import batches queued behind the active warm, and continue draining the queue even if one warm fails.
- Persisted detail decode cache: save decoded RAW and SVG Detail images into the repo-local `decoded-cache` directory, invalidate entries with normalized source fingerprints plus an explicit cache-contract version, validate same-size rewrites when metadata changes, clean up temp files on failed writes, use collision-safe temp writes, and cover cache creation, reuse, corruption fallback, repo-root path selection, RAW reuse, and retention pruning with regression tests.
- Detail-open crash fix: reproduce the first-render `wgpu` panic locally, trace it to a Rust/WGSL uniform-buffer layout mismatch around crop fields, add an explicit layout regression test, and restore both debug and release builds so they stay alive when opened directly into `test.jpg` and persisted `.ARW` files.
- Release click crash follow-up: make the viewer blur pre-pass lazy so first-open Detail renders only build blur resources when clarity or dehaze is active, reset blur bindings back to the placeholder on image changes, add lazy-blur state-transition coverage, and record the remaining manual-release-verification risk in the debugging note.
- Crash hardening follow-up: add/toggle photo-in-collection actions now verify their destination collection still exists, save-as-copy uses the visible crop state, loading-time save attempts no-op safely, and regression coverage locks down save status plus collection/save edge cases.
- Detail-view rotation controls: replace worded rotate actions with icon buttons that keep compact `-90°` and `+90°` cues inside the button, route the glyphs through `Segoe UI Symbol` plus advanced shaping so they render reliably in the Windows `iced` UI, and add regression coverage for both the edit-panel structure and the actual icon-label text settings.
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
