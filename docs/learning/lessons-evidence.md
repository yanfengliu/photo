# Lessons — evidence

The war story and the evidence-anchor table behind each linked line in [lessons.md](lessons.md). Not session-start reading: open an entry when its rule is in doubt, or when the work is in that area.

## "Tracked follow-up" duplication debt is not inert

**Date:** 2026-07-08

"Tracked follow-up" duplication debt is not inert: it silently breaks every NEW invariant added to the surviving copy. Repo-root discovery existed twice (repo.rs + decode.rs's private copy, flagged "dedup is a tracked follow-up" since the 2026-06-10 split); when the harness added a runtime sandbox override to repo.rs's copy, decode.rs's copy ignored it by construction and sandboxed harness sessions read/wrote/pruned the REAL decoded-cache/. When adding an invariant to a resolver/helper known to be duplicated, either dedup first (it's now load-bearing) or grep every duplicate for the new contract — a follow-up note is not a guard. The refuting-verifier pass is what caught it: same-model finders + verifiers grounded in the live code, not the diff.

| Field | Value |
|---|---|
| Surfaced by | docs/threads/done/agent-harness/2026-07-08/1/REVIEW.md (in-process adversarial workflow, sandbox dimension) |
| Reviewer findings | Workflow finder + refuting verifier, CONFIRMED HIGH ("Sandbox hole: decoded-cache/ still reads, writes, and prunes the REAL repo cache during sandboxed harness sessions") |
| Fix commit | 36975dd (v0.2.8) |
| Test added | decode::tests::decoded_cache_resolution_delegates_to_the_shared_repo_resolver (failing-first) |
| Behavior delta | before: `photo --harness` (sandboxed by default) warmed/pruned the real repo's decoded-cache/ — a harness session could evict the user's real cached RAW develops and pollute the cache with fixture entries; after: all cache roots resolve through repo.rs's single owner, so the sandbox override holds for every repo-local directory |

## Async completions that outlive their client must carry connection identity

**Date:** 2026-07-08

Async completions that outlive their client must carry connection identity, or reconnect + reused request ids turns them into silent wrong answers: harness screenshot/dump/stats completions held only a request id; a client timing out mid-render and reconnecting (harnessctl restarts ids at 1 every invocation) could receive the PREVIOUS command's stale result as the answer to a different command — the agent would conclude "the slider had no effect" from a pre-change render, corrupting the exact feedback loop the harness exists to provide. Tag in-flight completions with a connection generation at dispatch and drop stale ones at delivery (keep their artifacts — the files are real); ids correlate within a connection, never across connections.

| Field | Value |
|---|---|
| Surfaced by | docs/threads/done/agent-harness/2026-07-08/1/REVIEW.md (in-process adversarial workflow, server-concurrency dimension; found independently by two finders) |
| Reviewer findings | Workflow finders + refuting verifier, CONFIRMED MEDIUM ("Stale in-flight async completion is delivered to the next connection and mis-correlated due to harnessctl's fixed id=1") |
| Fix commit | 36975dd (v0.2.8) |
| Test added | app::harness_tests::stale_async_completions_are_dropped_not_misdelivered |
| Behavior delta | before: `harnessctl dump_render` timeout → retry → the retry can print the pre-change RenderReport with ok:true (deterministic id collision), and the stale render's stats masquerade as current; after: stale completions are logged and dropped, the retry's own render answers, verified live across four reconnections |

## Fixing an invariant for the path in your diff without auditing the sibling paths that share it

**Date:** 2026-06-11

Fixing an invariant for the path in your diff without auditing the sibling paths that share it leaves the same failure class open: v0.2.2 closed "session edit state must stay relative to its recorded base" for decodes whose base was the Original, and the post-hoc reviewer found the identical corruption alive for bake-as-base reloads (edit over an existing bake → session-cache eviction → reopen loads the new bake → state re-applies and re-bakes, compounding). When a review fix closes an invariant violation, enumerate every site that loads/produces the artifacts the invariant relates (here: every `ImageLoaded` consumer) before declaring it closed — audit the invariant, not the diff. The structural fix was making absorption observable (persists report their written generation; reloads matching `last_baked_generations` reset the absorbed state).

