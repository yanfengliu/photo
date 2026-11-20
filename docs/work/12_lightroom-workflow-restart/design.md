# Lightroom-workflow restart — Design

Date: 2026-11-19. Objective: rebuild Photo from an empty repository as a catalog-based, non-destructive photo organiser and raw developer whose workflow a Lightroom Classic user already knows — Library and Develop modules, parametric edits, history, presets, export — on macOS and Windows.

This document is the contract between the crates. Where it gives a formula it is a starting point the owning crate may improve, provided the listed contracts still hold and are tested. Where it gives a boundary it is not negotiable without updating this file.

## 1. Scope

In scope for this objective: a catalog (import, folders, collections, smart collections, keywords, ratings, flags, colour labels, virtual copies, filtering, sorting), the Library views (grid, loupe, compare, survey), the Develop module (the full panel set listed in §8, crop and straighten, healing spots, linear/radial/brush masks, before/after, history, snapshots, presets, copy/paste/sync), export (JPEG/PNG/TIFF, resize, colour space with embedded ICC profile, naming templates, output sharpening, watermark), XMP sidecars, and RAW decoding.

Out of scope for now, stated so nobody mistakes absence for a bug: the Map, Book, Slideshow, Print and Web modules; tethered capture; cloud sync; AI masks (subject, sky, people); profile-based lens corrections from a lens database; camera-matching DCP profiles; HDR and panorama merge; content-aware remove; soft proofing; stacks; a second-monitor window; video.

Not a goal at all: pixel-identical output to Adobe's renderer. Slider names, ranges and directions follow the conventions photographers already know, and `crs:` values in XMP map one to one, but the rendering math is our own and is documented with its real sources (§5). Per the carried-over local rule, nothing in code, commits or docs may call a tuning "Lightroom-style"; cite darktable, the DNG SDK or a paper.

Identity: the app is called Photo. It uses no Adobe names, logos, icons or artwork, and its visual design is its own (§8.8).

## 2. Why a restart

The previous app (iced 0.13 GPU viewer, v0.2.10, preserved at commit `94bd2cf`) was judged not to be on the path to this design. Three of its load-bearing contracts are the opposite of what a parametric raw developer needs, and most of its 24 k lines exist to serve them:

- Its edit source is 8-bit sRGB for every format, RAW included (`decode::ImageData.pixels: Vec<u8>`, `Rgba8UnormSrgb` textures, default tone baked into the cached pixels). Highlight recovery and multi-stop exposure moves have no data to work with.
- It persists baked pixels with session-only undo (`local_edits.rs`, by explicit product decision). Here the parametric settings are the source of truth and pixels are always derived.
- Its adjustment math exists twice, in WGSL for preview and in Rust for save, kept in parity by hand across three layers. Here there is one pipeline.

What carries over is knowledge, recorded in §10 and §5: the perceptual-space tone-control rules, the RAW default-tone calibration, the monotonicity contract, embedded-preview-first loading, the test-isolation lessons, and the idea of an agent harness.

## 3. Principles

1. **Settings are the edit.** The original file is never written to. A photo's look is `DevelopSettings` (JSON in the catalog). Every pixel on screen or in an export is derived from the source file plus those settings.
2. **One pipeline.** `photo-develop` is the only owner of "settings + source in, pixels out". Preview, thumbnail, histogram and export all call it. There is no second implementation to keep in parity.
3. **Linear, high-precision, scene-first.** Sources are decoded to linear light (`u16`, 65535 = 1.0) and the pipeline computes in `f32` linear Rec. 2020. RAW data stays camera-native and un-white-balanced until render time.
4. **Resolution independence.** Every spatial parameter is defined at full resolution and scaled by the view scale, and every position-dependent effect is defined in image coordinates. A preview is the export seen through a smaller window (contract R1–R3 in §5.9).
5. **Boundaries are crates.** Each responsibility in §4 is a crate; the dependency graph is the architecture. A crate that needs something outside its row asks for the boundary to change here first.
6. **Agent-verifiable.** Every behaviour is reachable headlessly: engine behaviour through plain method calls, UI behaviour through `egui_kittest` (accessibility-tree queries plus real rendered frames).
7. **Tests cannot touch real data.** No library code computes a default storage location. Only the root package does, in `src/repo.rs`, which owns storage-root discovery and the harness's sandbox override as `AGENTS.md` requires; everything else is handed explicit paths, and tests hand it temp directories.

