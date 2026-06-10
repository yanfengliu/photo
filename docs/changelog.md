# Changelog

External-facing changes per version. Newest first. Dev-internal detail lives in `docs/devlog/`.

## 0.1.4 — 2026-06-10

Developer-experience release; no behavior changes. Dev/test builds now compile dependencies at `opt-level = 2` (the crate itself stays unoptimized for debugging), which cuts the test suite from ~105s to ~14s — rawler's RAW develop path was pathologically slow unoptimized, with single tests taking 100 seconds.

## 0.1.3 — 2026-06-10

Internal restructuring release; no behavior changes. The 9,006-line `src/main.rs` monolith is split into focused modules: an `app/` module (state/messages, update loop, view composition, tests), plus `theme`, `widgets`, `detail_load`, `session_cache`, `local_edits`, `loading`, `library`, and `repo` modules; `main.rs` is now a 31-line entry point. All 298 tests pass unchanged; the GPU pipeline, edit math, and on-disk formats are untouched.

## 0.1.2 — 2026-06-10

Build reproducibility release; no behavior changes. Source builds now pin rustc 1.94.0 via `rust-toolchain.toml` (with clippy/rustfmt components), `Cargo.lock` is committed so builds resolve identical dependencies, and GitHub Actions CI now runs the four quality gates (fmt, clippy `-D warnings`, test, build) on every push and pull request on a Windows runner.

## 0.1.1 — 2026-06-10

Code hygiene release; no behavior changes. Restored the project's quality gates on rustc 1.94.0: applied rustfmt formatting (24 stale diffs in decode/edit/main) and fixed 26 clippy lints that newer toolchains flag (iterator `flatten`/`repeat_n` modernizations, struct-literal initialization in tests, a type alias for test hook cells, and a tidier test-DNG helper signature — all in test code except one cache-dir early-return cleanup). Validation: 298 tests pass, clippy `-D warnings` clean, fmt clean.

## 0.1.0 — 2026-06-10 (retroactive baseline)

First recorded baseline; versions before this entry were not tracked. Current shipped state: GPU-accelerated image viewer/editor for Windows. Library tab with collections, drag-and-drop, and responsive thumbnail grid; Detail tab with zoom/pan, freeform/square crop, and 90° rotation; 12 real-time GPU adjustments (exposure, temperature/tint, contrast, four tone zones, vibrance, saturation, clarity, dehaze) with darktable-derived math; Lensfun-based lens corrections (distortion, vignetting, TCA); broad RAW support with staged preview-then-full Detail loading; persisted decoded-image cache and baked local-edit persistence across restarts; save-as-copy export matching the on-screen preview. Validation: 298 unit tests passing.
