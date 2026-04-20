# Crash Debugging - Release Click Crash

## Session
- Date: 2026-04-20
- Owner: Codex
- Scope: Release-build crash when opening a photo from the Library into Detail view
- Related issue or symptom: User reported that the released build still crashes, especially after clicking an image, with rendering delayed or apparently stuck just before the crash

## Environment
- Branch or commit: `main` with local uncommitted changes
- OS: Windows
- Runtime or tool versions: Rust/Cargo workspace toolchain, iced/wgpu desktop app, release profile with thin LTO
- Relevant docs or notes: `docs/devlog/summary.md`, `docs/architecture/ARCHITECTURE.md`, `docs/debugging/2026-04-20-stale-detail-state-crash.md`

## Reproduction
- Steps to reproduce:
  - Start the released build.
  - Click a Library image to open it in Detail view.
- Expected result: Detail view opens promptly and renders the selected image without crashing.
- Actual result: User reported that rendering delays or appears stuck after the click, then the app crashes.
- Frequency: User-reported on released builds; local shell validation reproduced the expensive first-open render path in code inspection, but I did not manually drive the packaged GUI during this session.

## Investigation
- Hypothesis: The first Detail render was doing unnecessary GPU work on image open, and that release-path workload was a better fit for the reported stall-then-crash symptom than the earlier stale-state save/menu bug.
- Checks performed:
  - Re-read the current architecture and devlog summaries to confirm the Detail-view load/render boundaries.
  - Inspected `src/main.rs` image-click and async load flow (`LibraryItemClicked`, `start_load`, `ImageLoaded`).
  - Inspected `src/viewer.rs` GPU `prepare()` path and found that the blur pre-pass ran for every newly opened image, even when clarity and dehaze were both still at their default zero values.
  - Queried Windows Application / WER logs and found no direct new Rust panic report for `photo.exe`; the user-visible symptom and render-path inspection still pointed toward the expensive first-frame GPU path.
  - Added a focused regression for the blur-demand predicate, then expanded that coverage with pure lazy-blur state-transition tests before widening validation.
- Commands run:
  - `cargo build --release`
  - `cargo test blur_prepass_is_only_needed_for_clarity_or_dehaze`
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `Get-WinEvent ...`
  - `Get-Content src/main.rs`
  - `Get-Content src/viewer.rs`
- Important outputs:
  - `cargo test` passed with 194 tests after the fix.
  - `cargo clippy -- -D warnings` passed.
  - `cargo build --release` passed.
  - The viewer inspection showed unconditional blur-texture generation on image change before the fix.
- Files inspected:
  - `src/main.rs`
  - `src/viewer.rs`
  - `assets/shaders/image.wgsl`
  - `docs/devlog/summary.md`
  - `docs/architecture/ARCHITECTURE.md`

## Findings
- Likely cause: The Detail-view GPU prepare path always generated the blur pre-pass whenever a new image was opened, even when the user had not enabled clarity or dehaze. That made first-open rendering heavier than necessary in the exact click-to-open path the user reported as stalling and crashing.
- Secondary factors: Because the blur pass lived inside the image-upload path, the unnecessary cost happened before the first Detail frame could settle, which matched the reported delayed or stuck rendering feel.
- What was ruled out: The earlier stale-state save/menu crash hardening in `src/main.rs` did not match the updated user symptom, and this pass did not find evidence that the click crash was driven by collection state or loading-time save behavior.

## Fix
- Change made:
  - Added `AdjustmentUniforms::needs_blur()` in `src/viewer.rs`.
  - Changed the GPU prepare path to skip blur-texture generation on image open unless clarity or dehaze is actually non-zero.
  - Made blur generation lazy for the current image, so if the user later enables clarity or dehaze, the blur texture is still generated and the main bind group is rebuilt at that point.
  - Added `BlurUpdatePlan` plus pure tests for image-change reset, first-enable blur generation, and the no-work steady state, alongside the blur-demand predicate test.
- Tradeoffs: This keeps default image-open behavior lighter without changing output for default edits. The blur texture is now computed on demand when blur-based controls are actually used.
- Follow-up work: Manual smoke testing of the packaged release build on the same machine would still be valuable, since I did not click through the live GUI from this session.

## Validation
- Tests run:
  - `cargo test blur_prepass_is_only_needed_for_clarity_or_dehaze`
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build --release`
- Manual verification: Not run
- Remaining risk: This fix removes a credible source of first-open GPU pressure, but I did not manually reproduce and then re-verify the released executable crash on the live desktop after the change, so the cause should still be treated as likely rather than conclusively proven.

## Cleanup
- Temporary files to remove: None created by this debugging pass.
- Notes to keep in `docs/learning/lessons.md`: Do not build expensive GPU pre-pass resources on image open unless the current edit state actually needs them; lazy generation keeps the default Detail transition fast and reduces release-path render pressure.