## 4. Stack and crate map

Rust 1.95, edition 2024, `unsafe_code = "forbid"` workspace-wide. GUI: egui/eframe 0.37 on wgpu. Parallelism: rayon. Storage: SQLite via rusqlite (bundled). RAW: rawler 0.8 (LGPL-2.1). Raster codecs: image 0.25. EXIF: kamadak-exif. XML: quick-xml. File dialogs: rfd.

| Crate | Path | Owns | May depend on |
| --- | --- | --- | --- |
| `photo-core` | `crates/core` | Shared data types: `DevelopSettings` and the `Adj` table, ids, rating/flag/label, `PhotoMetadata`, `SourceImage`, `DateTime`, colour-space constants and matrices. No I/O, no pixel loops. | serde |
| `photo-color` | `crates/color` | Per-pixel colour science on `[f32; 3]`: white balance, base curves, tone, curves, colour mixer, grading, output transform. No buffers, no threads. | core |
| `photo-imaging` | `crates/imaging` | Buffer primitives: planes, mip pyramid, resampling, geometry mapping, blur/guided/wavelet filters, mask rasterization, healing, histogram, grain noise. Knows nothing about pipeline order or colour operators. | core, rayon |
| `photo-develop` | `crates/develop` | The pipeline: stage order, region/scale handling, stage caches, auto tone and auto white balance. | core, color, imaging, rayon |
| `photo-decode` | `crates/decode` | Files in, `SourceImage` + `PhotoMetadata` + embedded preview out. The only reader of image codecs and EXIF. | core, image, rawler, kamadak-exif, rayon |
| `photo-catalog` | `crates/catalog` | The SQLite catalog. The only crate that speaks SQL. | core, rusqlite, serde_json |
| `photo-xmp` | `crates/xmp` | XMP sidecar parse and write. | core, quick-xml |
| `photo-export` | `crates/export` | Render through `photo-develop` and encode to disk: sizing, naming, ICC, metadata, watermark. The only writer of image files. | core, decode, develop, image |
| `photo-engine` | `crates/engine` | The application without a UI: open catalog, background jobs, preview service, undo, selection, clipboard, presets. | all of the above |
| `photo-ui` | `crates/ui` | egui views and input mapping. Holds no business logic; calls the engine. | core, engine, egui, eframe, rfd |
| `photo-testkit` | `crates/testkit` | Dev-only synthetic fixtures (images, a synthetic DNG). | core, image, tempfile |
| `photo` | `src/main.rs` | Entry point: CLI parsing, default catalog path, logging, launching the UI. | ui, engine, directories |

Rules that follow from the table: `photo-ui` never imports `photo-catalog`, `photo-develop`, `photo-decode` or `photo-export` directly; `photo-color` and `photo-imaging` never import each other; only `photo-decode` and `photo-export` touch the `image` crate's codecs (the engine may use `image` only to encode and decode its own JPEG preview cache files).

## 5. The develop pipeline

### 5.1 Frames and the view

Frames of reference are defined in `photo_core::develop::geometry`: source → corrected (lens distortion) → oriented (quarter turns, mirror) → transformed (perspective) → output (crop). Spots and masks are stored in normalized **source** coordinates so they stay attached to content.

`photo_imaging::Geometry` is built from the source size and the lens, orientation, transform and crop settings. It answers `output_size()` (full-resolution output pixels), `out_to_src(p)` and `src_to_out(p)`. A `View { roi, width, height }` asks for the output-space rectangle `roi` rendered into `width × height` pixels; `scale = width / roi.width()`. Fit-to-window, 1:1 tiles, thumbnails and export strips are all just views.

