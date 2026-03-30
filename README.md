# photo

A GPU-accelerated image viewer for Windows, built with Rust, [iced](https://github.com/iced-rs/iced), and wgpu.

Supports JPEG, PNG, GIF, BMP, TIFF, WebP, ICO, TGA, QOI, HDR, EXR, and SVG formats.

## Features

- **Library tab** — Browse image collections as a scrollable thumbnail grid. Load images via folder picker or file picker.
- **Detail tab** — View individual images with GPU-rendered zoom and pan.
- **Keyboard navigation** — Arrow keys to cycle through images.
- **CLI support** — Open a file directly: `photo.exe path/to/image.jpg`

## Prerequisites

- **Rust toolchain** — Install via [rustup](https://rustup.rs/). Minimum edition: 2021.
- **GPU** — A GPU with Vulkan, DX12, or Metal support (required by wgpu).

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

There are 31 unit tests across three modules (`decode`, `nav`, `viewer` math). All tests run without a GPU.

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

   `photo.exe` is a single self-contained binary — no installer or runtime dependencies required. Ship the `.exe` directly.

## Project Structure

```
src/
  main.rs    — App state, message loop, tab routing, keyboard/event handling
  viewer.rs  — GPU shader pipeline for image rendering (zoom, pan, texture upload)
  decode.rs  — Image decoding (raster via image crate, SVG via resvg)
  nav.rs     — Directory scanning and file navigation
assets/
  shaders/
    image.wgsl — Vertex/fragment shader for textured quad rendering
docs/
  ARCHITECTURE.md — Full architecture documentation
```

## License

See [LICENSE](LICENSE) for details.