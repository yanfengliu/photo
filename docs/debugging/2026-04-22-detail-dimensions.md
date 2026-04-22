# Detail Dimension Mismatch

## Session
- Date: 2026-04-22
- Owner: codex
- Scope: Detail-view status bar reported impossible image dimensions for a RAW file.
- Related issue or symptom: A file that should read as `6656 x 9728` showed up as `10923 x 16384` in the Detail bottom-left status bar.

## Environment
- Branch or commit: `main`
- OS: Windows
- Runtime or tool versions: Rust app with `cargo test`, `cargo clippy -- -D warnings`, and `cargo build --release` as the validation gates.
- Relevant docs or notes: `docs/devlog/summary.md`, `docs/architecture/ARCHITECTURE.md`

## Reproduction
- Steps to reproduce:
  1. Open a large RAW image in Detail view.
  2. Inspect the bottom-left status text.
- Expected result: The status bar should report the logical dimensions of the image the user opened.
- Actual result: The status bar reported the dimensions of the decoded upload buffer instead.
- Frequency: Reproducible when the decode path used a scaled full image or a stale persisted RAW cache entry.

## Investigation
- Hypothesis: The Detail status bar was reading the currently loaded buffer dimensions instead of source-image dimensions.
- Checks performed:
  - Traced the status-bar path through `status_bar_text()` and `current_display_dimensions()`.
  - Checked persisted decoded-cache entries and found stale RAW cache files with `16384 x 10923` dimensions.
  - Verified the RAW container itself still reported sane dimensions when read directly.
- Commands run:
  - `cargo test`
  - Temporary inspection commands against `decoded-cache` and the affected RAW files
- Important outputs:
  - The status bar used `self.image.width` / `self.image.height`.
  - Persisted decoded RAW cache entries existed at oversized dimensions.
  - Fresh RAW metadata reads still matched the expected source dimensions.
- Files inspected:
  - `src/main.rs`
  - `src/decode.rs`

## Findings
- Root cause: The Detail status bar derived dimensions from the current decoded `ImageData`, which is a GPU-ready buffer and not necessarily the logical size of the opened image.
- Secondary factors: The persisted decoded RAW cache had older oversized entries, so the wrong buffer dimensions could survive across restarts.
- What was ruled out: The original RAW container metadata was not corrupt.

## Fix
- Change made: Added a source-dimension lookup in `decode.rs`, taught `main.rs` to carry logical base-image dimensions separately from the loaded buffer, used those logical dimensions in status-bar rotation/crop math, threaded original-source logical dimensions back on the worker-thread full-image load result instead of probing them in `start_load()`, and bumped the decoded-cache contract version to invalidate stale oversized RAW cache entries.
- Tradeoffs: Original-source loads still do a background source-dimension read so the app can report truthful dimensions, and persisted decoded-cache hits still pay that read off the UI thread because the on-disk cache schema does not yet store logical dimensions.
- Follow-up work: None required for the visible bug beyond watching the existing intermittent decode-cache pruning test and, if cache-hit latency becomes a concern again, considering whether logical dimensions should be stored in the persisted decoded-cache schema too.

## Validation
- Tests run:
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build --release`
- Manual verification: The fix is designed so the status bar now follows logical base-image dimensions while still falling back to baked local-edit dimensions on persisted reopen paths, and the same-session cache restores those logical dimensions immediately on repeat opens.
- Remaining risk: Persisted decoded-cache hits still reopen the source file off the UI thread to recover logical dimensions because the on-disk cache schema only stores decoded buffer dimensions today.

## Cleanup
- Temporary files to remove: None
- Notes to keep in `docs/learning/lessons.md`: The UI should track logical image dimensions separately from transient decode/upload buffers.
