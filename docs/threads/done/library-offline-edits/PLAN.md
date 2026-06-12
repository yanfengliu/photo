# Implementation plan: library-offline-edits

Target version: 0.2.2 (non-breaking fix). TDD throughout: each step writes failing tests first, then the change, then green.

## Step 1 — F1: candidate keys + fail-open-when-absent (local_edits.rs)

Tests (new, in the modules that own the behavior):
- `candidate_source_path_keys_cover_canonical_verbatim_and_raw_forms` — for an existing file the canonical key leads; for a missing absolute path the verbatim guess (`\\?\…`) and raw forms remain.
- `persisted_local_edit_loads_after_source_file_disappears` — bake under a test repo root, delete the source, assert `load_persisted_local_edit` returns the baked full pixels.
- `persisted_local_edit_exists_after_source_file_disappears`.
- `persisted_local_edit_still_ignored_when_source_metadata_changed` — guard: rewrite source with different content/mtime, loaders return None (fail-closed unchanged).
- `remove_persisted_local_edit_removes_cache_for_missing_source`.

Changes: `candidate_source_path_keys`, candidate-resolving lookups in `load_persisted_local_edit_variant_header`, `load_persisted_local_edit_variant`, `persisted_local_edit_exists`, `remove_persisted_local_edit`, repair write path; `read_validated_local_edit_cache_header` accepts `Option` source state (None skips only the metadata equality); loaders treat missing source as absent (fail-open) instead of `Ok(None)`.

## Step 2 — F1: offline thumbnail + full-image serving (loading.rs)

Tests:
- `library_thumbnail_serves_baked_local_edit_when_source_is_missing` — pixels equal the baked thumbnail.
- `load_full_image_serves_baked_local_edit_when_source_is_missing` — base_source PersistedLocalEdit, fingerprint None.
- Existing stale/generation/repair tests stay green (fail-closed regressions).

Changes: `load_library_thumbnail_base_image` returns base + provenance (`LoadedThumbnailBase { image, base_source }`); offline full load already flows through `load_persisted_local_edit` once Step 1 lands.

## Step 3 — F5: provenance-aware thumbnail handles (app/)

Tests:
- `thumbnail_loaded_does_not_reapply_session_edits_to_baked_base` — history holds state S, ThumbnailLoaded delivers a baked base, handle pixels equal the baked pixels.
- `thumbnail_loaded_applies_session_edits_to_original_base` (existing behavior pinned).
- `persist_completed_with_removed_bake_reloads_original_thumbnail` — Ok(None) completion clears the baked base and yields a reload task (simulated ThumbnailLoaded then shows the original).
- `set_library_thumbnail_updates_stored_base_and_provenance`.

Changes: `LibraryEntry.thumbnail_base_source`, ThumbnailLoaded handler branches on provenance, `refresh_library_thumbnail_for_path` no-ops for baked bases, `set_library_thumbnail_for_path` stores image + provenance, Ok(None) completion path reloads the original thumb asynchronously.

## Step 4 — F2: owed bakes for superseded staged loads (app/)

Tests:
- `commit_during_preview_then_navigate_away_still_bakes_after_full_decode` — staged-load state, slider commit (no persist task), start_load(B), stale ImageLoaded(Ok) for A → persist request enqueued for A with A's state.
- `owed_bake_dropped_when_stale_full_decode_fails` (warn, no persist).
- `owed_bake_skipped_when_state_is_default_and_no_bake_exists`.

Changes: `owed_local_edit_bakes: HashMap<u64, OwedLocalEditBake { path, lens }>`; registration in `start_load` before `begin_request` when outgoing stage blocks save and state is bake-worthy; fulfillment in the stale branch of `Message::ImageLoaded`; warn log when a commit is dropped with no accounting.

## Step 5 — gates, review, docs

- `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`, `cargo build --release`.
- Remove the two `diag_*` tests (reference the real user library; never committed).
- Multi-CLI review (Codex gpt-5.5 xhigh, Gemini 3.1 Pro plan-mode, Claude fable-5 max) on the diff per AGENTS.md; synthesize to `docs/threads/current/library-offline-edits/2026-06-11/1/REVIEW.md`; iterate until nitpicks only.
- Docs: changelog v0.2.2; devlog detailed + summary; ARCHITECTURE.md (loading/local_edits paragraphs: bake authoritative when source absent; thumbnail provenance; owed bakes); drift-log row; decisions.md row; lessons.md evidence-anchored entries (offline tether, silent drop, double apply).
- Version bump 0.2.1 → 0.2.2; commit to main; push.