### 5.2 Stage order

| # | Stage | Kind | Notes |
| --- | --- | --- | --- |
| 1 | Heal | source space | Applied to the pyramid level being sampled; cached until spots change. |
| 2 | Resample | geometry | Inverse-map each output pixel to the source and sample the mip level whose scale is nearest above the view scale. Output: padded region, source-native linear `f32`. |
| 3 | Input | point | Camera: white-balance gains → highlight reconstruction → camera-to-working matrix. Raster: source-space-to-working matrix → relative white balance. Lens vignetting correction (position-dependent gain) is applied here. |
| 4 | Denoise | spatial | Luminance and chroma noise reduction, before any tone change so thresholds do not depend on exposure. |
| — | *cache A* | | Result of 1–4, keyed by source, view, spots, geometry, white balance, lens vignetting and noise settings. Moving a tone or colour slider reuses it. |
| 5 | Exposure, highlights and shadows, base curve | point + one spatial | Scene-linear exposure gain; Highlights and Shadows as a further scene-linear gain looked up from an edge-aware base (§5.5); then the profile's base curve (RAW) or the gain-aware shoulder (raster). Output is display-linear, nominally 0 to 1 but not clamped. The base plane depends on exposure and profile. |
| 6 | Tone | point | Whites and blacks; contrast; tone curves. |
| 7 | Colour | point | Calibration, colour mixer (HSL) or black-and-white mix, vibrance, saturation, colour grading. |
| 8 | Presence | spatial | Dehaze, clarity, texture, on perceptual luminance. |
| 9 | Sharpen | spatial | Luminance unsharp mask with detail threshold and edge masking. |
| 10 | Effects | point | Post-crop vignette, grain. Defined in output coordinates. |
| 11 | Output | point | Working to output space, gamut compression, transfer encode, quantize (8-bit for screen, 16-bit for export). |

Masks are not a stage. Each enabled mask is rasterized once per view into an alpha plane, and the per-pixel effective value of every mask-adjustable parameter is `global + Σ alphaᵢ · amountᵢ · localᵢ`. Stages 3 and 5–9 read those effective values. Two overlapping +1 EV masks therefore add up to +2 EV, and a mask with no adjustments costs nothing.

### 5.3 Input

Camera sources. Let `A = xyz_to_camera`. The camera's response to D65 white is `w = A · XYZ(D65)`, and `A' = diag(1/w) · A` maps D65 XYZ to (1, 1, 1). White-balanced camera RGB goes to the working space through `M = XYZ→Rec2020 · A'⁻¹`. White balance gains for an illuminant of chromaticity `xy` are `g = 1 / (A · XYZ(xy))`, normalized so green is 1; the as-shot temperature and tint are found by inverting that relation for the file's as-shot gains. Temperature and tint map to `xy` exactly as in the DNG SDK's `dng_temperature` (Robertson's isotherm method; tint is `-3000 × Δuv` along the isotherm). Cross-check the isotherm table against Kim et al.'s cubic approximation of the Planckian locus in a test, so a mistyped table entry cannot hide.

Highlight reconstruction: sensor clipping happens at 1.0 per channel before white balance, so after gains a clipped pixel is tinted. Let `c = max(camera RGB before gains)`. Blend the white-balanced pixel toward its own maximum channel (neutral) by `smoothstep(0.90, 1.0, c)`. Channels above 1.0 after gains are kept, not clipped: that headroom is what the Highlights slider recovers.

Raster sources. Convert with `rgb_to_rgb(space, Rec2020)`. Relative white balance: `temp` ±100 is ∓90 mired around 6500 K and `tint` ±100 is ∓0.033 Δuv; the image is adapted from that implied illuminant to D65 with Bradford, and results are clamped to non-negative (tungsten-range adaptations can push a channel below zero, which silently breaks every later luminance-based stage). `temp = tint = 0` must be exact identity, not approximately.

