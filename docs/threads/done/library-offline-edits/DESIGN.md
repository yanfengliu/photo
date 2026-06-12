# Library reflects committed edits without the source media (library-offline-edits)

Date: 2026-06-11. Reported by user: "library view should have the thumbnails of the updated and edited images after I exit details view and upon startup."

## Diagnosis (evidence)

The user's library is 17 Sony ARW files on a camera SD card (`E:\DCIM\100MSDCF\`). A valid bake for `DSC09218.ARW` exists in repo-local `local-edits/` (written 2026-06-11 08:29; header source-metadata matches the source exactly). A diagnostic run of `load_library_thumbnail_base_image` against the real library returned `None` for every entry because the card was unplugged: with the source offline, `std::fs::canonicalize` fails so `normalized_source_path_key` falls back to the raw path string, which hashes to a different cache filename (`bbfe78d06710080f` vs the real `8b04dd73ae4a9d4d`), and `source_file_state` returns `None` which makes every loader return `Ok(None)` before even opening the cache file. The thumbnail fallback `decode::decode_thumbnail` also needs the source, so library slots end up blank. The store that exists precisely to make edits durable is unreachable exactly when it is needed most.

## Findings

- **F1 — local-edit store tethered to live source.** (a) Cache key derivation depends on source reachability (canonicalize fallback changes the hash). (b) All cache reads hard-require `source_file_state(path)`; a missing source is treated like an invalid cache. (c) The thumbnail/full fallbacks decode the source, so offline entries render blank. (d) `persisted_local_edit_exists`/`preferred_base_image_source` resolve to `Original` offline, so Detail open tries (and fails) to decode the missing source instead of opening the bake.
- **F2 — silent bake drops during staged loads.** Commits made while `detail_load.blocks_save()` (RAW embedded-preview stage) produce no persist request. The catch-up in `Message::ImageLoaded` is gated on `is_current_request`, so navigating to another image before the full develop lands drops the bake silently; edit histories are session-only, so the edit is lost at exit.
- **F5 — double-applied edits in thumbnails.** `Message::ThumbnailLoaded` renders the session edit state on top of the returned base. When that base is the bake (which already contains those edits) the state applies twice (e.g. exposure doubled). Reachable by editing soon after import while thumbnail jobs are still in flight, since `load_library_thumbnail_base_image` starts preferring the fresh bake. Relatedly, `LocalEditPersistCompleted(Ok(None))` (bake removed because edits reset) refreshes from a possibly-baked cached base instead of reloading the original.

## Decision: the bake is authoritative when the source is absent

- **Source present + metadata matches** → use bake (unchanged).
- **Source present + metadata differs** → source was rewritten; bake is stale; fail closed to the new original (unchanged).
- **Source absent (metadata unreadable)** → fail open to the bake. The bake holds exactly the pixels the user last committed; with no source on disk there is nothing newer to contradict it, and the alternative is showing nothing.

## Design

1. **Reachability-independent cache keys** (`local_edits.rs`). New `candidate_source_path_keys(path)`: `[canonicalize(path) (when it succeeds), verbatim guess (`\\?\` + absolute path), raw path string]`, deduped in that order. A resolver picks the first candidate whose cache file exists (else the primary candidate, for writes/removes). All lookup, exists, remove, and repair paths go through the resolver. The write path keeps canonicalize-first keys (persist requires source metadata anyway). The embedded path-key check inside the header still validates against the candidate that located the file, so cross-key collisions stay impossible.
2. **Fail-open header validation when source absent** (`local_edits.rs`). `read_validated_local_edit_cache_header` takes `Option<(size, mtime_secs, mtime_nanos)>`; `None` skips only the source-metadata equality check (magic/schema/lengths/path-key checks remain). Loaders treat `source_file_state == None` as "absent", not "invalid".
3. **Thumbnail provenance** (`loading.rs`, `app/`). `load_library_thumbnail_base_image` returns the base plus its `BaseImageSource`. `LibraryEntry` stores `thumbnail_base_source`. Handle construction applies session edit state only to `Original` bases; `PersistedLocalEdit` bases are displayed as-is (the persist pipeline owns delivering newer-edit thumbnails via `set_library_thumbnail_for_path`, which now also updates the stored base + provenance). `LocalEditPersistCompleted(Ok(None))` reloads the original thumbnail asynchronously instead of re-rendering a possibly-baked cached base.
4. **Owed bakes for superseded staged loads** (`app/`). `start_load` snapshots an owed bake (path + lens correction at that moment) keyed by the superseded request id whenever the outgoing image was still in a blocks-save stage and its state is bake-worthy. The stale-`ImageLoaded` branch fulfills the owed bake: on `Ok(loaded)` it enqueues a persist built from the loaded full image plus that path's current session edit state; on `Err` it drops the entry with a warning. Each in-flight decode terminates in exactly one `ImageLoaded`, so the registry stays bounded.

## Systematic prevention (user-requested)

- Tests simulate the failure modes end-to-end: bake → delete source → assert every loader (thumbnail base, full image, exists, preferred source) serves the bake; commit-during-preview → navigate away → stale ImageLoaded → assert the bake still lands; baked-base ThumbnailLoaded with live session state → assert pixels are not double-rendered.
- A warning log fires whenever a commit produces neither a persist request, an owed registration, nor a legitimate no-op, so future silent-drop regressions are visible in logs instead of invisible.
- `docs/learning/lessons.md` gains evidence-anchored entries (offline-tether, silent-drop, double-apply) so re-reviewers check these invariants on future cache/persist changes; `decisions.md` records the fail-open-when-absent rule.

## Out of scope

- Thumbnails for never-edited offline sources (nothing exists locally to show; decoded-cache fallback rejected — entries are full-resolution RGBA, ~700 MB reads per thumbnail).
- Park-bench `%LOCALAPPDATA%\photo\decoded-cache` leftover from a pre-repo-cache build; harmless, not touched.
- Export location (`_edited` files written next to the source) — separate objective `repo-local-exports`.
- RAW develop tone ("duller than Lightroom") — separate objective.
