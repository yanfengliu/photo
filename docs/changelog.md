# Changelog

External-facing changes per version. Newest first. Dev-internal detail lives in `docs/devlog/`.

## 0.2.0 — 2026-06-10

Breaking: editing sliders now follow Adobe Lightroom's UI conventions, and saved slider values from earlier versions mean different strengths on the new scales (baked local edits keep rendering identically — pixels, not operations, are persisted). What changed: Exposure runs −5..+5 EV (was ±3); Contrast, Saturation, Clarity, Dehaze, Temperature, and Tint all run −100..+100 (were ±50/±50/±50/±50/±60/±60); Saturation −100 now produces exact grayscale and +100 doubles chroma; Temperature ±100 spans the same 3200K–9800K tungsten-to-cloudy range as before at finer per-unit resolution. Strength envelopes validated by the April tuning audit are preserved at the new extremes (contrast power-law exponent 0.5–1.5, clarity/dehaze gain ±0.5). The four tone zones were retuned for targeted response: band Gaussians tightened to σ=1 (from √2) and per-slider reach set to 1.5 EV at the band center (combinations still clamp at ±2 EV total), so Highlights −100 reads as highlight recovery instead of a global exposure cut. Also fixed under review: neutral temperature/tint now produces an exact identity matrix, closing a pre-existing subtle preview-vs-export mismatch at default settings, and the slider readout shows Exposure at two decimals and the ±100 sliders as integers. Validation: 317 tests including new mapping, band-isolation, smoothness, extreme-value, and neutral-identity regressions; CPU/GPU parity is now structural (both paths share the same `edit::*_amount` mapping functions); rendered before/after grid reviewed visually; three-reviewer multi-CLI review converged at iteration 1.

## 0.1.6 — 2026-06-10

Test-reliability release; no behavior changes. Fixed a CI-only flake in the persisted decode cache's same-size-rewrite test: the cache's metadata fast path intentionally treats same-size, same-mtime files as unchanged, and fast runners could rewrite a file within one Windows file-time tick, making the change invisible. The test now guarantees an observable mtime change before asserting invalidation, with a new regression covering the helper (suite is now 299 tests).

## 0.1.5 — 2026-06-10

Security maintenance release; no behavior changes. Established the project's first `cargo audit` baseline and resolved its one finding: transitive `jxl-grid` 0.6.1 → 0.6.2 (RUSTSEC-2026-0151, out-of-bounds writes on 32-bit platforms — low impact for this 64-bit-only app, fixed anyway). Five informational warnings remain on transitive crates (`instant`/`paste` unmaintained, `lru`/`rand` unsound advisories) with no compatible upstream fixes; tracked in the devlog.

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
