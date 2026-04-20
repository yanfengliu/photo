# Debugging - Detail Load Latency

## Session
- Date: 2026-04-20
- Owner: Codex
- Scope: Reduce the time between opening an image in Detail view and seeing a usable image
- Related issue or symptom: User reported that image loading still feels long

## Environment
- Branch or commit: `main` with local uncommitted changes
- OS: Windows
- Runtime or tool versions: Rust/Cargo workspace toolchain, iced/wgpu desktop app
- Relevant docs or notes: `docs/devlog/summary.md`, `docs/architecture/ARCHITECTURE.md`, `docs/debugging/2026-04-20-detail-open-uniform-layout-crash.md`

## Reproduction
- Steps to reproduce:
  - Open a RAW image from the Library or by launching the app directly into a RAW file.
  - Watch how long Detail view stays blank before the image appears.
- Expected result: Users should see a usable image quickly, even if a higher-quality decode is still running.
- Actual result before the fix: Detail view waited for full RAW development before showing anything.
- Frequency: Expected on RAW images because the detail decode path was raw-pixel-first.

## Investigation
- Hypothesis: The slow path was dominated by full RAW development and a synchronous EXIF read on the same load transition.
- Checks performed:
  - Inspected `App::start_load()` and confirmed that Detail used a single blocking decode task plus a synchronous EXIF read in `Message::ImageLoaded`.
  - Re-read `decode.rs` and confirmed that RAW Detail loads called `decode_raw(..., prefer_thumbnail = false)`, which develops RAW pixels first and only falls back to embedded images later.
  - Inspected `rawler`'s ARW decoder and confirmed that `full_image()` exposes the embedded JPEG preview, which is much cheaper than full RAW development.
  - Added regression tests for embedded-preview decoding, preview-to-full upgrade behavior, stale async result suppression, and save waiting for auto lens metadata when needed.
- Commands run:
  - `cargo test decode_embedded_preview_returns_none_when_raw_has_no_embedded_image`
  - `cargo test decode_embedded_preview_falls_back_to_preview_then_thumbnail`
  - `cargo test raw_preview_load_keeps_image_visible_while_full_resolution_finishes`
  - `cargo test stale_preview_and_full_results_are_ignored_after_a_newer_load_starts`
  - `cargo test current_save_request_waits_for_auto_lens_metadata_when_needed`
- Important outputs:
  - The new preview-load regressions passed after the staged-load change.
  - Existing save/lens tests exposed the expected EXIF-timing guard and were updated to keep checking lens math instead of the new loading gate.
- Files inspected:
  - `src/main.rs`
  - `src/decode.rs`
  - `src/lens.rs`
  - `C:\Users\38909\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\rawler-0.7.2\src\decoders\arw.rs`

## Findings
- Root cause: RAW Detail loads were blocking on full RAW development before the first image became visible, and EXIF/lens metadata was also being read synchronously on the UI-side completion path.
- Secondary factors: The existing RAW thumbnail path was already fast because it preferred embedded images, but Detail was intentionally using the slower raw-pixel-first path to preserve final quality.
- What was ruled out: The delay was not primarily coming from the shader or blur pre-pass anymore; the visible wait was already present before the first high-quality image reached the viewer.

## Fix
- Change made:
  - Added `decode_embedded_preview()` in `src/decode.rs` to extract a fast embedded RAW image for Detail when one exists.
  - Changed `App::start_load()` to choose the load plan up front: non-RAW files still go straight to full decode plus EXIF, while RAW files stage Detail loading by showing the embedded preview first and only then launching the full-resolution decode plus EXIF for the still-current request.
  - Centralized the staged Detail lifecycle in `DetailLoadState` so request id, preview visibility, loading state, and EXIF readiness stay in sync.
  - Moved EXIF reading onto its own blocking task so preview/full image display no longer waits on metadata parsing.
  - Kept the preview-to-full upgrade on the user's current zoom and pan instead of applying a resolution-based zoom correction; the viewer's fit math already normalizes source resolution, so rewriting zoom made the handoff less stable rather than more stable.
  - Prevented save/export from running while only an embedded preview is available or while required auto lens metadata is still pending.
- Tradeoffs: RAW Detail view can now show a temporary embedded preview before the final developed image replaces it. This improves time-to-first-image without changing the final quality path, but RAW files without embedded previews still wait for the slower full decode.
- Follow-up work: A numeric benchmark harness for decode time would make future speed work easier to compare, since this pass improved the user-visible pipeline but did not add persistent timing telemetry.

## Validation
- Tests run:
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build --release`
- Manual verification: The packaged and debug builds were still launch-tested into real files after the staged-load change; no dedicated stopwatch benchmark was captured in this session.
- Remaining risk: RAW files without any embedded image still fall back to the slower full RAW decode before anything can be shown, so the speedup is largest on formats/cameras that ship an embedded JPEG preview.

## Cleanup
- Temporary files to remove: None created by this debugging pass.
- Notes to keep in `docs/learning/lessons.md`: If full-quality image development is expensive, prefer a staged Detail load that shows a fast embedded preview first and upgrades in place once the final decode is ready.
