# Devlog Summary

## Current state
- Rust + iced 0.13 + wgpu 0.19 image viewer/editor for Windows.
- GPU shader pipeline supports 12 real-time adjustments and Lensfun-based lens corrections.
- Library browsing, Detail viewing, collection management, and common camera RAW viewing are in place.
- Docs now use the directory layout expected by AGENTS.md.
- 126 unit tests currently pass.

## Recent milestones
- RAW support: shared image-extension list, file-dialog coverage, thumbnail-first RAW embedded-image loading, raw-pixel-first detail decoding with embedded-image fallback, RAW-safe save copies, and synthetic DNG tests.
- Image editing stack: EditState, CPU save path, blur pre-pass, shader parity, and Lensfun integration.
- Collections stack: persistence, sidebar UI, context menus, collection grid, and drag-and-drop.
- Documentation stack: split architecture docs, dated detailed devlogs, compact summary, learning notes, debugging template, and review landing page.

## Keep it current
- Preserve only durable facts.
- Prefer short bullets over long chronology.
- Move deeper history into `docs/devlog/detailed/`.
