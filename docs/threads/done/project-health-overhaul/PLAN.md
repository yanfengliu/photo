# Project Health Overhaul — Plan

Date: 2026-06-10. Driver: Claude (main agent). Trigger: project-state assessment found 2 of 4 quality gates failing (clippy: 26 errors, fmt: 24 diffs), no CI, `src/main.rs` at 9,006 lines vs the 500-LOC review rule, and several AGENTS.md-mandated artifacts missing (changelog, version bumps, threads convention).

## Commit sequence

| Commit | Scope | Version |
| --- | --- | --- |
| A | Docs scaffolding: this plan, `docs/changelog.md` baseline, decisions entry for repo-local runtime caches, reviews→threads redirect, lessons grandfather note, new devlog file | none (docs only) |
| B | Gate fixes: `cargo fmt` (24 diffs in decode/edit/main) + 26 mechanical clippy fixes (flat_map identity, field_reassign_with_default, repeat().take(), type_complexity, bool asserts, needless_return, too_many_arguments) | 0.1.1 |
| C | CI: `.github/workflows/ci.yml` (windows-latest, four gates) + `rust-toolchain.toml` pinning 1.94.0 with clippy/rustfmt components | 0.1.2 |
| D | Split `src/main.rs` (9,006 lines) into focused modules per the map below; pure move refactor, no behavior change; ARCHITECTURE.md + drift-log updated | 0.1.3 |
| E | Slow-test speedup: 3 decode RAW tests run >60s each; shrink synthetic fixtures if behavior-preserving, else document deferral | 0.1.4 if shipped |
| F | `cargo audit` baseline run; record result in devlog (no code) | none |
| G | Multi-CLI review (Codex + Gemini + Claude) of cumulative diff vs 1480f57; synthesize REVIEW.md iterations here; address findings; move thread to done | none |

## Module map for commit D

| New module | Contents (from current main.rs lines) |
| --- | --- |
| `src/theme.rs` | Color consts (40-57), style fns (3584-3743) |
| `src/widgets.rs` | Rotation icon consts + buttons (58-64, 105-150), `ThumbnailGridLayout` (151-172), thumbnail slots (622-651), section label/divider, context_menu_item (3685-3710) |
| `src/detail_load.rs` | `DetailLoadStage`, `DetailLoadState` (652-727) |
| `src/session_cache.rs` | `SourceFileFingerprint` + helpers (304-373, 80-81), session full-image cache types + impl (65-69, 374-589) |
| `src/repo.rs` | `photo_repo_root` family + test override (87-93, 3807-3862) — note: decode.rs keeps its own pre-existing duplicate; dedup is a tracked follow-up |
| `src/local_edits.rs` (dir module: `mod.rs`, `io.rs`, `repair.rs`) | Local-edit cache consts/statics (70-79, 82-103), variant/request/header types (278-303, 396-431), path/id/lock/test-hook fns (3863-3995), header+pixel io (4021-4257), load/validate (4258-4476), repair (4477-4757), `persist_local_edit` (4877-4923) |
| `src/loading.rs` | `BaseImageSource` (272-277), `LoadedFullImage` (389-395), `loaded_image_logical_dimensions` (590-610), `load_library_thumbnail_base_image` (4758-4829), `load_full_image` (4830-4876) |
| `src/library.rs` | `LibraryEntry` (257-263), `local_app_storage_dir`/`library_file_path` (3799-3806), `save_library`/`load_library` (4924-4953), `image_file_dialog_extensions` (4954-4962) |
| `src/app/mod.rs` | `App` struct, `Message`, `Tab`/`SliderKind`/`CropAspect`/`ContextMenu`/`DragState`/`SaveRequest` types, lifecycle (new/title/theme/subscription), shared state accessors and zoom math (2855-2928, 3014-3199), slider field helpers (3744-3798), `path_filename_str`, `display_dimensions_for_edit_state` |
| `src/app/update.rs` | `impl App`: `update` (980-1664), event/key handlers (1665-1962), library mutation + import warm + persist orchestration + load tasks (1963-2520), `build_adjustment_uniforms` |
| `src/app/view.rs` | `impl App`: `view`, tab bar, library/collection/detail views, status bar, edit panel, overlays (2521-3014 view parts, 3200-3583) |
| `src/app/tests.rs` + per-module `mod tests` | Existing `mod tests` (4963-9006) moves wholesale to `app/tests.rs`; clearly module-owned clusters (session cache eviction, local-edit round-trip, thumbnail helpers, widget draw tests) relocate to their home modules; shared helpers go to `#[cfg(test)] src/test_support.rs` |
| `src/main.rs` (after) | `main()`, module declarations only |

Known post-split residuals, accepted and tracked as follow-ups: `app/update.rs`, `app/view.rs`, `app/tests.rs`, `local_edits/` files may still exceed 500 LOC (further splits need behavior-aware refactors of the `update` match); pre-existing oversize `decode.rs` (2,481), `edit.rs` (1,984), `viewer.rs` (1,944); repo-root duplication between `repo.rs` and `decode.rs`; dual image-crate versions (0.24 via iced 0.13 + 0.25 for rawler) pending an iced upgrade.

## Verification

Every commit: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build` all green before committing. Commit D additionally: test count stays 298 (or grows), zero behavior diffs (pure moves; compiler + suite as the net). Push after C so CI proves itself on the real runner; `gh run watch` to confirm. Final: multi-CLI review per AGENTS.md on `git diff 1480f57`, iterate until reviewers nitpick only.

## Risks

- Move refactor breaks visibility/imports: mitigated by `pub(crate)` on moved items, compiler-driven fixup, `cargo fix` for unused imports, full suite after each sub-step.
- CI runner divergence (Windows runner toolchain/GPU): tests are GPU-free by design; toolchain pinned via rust-toolchain.toml.
- Clippy "too_many_arguments" fix could change a signature: internal helper only; prefer a params struct, fall back to a scoped `#[allow]` with justification if the struct would be artificial.