### 5.4 Exposure and base rendering

Exposure is a scene-linear gain `2^EV`.

RAW (profile ≠ Linear): gain `2^(EV + bias)` with `bias = 0.85 + baseline_exposure`, then a filmic base curve applied with the hue-preserving RGB tone method from the DNG SDK (`RefBaselineRGBTone`: curve the largest and smallest channel, interpolate the middle one so its relative position is preserved). The curve maps scene 0 → 0, middle grey 0.18 → about 0.18–0.20, has a toe and an asymptotic shoulder that reaches ≈0.99 at the sensor clip point, and never exceeds 1. The +0.85 EV bias is carried over from the previous app's calibration against the embedded camera previews of 35 Sony ARWs; the curve shape is our own and should be tuned by rendering, not asserted. Profiles differ only in curve contrast and a base saturation factor: Standard, Neutral (flatter), Vivid (stronger). Linear applies gain only, with a soft clip.

Raster: the file is already a rendering, so at `EV = 0` this stage is exact identity. For `EV > 0` apply the gain then a shoulder that is identity below a knee `k(g)` and lands smoothly on 1.0 for the brightest possible input, with `k → 1` as `g → 1` so the response is continuous and monotonic in both pixel value and EV. For `EV < 0` it is a pure gain (whites go grey, as photographers expect).

### 5.5 Tone

All curves in this stage are defined on perceptual values `v = Y^(1/2.2)` with the pivot at `v = 0.5`, per the carried-over rule: photographer-facing tone controls act in perceptual space, because a pivot at linear 0.5 sits at L\* ≈ 76 and feels wrong.

Zone centres: Blacks act near black only; Shadows is a bell around `v ≈ 0.32`; Highlights a bell around `v ≈ 0.72`; Whites near white only. Each control is its own monotonic curve and the controls are composed, never summed as offsets, so any combination is monotonic by construction.

Highlights and Shadows are applied in front of the base curve, as a scene-linear gain looked up from an edge-aware base, so that RAW headroom above nominal white is recovered rather than darkened after the curve has already compressed it. Let `c` be the scalar scene-to-display curve of stage 5, exposure included, and `Z` the perceptual zone curve. The base is `b = guided_filter(t)`, where `t` is the perceptual display value the input stage's output would have (`PixelPipeline::tone_base`, exact for neutrals), and the gain is `k(b) = c⁻¹(decode(Z(b))) / c⁻¹(decode(b))`, applied to the scene-linear pixel before `c`. A flat neutral region therefore lands exactly at `decode(Z(b))`, while its texture is rendered through the curve at the new operating point; where `c` is the identity (a raster source at exposure 0) this reduces to a gain applied after it. Because the gain varies slowly across regions, local contrast survives the full ±100 range; this is the structure of darktable's tone equalizer (`toneequal.c`) with the guided filter of He, Sun and Tang (2010). With the filter radius at zero it degenerates to a plain global curve, which is how the curve is unit-tested.

