# Agent Harness — Review Iteration 1 (2026-07-08)

Reviewers: in-process adversarial Workflow (5 finder dimensions × refuting verifiers; 9 of 13 agents completed — 4 died on the Claude session-quota limit) + Codex CLI (gpt-5.5, xhigh) on the staged diff. Claude CLI was unreachable (same quota limit); per the unreachable-CLI protocol the review proceeded with the remaining reviewers.

## Findings and dispositions

**[HIGH — CONFIRMED by refuting verifier] Sandbox hole: decoded-cache/ escaped the harness storage sandbox.** `decode.rs` resolved the decoded cache through a private duplicate of repo-root discovery (its own `PHOTO_REPO_ROOT: OnceLock` + `discover/find/is_photo_repo_root`), which the harness runtime override in `repo.rs` never touched — so sandboxed sessions read, wrote, and pruned the REAL repo's `decoded-cache/`. **Fixed:** `decode::photo_repo_root()` now delegates to `crate::repo::photo_repo_root()` (single owner of discovery + sandbox override); the duplicate statics/functions are deleted (the ARCHITECTURE.md "dedup is a tracked follow-up" debt, now forced closed); pinned by failing-first test `decode::tests::decoded_cache_resolution_delegates_to_the_shared_repo_resolver`; the displaced discovery regression test moved to `repo::tests`.

**[MEDIUM — CONFIRMED by refuting verifier] Stale async completions crossed connection boundaries.** Screenshot/dump_render/image_stats/compare_images completions carried only a request id; after a client disconnect (harnessctl timeout, Ctrl-C) the completion was delivered to the NEXT connection, where harnessctl's fixed `id=1` made mis-correlation deterministic (a stale pre-change RenderReport could be accepted as the answer to a different command — silent wrong-answer corruption of the agent feedback loop). **Fixed:** `App.harness_connection_generation` bumps on every `Connected`; all async completion messages carry the generation captured at dispatch; `respond_harness_result_if_current` drops stale responses (artifacts still recorded so the manifest stays truthful); pinned by `stale_async_completions_are_dropped_not_misdelivered`; live smoke re-verified 4-reconnection flow.

**[MEDIUM — REFUTED by verifier] `prepare_runtime` partial failure leaves storage sandboxed on a "normal" launch.** The verifier confirmed the code structure but demonstrated the claimed harm cannot occur on any reachable path. No change.

**[MEDIUM — unverified claim (verifier lost to quota); hardened anyway] `import_files` accepted paths no real dialog could produce** and persisted them into `library.txt` with no removal affordance. **Fixed:** the command now rejects missing/non-file paths listing the offenders; pinned by `import_files_rejects_paths_no_dialog_could_produce`.

**[LOW/MEDIUM — unverified claim (verifier lost to quota); hardened anyway] `click` executed controls the observe list advertised as disabled**, and the vocabulary included `tab_library`/`tab_detail` controls that correspond to no real UI button. **Fixed:** a single `harness_control_enabled` predicate now backs BOTH the observe controls list and a click gate (list and behavior cannot drift); the synthetic tab controls are removed (real navigation is `back`/Escape/`open`); pinned by `click_refuses_controls_the_list_advertises_as_disabled`; guide/DESIGN command tables updated.

**[MEDIUM — Codex, docs accuracy] Doc/code drift.** (a) DESIGN.md documented the drafted command vocabulary (`dump_render {which}`, `toggle_crop`, `zoom`/`next`/`prev`/`undo`/`redo`/`save_edited`) instead of the shipped one — rewritten to the as-shipped vocabulary with the fold-into-`key`/`click` rationale. (b) Guide/devlog links to `docs/threads/done/agent-harness/` — correct after the ship-step thread move; no change needed.

## Validation after fixes

All four gates green: `cargo test` 424 passed (+3 new guard tests), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build --release`. Live smoke on the release binary against a RAW fixture: ping → wait_idle → set_slider → dump_render (full-res stats + downscaled PNG) → screenshot → quit across four separate connections, all correctly correlated.

## Disposition

Both confirmed defects fixed and pinned; both unverified claims hardened with tests rather than argued; one claim refuted; doc drift corrected. Iteration 2 (Codex re-review of the fix delta) verifies the fixes land clean.
