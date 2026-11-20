# Changelog

One entry per version, newest first. Versions through 0.2.10 belong to the previous app and are in git history before commit `94bd2cf`.

## 0.3.0 — 2026-11-19

A from-scratch restart. Photo is being rebuilt as a catalog-based, non-destructive photo organiser and raw developer with Library and Develop modules.

**Breaking:** everything. The previous iced-based viewer, its `library.txt` and `collections.json` files, its baked `local-edits/` and `decoded-cache/` directories and its agent harness protocol are gone. Nothing from a 0.2.x installation is read or migrated. The old app remains available at commit `94bd2cf`.

What this version contains:

- A Cargo workspace of twelve packages whose dependency graph is the architecture (see `docs/architecture/ARCHITECTURE.md`).
- `photo-core`, the shared data model: `DevelopSettings` (white balance, tone, presence, tone curve, colour mixer, colour grading, detail, lens corrections, transform, effects, calibration, orientation, crop, healing spots, masks), the `Adj` table that is the single source of every slider's key, label, range and step, settings groups for copy/paste and presets, history-step naming, photo metadata, ratings, flags and labels, a dependency-free `DateTime`, the `SourceImage` container (linear 16-bit, camera-native for RAW), and RGB colour spaces with matrices derived from their chromaticities.
- Proof that the UI can be driven and rendered without a window, which is how the app will be verified from here on.
- The design and plan for the rest of the build.

Not yet usable as an application: running it opens a placeholder window.

Validation: `cargo test` (67 tests), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` and `cargo build` pass. `cargo audit`: 0 vulnerabilities, 5 unmaintained-crate warnings in transitive dependencies (tracked in the devlog).