As built, the pipeline filters `log2` of luminance rather than `t`: in `t` the recovered headroom sits just below 1 and the filter left a bright halo along the edges of recovered highlights, which a test now pins. The guided-filter coefficients for this base, for dehaze's transmission and for clarity's local mean are fitted once on the 512-pixel whole-image reduction (He and Sun's fast guided filter on that grid) and applied per pixel, so they need no padding and every tile is bit-identical to the matching crop of a full render. Measured through the whole pipeline, the six-slider recovery recipe still re-clips the brightest of two stops of headroom (texture contrast 0.0055 at default against 0.0006); that is a tuning item for real RAW files through the harness.

The zone shapes reach the top end: for RAW, Highlights −100 returns to white as a cube root, which compresses the headroom into the visible range; raster uses a gentler parabola so that 8-bit sources do not band. Measured on a camera ramp carrying texture through the two stops above nominal white, Highlights −100 multiplies that texture's contrast by about 65 against default, where the earlier after-the-curve method gave 0.88. A full Highlights or Shadows slider moves the other zone's peak by nothing and middle grey by 4 to 34 % of its own peak, inside the σ = 1 EV band of the owner's tone-band rule (`docs/policies/local-rules.md`), which a test pins.

Whites and Blacks move the end points by a luminance gain on the pixel's own value, after the base curve. Whites > 0 pushes highlights into clipping, Whites < 0 dulls them; Blacks < 0 crushes, Blacks > 0 lifts. Whites stays a point operation: a point operation renders the same before or after the curve, and a scene-linear Whites + could never reach clipping through an asymptotic curve.

Contrast is the rational sigmoid `u' = u(1+a) / (1 + a|u|)` on `u = 2v − 1`, `1 + a = 2^(contrast/100)`: slope `1+a` at the pivot, fixed end points, bounded end slopes in both directions. Contrast, the parametric curve and the RGB point curve are composed into one 1D LUT in the perceptual domain and applied with the hue-preserving RGB tone method. Point curves interpolate with Fritsch–Carlson monotone cubics. The per-channel red, green and blue point curves are applied afterwards, per channel.

Contracts (each needs a test): T1 neutral settings are identity; T2 the composite luminance response is strictly increasing for every combination of the six tone sliders at their extremes; T3 each slider moves its own zone's peak more than any other zone's; T4 the sample pixel for a zone test sits inside that zone's peak, never on a boundary; T5 no finite input produces NaN or a negative value.

### 5.6 Colour

Hue-selective work is done in Oklab (Ottosson, 2020), reached from linear working RGB through XYZ. One round trip serves the whole stage.

Colour mixer: eight bands with smooth, partition-of-unity weights over Oklab hue, centred on the hues of the sRGB reference colours (red, orange, yellow, green, aqua, blue, purple, magenta) and scaled by chroma so greys are untouched. Hue ±100 moves toward the neighbouring band; Saturation scales chroma; Luminance shifts lightness. Black and white: lightness shifted by the chroma-weighted `bw_mix`, then chroma set to zero. Saturation −100 must be exactly greyscale. Vibrance scales chroma less the more saturated a colour already is, with protection around skin hues. Colour grading adds a tint and lightness offset weighted by luminance-range masks whose overlap is `blending` and whose split moves with `balance`. Calibration rotates and scales the working primaries and tints the shadows.

### 5.7 Spatial stages

Radii are full-resolution pixels multiplied by the view scale. A stage whose scaled radius falls below half a pixel is skipped, which is what makes draft previews cheap.

Denoise: an à-trous wavelet decomposition (Starck and Murtagh) with soft thresholding on a variance-stabilized (square-root) encoding, luminance and chroma thresholds set separately. Clarity: `v + amount · (v − guided(v, r≈1% of the long edge)) · midtone_weight(v)`. Texture: the same on a band between two smaller radii. Dehaze: dark-channel prior (He, Sun and Tang, 2009) with the atmospheric light taken from a whole-image analysis so every tile agrees; negative values add haze. Sharpen: unsharp mask on luminance with `radius`, a `detail` threshold that suppresses halos, and a gradient-magnitude `masking` weight. Grain: band-limited noise that is a pure function of output coordinates and a fixed seed. Vignette: an elliptical falloff in output coordinates with `midpoint`, `roundness` and `feather`.

### 5.8 Output

Convert working RGB to the output space. Out-of-gamut colours are brought in by desaturating toward their own luminance until no channel is negative, then clipping at 1; this preserves hue and lightness where a per-channel clip would shift both. Encode with the output space's transfer function. The on-screen path quantizes to 8 bits through a lookup table; export keeps 16.

### 5.9 Resolution and region contracts

- R1: rendering a sub-rectangle gives the same pixels as cropping a render of the whole image (within 1/255), for every stage. Spatial stages therefore pad their input by their own support.
- R2: rendering at half scale approximates the full-resolution render downsampled 2× (within a small tolerance), for every stage whose radius survives scaling.
- R3: whole-image statistics (dehaze atmospheric light, auto tone, auto white balance) come from one analysis of a fixed-size reduction and are passed into every render, never recomputed per tile.
- R4: rendering is deterministic: same inputs, same bytes, regardless of thread count.

### 5.10 Performance budget

On an 8-core Apple Silicon machine, a 2.5 MP view with cache A warm should render in under 60 ms with typical settings and under 120 ms with every stage active. The UI never waits for a render: it shows the last frame, asks for a draft at reduced scale while a slider moves, and a full-quality frame once input settles.

## 6. Storage

### 6.1 Sources

`photo-decode` returns a `SourceImage`: interleaved linear RGB `u16`, upright per EXIF orientation. Raster files are linearized with their colour space's transfer function (sRGB assumed unless an embedded ICC profile's primaries match Display P3, Adobe RGB or ProPhoto). RAW files are demosaiced, cropped to the recommended area, and stored camera-native with the as-shot gains divided back out. It also returns `PhotoMetadata` and, when the file has one, the embedded JPEG preview so the UI can show something within milliseconds while the real decode runs.

