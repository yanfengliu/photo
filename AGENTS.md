# AGENTS.md — photo

## What this is

A GPU-accelerated image viewer/editor for Windows in Rust: iced + wgpu UI with a library thumbnail grid, a detail view with GPU zoom/pan and editing (open, edit, crop, save/export), and broad format support — JPEG, PNG, GIF, BMP, TIFF, WebP, ICO, TGA, QOI, HDR, EXR, SVG, and camera RAW via rawler.

<!-- FLEET-CANON:BEGIN sha=f3fc55a93a9a generated from fleet/FLEET.md by `npm run sync-canon` — do not edit inside this block; this repo's own rules go in docs/policies/local-rules.md -->
## Fleet constitution

- Work headlessly by default. If only a browser or GUI can finish or verify the task, say why, and close what you opened.
- Concurrent sessions share one worktree and one index: commit by explicit pathspec (`git commit -- <files>`), never `git commit -a`, `git add -A`, or `git add .` — a sweeping commit captures whatever another session has staged. (voxel c024b33.)
- Commit each verified unit to `main` promptly without being asked, and push at the end of the task; never commit failing or partial work as a checkpoint. Gates pass before any commit that touches code; a dependency change re-runs the audit gate.
- Toolchain baseline is Node 24, pinned per repo in `.nvmrc`. A repo that must keep an older major says so in its Gates section and keeps a CI job proving it.
- Runtime model calls are authorized and already paid for — this fleet has one user, with Claude Code and Codex subscriptions — so a program here may call a model at runtime, vision included, wherever that beats a hand-written heuristic. Model output proposes; a deterministic check disposes.
- A fix is done when the failing case has been rerun and a regression test or fixture fails if the fix reverts. A diff is not evidence.
- High-risk work — persistence/migrations, security/auth, concurrency, money, supply chain, edits that reach sibling repos — escalates to the multi-cli-review skill.
- Error messages are a product surface: audit them as a class, including paths the task did not touch. Every path that rejects or throws names what happened, which input caused it, and what would satisfy it — never a bare `Validation failed`.
- Docs are part of the change: update every affected surface in the same commit, and write prose one line per paragraph (no hard wrapping).
- Task-run evidence — raw traces, per-sample results, screenshots, recordings, generated reports, archives — lives only under ignored paths and is deleted once nothing active needs it; never commit, push, or move it to LFS. Tracked docs keep conclusions and provenance only. Such output enters Git only when review promotes it into a genuine repository input — a fixture, golden, snapshot, or contract.
- Git blob ceilings: a new or changed blob over 256 KiB needs an explicit repository-input reason; over 512 KiB binary, or 1 MiB anything, never enters ordinary Git. An external asset store or LFS requires explicit user approval, and an existing oversized blob is never precedent for another.
- Steering compounds: when the user gives a direction that outlives the immediate task, land it that same session — `../fleet/FLEET.md` if fleet-wide, else this repo's `docs/policies/local-rules.md` — and say where it went.
- Citations are part of the deliverable: anything with a public answer — a numerical method, a library's behaviour, an engine parameter, a format, a protocol — carries the source it was read from, and so does any mechanism offered to explain a measured result. A dependency's source is one call away (`gh api repos/<owner>/<repo>/contents/<path>`).
- Reviewer model pins live only in `../fleet/docs/skills/multi-cli-review.md`; a model a product itself calls is pinned in the repo that calls it. Never hardcode a model ID anywhere else.
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
