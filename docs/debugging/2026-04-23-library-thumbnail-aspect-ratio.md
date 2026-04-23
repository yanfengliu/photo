# Debugging Template

## Session
- Date: 2026-04-23
- Owner: Codex
- Scope: Library thumbnails for original images still not preserving the expected display shape in the live app
- Related issue or symptom: Real `.ARW` files in the Library view still look wrong after the earlier thumbnail containment and RAW-orientation fixes

## Environment
- Branch or commit: `main` (working tree initially clean)
- OS: Windows
- Runtime or tool versions: Rust/Cargo project; live repro files under `C:\Users\38909\Documents\images\`
- Relevant docs or notes:
  - `docs/devlog/summary.md`
  - `docs/architecture/ARCHITECTURE.md`
  - `docs/devlog/detailed/2026-04-23_2026-04-23.md`

## Reproduction
- Steps to reproduce:
  - Open the app Library with `C:\Users\38909\Documents\images\DSC01167.ARW`, `DSC01168.ARW`, and `DSC01169.ARW`.
  - Compare the visible thumbnail shape/orientation against the expected original image display.
- Expected result: Library thumbnails preserve the source image display orientation and aspect ratio inside the square slot.
- Actual result: The live screenshot still shows the originals looking wrong in Library even after the prior fixes.
- Frequency: Reported as deterministic in the current build/session.

## Investigation
- Hypothesis:
  - The square-slot `iced` containment helper is already correct, so either the RAW thumbnail decode is still emitting the wrong pixel orientation/dimensions for these files or a Library-specific path is bypassing the fixed contract.
- Checks performed:
  - Re-read repo instructions, summary, and architecture docs.
  - Inspected `thumbnail_slot_with_renderer`, `thumbnail_card_content`, `load_library_thumbnail_base_image`, and `decode_thumbnail`.
  - Confirmed the widget-level containment tests still pass.
  - Exported the real `DSC01169.ARW` decode output from `decode_thumbnail(...)` and confirmed it is already aspect-correct.
  - Exported the baked `local-edits/` thumbnail for `DSC01169.ARW` and confirmed the on-disk thumb was square and sideways even though the matching full baked image was wide.
  - Confirmed `thumbnail_from_rendered_image(...)` itself was generating square `200x200` outputs from large wide baked images, which explained both new writes and repair fallbacks.
- Commands run:
  - `cargo test decode_raw_thumbnail_applies_orientation_metadata -- --nocapture`
  - `cargo test library_thumbnail_ignores_a_same_generation_persisted_thumbnail_when_its_aspect_ratio_disagrees_with_the_full_copy -- --nocapture`
  - `cargo test library_thumbnail_ -- --nocapture`
- Important outputs:
  - The existing synthetic RAW orientation regression passed, so the RAW decode thumbnail path was not the failing seam anymore.
  - Real-file inspection showed `decode_thumbnail(DSC01169.ARW)` returned a portrait `134x200` image, but the baked local-edit thumb on disk was `200x200`.
  - A temporary real-file verification hook showed the matching baked full image was `16384x10923`, and `thumbnail_from_rendered_image(...)` was incorrectly deriving `200x200` from that full copy before the fix.
- Files inspected:
  - `src/main.rs`
  - `src/decode.rs`
  - `docs/devlog/detailed/2026-04-23_2026-04-23.md`

## Findings
- Root cause: `thumbnail_from_rendered_image(...)` used `image::imageops::thumbnail(...)`, which was producing square `200x200` baked thumbnails for large wide rendered images instead of preserving aspect ratio. Library then trusted those baked local-edit thumbs whenever they matched the full copy's generation, so `DSC01169.ARW` kept showing a square Library preview even though the RAW decode path itself was already correct.
- Secondary factors: The user-facing screenshot looked like another RAW/orientation regression at first, but the real failing seam was the baked local-edit thumbnail helper plus Library's willingness to trust an impossible same-generation thumb.
- What was ruled out:
  - The shared square-slot `ContentFit::Contain` helper itself.
  - The generic raster thumbnail resize path.
  - The RAW `decode_thumbnail(...)` path for the real `DSC01169.ARW` file.

## Fix
- Change made:
  - Replaced `thumbnail_from_rendered_image(...)`'s square `image::imageops::thumbnail(...)` call with an explicit max-axis resize that preserves aspect ratio.
  - Tightened `load_library_thumbnail_base_image(...)` so it only trusts a persisted thumb when its dimensions match the thumbnail that would be derived from the matching full baked image.
  - Added a header-only persisted-full fast path so valid baked thumbnails do not force a full-image pixel read before Library can trust them.
  - Serialized stale-thumb repair against local-edit writes, re-checked persisted state inside that lock, and kept the repair write best-effort so a transient cache-write failure cannot blank the visible Library thumbnail.
  - Tightened the cache readers to validate file length from the opened file handle, reject impossible cache file lengths before allocating the cached path buffer, keep the outer Library fast path header-only for the thumbnail too, fail invalid-cache reads closed instead of deleting by path, and retry/recheck stale-thumb repair against the current baked state instead of painting an older precomputed thumb or falling back to the original source thumbnail when a newer baked generation lands mid-load.
  - Added regressions for same-generation aspect mismatch repair, same-shape generation mismatch, in-lock recheck behavior, repair-write failure behavior, portrait downscaling, and already-in-bounds no-op behavior.
- Tradeoffs:
  - The repair path may still do one extra resize from the baked full image the first time it encounters a stale thumb, and the on-disk rewrite remains synchronous so the first stale-thumb load pays that cost once.
- Follow-up work:
  - None for this bug beyond the normal review and full-suite validation loop.

## Validation
- Tests run:
  - `cargo test decode_raw_thumbnail_applies_orientation_metadata -- --nocapture`
  - `cargo test library_thumbnail_ignores_a_same_generation_persisted_thumbnail_when_its_aspect_ratio_disagrees_with_the_full_copy -- --nocapture`
  - `cargo test library_thumbnail_ -- --nocapture`
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build --release`
- Manual verification:
  - Exported the stale baked thumb before the fix to `tmp/debug-real-thumbnails/DSC01169-local-edit-thumb-before.png`.
  - Exported the real Library thumbnail result after the fix to `tmp/debug-real-thumbnails/DSC01169-library-thumb-after.png`.
  - Rendered the post-fix image into a square Library-slot canvas at `tmp/debug-real-thumbnails/DSC01169-library-thumb-after-slot.png`.
  - Generated a square-slot pixel diff at `tmp/debug-real-thumbnails/DSC01169-library-thumb-slot-diff.png`.
  - Confirmed the real on-disk baked thumb header changed from `200 x 200` to `200 x 133` after the repair path ran.
- Remaining risk:
  - Full Rust validation now passes with `278` tests, but Gemini review coverage remained blocked by external CLI/argument failures in this session.

## Cleanup
- Temporary files to remove:
  - Temporary debug test hooks were removed from `src/main.rs` and `src/decode.rs`.
  - The visual verification artifacts under `tmp/debug-real-thumbnails/` are disposable once the task is fully signed off.
- Notes to keep in `docs/learning/lessons.md`:
  - Baked thumbnail helpers should use explicit aspect-ratio-preserving resize logic, and Library should not trust a persisted thumb that disagrees with its matching full baked copy.