### 6.2 Catalog

A catalog is one SQLite file (`*.photocat`) with a sibling `<name>.previews/` directory. WAL mode, foreign keys on, migrations keyed on `PRAGMA user_version`; opening a catalog written by a newer schema is refused rather than risked. Develop settings are stored as JSON text; `develop_version` increments on every settings change and is the preview-cache key. History steps, snapshots and presets store full `DevelopSettings` JSON. Smart collections store rules as JSON and are compiled to parameterized SQL (field names come from an enum, never from user text).

Tables: `photos`, `folders`, `collections`, `collection_photos`, `keywords`, `photo_keywords`, `history`, `snapshots`, `presets`, `imports`, `settings`. A virtual copy is a `photos` row whose `master_id` points at the row that owns the file.

### 6.3 XMP

RAW files use the `basename.xmp` convention so existing sidecars are found; raster files use `filename.ext.xmp` because we never write into an original. Reading maps `xmp:Rating`, `xmp:Label`, `dc:subject`, `lr:hierarchicalSubject`, `dc:title`, `dc:description`, `dc:creator`, `dc:rights` and the `crs:` develop properties into our types. Writing updates a small DOM of the existing sidecar, so properties we do not understand survive. Our complete settings are also written losslessly as JSON in a private `photo:` namespace; on read that wins over the `crs:` mapping when present.

## 7. Engine

`photo-engine` exposes one `Engine` that the UI drives with plain method calls and polls once per frame. It owns the catalog (on the calling thread), a worker pool for import, thumbnails and export, and one render worker that always works on the newest request. `Engine::poll()` drains finished background work into state; `Engine::is_idle()` and `Engine::wait_idle(timeout)` exist so tests and the harness never sleep-and-hope.

State it owns: the current source (folder, collection, …), filter and sort; the resulting ordered photo list; the selection (active photo plus selected set); the develop session for the active photo (settings, history, as-shot white balance, source pyramid); the settings clipboard; the undo and redo stacks (every catalog mutation is a reversible command: develop changes, ratings, flags, labels, keywords, collection membership, metadata text); background task progress; and a toast queue for the brief on-screen confirmations ("Set Rating to 3").

Previews: thumbnails (≤ 384 px) and standard previews (≤ 2048 px) are JPEG files under the catalog's previews directory, named by photo id and `develop_version`, rendered by the develop pipeline. A fresh import shows the embedded camera preview until the first real render replaces it.

## 8. User interface

### 8.1 Window layout

Top: a slim bar with the app name at left and the module picker (Library, Develop) at right. Left and right: collapsible panel columns whose contents depend on the module. Centre: the work area with a toolbar underneath it. Bottom: the filmstrip with a source and filter line above it. Each of the four edges collapses independently; Tab toggles the side columns, Shift+Tab all four.

### 8.2 Library

