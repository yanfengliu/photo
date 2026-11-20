# Review — perf-and-calibration, 2026-06-12, iteration 1

Reviewers: Claude (fable-5, effort max, codebase-grounded — verified against the live tree and git history), Gemini 3.1 Pro (plan mode). Codex quota-blocked until 2026-07-10.

## Findings

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| Claude 1-H1 | HIGH | The recalibrated tone constants did NOT invalidate the decode cache: cached develops store post-tone pixels and the contract hash was just the version (still 6 from v0.2.5), so every already-viewed RAW would keep serving the old (1.0 EV) look indefinitely — the release's headline feature would not reach exactly the photos the user has been looking at, and the changelog's "re-derive automatically" claim was false. | Fixed structurally per the reviewer's stronger suggestion: `decoded_cache_contract_hash_for(version, tone_ev, tone_contrast)` folds the constants into the contract hash, so this retune (and every future one) invalidates stale cached looks without anyone remembering a bump. Test: `decoded_cache_contract_hash_changes_with_tone_constants`. |
| Claude 1-L2 | LOW | Degenerate (empty) crop ranges, unreachable through the UI, got a worse failure mode under the parallel loop: the old serial `y0..y1` produced an empty buffer that downstream consumers reject; the new always-≥1-row loop would index past the image (panic) or save black. | Fixed: an `x0 >= x1 || y0 >= y1` guard reproduces the old empty-buffer outcome exactly. Test: `render_returns_empty_pixels_for_degenerate_crop_bounds`. |
| Claude 1-L3 | LOW | The constants comment overclaimed "minimize both the mean and the worst-case": (0.85, 75) holds the best mean (35.9 vs 36.0); the chosen cell is the worst-case minimum with the mean within 0.1, tie-broken on shadow clipping (devlog stated this honestly; the comment didn't). | Fixed: comment now states the worst-case-first criterion; DESIGN/devlog wording verified consistent. The changelog's "beats the old on both" stands (it compares against the old constants). |
| Claude 1-info | INFO | Methodology notes: reference stats use the native-resolution preview vs ≤1400 px candidates (slight bias toward higher-contrast candidates); clip fractions are printed but not in the score (tiebreak applied manually). | Documented as caveats in the harness doc comment for future retunes. |
| Gemini 1-1 | NOTE | The `needs_blur` gate tightly couples `render_edited_image` to which `EditState` fields consume the blur atlas; a future blur-consuming adjustment must join the gate or silently receive zeros. | Documented at both sides: the gate comment in `render_edited_image` and the `blurred` parameter doc on `apply_all`. |
| Gemini 1-2 | LOW | The new identity fast paths lacked dedicated zero-identity unit tests (vibrance had none; the others already had them). | Fixed: `apply_vibrance_zero_is_identity` (exact equality). Exposure/contrast/saturation/tone-zones zero-identity tests already existed and pass through the new early-returns. |

## Verified clean (with evidence)

Both reviewers independently confirmed the parallel chunking math exact (output length is a whole multiple of the row stride, so the last partial chunk drops nothing; y-mapping covers `y0..y1` exactly once across non-divisible heights, height < threads, and nonzero crop origins; per-pixel math untouched → byte-identical and deterministic). Claude additionally verified: the blur gate's predicate agrees exactly (including `-0.0`) with the only two readers of `blurred` in `apply_all`; the tone-zones early return is a TRUE no-op (2⁰ = 1.0 exactly), with saturation/vibrance within 1 ULP pre-quantization and inside the formula-level parity tolerance; scoped-thread captures are `Sync`-sound with no interior mutability; and the histogram stats are behaviorally exact vs the old sorted-index percentiles — Gemini noted the rewrite also fixed a latent f32 precision bug in the old percentile index above ~16.7 MP. Calibration methodology judged sound by both (L1 percentile distance, mean + worst-case aggregation, 35-image set).

## Outcome

Converged at iteration 1: one HIGH (cache invalidation — exactly the class of miss the review pipeline exists for), fixed structurally with a regression test; the LOW/NOTE items all addressed in the same pass. 357 tests, clippy -D warnings, fmt --check, debug + release builds green.
