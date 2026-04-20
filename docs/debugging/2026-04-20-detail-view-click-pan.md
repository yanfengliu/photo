## Session
- Date: 2026-04-20
- Owner: Codex
- Scope: Detail-view click and drag behavior
- Related issue or symptom: Clicking the image and moving the mouse immediately shifts the image, which feels confusing in the default detail view.

## Environment
- Branch or commit: `main` with uncommitted local changes
- OS: Windows
- Runtime or tool versions: Rust toolchain via `cargo`
- Relevant docs or notes: `AGENTS.md`, `docs/devlog/summary.md`, `docs/architecture/ARCHITECTURE.md`

## Reproduction
- Steps to reproduce:
  1. Open an image in the Detail view.
  2. Leave the image at the default fit-to-window zoom.
  3. Click on the image and move the mouse slightly.
- Expected result: The image should stay still unless there is meaningful room to pan.
- Actual result: The image begins following the mouse movement immediately.
- Frequency: Consistent

## Investigation
- Hypothesis: The shader viewer starts drag-pan on every left-button press, even when the image does not overflow the viewport.
- Checks performed:
  - Read the architecture and devlog summary.
  - Inspected `src/viewer.rs` pointer handling and `src/main.rs` viewer event handling.
  - Added tests for ignoring pan when the image fits the canvas and preserving pan when zoomed in.
- Commands run:
  - `Get-Content -Path "docs/devlog/summary.md" -TotalCount 220`
  - `Get-Content -Path "docs/architecture/ARCHITECTURE.md" -TotalCount 260`
  - `Select-String -Path "src/main.rs","src/viewer.rs" -Pattern "drag|pan|mouse|cursor|click|zoom|offset" -Context 3,3`
- Important outputs:
  - Before the fix, `src/viewer.rs` set `state.dragging = true` on every left-button press over the canvas.
  - `src/main.rs` currently applies every `ViewerEvent::Pan` delta directly to the image offset.
- Files inspected:
  - `src/viewer.rs`
  - `src/main.rs`

## Findings
- Root cause: The shader viewer started drag-pan on every left-button press over the canvas, even when the image was fit to the viewport and had no meaningful room to move. That made slight pointer movement after a click shift the image immediately.
- Secondary factors: The same pannability decision was initially split across viewer and app logic, the first tolerance was too coarse for small visible offsets, and adjacent viewer math also needed clamp-aware zoom offsets plus drag re-entry cleanup to avoid additional confusing motion.
- What was ruled out: Rotation-specific logic; the confusing motion came from the base viewer pan path and nearby pointer math.

## Fix
- Change made: Centralized pannability in `src/viewer.rs`, so drag start and grab-cursor affordance only activate when the image is actually pannable. Added viewer-event tests for fit-image no-drag, zoomed-image drag, off-center rescue drag, drag re-entry, and clamp-aware zoom-at-cursor offset behavior.
- Tradeoffs: Dragging still stops when the cursor leaves the widget bounds instead of continuing across the whole window. That behavior is unchanged from the previous model but is now handled without re-entry jumps.
- Follow-up work: A manual smoke test in the running app is still worthwhile for high-DPI pointer feel and fast edge drags.

## Validation
- Tests run:
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build`
- Manual verification: Not run in-app yet.
- Remaining risk: Runtime pointer feel on real hardware is still best validated manually, especially near the canvas edge and on high-DPI displays.

## Cleanup
- Temporary files to remove: None yet.
- Notes to keep in `docs/learning/lessons.md`: Keep drag affordances and actual drag behavior on the same pannability rule, and use pixel-scale thresholds so small visible offsets remain recoverable without making fit-to-window images drift on click.