Left panels: Navigator, Catalog (All Photographs, Quick Collection, Previous Import), Folders (tree with counts), Collections (sets, collections, smart collections; create, rename, delete, drag photos in). Import and Export buttons at the bottom.

Centre: the Library Filter bar (Text, Attribute, Metadata, None) above one of four views — Grid (virtualized cells with thumbnail, rating, flag, label, badges; thumbnail-size slider), Loupe (fit / fill / 1:1, pan), Compare (select and candidate side by side) and Survey (the selected photos tiled). The toolbar holds the view buttons, sort order, rating/flag/label controls and the thumbnail slider.

Right panels: Histogram, Quick Develop, Keywording, Keyword List, Metadata (file, capture and camera fields; editable title, caption, copyright, creator).

### 8.3 Develop

Left panels: Navigator (with preset hover preview), Presets, Snapshots, History, Collections. Copy and Paste buttons at the bottom.

Centre: the photo canvas (fit, fill, 1:1 and deeper zoom; drag to pan; before/after in toggle, side-by-side and split modes) with a toolbar for view mode, zoom and tool options.

Right panels: Histogram (with shadow and highlight clipping toggles), the tool strip (Crop, Healing, Masking), then Basic, Tone Curve, Color Mixer, Color Grading, Detail, Lens Corrections, Transform, Effects, Calibration. Previous and Reset buttons at the bottom. Every slider is built from the `Adj` table: double-click resets it, the value is editable as text, and arrow keys nudge by `step`.

Tools: Crop (drag handles, drag to move, aspect presets and lock, straighten slider and auto-shrink so the rectangle always fits, overlay guides, rotate and flip), Healing (click to place, drag to size, drag the source, heal/clone, feather, opacity), Masking (add linear, radial or brush; a mask list with show/hide, invert, rename, delete; the local adjustment sliders; red overlay toggle).

### 8.4 Dialogs

Import (source folder, include subfolders, add in place or copy to a destination organised by date, skip suspected duplicates, keywords and preset to apply, a checkable preview grid). Export (location, existing-file policy, naming template, format and quality, colour space, sizing, output sharpening, metadata, watermark, saved presets). Copy Settings (the `SettingsGroup` checklist). New preset, new snapshot, new collection, new smart collection (rule editor).

### 8.5 Keyboard

