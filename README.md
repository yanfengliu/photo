# Photo

A catalog-based, non-destructive photo organiser and raw developer for macOS and Windows, written in Rust. Its workflow is the one Lightroom Classic users already know: a Library module to import, organise and cull, and a Develop module to edit, with every edit stored as parameters and the original file never touched.

Photo is an independent project. It is not affiliated with Adobe, uses none of Adobe's code, artwork or names, and does not aim for pixel-identical output: slider names, ranges and `crs:` XMP values follow shared conventions, while the rendering math is its own and is documented with its real sources.

> Status: under construction. Version 0.3.1 has a working Library module: import, browse, rate, flag, label, keyword, collect and filter your photos. The Develop module and export come next. The previous app (an iced GPU viewer, v0.2.10) is preserved in history at commit `94bd2cf`. See [the plan](docs/work/12_lightroom-workflow-restart/plan.md) and [the design](docs/work/12_lightroom-workflow-restart/design.md) for what is being built and in what order.

## Feature overview

| Area | State at 0.3.1 |
| --- | --- |
| Library module: import, grid, loupe, compare, survey, filter bar, catalog, folders, collections and smart collections, keywords, metadata, undo | Working |
| Catalog (SQLite) and XMP sidecars | Working, reviewed by two models |
| RAW and raster decoding, colour science, imaging primitives, the develop pipeline | Built and tested; the pipeline is not yet wired into the app |
| Develop module, export | In progress |
| Agent harness (`photo --harness`, `harnessctl`) | Planned |

## Where your data lives

Run from a checkout of this repository, Photo keeps its catalog and previews in a `catalog/` folder at the repository root and exports in `edited/`, both visible and both ignored by git. Run from anywhere else, it keeps them in `Photo/` under your Pictures folder. Photo never writes to your original files; it writes XMP sidecars beside them only when you ask it to.

## Prerequisites

- Rust 1.95 (pinned in `rust-toolchain.toml`).
- A GPU with Metal, DX12 or Vulkan support (required by wgpu).

## Build, run, test

```sh
cargo run --release    # launch the app (recommended: fast previews)
cargo build            # debug build; dependencies and the pixel-math crates are optimized even in debug
cargo build --release  # optimized build with thin LTO
```

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
src/main.rs          Entry point: finds the stores and launches the UI
src/repo.rs          Where Photo keeps its data (the only code that decides a location)
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
  work/              one numbered folder per piece of work: plan, design, review rounds
  policies/          local-rules.md: the owner's rules specific to this repository
  learning/          lessons.md, lessons-evidence.md, gate-proofs.md
  debugging/         template.md
  changelog.md       One entry per version
```

Which crate may depend on which is the architecture; the table is in [docs/architecture/ARCHITECTURE.md](docs/architecture/ARCHITECTURE.md).

## Public surface

- `photo` — the application binary.
- The crates under `crates/` are internal to this workspace. Their Rust types and doc comments are the API reference; `cargo doc --no-deps --open` renders them.

## Third-party licences

RAW decoding uses [rawler](https://github.com/dnglab/dnglab) (LGPL-2.1). Everything else in the dependency tree is under permissive licences.
