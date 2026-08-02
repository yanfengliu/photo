# AGENTS.md — photo

## What this is

A GPU-accelerated image viewer/editor for Windows in Rust: iced + wgpu UI with a library thumbnail grid, a detail view with GPU zoom/pan and editing (open, edit, crop, save/export), and broad format support — JPEG, PNG, GIF, BMP, TIFF, WebP, ICO, TGA, QOI, HDR, EXR, SVG, and camera RAW via rawler.

<!-- FLEET-CANON:BEGIN sha=84502f2f7796 generated from loop-ops/FLEET.md by `npm run sync-canon` — do not edit inside this block; this repo's own rules go in docs/policies/local-rules.md -->
## Fleet constitution

- Work headlessly by default; go non-headless only when nothing else can complete or verify the task, and say why. Close every browser or GUI process the task opened.
- Concurrent sessions share one worktree and one index: commit by explicit pathspec (`git commit -- <files>`), never `git commit -a`, `git add -A`, or `git add .` — a sweeping commit captures whatever another session has staged. (Evidence: voxel c024b33, 2026-07-17.)
- Commit each verified unit to `main` promptly and push at the end of the task; never commit failing or partial work as a checkpoint. The repo's gates pass before every commit that touches code; doc-only changes need a self-reviewed diff. Dependency changes re-resolve the lockfile and run the audit gate — a new HIGH/CRITICAL is a blocker.
- Toolchain baseline: develop and run gates on Node 24, which every Node repo pins in its own `.nvmrc`. A repo that must keep supporting an older major says so in its Gates section and keeps a CI job proving it, because otherwise an agent on the wrong version reads a version failure as a code failure and starts debugging the repo. (Established 2026-07-31, after `node:check` failed on Node 22 and looked like a broken checkout.)
- A model call is a legitimate component of a program here, not only an authoring aid: this fleet has one user, with Claude Code and Codex subscriptions, so a pipeline may call a model at runtime — vision included — wherever that beats a hand-written heuristic. Model output proposes, a deterministic check disposes, and it never self-certifies. Reaching for a brittle heuristic to avoid a model call is the mistake, not the other way round. (Established 2026-07-31, after a geometric stud-pitch reader answered 4 of 50 booklet regions and a headless vision call answered 6 of 6.)
- Never claim a bug fixed from a code diff. Rerun the failing case and compare at bug-class granularity, then promote the fix with a regression test or fixture that fails if it reverts — a fix with no way to catch the class again is incomplete.
- When two attempts at the same problem have failed, stop iterating alone: build the fixed benchmark or reproduction that settles the question, fan out independent subagents on deliberately different approaches against it, then switch role to evaluator — score their output yourself rather than trusting their reports, and take the best. A third pass at the approach that already failed twice is the expensive mistake. (Established 2026-07-31.)
- High-risk work — persistence/migrations, security/auth, concurrency, money, supply chain, edits that reach sibling repos — escalates to the multi-cli-review skill. Reviewers must read the live code; verify their claims against the codebase before acting on them, and weigh substantive findings over approval votes.
- Error messages are a product surface, audited as a class rather than fixed when one happens to be touched: every path that rejects, fails, or throws says what happened, which specific input caused it, and what would satisfy it — never a bare `Validation failed` or a silent boolean false. Where a failure has known non-fixes, name the dead end too. (Established 2026-07-18 after city's `placeService` answered five rejected placements with only "Validation failed"; widened to coverage 2026-08-01 after voxel's Riverfall surface told three sessions running to "increase particle coverage" when coverage was the one thing measured not to fix it.)
- Docs are part of the change: update every affected surface in the same commit, write prose one line per paragraph (no hard wrapping), and never reference or mandate files that don't exist. Lessons files (`docs/learning/lessons.md` where present) require evidence anchors — source, fix commit, test id, behavior delta; unanchored lessons are folklore.
- Steering compounds: when the user gives a direction that generalizes past the immediate task, land it in the canon in that same session — `../loop-ops/FLEET.md` if it is fleet-wide, otherwise this repo's `docs/policies/local-rules.md` — so the next run inherits it instead of relearning it, and say what was captured and where. (Established 2026-07-18.)
- Research before you reason: when a question has a public answer — a physics or numerical method, a library's behaviour, an engine's parameter, a format, a protocol — read the docs, the source, or the literature before deriving it, and cite what you read so the next reader can check instead of re-deriving. A dependency's source is one call away (`gh api repos/<owner>/<repo>/contents/<path>`). Do not reinvent a method that has a name, and never explain a measured result with a mechanism you have not checked. (Established 2026-08-01, after a voxel session measured that Rapier's soft CCD was inert for a rotating cam and guessed the mechanism right, then in the same comment asserted a 45 Hz contact frequency "cannot be represented" at a 1/60 s step — the engine's own erp formula saturates smoothly and has no such limit.)
- Reviewer model pins live only in `../loop-ops/docs/skills/multi-cli-review.md`; a model a product itself calls is pinned in the repo that calls it. Never hardcode a model ID anywhere else.
<!-- FLEET-CANON:END -->

