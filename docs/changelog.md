# Changelog

External-facing changes per version. Newest first. Dev-internal detail lives in `docs/devlog/`.

## 0.1.0 — 2026-06-10 (retroactive baseline)

First recorded baseline; versions before this entry were not tracked. Current shipped state: GPU-accelerated image viewer/editor for Windows. Library tab with collections, drag-and-drop, and responsive thumbnail grid; Detail tab with zoom/pan, freeform/square crop, and 90° rotation; 12 real-time GPU adjustments (exposure, temperature/tint, contrast, four tone zones, vibrance, saturation, clarity, dehaze) with darktable-derived math; Lensfun-based lens corrections (distortion, vignetting, TCA); broad RAW support with staged preview-then-full Detail loading; persisted decoded-image cache and baked local-edit persistence across restarts; save-as-copy export matching the on-screen preview. Validation: 298 unit tests passing.
