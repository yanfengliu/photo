# Review — library-test-isolation, iteration 1 (2026-06-13)

Diff under review: `src/library.rs`, `src/collection.rs`, `src/app/tests.rs` (+ `Cargo.toml` version bump). ~120 insertions. Objective: stop the test suite from overwriting the real `%LOCALAPPDATA%/photo/library.txt` and `collections.json` with temp-dir fixtures (the reported "null images with fake titles" on Library startup).

All three reviewers read the live codebase (greps + file reads + a `rustc` scratch compile) per the grounding directive, not just the diff text.

## Disposition: CONVERGED at iteration 1 — three independent APPROVEs, zero blocking findings.

## Findings by provider

- **Codex (gpt-5.5, xhigh, read-only sandbox over the live tree):** No findings. Traced `local_app_storage_dir` / `library_file_path` / `save_library` / `load_library` / `collections_file_path` and confirmed they are reached only synchronously (App startup, `FilesPicked` import, collection mutations). Enumerated every `Task::perform` / `tokio::task::spawn_blocking` site and confirmed none call the storage-path helpers. Production `cfg(not(test))` unchanged; `cargo fmt --check` passed.
- **Gemini (3.1-pro, plan mode):** Clean approval; "No changes are required." Verified the thread-local choice is correct because production saves run on the main iced thread and the background thumbnail tasks use `photo_repo_root()` (its own mutex override), not the storage dir. Called out the unified library/collection source-of-truth as a design improvement. Working-tree contamination audit after the run: clean (no unexpected file edits).
- **Claude (opus[1m], effort max, Read/Glob/Grep):** APPROVE — correct and complete. Traced the full reachability graph (cross-thread answer: NO defect). Key stronger-than-claimed insight: the "never writes the real dir during tests" guarantee does not depend on the synchronous-call invariant at all — under `cfg(test)` the `LOCALAPPDATA` branch is compiled out, so every thread resolves through the thread-local (default `None`); a stray off-thread save would be a silent no-op that also *fails* the override tests. Empirically compiled the `#[cfg(...)] { … }`-block-as-tail-expression pattern with `rustc` (returned `Some(2)` non-test, `Some(1)` `--cfg test`) to rule out the fn silently returning `()`. Confirmed no clippy/fmt regressions (the `//` comment on the macro, the clippy-preferred `const {}` initializer, no unused imports in `collection.rs`, symbol visibility through `use crate::library::*` + `use super::*`).

## Non-blocking nits (no action taken)

1. `with_test_app_storage_dir` resets the override to `None` rather than restoring a previous value (Claude). Footgun only under nesting, which no call site does. **Declined for consistency** with `repo.rs::with_test_photo_repo_root_override`, which also resets to `None` (same established pattern). Re-evaluate only if nested overrides are ever introduced.
2. The synchronous-call invariant in the `library.rs` comment is documented but undefended (Claude). Accurate today, and violating it later is self-catching (cfg gate prevents real-dir pollution; override tests fail). No action needed.
3. Pre-existing weak coverage: `save_and_load_library_round_trips` and `load_library_filters_deleted_files` inline-reimplement parse/filter logic instead of calling the real functions (Claude). Out of scope; the new override round-trip test partially supersedes the former. Flagged so it is not mistaken for new coverage.

## Gates at review time
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (362 pass, 1 ignored), `cargo build --release` — all green.
