# photo

A GPU-accelerated image viewer for Windows, built with Rust, [iced](https://github.com/iced-rs/iced), and wgpu.

Supports JPEG, PNG, GIF, BMP, TIFF, WebP, ICO, TGA, QOI, HDR, EXR, SVG, and common camera RAW formats including DNG, CR2/CR3, NEF, ARW, RAF, RW2, ORF, and more.

## Features

- **Library tab** - Browse image collections as a scrollable thumbnail grid. Load images via folder picker or file picker.
- **Detail tab** - View individual images with GPU-rendered zoom and pan.
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

Rust unit tests cover decode, navigation, viewer math, collections, edit logic, and EXIF/Lensfun parsing without requiring a GPU.

### Lint

```sh
cargo clippy --all-targets -- -D warnings
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
   cargo clippy --all-targets -- -D warnings
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
  main.rs    - App state, message loop, tab routing, keyboard/event handling, staged Detail-load orchestration, baked local-edit persistence
  viewer.rs  - GPU shader pipeline for image rendering (zoom, pan, crop overlay, texture upload)
  decode.rs  - Image decoding (raster via image crate, RAW via rawler, SVG via resvg) plus persisted decoded-image cache
  collection.rs - Collection CRUD and JSON persistence
  edit.rs    - Edit state, undo/redo, and CPU-side save pipeline
  lens.rs    - Lensfun XML parsing, EXIF reading, and lens profile lookup
  nav.rs     - Directory scanning and file navigation
assets/
  shaders/
    image.wgsl - Fragment shader for the textured quad with adjustments, lens correction, and crop overlay
    blur.wgsl  - Separable 9-tap Gaussian blur pre-pass for clarity/dehaze
  lensfun/
    sample-lenses.xml - Bundled Lensfun profile data
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

At runtime the app also maintains two repo-local data directories (gitignored): `decoded-cache/` for persisted decoded RAW/SVG images and `local-edits/` for baked committed-edit copies.

The flat `docs/ARCHITECTURE.md` file is a temporary compatibility shim for older references. New content belongs in the canonical paths above.

## License

See [LICENSE](LICENSE) for details.
