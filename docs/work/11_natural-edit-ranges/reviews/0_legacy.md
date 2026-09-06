# Review — natural-edit-ranges iteration 1

Date: 2026-07-14. Scope: RAW downscale-only prerequisite for the supplied-photo calibration loop. Reviewer: in-process independent finder/refuter `raw_no_upscale_refute`. The reviewer was directed to verify every symbol, resize semantic, cache boundary, test assertion, and harness claim against the live codebase rather than approving from the prompt.

## Findings and dispositions

| ID | Severity | Finding | Disposition |
| --- | --- | --- | --- |
| RAW-1 | HIGH | `raw_dynamic_image_to_rgba` called image 0.25's upscale-capable `thumbnail` unconditionally, turning a 9504×6336 fixture into 16384×10923 and persisting the oversized decode. | Fixed with a downscale-only guard; decoded-cache contract 6→7 invalidates prior accidental upscales. The helper's red test reproduced 2×1→10×5, then passed at 2×1 with exact pixels. |
| RAW-2 | MEDIUM | Existing orientation tests compared rotated output with a normal output that suffered the same upscale, so they could agree while both dimensions were wrong. | Fixed by pinning embedded preview 2×1→1×2 and full develop 24×12→12×24 while retaining exact rotated-pixel comparisons. |
| RAW-3 | LOW | The helper comment and test name described only the GPU cap even though `max_dim` also carries thumbnail bounds. | Fixed: wording and test name now use the general requested-bound contract. |

## Verification and verdict

The decode test group passes. Fresh sandboxed harness run `20260714-111324Z-14504` observed buffer 9504×6336 and render-stat dimensions 9504×6336 for the affected RAW, versus 16384×10923 in the prior run; logical dimensions remain 9728×6656. The reviewer re-read the final diff and reported no substantive issue. Iteration 1 is clean for this coherent prerequisite; GPU transfer and tone-range findings remain open in the parent objective.
