# Architecture

> Last verified: 2026-11-25 (v0.3.2)

## Section map

- [System overview](#system-overview): what the app is.
- [Component map](#component-map): one row per crate, what it owns and what it may depend on.
- [Boundaries](#boundaries): the one crate allowed to do each thing.
- [Data flow](#data-flow): import, browsing, developing, export, the agent harness.
- [Technology map](#technology-map): each library, its version and what it is for.
- [See also](#see-also): decisions, drift log, design.

## System overview

Photo is a catalog-based, non-destructive photo organiser and raw developer for macOS and Windows. A catalog (one SQLite file) records every imported photo, its metadata, how it is organised, and its edit as a set of parameters. Original files are never modified. Everything shown on screen or written by an export is derived from the original file plus those parameters by a single develop pipeline.

The workspace is layered so that the dependency graph is the architecture: shared data types at the bottom, independent engines above them (colour math, image primitives, decoding, catalog, XMP), the develop pipeline and export composing those, an engine that is the whole application without a user interface, and a thin egui layer on top.

At v0.3.2 both modules run end to end: the engine drives the develop pipeline for previews, thumbnails and the Develop canvas, and `photo-export` writes finished files. The same app runs without a window behind the agent harness, which is how it is driven and measured from scripts.

## Component map

| Crate | Path | Owns | May depend on | State |
| --- | --- | --- | --- | --- |
| `photo` | `src/main.rs`, `src/repo.rs`, `src/launch.rs`, `src/harness/`, `src/harnessctl.rs`, `src/bin/harnessctl.rs`, `src/lib.rs` | Entry point: finds the stores and launches the UI, in a window or behind the agent harness. `src/repo.rs` is the only code that decides where data lives (a checkout's root, else `Pictures/Photo`) and owns the harness's sandbox override. `src/harness/` is the JSON-lines server that drives `photo_ui::harness` and measures frames and renders; `harnessctl` is its client. | engine, ui, directories, eframe, egui, image, serde_json, env_logger, log | Working |
| `photo-core` | `crates/core` | Shared data types: `DevelopSettings` and the `Adj` table, `SETTINGS_FORMAT` (pinned to the JSON shape by a gate), `Turn` and `DevelopSettings::turn` (the one place a quarter turn or mirror is applied, crop included), settings groups, ids, rating/flag/label, `PhotoMetadata`, `DateTime`, `SourceImage`, colour-space constants and matrices. No I/O, no pixel loops. | serde | Done |
| `photo-color` | `crates/color` | Per-pixel colour science on `[f32; 3]`: white balance, base curves, Highlights and Shadows as a scene-linear gain in front of the base curve, tone, curves, colour mixer, grading, output transform. | core | Done |
| `photo-imaging` | `crates/imaging` | Buffer primitives: planes, mip pyramid, resampling, geometry mapping, filters, mask rasterization, healing, histogram. | core, rayon | Done |
| `photo-develop` | `crates/develop` | The pipeline: stage order, region and scale handling, stage caches, auto tone and auto white balance; the single source of the rotated, cropped output rectangle. | core, color, imaging, rayon | Done |
| `photo-decode` | `crates/decode` | Files in; `SourceImage`, `PhotoMetadata` and an embedded preview out. | core, image, rawler, kamadak-exif, rayon | Done |
| `photo-catalog` | `crates/catalog` | The SQLite catalog: schema 1, format numbers on stored settings and rules, restore records for every removal and deletion; `same_name_key`, the one comparison of file names (composed, then case-folded) that decides which originals an export protects. | core, rusqlite, serde, serde_json, icu_normalizer | Done, reviewed |
| `photo-xmp` | `crates/xmp` | XMP sidecar parsing and writing; never overwrites what it cannot read. | core, quick-xml, serde_json | Done, reviewed |
| `photo-export` | `crates/export` | Rendering through `photo-develop` in strips and encoding to JPEG, PNG and TIFF; sizing, name templates, ICC profiles, metadata, output sharpening, watermark. `place/` gives a finished file its name without ever replacing a file it has not identified: the source and the caller's protected files are recognised by file identity at the moment of replacement. | core, decode, develop, imaging, image, same-file, tempfile, serde, rayon | Done, reviewed |
| `photo-engine` | `crates/engine` | The application without a UI: open catalog, background jobs, preview service, undo, selection, organisation, import, sidecars; the develop session (canvas frames, history, snapshots, presets, copy and paste, Quick Develop, auto adjustments) and export jobs with their guard for the catalog's originals. | core, catalog, decode, xmp, imaging, develop, export, color (re-exported curve shapes only), image, same-file, serde_json | Working, reviewed |
| `photo-ui` | `crates/ui` | egui views, input mapping and the headless driver: the shell, the Library module, the Develop module with its panels and tools, the dialogs. No business logic. | core, engine, egui, eframe, egui_kittest, image, rfd | Both modules working |
| `photo-testkit` | `crates/testkit` | Dev-only synthetic fixtures, and an example that writes a folder of them for trying the app. | core, image, tempfile | Done |

## Boundaries

- Only `photo-develop` turns settings and source pixels into rendered pixels. Preview, thumbnail, histogram and export all go through it; there is no second implementation of any adjustment.
- Only `photo-catalog` speaks SQL. No SQL is ever built from user text.
- Only `photo-decode` reads image codecs and EXIF; only `photo-export` writes image files, and it never replaces a file it has not identified. The engine may use the `image` crate solely for its own JPEG preview-cache files; the harness uses it for the PNG artifacts it measures.
- A turn of a developed photo (a quarter turn or a mirror) is applied only by `DevelopSettings::turn` in `photo-core`, which carries the crop; no other code sets the orientation for a turn.
- An edit that is still open (a slider being dragged, a tool's unrecorded changes) is committed by the engine before any command that reaches that photo, and when the engine closes.
- Only `photo-xmp` reads and writes sidecars. It never writes into an original image file.
- Only `src/repo.rs`, called from `src/main.rs`, resolves a default storage location. Every library function takes explicit paths, so tests cannot reach real user data.
- The agent harness acts only through the app's own input (pointer, keys, text on controls found by accessible name) and reads only through read-only accessors; its storage is sandboxed in the run directory unless asked otherwise.
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
- **Developing.** A slider change becomes an engine call that updates the active photo's settings and posts a render request; the render worker always takes the newest request, and the result comes back on the next `poll()`. When the change is committed it is written to the catalog with a history step and an undo entry. A tool (crop, healing, a mask) keeps its changes as one open edit until it is closed.
- **Export.** The engine queues one job per photo, gives each the catalogued originals that share its output's file name, and refuses names that belong to the catalog's originals; `photo-export` renders through `photo-develop` in strips at the requested size and colour space, encodes into a hidden temporary file in the destination folder, and gives it its name by a move that cannot replace.
- **Agent harness.** `photo --harness` runs the same `PhotoApp` offscreen and answers JSON-line commands on localhost: gestures in, and state, screenshots, full-resolution renders and their statistics out. See [the guide](../guides/agent-harness.md).

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
| File identity | same-file | 1.0 | Export placement and the engine's export guard: is this the same file, whatever its name |
| Unicode normalization | icu_normalizer | 2.4 | Composing file names before comparing them, as APFS does; already in the tree through `url` |
| No-replace moves | tempfile | 3.27 | `persist_noclobber` in export placement; also test directories |
| Platform paths | directories | 6.0 | Entry point only |
| Serialization | serde, serde_json | 1.x | Develop settings as JSON |
| Errors, logging | thiserror, log, env_logger | 2.x, 0.4, 0.11 | |

## See also

- [Architectural decisions](decisions.md)
- [Architecture drift log](drift-log.md)
- [Plan](../work/12_lightroom-workflow-restart/plan.md) and [design](../work/12_lightroom-workflow-restart/design.md) for the restart
