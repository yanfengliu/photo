# Debugging - Detail Reopen Latency

## Session
- Date: 2026-04-23
- Owner: Codex
- Scope: Make leaving Detail for Library and reopening the same image feel instant when the full image is already in memory
- Related issue or symptom: User reported that entering, exiting, and re-entering Detail still takes a long time even though the in-memory cache exists

## Environment
- Branch or commit: `main` with local uncommitted changes
- OS: Windows
- Runtime or tool versions: Rust/Cargo workspace toolchain, iced/wgpu desktop app
- Relevant docs or notes: `docs/devlog/summary.md`, `docs/architecture/ARCHITECTURE.md`, `docs/debugging/2026-04-20-detail-load-latency.md`

## Reproduction
- Steps to reproduce:
  - Open an image into Detail.
  - Return to Library.
  - Double-click the same item to reopen it.
- Expected result: If the full image is still the one already on screen and the source metadata still matches, Detail should come back immediately without a visible reload.
- Actual result before the fix: The app still routed the reopen through the generic load path, so the user paid avoidable reopen work even though the image was already loaded and reusable.
- Frequency: Expected whenever the user left Detail and quickly reopened the same image from Library.

## Investigation
- Hypothesis: The in-memory cache itself was working, but the same-image Library reopen path was still doing generic reload work after the cache-hit eligibility check.
- Checks performed:
  - Inspected `Message::LibraryItemClicked`, `App::start_load()`, and `displayed_full_image_for_path(...)` in `src/main.rs`.
  - Confirmed that the already displayed full image could be recognized as reusable, but the Library double-click handler still called `start_load(path)` afterward.
  - Strengthened reopen regressions to distinguish the true no-reload path from the metadata-changed fallback path.
  - Verified follow-up behavior after a deleted-source reopen by exercising a real save/export call instead of only checking that a save request exists.
- Commands run:
  - `cargo test library_reopen_ -- --nocapture`
  - `cargo test reopening_a_recently_viewed_detail_image_reuses_the_session_memory_cache -- --nocapture`
  - `cargo test displayed_full_image_fast_path_does_not_reuse_a_stale_base_source -- --nocapture`
  - `cargo test repeat_raw_open_ -- --nocapture`
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build --release`
- Important outputs:
  - The Library reopen regressions passed once the true fast path stopped incrementing `detail_load.request_id` and `image_id`.
  - The deleted-source reopen regression proved that exporting a PNG from the reopened image still succeeds even after the source file is gone.
  - The full suite passed at `286` tests.
- Files inspected:
  - `src/main.rs`
  - `docs/debugging/2026-04-20-detail-load-latency.md`

## Findings
- Root cause: The already displayed full-image fast path existed as an eligibility check, but the Library reopen handler still fell through to `start_load(path)` afterward, so the same image was effectively re-opened through the generic loading lifecycle instead of being shown directly.
- Secondary factors: The reuse decision still needs a cheap source-metadata validation so same-size rewrites do not keep stale pixels alive.
- What was ruled out: The reopen delay was not caused by the session cache being absent or by stale EXIF logic; the main avoidable work was the generic reload path itself.

## Fix
- Change made:
  - Added `try_reopen_current_library_image_without_reload(...)` so same-image Library returns can switch back to Detail without starting a new load when the currently displayed full image is still valid.
  - Extracted `reset_transient_detail_reopen_state()` and used it for both the no-reload fast path and the metadata-changed fallback load so the same user gesture keeps one reset contract.
  - Strengthened the reopen regressions to prove request/image ids stay stable on the true fast path, EXIF stays warm, deleted-source reopen still supports a real export, and same-size source rewrites still force a fresh load.
- Tradeoffs: The optimization still performs a synchronous metadata stat on the UI reopen path before trusting the already displayed image. That keeps the correctness guard cheap and narrow, but it is the remaining non-blocking performance follow-up from the final Codex review.
- Follow-up work: If reopen latency still matters after this change, measure whether the remaining metadata stat is noticeable enough to justify a broader redesign of the displayed-image validation contract.

## Validation
- Tests run:
  - `cargo test library_reopen_ -- --nocapture`
  - `cargo test reopening_a_recently_viewed_detail_image_reuses_the_session_memory_cache -- --nocapture`
  - `cargo test displayed_full_image_fast_path_does_not_reuse_a_stale_base_source -- --nocapture`
  - `cargo test repeat_raw_open_ -- --nocapture`
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build --release`
- Manual verification: This pass did not capture a new UI screenshot because the optimization is a behavior-preserving latency fix in the reopen path rather than a visible layout change. The regression suite was used to prove the user-facing fast path and fallback behavior directly.
- Remaining risk: The fast path still performs a synchronous metadata stat before reuse, so there is still a small amount of UI-thread filesystem work on reopen even though the generic reload is gone.

## Cleanup
- Temporary files to remove: None. The temporary reviewer-history files under `code_review/` were deleted after the detailed devlog captured their contents.
- Notes to keep in `docs/learning/lessons.md`: If the user is simply returning from Library to the same fully loaded Detail image, skip the generic load path and reuse the already displayed image directly, but keep the metadata guard and reset contract aligned with the real reload fallback.
