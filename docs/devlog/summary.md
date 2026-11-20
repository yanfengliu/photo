# Devlog Summary

## Recent milestones

- 2026-11-19 — Restart (v0.3.0): repository cleared to `AGENTS.md` + `CLAUDE.md` (old iced viewer v0.2.10 kept at `94bd2cf`) after judging it not on the path — 8-bit sRGB edit source even for RAW, baked-pixel persistence with session-only undo, and hand-mirrored GPU/CPU math are each the opposite of a parametric raw developer. New foundation: twelve-package workspace, `photo-core` data model (67 tests), headless UI rendering proven with egui 0.37 + `egui_kittest`, design and plan in `docs/threads/current/lightroom-workflow-restart/`.

## Current state

- Rust 1.95, edition 2024, `unsafe` forbidden workspace-wide. Version 0.3.0. External changelog: `docs/changelog.md`.
- Target: a Lightroom Classic-style workflow (catalog, Library and Develop modules, parametric edits, export) on macOS and Windows. Not affiliated with Adobe; rendering math is our own with cited sources.
- Workspace: `photo` (entry point) + `photo-core`, `photo-color`, `photo-imaging`, `photo-develop`, `photo-decode`, `photo-catalog`, `photo-xmp`, `photo-export`, `photo-engine`, `photo-ui`, `photo-testkit`. The root manifest's `default-members` makes the four gates cover every crate.
- Done: `photo-core` (settings, `Adj` table, groups, history labels, metadata, `DateTime`, `SourceImage`, colour spaces). Every other crate is a documented skeleton being filled in per the plan's milestones M2–M7.
- Stack decisions: egui/eframe 0.37 on wgpu for the UI; one CPU pipeline in linear Rec. 2020 `f32` for preview and export; SQLite (rusqlite, bundled) catalog with develop settings as JSON; rawler 0.8 for RAW; sources stored as linear `u16`, camera-native and un-white-balanced for RAW.
- Verification approach: contract tests per crate; UI driven and rendered headlessly through `egui_kittest` (accessibility-tree queries plus real frames written to `target/ui-snapshots/`).
- Tests cannot touch real data by construction: only `src/main.rs` resolves a default catalog location; all library code takes explicit paths.
- Dev builds optimize dependencies (opt-level 2) and the pixel-math crates (opt-level 3).
- Run the format gate as plain `cargo fmt --check` from the workspace root. With `--manifest-path` it silently checks only the root package (this let v0.3.0 ship with a red format gate; see `docs/learning/lessons.md`).
- `cargo audit`: 0 vulnerabilities; 5 unmaintained-crate warnings, all transitive (chrono via rawler; ttf-parser, zbus_names, zvariant, zvariant_derive only on Linux dependency paths). Re-check by 2027-02-19.
- Carried-over standing rules live in `docs/policies/local-rules.md` (no unfalsifiable "Lightroom-style" claims; tone controls in perceptual space; zone tests sample inside the peak; clamp at regime cliffs).

## Keep it current

- Preserve only durable facts.
- Prefer short bullets over long chronology.
- Move deeper history into `docs/devlog/detailed/`.
