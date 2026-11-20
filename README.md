# Photo

A catalog-based, non-destructive photo organiser and raw developer for macOS and Windows, written in Rust. Its workflow is the one Lightroom Classic users already know: a Library module to import, organise and cull, and a Develop module to edit, with every edit stored as parameters and the original file never touched.

Photo is an independent project. It is not affiliated with Adobe, uses none of Adobe's code, artwork or names, and does not aim for pixel-identical output: slider names, ranges and `crs:` XMP values follow shared conventions, while the rendering math is its own and is documented with its real sources.

> Status: under construction. Version 0.3.0 is the foundation of a from-scratch restart — the workspace, the shared data model and the design. The previous app (an iced GPU viewer, v0.2.10) is preserved in history at commit `94bd2cf`. See [the design](docs/threads/current/lightroom-workflow-restart/DESIGN.md) and [the plan](docs/threads/current/lightroom-workflow-restart/PLAN.md) for what is being built and in what order.

## Feature overview

| Area | State at 0.3.0 |
| --- | --- |
| Shared data model (`photo-core`): develop settings, the adjustment table, metadata, colour spaces | Done |
| Headless UI rendering and automation (egui + `egui_kittest`) | Proven by a spike test |
| Colour science, imaging primitives, decoding, catalog, XMP sidecars | In progress |
| Develop pipeline, export, application engine | Planned |
| Library and Develop modules | Planned |

## Prerequisites

- Rust 1.95 (pinned in `rust-toolchain.toml`).
- A GPU with Metal, DX12 or Vulkan support (required by wgpu).

## Build, run, test

```sh
cargo build            # debug build; dependencies and the pixel-math crates are optimized even in debug
cargo run              # launch the app
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
src/main.rs          Entry point (CLI, default catalog location, launching the UI)
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
  threads/           design, plan and review records per objective
  policies/          local-rules.md: standing rules specific to this repository
  learning/          lessons.md
  debugging/         template.md
  api-reference.md   Public API of each crate
  changelog.md       One entry per version
```

Which crate may depend on which is the architecture; the table is in [docs/architecture/ARCHITECTURE.md](docs/architecture/ARCHITECTURE.md).

## Public surface

- `photo` — the application binary.
- `photo-core` — `DevelopSettings`, `Adj`, `GroupSet`, `PhotoMetadata`, `SourceImage`, `RgbSpace`, `DateTime` and friends; see [docs/api-reference.md](docs/api-reference.md).

## Third-party licences

RAW decoding uses [rawler](https://github.com/dnglab/dnglab) (LGPL-2.1). Everything else in the dependency tree is under permissive licences.
