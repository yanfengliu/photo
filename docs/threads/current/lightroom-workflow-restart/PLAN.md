# Lightroom-workflow restart — Plan

Date: 2026-11-19. Design: [DESIGN.md](DESIGN.md). Each milestone lands as its own commit on `main` with a patch-version bump, a changelog entry and a devlog entry, after the four gates pass (`cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`).

## Milestones

| # | Version | Milestone | Done when |
| --- | --- | --- | --- |
| M0 | — | Clear the repository | Only `AGENTS.md` and `CLAUDE.md` remain; old app reachable at `94bd2cf`. |
| M1 | 0.3.0 | Foundation | Workspace of twelve packages builds; `photo-core` data model and its contract tests pass; headless UI rendering proven; design and plan written. |
| M2 | 0.3.1 | Engines, wave 1 | `photo-color`, `photo-imaging`, `photo-decode` (+ `photo-testkit`), `photo-catalog`, `photo-xmp` and the UI widget kit each pass their contracts in isolation. |
| M3 | 0.3.2 | Pipeline and export | `photo-develop` renders through all eleven stages and satisfies T1–T5 and R1–R4; `photo-export` writes files that decode back to what was rendered. |
| M4 | 0.3.3 | Engine | `photo-engine` runs the end-to-end scenario headlessly: import a folder, browse, rate, edit, undo, export. |
| M5 | 0.3.4 | Library module | The app launches into a working Library: import dialog, grid, loupe, compare, survey, panels, filter bar, filmstrip, shortcuts. |
| M6 | 0.3.5 | Develop module | All Develop panels, crop, healing, masking, before/after, history, snapshots, presets, copy/paste; export dialog. |
| M7 | 0.3.6 | Hardening | Adversarial review findings fixed and re-verified; multi-CLI review of the catalog (persistence is high-risk); docs complete; thread moved to `docs/threads/done/`. |

## How the work is split

Milestone M2 is independent by construction (every wave-1 crate depends only on `photo-core`), so it is built by parallel subagents, one crate each, in the same checkout. Rules for them: touch only your own crate; run cargo only with `-p <your crate>`; no git commands; no workspace-wide formatting; report new third-party dependencies instead of adding them silently.

M3 depends on the real APIs of `photo-color` and `photo-imaging`, and M4 on everything below it, so those are sequenced and each brief is written against the code that actually landed, not against this plan.

The UI (M5, M6) is built on the engine's real API. A widget kit and the headless harness are built during M2 so that module work starts from finished parts.

## Verification

- Contract tests listed in DESIGN §5.5, §5.9 and §9, written before the code they pin.
- Rendered inspection: a tuning-grid test renders the synthetic scene across slider extremes to PNG for visual review; UI screens are rendered through the harness and read back.
- Adversarial review: an in-process workflow of finder and refuter agents over each milestone's diff; the catalog and XMP writer additionally get the Codex + Claude CLI review because they can lose user data.
- Docs: `docs/changelog.md`, `docs/devlog/`, `docs/api-reference.md`, `README.md` and `docs/architecture/` updated with each milestone, then a doc-accuracy pass over the final state.

## Open decisions (made, recorded for visibility)

- Target is the Lightroom Classic workflow (modules, catalog, panels), not the cloud-first Lightroom layout.
- Version continues at 0.3.0: a restart is a breaking change, and only the owner bumps the major version.
- CPU pipeline first. If preview speed needs more, the route is a 3D LUT baked from the same Rust functions and applied by a trivial shader, which adds speed without adding a second implementation of the math.
