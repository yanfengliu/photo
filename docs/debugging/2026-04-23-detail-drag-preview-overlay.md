# Detail Drag Preview Overlay

## Session
- Date: 2026-04-23
- Owner: Codex
- Scope: Remove the floating Library drag thumbnail from Detail view.
- Related issue or symptom: The thumbnail drag preview kept following the mouse over the Detail editor even though it had no effect there.

## Environment
- Branch or commit: `main` with pre-existing uncommitted work in the tree.
- OS: Windows
- Runtime or tool versions: Rust app with `iced 0.13`; validation via `cargo test`, `cargo clippy -- -D warnings`, and `cargo build --release`.
- Relevant docs or notes: `docs/devlog/summary.md`, `docs/architecture/ARCHITECTURE.md`, `docs/learning/lessons.md`, user screenshot `C:\Users\38909\Documents\screenshots\Screenshot 2026-04-23 131130.png`

## Reproduction
- Steps to reproduce:
  - Double-click a Library thumbnail to enter Detail.
  - Move the cursor after the Detail view opens.
- Expected result:
  - Detail should show only the image/editor UI.
- Actual result:
  - The Library drag preview can remain alive and follow the cursor over the Detail editor.
- Frequency:
  - Reproducible from the Library open path and the same-image fast reopen path before the fix.

## Investigation
- Hypothesis:
  - `drag_state` survives the Library-to-Detail transition and the overlay keeps rendering because that state is still active.
- Checks performed:
  - Read `Message::LibraryItemClicked`, `start_load(...)`, `try_reopen_current_library_image_without_reload(...)`, and the drag overlay path in `src/main.rs`.
  - Added regressions for the normal Library-to-Detail load path and the fast same-image reopen path.
- Commands run:
  - `cargo test opening_detail_from_library_clears_pending_drag_state`
  - `cargo test library_reopen_reuses_the_displayed_full_image_immediately`
  - `cargo test library_reopen_reloads_when_the_current_source_metadata_changes`
  - `cargo test drag_`
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build --release`
  - Focused reviewer attempts with local `codex`, `gemini`, and `claude` CLIs
- Important outputs:
  - Pre-fix targeted tests failed on `assert!(app.drag_state.is_none())`.
  - Final validation passed with `cargo test` (`287` tests), `cargo clippy -- -D warnings`, and `cargo build --release`.
  - Reviewer tooling status: Codex review attempts timed out, Gemini returned `ModelNotFoundError` for `gemini-3-pro`, and Claude was unavailable because the CLI was not logged in after an earlier timeout.
- Files inspected:
  - `src/main.rs`
  - `docs/devlog/summary.md`
  - `docs/architecture/ARCHITECTURE.md`
  - `docs/learning/lessons.md`

## Findings
- Root cause:
  - Library clicks always seeded `drag_state`, and entering Detail did not clear that Library-only state before the normal load path or the fast same-image reopen path completed.
- Secondary factors:
  - The overlay was driven entirely by the presence of active drag state, so stale Library drag state remained visible in Detail.
- What was ruled out:
  - The problem was not thumbnail decoding, RAW preview loading, or the thumbnail contain helper.

## Fix
- Change made:
  - Added `clear_library_drag_state()` and called it from both `start_load(...)` and `try_reopen_current_library_image_without_reload(...)`, then added regressions that prove Detail opens clear pending drag state in both paths.
- Tradeoffs:
  - This keeps the fix narrow and avoids changing Library drag/drop mechanics.
- Follow-up work:
  - If a future Library-to-Detail transition bypasses these helpers, it should clear the same Library-only state.

## Validation
- Tests run:
  - `cargo test opening_detail_from_library_clears_pending_drag_state`
  - `cargo test library_reopen_reuses_the_displayed_full_image_immediately`
  - `cargo test library_reopen_reloads_when_the_current_source_metadata_changes`
  - `cargo test drag_`
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build --release`
- Manual verification:
  - Used the user-provided before screenshot to confirm the symptom. I did not capture a live after screenshot or pixel diff from this shell because there is no session-local GUI automation/screenshot harness available here.
- Remaining risk:
  - Low. The visible bug is now covered by focused state-transition regressions, but the manual visual after-capture remains unverified in this session.

## Cleanup
- Temporary files to remove:
  - `code_review/drag_detail_overlay_review_context.txt`
  - `code_review/claude-round1.txt`
- Notes to keep in `docs/learning/lessons.md`:
  - Clear Library-only drag state when the app transitions into Detail so library overlays cannot survive into editor-only UI.
