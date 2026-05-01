# Architecture Drift Log

| Date | What Changed | Why | Updated By |
| --- | --- | --- | --- |
| 2026-03-29 | Initial architecture doc created from template | Project reached stable multi-module state with Library/Detail tabs | agent |
| 2026-03-30 | Added jpeg-decoder direct dependency | DCT-level downscaling for fast JPEG thumbnails; it was already a transitive dependency | agent |
| 2026-03-30 | Added image editing system (`edit.rs`, `lens.rs`, extended shader, blur pre-pass) | 12 GPU shader-based adjustments, Lensfun lens corrections, undo/redo, save-as-copy | agent |
| 2026-03-30 | Added `kamadak-exif` and `quick-xml` dependencies | EXIF reading for lens auto-detection and Lensfun XML database parsing | agent |
| 2026-04-03 | Added collection module (`collection.rs`) and sidebar UI integration | Named photo collections with JSON persistence, collection sidebar in Library view, context menu/drag types, 19 new message variants (stubbed) | agent |
| 2026-04-03 | Completed collections system (`collection.rs`, sidebar UI, context menus, drag-drop, grid view, detail nav) | Named photo collections with JSON persistence, context menu overlay, drag-and-drop, collection-scoped navigation | agent |
| 2026-04-03 | Added serde and serde_json dependencies | JSON serialization for collection persistence | agent |
| 2026-04-19 | Added RAW image support in `decode.rs`/`nav.rs` and file-dialog filters | The app now treats common camera RAW formats as first-class images by extracting embedded previews or developing raw pixels when needed | codex |
| 2026-04-19 | Split canonical docs into `docs/architecture/`, `docs/devlog/`, and support subdirectories with legacy shims at the old flat paths | Align the repo with AGENTS.md while preserving older links during the transition | codex |
| 2026-04-20 | Switched RAW Detail loading to a staged preview-first flow with async EXIF in `main.rs`/`decode.rs` | Reduce time-to-first-image in Detail while keeping the full-quality RAW decode path | codex |
| 2026-04-21 | Added local per-image edit persistence in `main.rs`/`edit.rs` | Non-destructive Detail edits, including rotation and crop, now survive reopen and restart through a local edit-state store | codex |
| 2026-04-21 | Moved local edit persistence from profile-scoped storage to repo-local `local-edits.json` | Make saved edit state inspectable inside the repo, matching the repo-local decoded-cache direction | codex |
| 2026-04-21 | Replaced persisted edit-operation history with repo-local baked local-edit files in `local-edits/` | The product contract is "reopen the edited image" rather than "restore undo history", so cross-session persistence now stores edited pixels while undo/redo remains session-only | codex |
| 2026-04-22 | Hardened repo-local baked local-edit persistence with shared full/thumbnail generations and source-metadata validation | Keep Library thumbnails and persisted local copies fail-closed after partial writes, EXIF-delayed auto-lens updates, or source rewrites | codex |
| 2026-05-01 | Documented persisted decoded-image cache (`decoded-cache/`) and `image25` rawler-interop dep in `architecture/ARCHITECTURE.md`; corrected `apply_tone_zones` doc-comment in `src/edit.rs` to describe the actual log₂-EV Gaussian implementation; removed Node/TypeScript and made-up reviewer-model references from `AGENTS.md` | Docs audit: align in-scope docs and code comments with current Rust+iced codebase | claude |
