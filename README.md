# photo

A GPU-accelerated image viewer for Windows, built with Rust, [iced](https://github.com/iced-rs/iced), and wgpu.

Supports JPEG, PNG, GIF, BMP, TIFF, WebP, ICO, TGA, QOI, HDR, EXR, SVG, and common camera RAW formats including DNG, CR2/CR3, NEF, ARW, RAF, RW2, ORF, and more.

## Features

- **Library tab** - Browse image collections as a scrollable thumbnail grid. Load images via folder picker or file picker.
- **Collections** - Group photos into named collections with drag-and-drop and a sidebar; persisted to `%LOCALAPPDATA%/photo/collections.json`.
- **Detail tab** - View individual images with GPU-rendered zoom and pan, plus 90-degree rotation and freeform/square crop with overlay preview.
- **Real-time editing** - 12 GPU-shader adjustments (exposure, contrast, highlights, shadows, whites, blacks, temperature, tint, vibrance, saturation, clarity, dehaze) plus Lensfun-based lens distortion, vignetting, and TCA correction. Undo/redo within a session; committed edits bake into repo-local files under `local-edits/` so reopening preserves the edited image. Save-as-copy exports a separate edited file.
- **RAW support** - Camera RAW files load fast via embedded preview, then upgrade to a fully developed image in the background.
- **Keyboard navigation** - Arrow keys to cycle through images.
- **CLI support** - Open a file directly: `photo.exe path/to/image.jpg`

## Prerequisites

- **Rust toolchain** - Install via [rustup](https://rustup.rs/). Minimum edition: 2021.
- **GPU** - A GPU with Vulkan, DX12, or Metal support (required by wgpu).

## Development

### Build

```sh
# Debug build (faster compilation, slower runtime)
cargo build

# Release build (slower compilation, optimized runtime)
cargo build --release
```

The release binary is written to `target/release/photo.exe`.

### Run

```sh
# Run debug build
cargo run

# Run with a specific image
cargo run -- path/to/image.jpg

# Run release build
cargo run --release -- path/to/image.jpg
```

### Test

```sh
cargo test
```

Rust unit tests cover decode, navigation, viewer math, collections, and edit logic without requiring a GPU.

### Lint

```sh
cargo clippy -- -D warnings
```

### Format

```sh
cargo fmt --check   # Check formatting
cargo fmt           # Auto-format
```

## Release

1. **Run all checks:**

   ```sh
   cargo fmt --check
   cargo clippy -- -D warnings
   cargo test
   ```

2. **Build the release binary:**

   ```sh
   cargo build --release
   ```

3. **Locate the artifact:**

   The optimized binary is at `target/release/photo.exe`. Release builds use `opt-level = 3` and thin LTO (see `Cargo.toml` `[profile.release]`).

4. **Distribute:**

   `photo.exe` is a single self-contained binary - no installer or runtime dependencies required. Ship the `.exe` directly.

## Project Structure

```
src/
  main.rs    - App state, message loop, tab routing, keyboard/event handling
  viewer.rs  - GPU shader pipeline for image rendering (zoom, pan, texture upload)
  decode.rs  - Image decoding (raster via image crate, RAW via rawler, SVG via resvg)
  collection.rs - Collection CRUD and JSON persistence
  edit.rs    - Edit state, undo/redo, and CPU-side save pipeline
  lens.rs    - Lensfun and EXIF metadata lookup
  nav.rs     - Directory scanning and file navigation
assets/
  shaders/
    image.wgsl - Vertex/fragment shader with adjustments, lens correction, crop overlay
    blur.wgsl  - Separable Gaussian blur pre-pass for clarity/dehaze
  lensfun/
    sample-lenses.xml - Bundled Lensfun lens profile sample
docs/
  README.md - Docs index and entry point
  architecture/
    ARCHITECTURE.md - Main architecture narrative
    decisions.md - Key architectural decisions
    drift-log.md - Architecture drift history
  devlog/
    summary.md - Compact project summary
    detailed/ - Dated detailed devlogs
  learning/
    lessons.md - Short maintained lessons
  debugging/
    template.md - Debugging session template
  reviews/
    README.md - Review artifacts and summaries
```

The flat `docs/ARCHITECTURE.md` file is a temporary compatibility shim for older references. New content belongs in the canonical paths above.

## License

See [LICENSE](LICENSE) for details.
