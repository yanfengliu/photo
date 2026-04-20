# Crash Debugging - Stale Detail State

## Session
- Date: 2026-04-20
- Owner: Codex
- Scope: Crash-hardening follow-up for stale collection actions and save requests during async image loads
- Related issue or symptom: User reported "App crashed"

## Environment
- Branch or commit: `main` with local uncommitted changes
- OS: Windows
- Runtime or tool versions: Rust/Cargo workspace toolchain, iced/wgpu desktop app
- Relevant docs or notes: `docs/devlog/summary.md`, `docs/architecture/ARCHITECTURE.md`

## Reproduction
- Steps to reproduce:
  - Open a photo in Detail and start loading another image, then trigger Save before the new load finishes.
  - Open a photo context menu and delete or otherwise invalidate the destination collection before running the add/toggle action.
- Expected result: Save and collection actions either target valid state or no-op safely.
- Actual result: Save could pair a new path with the previous image while loading, and collection actions still assumed the stored destination collection index existed.
- Frequency: Deterministic in targeted regression tests once the stale-state setup is created.

## Investigation
- Hypothesis: The remaining crash-prone paths were fail-open follow-ups around async save state and collection-action destination validity.
- Checks performed:
  - Inspected `src/main.rs` save request construction plus collection add/toggle handling.
  - Added regression coverage for removed collections, visible crop save state, save requests while loading, save status/no-image behavior, and lens-vignetting save state.
  - Reviewed how `current_image_path`, `image`, and `loading` change across async image loads.
- Commands run:
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build`
  - `codex exec "..."` clean-code review/verification
  - `gemini -p "..." --approval-mode yolo` correctness and clean-code reviews
- Important outputs:
  - Review feedback confirmed stale-state risks around library/menu targeting.
  - Codex correctness review found that `Save` could write old pixels to a newly selected path while the new image was still loading.
  - Final validation passed with 189 tests plus clean `clippy` and `build`.
- Files inspected:
  - `src/main.rs`
  - `docs/devlog/summary.md`
  - `docs/architecture/ARCHITECTURE.md`

## Findings
- Root cause: Save requests could be built from a path that had advanced ahead of the loaded image, and collection actions did not fail closed when their destination collection disappeared after the menu opened.
- Secondary factors: Review tooling created temporary repo-root artifacts during the review loop, which needed cleanup before the final verification pass.
- What was ruled out: The final regression set did not point to viewer, shader, decode, or collection-persistence failures for this crash path.

## Fix
- Change made:
  - Added collection-existence guards so add/toggle context-menu actions no-op if the destination collection is gone.
  - Introduced `SaveRequest`, `visible_edit_state()`, `current_lens_vignetting()`, and `current_save_request()` so save uses the visible crop state and becomes a safe no-op while a new image is loading.
  - Expanded regression coverage for removed collections, loading-time save requests, save status, and lens-vignetting save state.
- Tradeoffs: The Save button still renders during loading, but the action now fails closed instead of trying to save partially updated state.
- Follow-up work: Manual in-app smoke testing of the original crash path would still be useful.

## Validation
- Tests run:
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build`
- Manual verification: Not run
- Remaining risk: The code paths are covered by regression tests, but I did not reproduce the original crash manually in the live app after the fix.

## Cleanup
- Temporary files to remove: Removed repo-root `tmp_codex_*.txt` review artifacts created during the review loop.
- Notes to keep in `docs/learning/lessons.md`: Build save/export from the currently visible image state, and fail collection actions closed when their target collection may have disappeared.
