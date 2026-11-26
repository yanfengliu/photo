# Changelog

One entry per version, newest first. Versions through 0.2.10 belong to the previous app and are in git history before commit `94bd2cf`.

## 0.3.2 — 2026-11-25

The Develop module and export: Photo now edits photos and writes finished files. A catalog made by 0.3.1 opens as it is; nothing is migrated. Run `cargo run --release` from a checkout, or `cargo run --release -- path/to/photo` to start with one photo imported and open.

- **Develop.** Press D on a photo. Nine panels: Basic (colour or black and white, profile, white balance with a picker and Auto, exposure, contrast, highlights, shadows, whites, blacks, texture, clarity, dehaze, vibrance, saturation, and Auto tone), Tone Curve (parametric, and point curves for RGB and each channel), Color Mixer, Color Grading, Detail (sharpening and noise reduction), Lens Corrections, Transform, Effects (vignette and grain) and Calibration. Double-click a slider to put it back.
- **Tools.** Crop and straighten (R), with fixed shapes, a lock, quarter turns and mirrors, and a line you draw along what should be level. Healing and cloning spots (Q). Linear (M) and radial (Shift+M) gradients and a brush (K), each mask with its own adjustments, a red overlay (O), and shapes that add to it or cut out of it. Enter records what a tool did as one step; Escape drops it.
- **Seeing the edit.** Fit, fill, 1:1, 2:1 and 4:1; before and after as a toggle, side by side or split; clipping warnings; a histogram of what is shown.
- **History, snapshots, presets.** Every change is a step in the History panel and can be undone and redone. A snapshot keeps the settings by name. Six looks come with the app and you can save your own; pointing at one previews it. Copy and Paste carry the groups of settings you tick from one photo to another.
- **Quick Develop** in the Library changes white balance and tone for the selected photos without leaving the grid.
- **Export** (Shift+Cmd+E, from either module). JPEG, PNG or TIFF, 8 or 16 bits where the format has them; sRGB, Display P3, Adobe RGB (1998), ProPhoto RGB or Rec. 2020, with the profile embedded; full size, or fitted by long edge, short edge, a box, megapixels or percent; names from a template with `{name}`, `{seq}`, `{date}` and `{text}`; a subfolder; sharpening for the screen; all metadata, copyright and contact only, or none, with location removed if you ask; a text watermark. Settings can be saved as export presets. When a file of that name exists you choose: give the new file another name, skip it, or replace it.
- **Originals are never written over.** An export refuses to replace the photo it is made from or any other photo in the catalog, whatever name, link or letter case reaches that file, and the other two choices never replace any file at all. This holds on drives without hard links (FAT, exFAT, many network shares), where a file can be seen growing under its final name while it is written.
- **Nothing needs saving.** A change still in progress (a tool left open, a slider mid-drag) is kept when you quit, export, make a virtual copy, undo, or remove the photo.
- **For scripts and agents.** `photo --harness` runs the app without a window and `harnessctl` drives it like a user and measures what it shows; see `docs/guides/agent-harness.md`.

Known limits. This version has been run on synthetic photos only, a synthetic RAW among them; camera RAW files get a fixed +0.85 EV before the base curve, a figure carried over from the previous app's calibration against Sony files, and no real camera's files have been through this version. Turning a photo that has a Transform correction changes what the correction acts on. Starting an export looks at every folder in the catalog first, and waits if one is on a drive that hangs. There are no lens profiles, and no soft proofing.

Validation: the four gates pass on the merged tree (1,844 tests). The app was driven end to end through the harness on six synthetic photos: import, rating and filtering, every Develop panel, the three tools, undo, a snapshot, a preset, copy and paste, and export; a PNG export matched the on-screen render to within 1 level of 255, and the originals' hashes were unchanged. Export and the engine had three rounds of independent review by one model, recorded in `docs/work/12_lightroom-workflow-restart/reviews/`; a second model's review of them did not run for lack of quota. Nothing was run on Windows beyond CI.

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