The shortcuts follow the conventions a Lightroom Classic user has in their hands: G grid, E loupe, C compare, N survey, D develop; 0–5 rating, `[` `]` rating down/up; P pick, X reject, U unflag; 6–9 colour labels; B quick collection; arrow keys to navigate; Space or Z zoom toggle; Tab / Shift+Tab panels; T toolbar; F full screen; L lights out; R crop; Q healing; M linear gradient; Shift+M radial; K brush; W white-balance picker; J clipping; `\` before/after; Y side by side; ⌘Z / ⇧⌘Z undo/redo; ⇧⌘C / ⇧⌘V copy/paste settings; ⇧⌘R reset; ⌘' virtual copy; ⌘N new collection (Library) or snapshot (Develop); ⇧⌘I import; ⇧⌘E export; ⌘A / ⌘D select all/none; ⌘[ / ⌘] rotate; Delete remove; ⌘F focus the text filter; Enter and Escape to commit and cancel a tool. ⌘ is Ctrl on Windows.

### 8.6 UI architecture

`photo-ui` is a thin layer: `PhotoApp { engine, ui_state }` implements `eframe::App`. `ui_state` holds only presentation state (which panels are open, zoom and pan, the active tool's in-progress drag, dialog fields). Every user action becomes an engine call. Textures are owned by a small cache keyed by photo id and `develop_version`.

Every interactive widget must have an accessible name. `egui_kittest` panics on unnamed inputs, and the name is how tests and the harness find a control; icon-only buttons get an explicit label, sliders are named by their `AdjSpec` (prefixed by panel where a label repeats).

### 8.7 Harness

There are two layers, and the second is the contract `AGENTS.md` states.

The in-process driver, `photo_ui::harness`, is built on `egui_kittest`: create an app on a temp catalog, press keys, click or set controls by name, drag at coordinates, wait until the engine is idle, and save the rendered frame as a PNG. UI tests use it directly.

The agent harness runs that same driver behind a process boundary. `photo --harness[=PORT]` starts the real app without a window (real GPU frames, rendered offscreen) and serves JSON lines over localhost TCP; `harnessctl <cmd> [params-json]` sends one command and `harnessctl script <file.jsonl>` replays a recorded session. Each launch creates `tmp/harness-runs/<run-id>/` holding `session.json` (port and token), `manifest.json`, `session.jsonl` (every request and response) and `artifacts/`. Storage is sandboxed by default: the catalog, its previews and the export folder resolve into `<run-dir>/storage/`, and only `--harness-real-storage` reaches the user's own catalog.

Commands, following the previous app's guide so an agent that knew it is at home: `ping`, `observe` (module, view, selection, the active photo, every control with its kind, label, range and value, pending work), `observe_library` (a page of the current photo list), `screenshot` (the frame the user would see), `dump_render` (the develop pipeline's output for the active photo, with statistics computed at full resolution), `image_stats`, `compare_images`, `import_files`, `import_folder`, `open`, `set_slider` (a real drag: change events then a release), `reset_slider`, `click`, `key`, `drag`, `set_crop`, `wait_idle`, `quit`. An action response means the gesture was performed; `wait_idle` is how a caller learns that the work it started has finished. The guide, with the statistics semantics, the improvement loop and the findings schema, is `docs/guides/agent-harness.md`.

### 8.8 Visual design

A neutral dark interface so it does not bias colour judgement: near-black work area, mid-dark panels, one restrained accent colour for selection and active controls, no saturated chrome. Compact slim sliders with a label on the left and an editable value on the right, a centre tick for signed ranges, and a filled track from the default to the current value. Panel headers are small caps with a disclosure triangle. Icons are drawn as vector strokes in code. Photo cells sit on a slightly lighter mat; the selected cell is lighter still and the active cell lightest.

## 9. Testing

Tests pin contracts, not implementations: pipeline contracts T1–T5 and R1–R4 and the per-operator direction and neutrality rules in `photo-color`, `photo-imaging` and `photo-develop`; catalog behaviour through its public API on in-memory and temp-file databases; XMP against hand-written sidecars in the form other applications write plus round trips of our own; decode against files the testkit synthesizes (including a Bayer DNG); export by decoding what was written; engine scenarios end to end (import a folder, rate, edit, undo, export); UI wiring through the harness. Rendering tests that need a GPU adapter skip cleanly when none exists.

A process-wide test override is guarded by a mutex held across the whole override window, or is thread-local when only the calling thread reads it. After any `cargo fix`, re-verify with `cargo check --all-targets`.

## 10. Rules carried over from the previous app

These were the owner's standing local rules and still apply; `docs/policies/local-rules.md` keeps them in the owner's words, and this list only says where each one lands in the design.

- The tone bands are deliberately tight: a single full-strength Highlights or Shadows slider must read as targeted recovery, not as a global exposure shift. The previous app chose σ = 1 with 1.5 EV of reach per band against darktable's σ = √2 for that reason.
- Undo and redo are session-scoped. The undo stack lives in the engine and is not persisted; what persists is each photo's settings and its history steps.

- Do not describe a tuning as "Lightroom-style" without a published Adobe reference. The DNG specification, the DNG SDK and the XMP specification are published Adobe references; the Basic-panel math is not. Cite what was actually used.
- Photographer-facing contrast and tone controls operate in perceptual space with the pivot at perceptual 0.5; zone centres as in §5.5.
- When a tone-zone test picks a sample pixel, put it inside the zone's peak.
- Watch for regime cliffs where an intermediate RGB value can go negative (chromatic adaptation at tungsten temperatures is the known one); clamp at the source of the cliff.
- Full-quality RAW decoding is expensive: show the embedded preview first and keep the user's zoom and pan across the swap.
