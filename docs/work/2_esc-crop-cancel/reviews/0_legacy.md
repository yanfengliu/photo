# Multi-CLI Review — esc-crop-cancel, iteration 1

Date: 2026-06-10. Scope: diff `deec052..a5fa2d8` (v0.2.1: Escape cancels crop mode and stays in Detail; viewer discards cancelled crop drags). Reviewers: Codex (gpt-5.5, xhigh, live-tree), Claude (claude-fable-5[1m], max effort, Read/Glob/Grep/git; ran the suite itself, 319 green), Gemini (gemini-3.1-pro-preview, plan mode).

## Verdicts

- All three reviewers: the code fix is correct. Claude adversarially verified every Escape-branch combination (context menu while cropping, rename while cropping, crop inside collection Detail, active collection) and the full viewer drag state machine (release-after-cancel, press outside image, cursor leaving bounds, every `crop_mode = false` site, the preview→full upgrade path), and confirmed the gate also fixes a pre-existing hazard where a stale drag could commit onto a newly navigated image. Codex independently confirmed the ordering and release path against the live tree. Gemini concurred on ordering, state cleanup, and test contracts.
- Findings were entirely docs/process plus one coverage note.

## Findings and dispositions

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| Codex 1-M1 / Claude L1 | MEDIUM/LOW | The v0.2.1 devlog entry referenced this synthesis at its `done/` path before the artifact existed — a dangling audit-trail pointer in the committed tree | Fixed: devlog line rewritten to describe the actual review outcome; the synthesis lands (and the thread moves to `done/`) in the same close-out commit, so the pointer is true at commit time |
| Codex 1-M2 / Claude L2 | MEDIUM/LOW | `docs/devlog/summary.md` stale: still said v0.2.0 and 299 tests (the count had been stale since v0.1.6, spanning the Lightroom work) | Fixed: v0.2.1, 320 tests, a Lightroom+escape milestone line, and an Escape-behavior bullet added |
| Claude L3 | LOW | Changelog 0.2.1 precedence wording lumped menu dismissal in with the "next press" navigation meanings, inverting the actual order for overlays | Fixed: overlays-take-priority and navigation-on-next-press now stated separately |
| Claude INFO | INFO | The CursorMoved purge branch was untested (deletable without failing the suite), and the purge design assumes crop mode can only re-enter via toolbar click (a future keyboard shortcut could resurrect a cancelled anchor) | Fixed: new viewer regression `crop_drag_is_purged_on_cursor_move_after_crop_mode_cancel` pins the branch; the re-entry assumption is documented in the code comment at the purge site |

## Convergence assessment

Zero code defects across three reviewers; all findings fixed (suite now 320). Iteration 1 closes the review.

## Worth keeping

- Claude's verification that `visible_crop()` must be captured *before* flipping `crop_mode` (it returns None while crop mode is on) — the cancel branch mirrors `ToggleCropMode` exactly for this reason.
- The devlog-pointer finding repeats the project-health-overhaul review's lesson: never commit a reference to a thread artifact before the artifact exists; land them atomically.
