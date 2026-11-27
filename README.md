# Photo

A catalog-based, non-destructive photo organiser and raw developer for macOS and Windows, written in Rust. Its workflow is the one Lightroom Classic users already know: a Library module to import, organise and cull, and a Develop module to edit, with every edit stored as parameters and the original file never touched.

Photo is an independent project. It is not affiliated with Adobe, uses none of Adobe's code, artwork or names, and does not aim for pixel-identical output: slider names, ranges and `crs:` XMP values follow shared conventions, while the rendering math is its own and is documented with its real sources.

> Status: under construction. Version 0.3.2 has both modules working end to end: import, organise and cull in the Library, edit in Develop, and export. It has been run on synthetic test photos and on one real camera RAW (a Sony A7R V file), which at default settings renders within about a quarter stop of the camera's own JPEG. The previous app (an iced GPU viewer, v0.2.10) is preserved in history at commit `94bd2cf`. See [the plan](docs/work/12_lightroom-workflow-restart/plan.md) and [the design](docs/work/12_lightroom-workflow-restart/design.md) for what is being built and in what order.

## Feature overview

| Area | State at 0.3.2 |
| --- | --- |
| Library module: import, grid, loupe, compare, survey, filter bar, catalog, folders, collections and smart collections, keywords, metadata, Quick Develop, undo | Working |
| Develop module: Basic, Tone Curve, Color Mixer, Color Grading, Detail, Lens Corrections, Transform, Effects and Calibration panels; crop and straighten, healing, linear, radial and brush masks; before and after, clipping, history, snapshots, presets, copy and paste | Working |
| Export: JPEG, PNG and TIFF, resizing, colour space, name templates, output sharpening, metadata choices, a text watermark, export presets | Working, reviewed by one model |
| Catalog (SQLite) and XMP sidecars | Working, reviewed by two models |
| Agent harness (`photo --harness`, `harnessctl`): drives the real app headlessly and measures what it shows | Working; see [the guide](docs/guides/agent-harness.md) |

## Where your data lives

Run from a checkout of this repository, Photo keeps its catalog and previews in a `catalog/` folder at the repository root and exports in `edited/`, both visible and both ignored by git. Run from anywhere else, it keeps them in `Photo/` under your Pictures folder. Photo never writes to your original files; it writes XMP sidecars beside them only when you ask it to.

## Prerequisites

- Rust 1.95 (pinned in `rust-toolchain.toml`).
- A GPU with Metal, DX12 or Vulkan support (required by wgpu).

## Build, run, test

```sh
cargo run --release                      # launch the app (recommended: fast previews)
cargo run --release -- path/to/photo.dng # launch with that photo imported and open
cargo build                              # debug build; dependencies and the pixel-math crates are optimized even in debug
cargo build --release                    # optimized build with thin LTO
```

No photos at hand? `cargo run -p photo-testkit --example write_fixtures` writes a small folder of synthetic ones (JPEG, a RAW DNG, PNG, 16-bit TIFF) under the system temporary directory and prints where.

The four gates every change must pass:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build
```

They cover the whole workspace: the root manifest lists every crate in `default-members`.

## Project structure

```
src/main.rs          Entry point: finds the stores and launches the UI or the harness
src/repo.rs          Where Photo keeps its data (the only code that decides a location)
src/launch.rs        Opening a photo named on the command line
src/harness/         The agent harness: the real app without a window, driven over JSON lines
src/bin/harnessctl.rs  The harness's command-line client
crates/
  core/              photo-core: shared data types and colour-space constants
  color/             photo-color: per-pixel colour science
  imaging/           photo-imaging: buffers, geometry, filters, masks, healing
  develop/           photo-develop: the one pipeline from settings to pixels
  decode/            photo-decode: raster and RAW files to linear images, metadata
  catalog/           photo-catalog: the SQLite catalog
  xmp/               photo-xmp: XMP sidecars
  export/            photo-export: render and write files
  engine/            photo-engine: the application without a UI
  ui/                photo-ui: egui views and the headless harness
  testkit/           photo-testkit: synthetic test fixtures
docs/
  architecture/      ARCHITECTURE.md, decisions.md, drift-log.md
  devlog/            summary.md and dated detailed logs
  guides/            agent-harness.md: driving the app from a script and reading what it measures
  work/              one numbered folder per piece of work: plan, design, review rounds
  policies/          local-rules.md: the owner's rules specific to this repository
  learning/          lessons.md, lessons-evidence.md, gate-proofs.md
  debugging/         template.md
  changelog.md       One entry per version
```

Which crate may depend on which is the architecture; the table is in [docs/architecture/ARCHITECTURE.md](docs/architecture/ARCHITECTURE.md).

## Public surface

- `photo` — the application binary. `photo PATH` opens that photo; `photo --harness[=PORT]` runs it without a window for an agent to drive.
- `harnessctl` — the harness's client: one command per call, or a script of them.
- The crates under `crates/` are internal to this workspace. Their Rust types and doc comments are the API reference; `cargo doc --no-deps --open` renders them.

## Third-party licences

RAW decoding uses [rawler](https://github.com/dnglab/dnglab) (LGPL-2.1). Everything else in the dependency tree is under permissive licences.
