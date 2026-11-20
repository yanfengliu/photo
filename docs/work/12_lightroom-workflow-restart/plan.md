# Restart as a catalog-based photo organiser and raw developer

Status: active
Owner: Coordinating Claude session of 2026-11-19
Created: 2026-11-19
Updated: 2026-11-20

## Problem and outcome

On 2026-11-19 the owner asked for the repository to be cleared, keeping only `AGENTS.md` and `CLAUDE.md`, and for an app that works like Adobe Lightroom to be built in its place, keeping the existing code only if it was already on the path to that. It was judged not to be ([design.md](design.md) §2): its edit source was 8-bit sRGB even for RAW, it persisted baked pixels, and its adjustment math existed twice, in a shader and in Rust.

The outcome is Photo: an original app that follows the Lightroom Classic workflow. A catalog holds every photo; the Library module imports, browses and organises them; the Develop module edits them non-destructively with parametric settings; export writes finished files. It uses no Adobe names, artwork or code, and it does not claim pixel-identical rendering.

## Scope

Included: the catalog (import, folders, collections, smart collections, keywords, ratings, flags, colour labels, virtual copies, filtering, sorting); Library views (grid, loupe, compare, survey); the Develop module (basic tone and presence, tone curve, colour mixer, colour grading, detail, lens, transform, effects, calibration, crop and straighten, healing spots, linear, radial and brush masks, before and after, history, snapshots, presets, copy, paste and sync); export to JPEG, PNG and TIFF with resizing, colour space, naming, sharpening and watermark; XMP sidecars; RAW decoding; the agent harness that `AGENTS.md` describes.

Excluded, as [design.md](design.md) §1 lists: the Map, Book, Slideshow, Print and Web modules, tethering, cloud sync, AI masks, lens-profile databases, camera-matching profiles, HDR and panorama merge, content-aware remove, soft proofing, stacks, a second window and video.

Ownership: the coordinator owns `photo-core`, the root package, the docs, integration and acceptance. Each other crate has one builder at a time, working only inside that crate.

Workspace: the first wave was built in the primary checkout before the coordinator re-read the worktree rule; from the second wave on, work happens in the linked worktree `.claude/worktrees/restart` on branch `restart`, and `main` moves only by fast-forward once the gates pass there.

## Approach

[design.md](design.md) is the contract between the crates: one CPU develop pipeline in linear Rec. 2020 `f32` shared by preview, thumbnail, histogram and export; settings as the edit, stored as JSON in a SQLite catalog; an egui interface that holds no business logic; every behaviour reachable headlessly.

The crate graph makes the work parallel. Crates that depend only on `photo-core` were built side by side, then the pipeline and the engine on top of them, then the two modules of the interface on the engine's real API. Each brief is written against the code that landed, not against this plan.

Releases follow what a user can see: 0.3.0 was the foundation, the next bump is the working Library, the one after is Develop and export.

## Acceptance criteria

- [ ] The four gates pass on `main` (`cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`), and CI is green on macOS and Windows for the final commit.
- [ ] The develop pipeline holds its contracts as tests: tone identity, monotonicity and zones (T1 to T5), and tile, scale, statistics and thread independence (R1 to R4).
- [ ] An end-to-end run through the agent harness imports a folder holding RAW and raster files, browses it, rates, flags and filters, edits with every Develop panel, crops, heals, masks, undoes, snapshots, applies a preset, pastes settings to a second photo and exports; the screenshots are read at full resolution and the exported file matches the preview render.
- [ ] Organisation and edits survive closing and reopening the catalog, and a catalog written by a newer schema is refused untouched.
- [ ] A sidecar written by Photo reads back to the same settings, and unknown content in an existing sidecar survives a rewrite.
- [ ] The original file is never written: a test hashes the sources before and after the end-to-end run.
- [ ] The app is drivable as `AGENTS.md` describes: `photo --harness` with sandboxed storage, `harnessctl` commands, artifacts under `tmp/harness-runs/`, and the guide at `docs/guides/agent-harness.md`.
- [ ] The high-risk parts (catalog, sidecar writer, export, engine concurrency) have an independent review recorded under `reviews/`, with the findings that matter fixed and the fixes checked.
- [ ] README, the architecture set, the changelog and the devlog describe what exists.

## Implementation steps

- [x] Clear the repository; old app reachable at `94bd2cf` (commit `00dc686`).
- [x] Foundation: twelve-package workspace, `photo-core` data model, headless UI rendering proven, design written (commit `c59c2bf`, version 0.3.0).
- [x] Wave 1, one builder per crate, each depending only on `photo-core`: `photo-color`, `photo-imaging`, `photo-decode` with `photo-testkit`, `photo-catalog`, `photo-xmp`, and the UI widget kit with its headless driver (2026-11-20).
- [x] `photo-develop`: the pipeline over `photo-color` and `photo-imaging`, with its contracts and a rendered tuning grid (2026-11-20; the recovery recipe's top-stop re-clipping is an open tuning item).
- [x] `photo-engine`, Library side: catalog session, import, previews, selection, undoable attributes, collections, keywords, sidecars (2026-11-20).
- [x] Independent review of the catalog and sidecars, rounds 0 and 1, and their repairs ([reviews](reviews/)).
- [ ] `photo-export`, and the engine's Develop side: edit session, render worker, history, snapshots, presets, copy and paste, export jobs.
- [x] Library module: shell, grid, loupe, compare, survey, panels, filter bar, filmstrip, import dialog. Version 0.3.1 (2026-11-20).
- [ ] Develop module: canvas, every panel, crop, healing and masking tools, before and after, export dialog. Version 0.3.2.
- [ ] Integration: `src/main.rs`, storage-root discovery, `photo --harness` and `harnessctl`, end-to-end run, performance check.
- [ ] Independent review of the whole, repairs, and a documentation pass.

## Outcome

Pending.
