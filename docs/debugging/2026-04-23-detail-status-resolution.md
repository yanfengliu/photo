## Session
- Date: 2026-04-23
- Owner: Codex
- Scope: Detail-view bottom status bar reports the wrong image resolution after reopening a baked local edit.
- Related issue or symptom: `DSC01169.ARW` shows `16384x10923` in Detail status text instead of the image's logical dimensions.

## Environment
- Branch or commit: `main` @ `f1d0261`
- OS: Windows
- Runtime or tool versions: Rust + iced photo app
- Relevant docs or notes: `docs/debugging/2026-04-22-detail-dimensions.md`, `docs/devlog/summary.md`, `docs/architecture/ARCHITECTURE.md`

## Reproduction
- Steps to reproduce:
  1. Open `C:\Users\38909\Documents\images\DSC01169.ARW`.
  2. Reopen the image after the repo-local baked local edit exists.
  3. Inspect the bottom-left Detail status bar.
- Expected result: The status bar should report the logical dimensions of the image the user opened.
- Actual result: The status bar reports `16384x10923`.
- Frequency: Reproducible with the current baked local edit for `DSC01169.ARW`.

## Investigation
- Hypothesis: Persisted local-edit reopens are treating baked pixel-buffer dimensions as the logical display dimensions.
- Checks performed:
  - Located the status-text formatter and `current_image_source_dimensions` plumbing in `src/main.rs`.
  - Parsed the repo-local `local-edits` cache header for the baked `DSC01169.ARW` full copy.
  - Queried Windows shell metadata for the real RAW dimensions.
- Commands run:
  - `Get-ChildItem -Path 'C:\Users\38909\Documents' -Recurse -Filter 'DSC01169.ARW'`
  - PowerShell shell-property probe for RAW dimensions
  - PowerShell binary-reader probe for `local-edits\f4067326cd507ef2.full.rgba`
- Important outputs:
  - Windows shell reports `6656 x 9728` for `DSC01169.ARW`.
  - The baked local-edit full cache for that same source path stores `16384 x 10923`.
- Files inspected:
  - `src/main.rs`
  - `src/decode.rs`
  - `docs/debugging/2026-04-22-detail-dimensions.md`

## Findings
- Root cause: The persisted-local-edit load path currently falls back to the baked file's pixel dimensions because the local-edit cache format does not store separate logical dimensions for the baked result.
- Secondary factors: Existing schema-2 baked local edits can already contain pixel dimensions that do not match the opened image's logical dimensions.
- What was ruled out: The bottom status formatter itself; RAW `source_dimensions()` can report the real logical dimensions for `DSC01169.ARW`.

## Fix
- Change made: Extended the repo-local `local-edits` cache format to schema v3 so baked Full entries persist logical dimensions alongside baked pixels, taught `src/main.rs` to compute those logical dimensions when persisting and restore them through `load_full_image(...)`, kept legacy schema-2 caches readable with a best-effort fallback that preserves baked crop/rotation geometry unless the old baked buffer clearly exceeds the source, rechecked the unlocked Library thumbnail fast path against the matching full generation before returning pixels, made the repair path fail closed when the baked edit disappears mid-retry, and added a zero-dimension guard for the thumbnail-dimension helper.
- Tradeoffs: Legacy schema-2 baked edits are still heuristic because the old cache format never stored logical dimensions, and invalid local-edit cache reads still fail closed instead of deleting by cache path because blind path deletion can race a newer writer.
- Follow-up work: If corrupt local-edit cache files become a real support burden, add a lock-safe cleanup path that proves the on-disk file identity before deletion instead of deleting by cache path after any failed read.

## Validation
- Tests run:
  - `cargo test`
  - `cargo clippy -- -D warnings`
  - `cargo build --release`
- Manual verification: Confirmed the real `C:\Users\38909\Documents\images\DSC01169.ARW` metadata through Windows shell (`6656 x 9728`) and confirmed the stale baked local-edit Full cache header still carried `16384 x 10923`; the new persisted-local-edit load path now restores persisted logical dimensions for new schema-v3 caches and falls back to the source dimensions for this legacy oversized case. I did not rerun a UI screenshot/pixel-diff capture for this text-only status-bar fix.
- Remaining risk: Old schema-2 caches that encode ambiguous geometry are still best-effort until they are rewritten as schema v3 by a later persist, and corrupt local-edit cache files now remain on disk until a later overwrite or manual cleanup because read-time deletion stays disabled to avoid racing a newer writer.

## Cleanup
- Temporary files to remove: None.
- Notes to keep in `docs/learning/lessons.md`: Persisted local-edit caches need their own logical-dimensions contract; baked pixel size is not always the user-facing image resolution.