| Field | Value |
|---|---|
| Surfaced by | Post-hoc verification pass, docs/threads/done/library-offline-edits/2026-06-11/3/REVIEW.md (finding 3-H1) |
| Reviewer findings | Claude post-hoc HIGH 3-H1 (+ MEDIUM 3-M2 mainline identity re-bakes, same predicate-reuse fix) |
| Fix commit | (library-offline-edits follow-up commit, v0.2.4) |
| Test added | app::tests::reload_of_a_bake_that_absorbed_the_session_edits_resets_history_instead_of_reapplying, app::tests::reload_of_an_older_bake_keeps_session_state_while_the_newer_persist_is_pending, app::tests::owed_fulfillment_skips_when_the_loaded_bake_already_absorbed_the_state |
| Behavior delta | before: edit a previously-baked photo, browse 4+ photos, return — edits render doubled on screen and the bake compounds (S², S³…) on every revisit, with exports corrupted identically; after: the reload recognizes the absorbed state, the image shows the committed look once, and the bake stays byte-stable |

## A durable store keyed through the live source file inverts durability exactly when it matters

**Date:** 2026-06-11

A durable store keyed through the live source file inverts durability exactly when it matters: the baked local-edit cache derived its filename from `fs::canonicalize(source)` and refused every read without fresh source metadata, so unplugging the camera card made committed edits unreachable (different key hash AND fail-closed validation) while the store sat intact on disk. Derive cache keys reachability-independently (candidate forms: canonical, parent-canonical+name, verbatim `\\?\` guess, raw), and distinguish source ABSENT (fail open to the last committed artifact — nothing newer exists to contradict it) from source CHANGED (fail closed — the bake no longer describes it). The same inversion hid in `load_library()` filtering entries on `Path::exists()`: a transient unplug emptied the library and the next save made the eviction permanent — never destructively filter user-intent data on transient environment state.

| Field | Value |
|---|---|
| Surfaced by | User report 2026-06-11 + live diagnosis in docs/threads/done/library-offline-edits/DESIGN.md (bake for E:\DCIM\100MSDCF\DSC09218.ARW present and valid, loader returned None for all 17 library entries with the card unplugged) |
| Reviewer findings | Gemini iter-1: approved, no defects; Claude iter-1: see thread REVIEW.md; Codex unreachable (usage limit) |
| Fix commit | (library-offline-edits commit, v0.2.2) |
| Test added | app::tests::library_thumbnail_serves_baked_local_edit_when_source_is_missing, app::tests::load_full_image_serves_baked_local_edit_when_source_is_missing, app::tests::persisted_local_edit_loads_after_source_file_disappears, app::tests::remove_persisted_local_edit_removes_cache_for_missing_source, app::tests::parsed_library_content_keeps_offline_paths |
| Behavior delta | before: startup with the card unplugged showed an empty library (entries evicted) and re-plugging couldn't restore edited thumbnails baked under the canonical key; after: all entries persist and the edited photo renders its baked 200x133 thumbnail offline (verified end-to-end against the real library and bake) |

## Async edit pipelines need two explicit invariants or they silently corrupt or lose work under timing

**Date:** 2026-06-11

Async edit pipelines need two explicit invariants or they silently corrupt or lose work under timing timing: (1) every commit must be accounted for — baked now, recorded as an obligation (owed-bake registry fulfilled by the stale decode completion), or a legitimate no-op; a guard that just skips persistence ("full image not ready yet") is a data-loss path the moment the user navigates away. (2) Any cached artifact that may or may not already contain a transformation must carry provenance — `ThumbnailLoaded` re-rendered session edit state onto whatever base arrived, which double-applied edits whenever the base was already baked (race: edit quickly while slow import thumbnail jobs are still in flight).

| Field | Value |
|---|---|
| Surfaced by | blocks_save()/is_current_request analysis during the library-offline-edits diagnosis (docs/threads/done/library-offline-edits/DESIGN.md F2/F5) |
| Reviewer findings | Gemini iter-1: approved (called the owed-bake registry "a precise solution"); Claude iter-1: see thread REVIEW.md |
| Fix commit | (library-offline-edits commit, v0.2.2) |
| Test added | app::tests::commit_during_preview_then_navigate_away_still_bakes_after_full_decode, app::tests::owed_bake_dropped_when_stale_full_decode_fails, app::tests::owed_bake_skipped_when_state_is_default_and_no_bake_exists, app::tests::thumbnail_loaded_does_not_reapply_session_edits_to_baked_base |
| Behavior delta | before: a slider commit during a multi-second RAW develop + arrow-key navigation never baked (the edit died with the session), and a quick edit after import could brighten a thumbnail twice (exposure applied in the bake and re-applied at render); after: the bake lands when the superseded decode completes, and baked bases render as-is |

## Peak-position tests and direction-only assertions cannot catch zone-slider leakage

**Date:** 2026-06-10

Peak-position tests and direction-only assertions cannot catch zone-slider leakage: a Gaussian tone band that peaks in exactly the right place can still read as a global exposure shift if its tails are too wide (σ=√2 carries weight 0.37 two stops from center, so Highlights -100 moved EV-3 midtones by -0.74 EV). Pin band ISOLATION with ratio bounds at off-center EVs, pin the per-band reach at the center, and verify slider extremes with rendered images plus summary metrics — the leak was invisible to every existing unit test and obvious in one render.

| Field | Value |
|---|---|
| Surfaced by | tmp/param-tuning render review during the lightroom-param-parity thread (docs/threads/done/lightroom-param-parity/DESIGN.md) |
| Reviewer findings | n/a — caught by the visual-verification step before review |
| Fix commit | (lightroom-param-parity commit, v0.2.0) |
| Test added | edit::tests::tone_zone_bands_stay_isolated_from_midtones, edit::tests::tone_zone_full_slider_reaches_1_5_ev_at_band_center, edit::tests::tone_zone_adjacent_bands_still_overlap_smoothly |
| Behavior delta | before: Highlights -100 on a bright photo darkened the whole frame (mean luma 0.62 → 0.36 on the test render); after: midtones move ≤ ~0.2 EV, band centers move exactly 1.5 EV, and the slider reads as targeted highlight recovery (mean 0.44 with the subject intact) |

## Tests that rewrite a file with same-length content race the filesystem timestamp tick

**Date:** 2026-06-10

Tests that rewrite a file with same-length content race the filesystem timestamp tick the filesystem timestamp tick: the persisted decode cache's metadata fast path treats same-size+same-mtime as unchanged by design, so on a fast machine both writes can land in one Windows file-time tick and the change is invisible. Make such tests force an observable change (verify the mtime advanced after the rewrite, retrying briefly) instead of assuming any write changes metadata — and expect this class of latent assumption to surface exactly when something else gets faster (here, the optimized-dependency test speedup).

| Field | Value |
|---|---|
| Surfaced by | GitHub Actions CI run 27305008375 (first run after the v0.1.4 test speedup); local suite passed all day |
| Reviewer findings | n/a — CI-caught, not reviewer-caught |
| Fix commit | (commit H, v0.1.6) |
| Test added | decode::tests::rewrite_with_distinct_mtime_makes_same_size_rewrites_observable |
| Behavior delta | before: `cached_full_image_redocodes_when_source_file_changes` flaked on fast runners (decode_calls 1 vs 2 — stale cached pixels served after a real content change in the test scenario); after: the rewrite is guaranteed metadata-observable and the suite is deterministic at 299 tests |

## `cargo fix` is not reliably multi-target-aware

**Date:** 2026-06-10

`cargo fix` is not reliably multi-target-aware: after the main.rs module split it removed imports (`use repo::*;`, `std::io::{BufWriter, Write}`, `std::fs::File`) that only the `cfg(test)` target used, leaving the bin target green while `cargo test` failed with 17 resolution errors. After any `cargo fix` run, immediately re-verify with `cargo check --all-targets`, and prefer `#[cfg(test)]`-gated imports for test-only names so the two targets cannot drift.

| Field | Value |
|---|---|
| Surfaced by | docs/devlog/detailed/2026-06-10_2026-06-10.md (commit D entry); caught live during the split |
| Reviewer findings | n/a — caught during implementation, before review |
| Fix commit | b00919b |
| Test added | n/a — process lesson; the existing 298-test suite is the detector |
| Behavior delta | without the re-added cfg(test) imports the test binary failed to compile (E0425/E0433 x17) while `cargo check` on the bin passed, so a bin-only check would have shipped a broken test build |