## Gates

`cargo test` · `cargo clippy --all-targets -- -D warnings` · `cargo fmt --check` · `cargo build` — all four before every code commit; only affected tests while iterating. Dependency changes re-lock with `cargo update` and audit with `cargo audit`.

## Session start

Read `docs/devlog/summary.md` and `docs/architecture/ARCHITECTURE.md` before starting work. Read `docs/learning/lessons.md` too — it records what has already been tried and what it cost, and a lessons file nothing tells anyone to open is write-only.

## Agent harness (drive the app like a user)

Launch `./target/release/photo.exe --harness` and drive it with `./target/release/harnessctl.exe <cmd>` from the repo root: screenshots (real GPU frames), CPU render dumps with image statistics (clipping, percentiles, histograms), and image diffs land in a per-run artifact directory under `tmp/harness-runs/`. Storage is sandboxed by default, so sessions never touch the real library. Use it whenever a change affects rendering, edit math, or UI behavior — observe the result yourself instead of asking the human how it looks, and record findings per the documented contract. Canonical guide (protocol, command table, statistics semantics, the observe→act→measure→verify improvement loop, findings schema): `docs/guides/agent-harness.md`.

## Invariants & boundaries

- TDD for behavior changes: tests first, testing the contract (app experience and mechanisms), not the code.
- File size: keep files under 500 LOC (hard ceiling 1000); split god-objects by lifecycle/role.
- Recursive loop status: photo does not run `playtest:recursive` yet — the agent harness is loop step 1.

## Known traps

- Visual changes verify with before screenshot → change → after screenshot → pixel diff alongside the normal gates; the harness provides all of this against the real app (`harnessctl screenshot` / `dump_render` / `compare_images`).
- Debugging sessions record their process in a new file per session from `docs/debugging/template.md`; if a later session invalidates an old conclusion, update the old doc; clean up temporary dumps when done.

## Conventions

- Devlog: `docs/devlog/summary.md` (one line per task; remove outdated info; compact past 50 lines — no cheating with mega-lines) + `docs/devlog/detailed/START_DATE_END_DATE.md` (per-task entry: timestamp, action, reviewer findings by provider/theme, result, reasoning, notes; archive via `git mv` when the active file passes 500 lines, starting a new file dated today).
- Changelog `docs/changelog.md` + `Cargo.toml` version: user-visible changes only (external audience; migration focus). Bump `c` per non-breaking change, `b` (reset `c`) per breaking change, `a` only when the user says so; one bump per coherent shipped change.
- Architecture: structural changes update `docs/architecture/ARCHITECTURE.md` and append a row to `docs/architecture/drift-log.md`; non-obvious tradeoffs append to `docs/architecture/decisions.md` (append-only — supersede, never delete). Non-structural fixes touch none of these.
- Lessons: `docs/learning/lessons.md` per the fleet evidence-anchor rule; code lessons need a real test id; photo-processing lessons record the affected sample image or batch and the before-after pixel diff or quality metric.
- Review threads: syntheses land in `docs/threads/current/<objective>/<date>/<n>/REVIEW.md` (synthesis only — no raw CLI output; unstaged temp captures go to `tmp/review-runs/`); `DESIGN.md`/`PLAN.md` live at the objective root; move closed objectives to `docs/threads/done/` and keep them as audit trail.
- Canonical doc surfaces are README, the architecture set, devlogs, changelog, and `docs/guides/agent-harness.md`. There is deliberately no `docs/api-reference.md` — the Rust types are the API reference. README changes when user-visible features change.
