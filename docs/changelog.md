# Changelog

External-facing changes per version. Newest first. Dev-internal detail lives in `docs/devlog/`.

## 0.1.1 — 2026-06-10

Code hygiene release; no behavior changes. Restored the project's quality gates on rustc 1.94.0: applied rustfmt formatting (24 stale diffs in decode/edit/main) and fixed 26 clippy lints that newer toolchains flag (iterator `flatten`/`repeat_n` modernizations, struct-literal initialization in tests, a type alias for test hook cells, and a tidier test-DNG helper signature — all in test code except one cache-dir early-return cleanup). Validation: 298 tests pass, clippy `-D warnings` clean, fmt clean.

## 0.1.0 — 2026-06-10 (retroactive baseline)

First recorded baseline; versions before this entry were not tracked. Current shipped state: GPU-accelerated image viewer/editor for Windows. Library tab with collections, drag-and-drop, and responsive thumbnail grid; Detail tab with zoom/pan, freeform/square crop, and 90° rotation; 12 real-time GPU adjustments (exposure, temperature/tint, contrast, four tone zones, vibrance, saturation, clarity, dehaze) with darktable-derived math; Lensfun-based lens corrections (distortion, vignetting, TCA); broad RAW support with staged preview-then-full Detail loading; persisted decoded-image cache and baked local-edit persistence across restarts; save-as-copy export matching the on-screen preview. Validation: 298 unit tests passing.
