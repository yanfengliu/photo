# Lessons evidence

The war story, the measurements and the anchor behind each line in [lessons.md](lessons.md). Not session-start reading.

## `cargo fmt` given `--manifest-path` checks only the root package

**Anchor:** 2026-11-19, GitHub Actions run 44680100632 on commit `c59c2bf`: the Format step failed on macOS and Windows thirty seconds after a push whose message said the format gate passed. Fixed in `5a64b64`.

**Gate:** CI's Format step in `.github/workflows/ci.yml`. It was red on the defect. It has not yet been green on `main`, because the UI spike in `c59c2bf` stayed unformatted until the widget kit replaced it, so the proof is half made.

Every cargo command in the session that built the foundation was run with `--manifest-path <root>/Cargo.toml`. For `cargo test`, `cargo clippy` and `cargo build` that is harmless. `cargo fmt` decides whether to format every workspace member by comparing the workspace root directory with the current directory; given `--manifest-path` it compares the root directory with the manifest file's path instead, the comparison is always false, and it formats only the package that manifest belongs to. `cargo fmt --check --manifest-path <root>/Cargo.toml` exited 0 while eleven files in `crates/core` were unformatted, and version 0.3.0 was committed and pushed with a red format gate reported as green.

What works: plain `cargo fmt --check` with the workspace root as the working directory, `cargo fmt --all --check` from anywhere, or `cargo fmt -p <crate> -- --check` for one crate, which selects packages by name.
