# Changelog

One entry per version, newest first. Versions through 0.2.10 belong to the previous app and are in git history before commit `94bd2cf`.

## 0.3.1 — 2026-11-20

The Library module: the first version you can use. Run `cargo run --release` from a checkout.

- **Your catalog.** Run from a checkout, the catalog and its previews live in a visible `catalog/` folder at the repository root and exports go to `edited/`; run from anywhere else, both live in `Photo/` under your Pictures folder. Original photo files are never written to.
- **Import.** Choose a folder (or type its path), include subfolders or not, add photos where they are or copy them into folders by date, skip suspected duplicates, give every photo keywords, and read ratings, labels, keywords and develop settings from existing XMP sidecars. Candidates appear with thumbnails as the folder is scanned; untick the ones you do not want.
- **Browse.** Grid, loupe (fit, fill and 1:1), compare and survey; a filmstrip; sources from the Catalog (All Photographs, Quick Collection, Previous Import), Folders and Collections panels; a filter bar that searches text, filters by flag, rating and colour label, and narrows by metadata columns.
- **Organise.** Ratings, pick and reject flags, colour labels, keywords (by typing, or by ticking them in the keyword list), titles, captions, creator and copyright; collections, collection sets and smart collections with a rule editor; virtual copies; rotation; removing photos from the catalog without touching the files. Every change can be undone and redone.
- **Keyboard.** The shortcuts a Lightroom Classic user already has in their hands: G, E, C and N for the views, 0 to 5 and the brackets for ratings, P, X and U for flags, 6 to 9 for labels, B for the Quick Collection, Cmd+Z to undo, and the rest listed in the design. Cmd is Ctrl on Windows.
- **Sidecars.** Ratings, flags, labels, text, keywords and develop settings are written to and read from XMP sidecars on request. A sidecar this version cannot read in full is never overwritten with less.

Not yet in this version: the Develop module, export, and an agent harness for driving the running app from scripts. Thumbnails are the camera's embedded previews and the files' own pixels until the Develop module renders edits.

Validation: the four gates pass on the merged tree (counts in the devlog); the catalog and sidecar code had two rounds of independent review by two models, recorded in `docs/work/12_lightroom-workflow-restart/reviews/`.

## 0.3.0 — 2026-11-19

A from-scratch restart. Photo is being rebuilt as a catalog-based, non-destructive photo organiser and raw developer with Library and Develop modules.

**Breaking:** everything. The previous iced-based viewer, its `library.txt` and `collections.json` files, its baked `local-edits/` and `decoded-cache/` directories and its agent harness protocol are gone. Nothing from a 0.2.x installation is read or migrated. The old app remains available at commit `94bd2cf`.

What this version contains:

- A Cargo workspace of twelve packages whose dependency graph is the architecture (see `docs/architecture/ARCHITECTURE.md`).
- `photo-core`, the shared data model: `DevelopSettings` (white balance, tone, presence, tone curve, colour mixer, colour grading, detail, lens corrections, transform, effects, calibration, orientation, crop, healing spots, masks), the `Adj` table that is the single source of every slider's key, label, range and step, settings groups for copy/paste and presets, history-step naming, photo metadata, ratings, flags and labels, a dependency-free `DateTime`, the `SourceImage` container (linear 16-bit, camera-native for RAW), and RGB colour spaces with matrices derived from their chromaticities.
- Proof that the UI can be driven and rendered without a window, which is how the app will be verified from here on.
- The design and plan for the rest of the build.

Not yet usable as an application: running it opens a placeholder window.

Validation: `cargo test` (67 tests), `cargo clippy --all-targets -- -D warnings` and `cargo build` pass. `cargo fmt --check` was reported as passing when 0.3.0 was committed but had only checked the root package; `photo-core` was formatted in a follow-up commit and the UI spike is replaced in the next version. `cargo audit`: 0 vulnerabilities, 5 unmaintained-crate warnings in transitive dependencies (tracked in the devlog).
