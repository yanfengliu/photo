# Gate proofs

Every lesson this repo retired left behind a gate, and every one of those gates was made to go red by reintroducing the defect the lesson describes before its prose was deleted.

This file is the standing answer to "did the gates actually do their job". A claim here that cannot be reproduced by re-running its mutation is a claim to fix, not a claim to trust.

Method: apply the smallest edit to product code that reintroduces the defect, run `cargo test --bin photo` (green at baseline: 435 passing), confirm the failure names the defect, revert byte-for-byte, confirm green. Proved 2026-09-02.

Four gates recorded below were rewritten or newly written during this pass because the mutation proved the existing test did not cover its lesson at all. Those are marked **was not gated**.

## "Tracked follow-up" duplication debt is not inert: it silently breaks every NEW invariant added to the surviving copy

- **Gate:** `src/decode.rs` :: `decode::tests::decoded_cache_resolution_delegates_to_the_shared_repo_resolver` — run by `cargo test`
- **Mutation:** replaced `crate::repo::photo_repo_root()` in `decode::photo_repo_root` with a private re-implementation walking `current_exe`/`current_dir`, as decode.rs once carried
- **Red:** `left: "C:\Users\38909\Documents\github\photo\decoded-cache"` / `right: "…\Temp\.tmpXaWxN8\decoded-cache"` — the sandboxed session resolving to the real repo cache
- **Green after revert:** yes

## Async completions that outlive their client must carry connection identity

- **Gate:** `src/app/harness_tests.rs` :: `app::harness_tests::stale_async_completions_are_dropped_not_misdelivered` — run by `cargo test`
- **Mutation:** deleted the generation check in `App::respond_harness_result_if_current`, so completions carry only a request id
- **Red:** `expected no queued harness response` — the previous connection's completion delivered to the next client
- **Green after revert:** yes

## A reload that already absorbed the session's edits must reset them, not re-apply and re-bake them

- **Gate:** `src/app/tests.rs` :: `app::tests::reload_of_a_bake_that_absorbed_the_session_edits_resets_history_instead_of_reapplying` (+ `absorbed_reload_clears_stale_redo_state`, `absorbed_reload_with_unbaked_commits_resets_and_surfaces_a_notice`, `owed_fulfillment_skips_when_the_loaded_bake_already_absorbed_the_state`) — run by `cargo test`
- **Mutation:** forced `App::loaded_bake_absorbed_session_state` to `false`, making absorption unobservable
- **Red:** `the loaded bake already contains the committed edits; keeping the session state would render and re-bake them twice`
- **Green after revert:** yes

## A durable store keyed through the live source file inverts durability exactly when it matters

- **Gate:** `src/local_edits.rs` / `src/app/tests.rs` :: `app::tests::persisted_local_edit_loads_after_source_file_disappears`, `library_thumbnail_serves_baked_local_edit_when_source_is_missing`, `load_full_image_serves_baked_local_edit_when_source_is_missing`, `remove_persisted_local_edit_removes_cache_for_missing_source` — run by `cargo test`
- **Mutation:** reduced `candidate_source_path_keys` to the single canonicalized key, which resolves only while the source file exists
- **Red:** `baked local edit should stay loadable when the source is offline`
- **Green after revert:** yes

## Never destructively filter user-intent data on transient environment state

- **Gate:** `src/library.rs` / `src/app/tests.rs` :: `app::tests::parsed_library_content_keeps_offline_paths` — run by `cargo test`
- **Mutation:** added `.filter(|path| path.exists())` to `library::parse_library_content`
- **Red:** `library membership is user intent; offline media must not evict entries` — `left: []`, `right: ["E:\DCIM\100MSDCF\DSC09218.ARW", "C:\does\not\exist.png"]`
- **Green after revert:** yes

## Every commit must be accounted for — baked now, recorded as an obligation, or a legitimate no-op

- **Gate:** `src/app/tests.rs` :: `app::tests::commit_during_preview_then_navigate_away_still_bakes_after_full_decode` (+ six `owed_fulfillment_*` / `fulfilled_owed_bake_*` tests) — run by `cargo test`
- **Mutation:** made `App::register_owed_local_edit_bake_for_superseded_load` return immediately — the "full image not ready yet, skip persistence" guard
- **Red:** `the stale full decode must still bake the committed edit`
- **Green after revert:** yes

## Any cached artifact that may or may not already contain a transformation must carry provenance

- **Gate:** `src/app/tests.rs` :: `app::tests::thumbnail_loaded_does_not_reapply_session_edits_to_baked_base` — run by `cargo test`
- **Mutation:** made the `ThumbnailLoaded` arm render session edit state onto whatever base arrived, ignoring `base_source`
- **Red:** that test alone, out of 431
- **Green after revert:** yes

