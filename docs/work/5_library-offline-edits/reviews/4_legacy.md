# Review — library-offline-edits, 2026-06-11, iteration 5 (final verification of the iteration-4 remedies)

Reviewers: Claude (fable-5, effort max, codebase-grounded), Gemini 3.1 Pro (plan mode), both on the full pre-commit v0.2.4 diff.

## Findings

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| Claude 5-M1 | MEDIUM | The 4-M2 lost-commits notice false-fires on every RE-absorption: the reset replaces the history with defaults but never updates `CompletedBake.state`, so a second eviction-reopen of the same bake compares the recorded non-default state against the now-default history and warns although nothing was lost — recurring on every idle revisit for the rest of the session. | Fixed along the reviewer's direction: `reset_absorbed_session_state` normalizes the record's state to default after the reset (post-absorption the absorbed state IS the identity relative to the re-anchored base; the reset is the field's only reader, grep-verified). The first-absorption semantics — including the edge where mid-reload commits net back to default — are untouched. The existing clean-absorption test now runs a second absorption round and asserts silence both times (red-first against the defect). |
| Claude 5-L (coverage) | LOW | No second-absorption-round test existed (the 5-M1 blind spot) and clean absorptions never asserted `save_status.is_none()`. | Both added in the same test extension. |
| Gemini 5-* | — | No findings: verified the revert re-bake pairing (`last_completed_bakes` vs `loaded_base_generations`, including the racing-adopted-base and session-cache-hit absence scenarios), the always-replace reset with correctly-guarded notice, the owed override's state/base-dimension pairing, and the four new tests. "All invariants are closed and verified. Clean for release." | Approved as-is. |

Claude explicitly verified as holding, with reasoning: the `(Some, None)` predicate arm produces no spurious identity re-bakes and cannot skip a real revert; f32 state equality is bit-stable across the whole capture lineage (no arithmetic, no NaN source); the owed racing comparison and `base_dimensions` recomputation are correct; completed-state capture cannot miss (persists are strictly serialized, at most one completion outstanding); and each iteration-4 regression test fails if its specific fix regresses. Verdict: "No corruption paths found anywhere in the new machinery."

## Outcome

Converged: the only iteration-5 finding was a false-alarm UX defect, fixed with a red-first regression; reviewers are otherwise clean. 349 tests, clippy -D warnings, fmt --check, debug + release builds green. v0.2.4 ships; thread closed (again) with iterations 3–5 appended to the done artifact trail.
