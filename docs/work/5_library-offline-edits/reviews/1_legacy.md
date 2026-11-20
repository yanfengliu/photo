# Review — library-offline-edits, 2026-06-11, iteration 2

Diff under review: full working tree — iteration-1 fixes (1-H1 base-source seeding, 1-M2 stale-preview rescue, 1-M3 identity re-bake skip, 1-L5 tri-state fail-open + cfg(windows) key guess) plus the repo-local-exports change flagged by iteration 1's scope note. 338 tests at dispatch.

Reviewers: Gemini 3.1 Pro (plan mode). Claude unavailable for this iteration — the CLI hit its session usage limit immediately after its heavyweight iteration-1 pass (resets 13:10 America/Los_Angeles); Codex remains quota-blocked until 2026-07-10. Per AGENTS.md's unreachable-CLI clause the iteration proceeds on the remaining reviewer plus fix-specific regression evidence; a post-hoc Claude pass can be run after the reset if desired.

## Findings

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| Gemini 2-* | — | No findings. Explicitly verified each iteration-1 fix: `fulfill_owed_local_edit_bake` seeds `base_image_sources` ("preventing a corrupting double-apply on the next bake"); the stale-`ImagePreviewLoaded` rescue chains the full decode ("a precise fix for the data-loss path"); `owed_bake_has_nothing_new` keeps the pending-reset case registered while filtering identity re-bakes; tri-state fail-open "maintains the safety guarantee that a reachable-but-changed source invalidates the bake"; candidate-key purge-all-on-remove prevents ghost-bake resurrection; repo-local exports + fallback + dir creation work as intended; called the 12 (now 16) new tests "high-signal". | Approved as-is. |

## Fix-evidence summary (compensating for the missing heavyweight reviewer)

- 1-H1: `fulfilled_owed_bake_seeds_base_image_source_for_reopen` persists the owed bake to a real cache file and asserts the reopen path resolves `Original` — the exact corruption sequence from the finding. A multiline grep audit confirms `base_image_sources` is mutated in production code only by the current-request `ImageLoaded` arm and the new fulfillment seed.
- 1-M2: `stale_preview_completion_still_fulfills_owed_bake_via_chained_full_decode` drives the real message loop through supersede-during-`Loading` → stale preview → chained `ImageLoaded` → persist enqueued. The spawn line itself is not directly assertable (iced `Task` is opaque); the owed-map retention plus end-to-end fulfillment pin the contract. Duplicate-decode audit: a stale preview can only exist for requests whose full task was never spawned (non-staged loads never produce previews; a current preview consumes itself when chaining the follow-up), so the rescue cannot double-spawn.
- 1-M3: `navigation_owes_nothing_for_unedited_baked_image` (registration guard) and `owed_bake_skipped_for_unedited_baked_image` (fulfillment guard, which still seeds the base map before skipping).
- Ordering race audit for the new seed: all interleavings of A→B→A-fast converge — the owed fulfillment and any newer current-request arm write the same `Original` value unless a current-request load actually completed against the bake, in which case its later insert wins and matches the pixels it loaded.

## Outcome

Converged: no open findings. 338 tests, clippy -D warnings, fmt --check, debug + release builds all green; cargo audit at the established 5-warning baseline (no dependency changes). Task closed; thread moves to done.
