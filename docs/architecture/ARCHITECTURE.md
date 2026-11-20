# Architecture

> Last verified: 2026-11-19 (v0.3.0)

## Section map

- [System overview](#system-overview): what the app is.
- [Component map](#component-map): one row per crate, what it owns and what it may depend on.
- [Boundaries](#boundaries): the one crate allowed to do each thing.
- [Data flow](#data-flow): import, browsing, developing, export.
- [Technology map](#technology-map): each library, its version and what it is for.
- [See also](#see-also): decisions, drift log, design.

## System overview

Photo is a catalog-based, non-destructive photo organiser and raw developer for macOS and Windows. A catalog (one SQLite file) records every imported photo, its metadata, how it is organised, and its edit as a set of parameters. Original files are never modified. Everything shown on screen or written by an export is derived from the original file plus those parameters by a single develop pipeline.

The workspace is layered so that the dependency graph is the architecture: shared data types at the bottom, independent engines above them (colour math, image primitives, decoding, catalog, XMP), the develop pipeline and export composing those, an engine that is the whole application without a user interface, and a thin egui layer on top.

At v0.3.0 only `photo-core` is implemented; the other crates are documented skeletons. This document describes the structure they are being built into, and is updated as each lands.

## Component map

| Crate | Path | Owns | May depend on | State |
| --- | --- | --- | --- | --- |
| `photo` | `src/main.rs` | Entry point: CLI parsing, the default catalog location, logging, launching the UI. | ui, engine, directories | Stub |
| `photo-core` | `crates/core` | Shared data types: `DevelopSettings` and the `Adj` table, settings groups, ids, rating/flag/label, `PhotoMetadata`, `DateTime`, `SourceImage`, colour-space constants and matrices. No I/O, no pixel loops. | serde | Done |
| `photo-color` | `crates/color` | Per-pixel colour science on `[f32; 3]`: white balance, base curves, tone, curves, colour mixer, grading, output transform. | core | In progress |
| `photo-imaging` | `crates/imaging` | Buffer primitives: planes, mip pyramid, resampling, geometry mapping, filters, mask rasterization, healing, histogram. | core, rayon | In progress |
| `photo-develop` | `crates/develop` | The pipeline: stage order, region and scale handling, stage caches, auto tone and auto white balance. | core, color, imaging, rayon | Planned |
| `photo-decode` | `crates/decode` | Files in; `SourceImage`, `PhotoMetadata` and an embedded preview out. | core, image, rawler, kamadak-exif, rayon | In progress |
| `photo-catalog` | `crates/catalog` | The SQLite catalog. | core, rusqlite, serde_json | In progress |
| `photo-xmp` | `crates/xmp` | XMP sidecar parsing and writing. | core, quick-xml | In progress |
| `photo-export` | `crates/export` | Rendering through `photo-develop` and encoding to disk. | core, decode, develop, image | Planned |
| `photo-engine` | `crates/engine` | The application without a UI: open catalog, background jobs, preview service, undo, selection, clipboard, presets. | every crate above | Planned |
| `photo-ui` | `crates/ui` | egui views, input mapping and the headless harness. No business logic. | core, engine, egui, eframe, rfd | In progress |
| `photo-testkit` | `crates/testkit` | Dev-only synthetic fixtures. | core, image, tempfile | In progress |

## Boundaries

- Only `photo-develop` turns settings and source pixels into rendered pixels. Preview, thumbnail, histogram and export all go through it; there is no second implementation of any adjustment.
- Only `photo-catalog` speaks SQL. No SQL is ever built from user text.
- Only `photo-decode` reads image codecs and EXIF; only `photo-export` writes image files. The engine may use the `image` crate solely for its own JPEG preview-cache files.
- Only `photo-xmp` reads and writes sidecars. It never writes into an original image file.
- Only `src/main.rs` resolves a default storage location. Every library function takes explicit paths, so tests cannot reach real user data.
- `photo-color` and `photo-imaging` do not depend on each other; `photo-develop` is where they meet.
- `photo-ui` depends on `photo-core` and `photo-engine` only. It never calls the catalog, decoder, pipeline or exporter directly.
- `photo-core` contains no I/O and no loops over pixels.
- `unsafe` is forbidden in every crate.

## Data flow

```
 file on disk
     │  photo-decode
     ▼
 SourceImage (linear u16; camera-native for RAW) ──► mip pyramid (photo-imaging)
                                                        │
 DevelopSettings (JSON in the catalog) ─────────────────┤
                                                        ▼
                                           photo-develop: heal → resample → input
                                           → denoise → [cache] → exposure/base curve
                                           → tone → colour → presence → sharpen
                                           → effects → output
                                                        │
                 ┌──────────────────────────────────────┼─────────────────────────┐
                 ▼                                      ▼                         ▼
        screen preview (RGBA8)              thumbnail / standard preview     photo-export
        via photo-engine → photo-ui         JPEGs in <catalog>.previews/     (16-bit → file)
```

- **Import.** The engine walks a folder on a worker, `photo-decode` reads metadata and the embedded preview, `photo-xmp` reads any sidecar, and the engine inserts `NewPhoto` records through `photo-catalog` on its own thread.
- **Browsing.** The UI asks the engine for the current photo list (source, filter, sort); the engine queries the catalog and serves thumbnails from the preview cache, rendering missing ones in the background.
- **Developing.** A slider change becomes an engine call that updates the active photo's settings and posts a render request; the render worker always takes the newest request, and the result comes back on the next `poll()`. When the change is committed it is written to the catalog with a history step and an undo entry.
- **Export.** The engine queues a job; `photo-export` decodes the source, renders it through `photo-develop` in strips at the requested size and colour space, and encodes it.

## Technology map

| Layer | Technology | Version | Notes |
| --- | --- | --- | --- |
| Language | Rust | 1.95, edition 2024 | Pinned in `rust-toolchain.toml` |
| GUI | egui / eframe | 0.37 | wgpu renderer |
| UI testing | egui_kittest | 0.37 | AccessKit queries and headless wgpu rendering |
| Parallelism | rayon | 1.12 | |
| Catalog | rusqlite | 0.43 | Bundled SQLite |
| RAW decoding | rawler | 0.8 | LGPL-2.1 |
| Raster codecs | image | 0.25 | JPEG, PNG, TIFF, WebP, BMP, GIF |
| EXIF | kamadak-exif | 0.6 | |
| XML | quick-xml | 0.41 | XMP sidecars |
| File dialogs | rfd | 0.18 | |
| Platform paths | directories | 6.0 | Entry point only |
| Serialization | serde, serde_json | 1.x | Develop settings as JSON |
| Errors, logging | thiserror, log, env_logger | 2.x, 0.4, 0.11 | |

## See also

- [Architectural decisions](decisions.md)
- [Architecture drift log](drift-log.md)
- [Design](../threads/current/lightroom-workflow-restart/DESIGN.md) and [plan](../threads/current/lightroom-workflow-restart/PLAN.md) for the restart
