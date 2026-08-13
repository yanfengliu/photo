# AGENTS.md — photo

## What this is

A GPU-accelerated image viewer/editor for Windows in Rust: iced + wgpu UI with a library thumbnail grid, a detail view with GPU zoom/pan and editing (open, edit, crop, save/export), and broad format support — JPEG, PNG, GIF, BMP, TIFF, WebP, ICO, TGA, QOI, HDR, EXR, SVG, and camera RAW via rawler.

<!-- FLEET-CANON:BEGIN sha=6828ee81a8f0 generated from ../fleet/FLEET.md by `npm run sync-canon` — do not edit inside this block; this repo's own rules go in docs/policies/local-rules.md -->
## Fleet constitution

- Verify visual work visually: capture the rendered result — screenshot, frame, recording — and look at it, because a passing test says nothing about what the pixels do. Work with no visual surface runs headlessly.
- Commit each verified unit of change to `main` without being asked, and push. Gates pass before any commit that touches code; a dependency change re-runs the audit gate.
- A repo chooses its own language and toolchain — Node, Python, and Rust all run here. Each pins its version where its own tooling reads it (`.nvmrc`, `requires-python`, `rust-toolchain.toml`) and names it in Gates, so a version mismatch is not read as a code failure. Node repos baseline at 24; an older major keeps a CI job proving it.
- Runtime model calls are authorized and already paid for — this fleet has one user, with Claude Code and Codex subscriptions — so a program here may call a model at runtime, vision included.
- The top reasoning tier is rationed: spend it only on the hardest problem, or on directing the workhorse tier that does the work — and only at maximum effort or orchestration.
- High-risk work — persistence/migrations, security/auth, concurrency, money, supply chain, edits that reach sibling repos — escalates to the multi-cli-review skill. That is a review you run yourself, not permission you ask the user for; nothing in this canon requires asking.
- Error messages are a product surface: audit them as a class, including paths the task did not touch. Each names what happened, which input caused it, and what would satisfy it — context the throw site holds for free and a reader can only buy back by running it again. That detail is what closes the loop: a bare `Validation failed` turns an already-diagnosed failure into a debugging session.
- When blocked, hand over the raw artifact — screenshot, rendered page, log line, data row — as soon as the blocker is named rather than after the analysis: your description of it is filtered through the misunderstanding that caused the block, so it cannot contain what you failed to notice.
- Task-run evidence lives only under ignored paths and is deleted once nothing active needs it; it enters Git only when review promotes it into a repository input — a fixture, golden, snapshot, or contract. Tracked docs keep conclusions and provenance only. Blob ceilings for anything promoted: over 256 KiB needs a stated reason, over 512 KiB binary or 1 MiB of anything never enters ordinary Git, and an asset store or LFS needs the user's approval.
- Write prose one line per paragraph (no hard wrapping).
- Keep a devlog: one short dated line per behaviour-changing session in `docs/devlog/summary.md`, newest first, and a section in `docs/devlog/detailed/` for anything a later session could trip over — what was believed and proved false, what a reviewer caught that the author missed, what number moved and from what. Both shapes are in `../fleet/docs/devlog-template.md`. It is history, not status: the repo's design docs hold the current position. Write it because the alternative is rediscovering your own dead ends.
- Read `docs/learning/lessons.md` at session start: the one-line index of what this repo has already paid to learn, short by construction, with each entry's war story and anchor in `lessons-evidence.md` — opened only when a rule is in doubt or the work is in that area. A lesson lands the session it is learned, as an entry there plus one line here, anchored to a measurement, commit, or test id; unanchored, it is folklore. When a lesson becomes a gate — a test, a lint rule, a fixed command — delete both halves, because the machine enforces it now and every line that stays spends the attention that keeps the rest read. Shape: `../fleet/docs/lessons-template.md`.
- Steering compounds: a direction that outlives the immediate task lands that same session — `../fleet/FLEET.md` if fleet-wide, else this repo's `docs/policies/local-rules.md` — and you say where it went.
- Reviewer model pins live only in `../fleet/docs/skills/multi-cli-review.md`; a model a product itself calls is pinned in the repo that calls it. Never hardcode a model ID anywhere else.
<!-- FLEET-CANON:END -->

## Gates

`cargo test` · `cargo clippy --all-targets -- -D warnings` · `cargo fmt --check` · `cargo build` — all four before every code commit; only affected tests while iterating. Dependency changes re-lock with `cargo update` and audit with `cargo audit`.

## Session start

Read `docs/devlog/summary.md` and `docs/architecture/ARCHITECTURE.md` before starting work.

## Agent harness (drive the app like a user)

Launch `./target/release/photo.exe --harness` and drive it with `./target/release/harnessctl.exe <cmd>` from the repo root: screenshots (real GPU frames), CPU render dumps with image statistics (clipping, percentiles, histograms), and image diffs land in a per-run artifact directory under `tmp/harness-runs/`. Storage is sandboxed by default, so sessions never touch the real library. Use it whenever a change affects rendering, edit math, or UI behavior — observe the result yourself instead of asking the human how it looks, and record findings per the documented contract. Canonical guide (protocol, command table, statistics semantics, the observe→act→measure→verify improvement loop, findings schema): `docs/guides/agent-harness.md`.

## Invariants & boundaries

- TDD for behavior changes: tests first, testing the contract (app experience and mechanisms), not the code.
- File size: keep files under 500 LOC (hard ceiling 1000); split god-objects by lifecycle/role.
- Recursive loop status: photo does not run `playtest:recursive` yet — the agent harness is loop step 1.

## Conventions

- Devlog: `docs/devlog/summary.md` (one line per task; compact past 50 lines — no cheating with mega-lines) + `docs/devlog/detailed/START_DATE_END_DATE.md` (per-task entry: timestamp, action, reviewer findings by provider/theme, result, reasoning, notes; archive via `git mv` when the active file passes 500 lines, starting a new file dated today).
- Changelog `docs/changelog.md` + `Cargo.toml` version: user-visible changes only (external audience; migration focus). Bump `c` per non-breaking change, `b` (reset `c`) per breaking change, `a` only when the user says so; one bump per coherent shipped change.
- Architecture: structural changes update `docs/architecture/ARCHITECTURE.md` and append a row to `docs/architecture/drift-log.md`; non-obvious tradeoffs append to `docs/architecture/decisions.md` (append-only — supersede, never delete). Non-structural fixes touch none of these.
- Lessons: `docs/learning/lessons.md` per the fleet evidence-anchor rule; code lessons need a real test id; photo-processing lessons record the affected sample image or batch and the before-after pixel diff or quality metric.
- Review threads: syntheses land in `docs/threads/current/<objective>/<date>/<n>/REVIEW.md` (synthesis only — no raw CLI output; unstaged temp captures go to `tmp/review-runs/`); `DESIGN.md`/`PLAN.md` live at the objective root; move closed objectives to `docs/threads/done/` and keep them as audit trail.
- Canonical doc surfaces are README, the architecture set, devlogs, changelog, and `docs/guides/agent-harness.md`. There is deliberately no `docs/api-reference.md` — the Rust types are the API reference. README changes when user-visible features change.
