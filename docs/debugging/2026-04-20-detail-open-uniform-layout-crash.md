# Crash Debugging - Detail Open Uniform Layout

## Session
- Date: 2026-04-20
- Owner: Codex
- Scope: Crash when opening a Library image into Detail view, especially when the image never appeared before the app died
- Related issue or symptom: User reported that clicking a Library image switched to Detail view, the image never loaded, and the app then crashed

## Environment
- Branch or commit: `main` with local uncommitted changes
- OS: Windows
- Runtime or tool versions: Rust/Cargo workspace toolchain, iced/wgpu desktop app, release and debug builds
- Relevant docs or notes: `docs/devlog/summary.md`, `docs/architecture/ARCHITECTURE.md`, `docs/debugging/2026-04-20-release-click-crash.md`

## Reproduction
- Steps to reproduce:
  - Launch the app with a file path so it opens directly into Detail view.
  - Let the first Detail render start.
- Expected result: The app stays open and renders the selected image.
- Actual result: The app panicked in `wgpu` during the first draw because the bound uniform buffer was smaller than the shader layout required.
- Frequency: Deterministic before the fix.

## Investigation
- Hypothesis: The earlier blur-path diagnosis explained extra GPU work, but the newer symptom of a blank Detail view followed by a crash suggested a render-contract problem on the first frame.
- Checks performed:
  - Reproduced the crash locally with `cargo run -- C:\Users\38909\Documents\github\photo\test.jpg`.
  - Confirmed the same fast failure pattern by launching the packaged executable with both `test.jpg` and a persisted library RAW file (`DSC01154.ARW`).
  - Captured the panic output from the debug run and traced it back to the uniform buffer layout in `src/viewer.rs`.
  - Compared the Rust `Uniforms` struct against `assets/shaders/image.wgsl` and found that the crop fields were no longer 16-byte aligned on the Rust side after recent additions.
  - Added a focused regression test that checks the `Uniforms` size and the offsets of the crop-related fields.
- Commands run:
  - `cargo run -- C:\Users\38909\Documents\github\photo\test.jpg`
  - `Start-Process ...\target\release\photo.exe -ArgumentList 'C:\Users\38909\Documents\images\DSC01154.ARW'`
  - `cargo test uniforms_layout_matches_wgsl_uniform_buffer`
  - `cargo test`
- Important outputs:
  - `wgpu error: Validation Error`
  - `Buffer is bound with size 220 where the shader expects 240 in group[0] compact index 0`
  - The new layout regression failed before the fix with `left: 220 right: 240`.
- Files inspected:
  - `src/viewer.rs`
  - `assets/shaders/image.wgsl`
  - `src/main.rs`

## Findings
- Root cause: The Rust `Uniforms` struct no longer matched the WGSL uniform-buffer layout. The crop fields were packed too tightly on the Rust side, so the bound buffer was only 220 bytes while the shader required 240 bytes.
- Secondary factors: The mismatch only surfaced once the first Detail render executed, which made the user experience look like a stalled or blank image load before the panic terminated the app.
- What was ruled out: The app did not need a user click inside Detail to crash, and the earlier lazy-blur issue was not the direct cause of this reproducible panic.

## Fix
- Change made:
  - Added `uniforms_layout_matches_wgsl_uniform_buffer` in `src/viewer.rs`.
  - Introduced explicit padding in the Rust `Uniforms` struct so the crop-related fields land at the same offsets the WGSL shader expects.
  - Verified the app now stays alive when launched directly into both `test.jpg` and `DSC01154.ARW`.
- Tradeoffs: The fix adds explicit padding fields to preserve the GPU contract, which is slightly less elegant than a tighter logical struct but much safer and easier to lock down with a test.
- Follow-up work: If the uniform block changes again, update the regression test at the same time so the layout contract fails in tests before it fails at runtime.

## Validation
- Tests run:
  - `cargo test uniforms_layout_matches_wgsl_uniform_buffer`
  - `cargo test`
- Manual verification:
  - Launched `target\debug\photo.exe` with `test.jpg` and confirmed the process was still alive after 5 seconds.
  - Launched `target\debug\photo.exe` with `DSC01154.ARW` and confirmed the process was still alive after 8 seconds.
  - Launched `target\release\photo.exe` with `DSC01154.ARW` and confirmed the process was still alive after 8 seconds.
- Remaining risk: This locks down the uniform-layout crash that was locally reproduced, but it does not add a GPU-backed test that executes the full WGPU renderer in CI.

## Cleanup
- Temporary files to remove: None created by this debugging pass.
- Notes to keep in `docs/learning/lessons.md`: Keep Rust and WGSL uniform-buffer layouts in sync with explicit padding and a dedicated size/offset regression test whenever the shader contract changes.