## Peak-position tests and direction-only assertions cannot catch zone-slider leakage

- **Gate:** `src/edit.rs` :: `edit::tests::tone_zone_bands_stay_isolated_from_midtones` — run by `cargo test`
- **Mutation:** `TONE_ZONE_SIGMA_SQ_2` from `2.0` back to `4.0` (darktable's σ = √2 default)
- **Red:** `highlights leak into EV -3 midtones: ratio 1.4659331`
- **Green after revert:** yes

## Tests that rewrite a file with same-length content race the filesystem timestamp tick — **strengthened**

- **Gate:** `src/decode.rs` :: `decode::tests::rewrite_with_distinct_mtime_makes_same_size_rewrites_observable` — run by `cargo test`
- **Mutation:** replaced the `rewrite_with_distinct_mtime` verify-and-retry loop with a plain `std::fs::write`
- **Was insufficient:** as written the test did one rewrite, so it reproduced the same-tick collision only sometimes — red on 2 of 3 runs against the mutation. A gate that fails two times in three is a flake, not a gate. It now hammers 40 back-to-back same-length rewrites; the mutation is red 5 of 5 runs.
- **Red:** `same-length rewrite 0 left the file metadata unchanged; the persisted cache would keep serving the pre-rewrite pixels` (`SystemTime { intervals: 134328407710626347 }` on both sides)
- **Green after revert:** yes

## Keep Rust and WGSL uniform-buffer layouts locked together with explicit padding — **was not gated**

- **Gate:** `src/viewer.rs` :: `viewer::tests::uniforms_layout_matches_wgsl_uniform_buffer` — run by `cargo test`
- **Was not gated:** the test asserted `size_of::<Uniforms>() == 240` and four hard-coded Rust field offsets. It described one side of a two-sided contract. Adding `new_knob: f32` to the WGSL `Uniforms` struct with no Rust counterpart — the exact defect class the lesson names — left **all 431 tests green**; the divergence would have surfaced only as a fatal `wgpu` validation panic on the first frame at runtime. Rewritten to parse image.wgsl's struct, compute its std140 layout, and assert size, field names, field order and every field offset against the Rust struct, keeping the pinned constants as a second line.
- **Mutation A:** added `new_knob: f32` to image.wgsl only
- **Red A:** `image.wgsl's uniform fields and the Rust struct's fields disagree in name or order` with both lists printed
- **Mutation B:** swapped `crop_overlay_enabled` and `output_needs_srgb_encode` in image.wgsl only
- **Red B:** same assertion, showing the reordered pair
- **Mutation C:** deleted the Rust `_pad_before_crop_preview` alignment field
- **Red C:** `the Rust Uniforms struct is 236 bytes but image.wgsl's is 240; wgpu writes the Rust bytes into a buffer the shader reads with the WGSL layout, so every field past the divergence is garbage`
- **Green after revert:** yes

## Persisted caches live in a visible repo-local directory, not a hidden or profile-scoped path — **was not gated**

- **Gate:** `src/repo.rs` :: `repo::tests::repo_local_stores_stay_visible_and_repo_relative` and `repo::tests::every_repo_local_store_follows_the_overridden_repo_root` — run by `cargo test`
- **Was not gated:** the two lessons claiming this each pointed at tests of the form `assert_eq!(cache_dir_for(root), root.join(DECODE_CACHE_DIR_NAME))` — asserting a path against the very constant that defines it. Renaming both `DECODE_CACHE_DIR_NAME` and `LOCAL_EDIT_CACHE_DIR_NAME` to dot-directories left **all 431 tests green**. The new gate asserts the shape of the values themselves, across every repo-local store.
- **Mutation A:** `DECODE_CACHE_DIR_NAME = ".photo-cache"`
- **Red A:** `DECODE_CACHE_DIR_NAME is ".photo-cache"; a leading dot hides the store from an ordinary directory listing`
- **Mutation B:** `DECODE_CACHE_DIR_NAME = "AppData/Local/photo/cache"`
- **Red B:** `a repo-local store is one directory directly under the repo root` (`left: 4` components)
- **Green after revert:** yes

## Feed the same rotated, pixel-snapped rectangle into preview fit, actual-size zoom, status text and save — **was not gated**

- **Gate:** `src/app/tests.rs` :: `app::tests::every_display_geometry_leg_reads_the_same_rotated_snapped_rectangle` — run by `cargo test`
- **Was not gated:** the status-text and actual-size-zoom legs were covered, but `App::fit_scale_for_rotation_and_crop` — the preview-fit leg — was not. Making it read the pre-rotation dimensions left **all 431 tests green**. `actual_size_zoom_for_rotation_and_crop` is literally `1.0 / fit_scale_for_rotation_and_crop`, so every test comparing the two agreed no matter how wrong both were. The new gate uses an asymmetric case with an expected number: a 200x100 photo turned a quarter turn fits a 300x100 canvas at 0.5, not 1.0.
- **Mutation A:** `fit_scale_for_rotation_and_crop` reads `(img.width, img.height)` instead of `rotated_dimensions(...)`
- **Red A:** `left: 1.0`, `right: 0.5`
- **Mutation B:** `display_dimensions_for_edit_state` drops the rotation
- **Red B:** `rotation swaps the logical dimensions` — plus five pre-existing status-bar tests
- **Green after revert:** yes

## Crop geometry is snapped to whole pixels, identically for the preview and the file

- **Gate:** `src/edit.rs` :: `edit::tests::crop_rect_snaps_to_pixel_grid_for_preview_and_save_parity`, with `app::tests::every_display_geometry_leg_reads_the_same_rotated_snapped_rectangle` — run by `cargo test`
- **Mutation:** `CropRect::pixel_bounds` rounds instead of flooring the near edge and ceiling the far edge
- **Red:** `left: (33, 200)`, `right: (34, 200)`
- **Green after revert:** yes

## Grid breakpoint math and grid rendering share one card size, driven by the latest window width — **breakpoint half was not gated**

- **Gate:** `src/app/tests.rs` :: `app::tests::grid_breakpoints_and_rendering_share_one_card_size` (new) and `library_grid_uses_latest_window_width_after_returning_from_detail` / `collection_grid_uses_latest_window_width_after_returning_from_detail` (existing) — run by `cargo test`
- **Was not gated:** giving `ThumbnailGridLayout::new` its own thumbnail-size constant, divergent from the `GRID_THUMB_SIZE` the renderer draws, left all tests green — the layout stayed internally consistent about a card nobody draws. The new gate walks exact-fit widths for 1..8 columns and demands the count the shared constant implies.
- **Mutation A:** `card_width` computed from a literal `180.0` instead of `GRID_THUMB_SIZE`
- **Red A:** `a window exactly wide enough for 2 cards of 162px reported a different column count; the breakpoint math is sizing a card the renderer does not draw`
- **Mutation B:** `ThumbnailGridLayout::new` ignores `content_width`
- **Red B:** `expected collection thumbnails to reflow after resizing in detail view`
- **Green after revert:** yes

## In the Windows `iced` UI, symbol-only toolbar glyphs need an explicit symbol font plus `Shaping::Advanced`

- **Gate:** `src/app/tests.rs` :: `app::tests::rotation_controls_use_icon_buttons` — run by `cargo test`
- **Mutation:** dropped `.font(ROTATION_ICON_FONT)` and `.shaping(ROTATION_ICON_SHAPING)` from `widgets::rotation_icon_label`
- **Red:** `left: Font { family: SansSerif, … }`, `right: Font { family: Name("Segoe UI Symbol"), … }` — read back off the real widget through a capturing paragraph, not off the constants
- **Green after revert:** yes

## Thumbnails in fixed square slots need explicit widget-layer containment, locked by draw-bounds tests

- **Gate:** `src/app/tests.rs` :: `app::tests::thumbnail_slot_draws_wide_images_without_stretching`, `…tall…`, `…square_images_at_full_slot_size` — run by `cargo test`
- **Mutation:** `ContentFit::Contain` to `ContentFit::Fill` in `widgets::thumbnail_slot_with_renderer`
- **Red:** `assertion failed: (bounds[0].height - 75.0).abs() < 0.01` — the bounds the renderer is actually handed
- **Green after revert:** yes

## Track the logical base-image dimensions separately from the decoded buffer, and carry them back on the async load

- **Gate:** `src/app/tests.rs` :: `app::tests::status_bar_uses_source_dimensions_when_loaded_buffer_is_scaled`, `persisted_local_edit_reopen_uses_persisted_logical_dimensions_in_status_text`, `session_full_image_cache_hit_restores_cached_source_dimensions`, `image_loaded_recovers_missing_source_dimensions_after_successful_original_load` — run by `cargo test`
- **Mutation:** `App::current_display_dimensions` reads `(img.width, img.height)` instead of `current_image_source_dimensions`
- **Red:** `assertion failed: status.contains("3×2")` — the UI reporting the downscaled buffer as the image size
- **Green after revert:** yes

## Tag a persisted cache's full image and thumbnail with the same generation, and trust a thumbnail only while it matches its full sibling

- **Gate:** `src/app/tests.rs` :: `app::tests::library_thumbnail_ignores_a_generation_mismatch_even_when_dimensions_match`, `library_thumbnail_fast_path_rechecks_generation_before_returning`, `library_thumbnail_ignores_a_same_generation_persisted_thumbnail_when_its_aspect_ratio_disagrees_with_the_full_copy` — run by `cargo test`
- **Mutation:** `local_edits::persisted_thumbnail_matches_generation_and_dimensions` ignores `generation_id`
- **Red:** pixel-for-pixel mismatch between the served thumbnail and the current bake
- **Green after revert:** yes

## Keep persisted-cache repair best-effort: a transient write failure must not blank the visible thumbnail

- **Gate:** `src/app/tests.rs` :: `app::tests::library_thumbnail_still_loads_when_repair_write_fails` — run by `cargo test`
- **Mutation:** propagated the repair write error with `?` instead of logging and continuing
- **Red:** `called Result::unwrap() on an Err value: "simulated repair write failure"`
- **Green after revert:** yes

## Apply RAW orientation consistently to embedded previews, full decode output, and reported source dimensions

- **Gate:** `src/decode.rs` :: `decode::tests::raw_source_dimensions_apply_orientation_metadata`, `decode_raw_image_applies_orientation_metadata`, `decode_embedded_preview_applies_orientation_metadata`, `decode_raw_thumbnail_applies_orientation_metadata` (+ three `raw_dynamic_image_to_rgba_*`) — run by `cargo test`
- **Mutation A:** `raw_source_dimensions` returns `(width, height)` without `oriented_dimensions`
- **Red A:** `left: (24, 12)`, `right: (12, 24)`
- **Mutation B:** full RAW decode skips `apply_raw_orientation` — 7 tests red
- **Mutation C:** embedded preview skips `apply_raw_orientation` — 4 tests red
- **Green after revert:** yes

## Persisted decoded caches need a normalized fingerprint, an explicit contract version, collision-safe temp writes, and cleanup on failed finalization

- **Gate:** `src/decode.rs` :: `decode::tests::cached_full_image_redocodes_when_cache_contract_changes`, `invalid_cache_schema_falls_back_to_a_fresh_decode`, `decoded_cache_temp_paths_are_unique_per_write_attempt`, `write_decoded_cache_removes_temp_file_when_finalize_fails`, `cached_full_image_redocodes_when_source_file_changes` — run by `cargo test`
- **Mutation A:** the reader ignores the schema version and contract hash — 2 tests red (`left: 1`, `right: 2` decode calls)
- **Mutation B:** temp file named `cache-file.tmp` with no pid or counter — `assertion left != right failed: "cache-file.tmp"`
- **Mutation C:** no `remove_file` on failed finalization — `assertion failed: leftover_temp_entries.is_empty()`
- **Green after revert:** yes

## A byte-bounded same-session cache keeps a recent-history floor

- **Gate:** `src/app/tests.rs` :: `app::tests::session_full_image_cache_keeps_two_recent_entries_hot_even_when_they_fill_the_budget`, `reopening_a_recently_viewed_detail_image_reuses_the_session_memory_cache` — run by `cargo test`
- **Mutation:** dropped `self.entries.len() > self.min_recent_entries` from `SessionCache::evict_as_needed`
- **Red:** `assertion failed: cache.get(&first, BaseImageSource::Original).is_some()` — a second large image evicting the first immediately
- **Green after revert:** yes

## Queue background warmups serially and make failure advance the queue

- **Gate:** `src/app/tests.rs` :: `app::tests::import_cache_warm_failure_still_advances_to_the_next_supported_image` — run by `cargo test`
- **Mutation:** returned early from `ImportCacheWarmCompleted` on `Err` instead of starting the next warm
- **Red:** `left: None`, `right: Some("…\overlay.svg")` — one bad file blocking every later image
- **Green after revert:** yes

## A same-session reopen prefers the displayed full image, guarded by a cheap source-metadata check

- **Gate:** `src/app/tests.rs` :: `app::tests::library_reopen_reuses_the_displayed_full_image_immediately`, `library_reopen_reloads_when_the_current_source_metadata_changes`, `repeat_raw_open_ignores_cached_full_image_after_the_source_changes` — run by `cargo test`
- **Mutation A:** `App::displayed_full_image_for_path` always returns `None` — `assertion failed: !app.detail_load.is_loading()`
- **Mutation B:** dropped the `metadata_matches_path` guard — `assertion failed: app.detail_load.is_loading()`, the fast path serving stale pixels after a rewrite
- **Green after revert:** yes

## Drag preview state that is only meaningful in Library is cleared when a Library action enters Detail

- **Gate:** `src/app/tests.rs` :: `app::tests::opening_detail_from_library_clears_pending_drag_state` — run by `cargo test`
- **Mutation:** removed `self.clear_library_drag_state()` from `App::start_load`
- **Red:** `assertion failed: app.drag_state.is_none()`
- **Green after revert:** yes

## Build save requests from the image state the user is currently seeing

- **Gate:** `src/app/tests.rs` :: `app::tests::save_uses_the_visible_crop_state`, `save_request_exports_the_visible_full_image_in_crop_mode` — run by `cargo test`
- **Mutation:** `App::visible_edit_state` returns the committed history state without overlaying `visible_crop()`
- **Red:** `left: Some(CropRect { left: 0.5, top: 0.0, right: 1.0, bottom: 1.0 })`, `right: None`
- **Green after revert:** yes

## Drag affordance and drag behaviour are backed by the same pannability rule

- **Gate:** `src/viewer.rs` :: `viewer::tests::mouse_interaction_is_default_when_image_fits_the_viewport`, `update_does_not_start_dragging_for_a_fit_image` — run by `cargo test`
- **Mutation A:** `mouse_interaction` drops `self.can_pan(bounds)` — `assertion failed: matches!(interaction, mouse::Interaction::None)`
- **Mutation B:** the drag handler drops `self.can_pan(bounds)` — `assertion failed: matches!(status, event::Status::Ignored)`
- **Green after revert:** yes

## Do not build expensive GPU pre-pass resources unless the current edit state needs them

- **Gate:** `src/viewer.rs` :: `viewer::tests::blur_prepass_is_only_needed_for_clarity_or_dehaze` — run by `cargo test`
- **Mutation:** `AdjustmentUniforms::needs_blur` returns `true`
- **Red:** `assertion failed: !default.needs_blur()`
- **Green after revert:** yes

## The background persist owns the Library thumbnail refresh, from its own render snapshot

- **Gate:** `src/app/tests.rs` :: `app::tests::exposure_commit_updates_library_thumbnail_after_persist_completes`, `rotate_clockwise_updates_library_thumbnail_after_persist_completes`, `persist_completed_thumbnail_updates_stored_base_and_provenance`, `exif_loaded_refreshes_library_thumbnail_and_persist_for_auto_lens_correction` — run by `cargo test`
- **Mutation:** discarded `bake.thumbnail` in the `LocalEditPersistCompleted` arm
- **Red:** `exposure-adjusted thumbnail handle` — the Library falling back to a stale or synchronously recomputed thumbnail
- **Green after revert:** yes

## Keep supported image extensions centralized so scans, navigation and the file dialog stay aligned

- **Gate:** `src/app/tests.rs` :: `app::tests::file_dialog_extensions_match_supported_image_extensions` — run by `cargo test`
- **Mutation:** `library::image_file_dialog_extensions` returns its own `["jpg", "jpeg", "png"]`
- **Red:** the two lists printed side by side, 3 entries against 37
- **Green after revert:** yes

## The lessons queue is a staging area: every entry reaches its evidence, no evidence is stranded, and no entry sits there without naming its gate — **reworked**

- **Gate:** `tests/lessons_pairing.rs` :: six tests — run by `cargo test`
- **Reworked:** the previous non-vacuity check asserted the LIVE files parse at least one entry, so it went red the moment the queue was correctly emptied — it made "no lessons pending" indistinguishable from "the parser broke". The parsers are now proved against inline fixtures with known answers, and the set-difference checks run against the live files, so an emptied queue passes and a half-emptied one still fails. Two checks were added for the queue discipline the canon now states: every active bullet carries an evidence link, and every evidence entry names a gate.
- **Mutation A:** a rule linking to an evidence heading that does not exist — `every_rule_points_at_an_evidence_entry_that_exists` red
- **Mutation B:** an evidence entry no rule points at — `every_evidence_entry_has_at_least_one_rule` red
- **Mutation C:** an active bullet with no `[evidence](...)` link — `every_active_lesson_carries_an_evidence_link` red, `lessons with no evidence anchor are folklore, not lessons`
- **Mutation D:** a paired entry whose table has no Gate or Test-added row — `every_evidence_entry_names_the_gate_it_is_waiting_for` red
- **Mutation E:** the index parser's link prefix changed so it matches nothing — `the_parsers_find_what_is_there_and_only_what_is_there` red (`left: []`), which is the check that makes the empty-file passes trustworthy
- **Green after revert:** yes (6 passed)
