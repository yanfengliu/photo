# Agent Harness — Design

Date: 2026-07-07. Objective: `agent-harness`. Status: authoritative design for the thread.

## Problem

Developing this app's edit pipeline has a human-relay bottleneck: the user must repeatedly look at an edited photo and tell the agent "it still looks weird" (value-range issues like clipped shadows/highlights, or tuning-algorithm differences vs Lightroom/darktable/camera-preview behavior). Every slider-tuning iteration (v0.2.0, v0.2.5–0.2.6) was bottlenecked on that relay. The agent needs to observe the rendered result itself, control the app the way a user does, and decide the next improvement step — closing the loop without the human as the feedback channel.

## Principles (adapted from civ-engine 1.3.0–1.6.0)

civ-engine's visual-playtest + recursive-improvement design is the model (`civ-engine/docs/guides/visual-playtest-harness.md`, `docs/threads/current/agent-recursive-improvement-loop/DESIGN.md`). Its governing statement: "The loop should not make agents more confident. It should make them harder to fool." Principles adopted, adapted from a headless TS game engine to a Rust desktop GUI app:

1. **Stable contract, thin adapters.** The app owns a small, versioned, JSON-stable harness vocabulary (observations, controls, actions, artifacts). The agent side stays a thin client (`harnessctl`) plus documentation. No LLM SDK, no provider client, no automation framework enters the app.
2. **Player-level actions, real dispatch.** The agent acts through the same code paths user input takes — harness commands become the exact `Message` values the widgets and event subscription produce (slider drag sequences, keyboard events through `handle_event`, viewer events through `handle_viewer`). The harness never pokes internal state directly.
3. **Observation = pixels + typed controls + labeled mechanical channels.** Every observation can carry: a real window screenshot (true GPU frame — what the user sees), a CPU render dump (full-res ground truth, pixel-identical to export), a typed control list (ids, labels, ranges, current values, enabled), and mechanical state channels (edit state, load stage, image statistics: per-channel percentiles, clip percentages, luma histogram). Value-range problems become numbers, not vibes — LLM eyes alone are unreliable for subtle tonal judgment.
4. **Replayable runs, findings as claims.** Every harness session logs all requests/responses to a run directory (`tmp/harness-runs/<run-id>/`) with a manifest (app version, timestamps, stop reason, artifact list). The session log doubles as a replayable script. Findings are structured JSON with severity, evidence refs (artifact paths + stats), verification status, and next action — a finding is a claim until a replayed scenario or stats comparison confirms it.
5. **Bounded autonomy.** The harness observes and acts; it never auto-fixes. Tuning changes to `edit.rs` math flow through the normal TDD + gates + adversarial-review process, with harness scenarios re-run before/after as evidence.

## Approaches considered

**A. In-process message driver only (extend the `tests.rs` pattern).** Drive `App::update()` directly in a test binary, observe via `edit::render_edited_image`. Cheapest; fully deterministic; but no real window, no GPU frame, no UI chrome — it cannot see what the user sees (slider positions, layout, thumbnail grid, GPU-only defects), and it is not "controlling the app like a user" in any observable sense. Rejected as the primary mechanism; its message-level idiom is reused for command execution and unit tests.

**B. OS-level automation (SendInput/UIAutomation + screen capture).** Maximum fidelity in theory; in practice focus-dependent, coordinate-fragile, undebuggable, and flaky — the opposite of civ-engine's deterministic-replay principle. Playwright-driving-a-browser (civ's game adapters) synthesizes events at the protocol level, not the OS level; the equivalent tier here is message-level injection into the real running app. Rejected.

**C. In-app harness mode (chosen).** `photo --harness` starts the real app with a localhost TCP control channel (JSON lines). A subscription feeds commands into `update()` as messages; responses flow back over the socket. Observation combines `iced::window::screenshot` (verified in vendored iced 0.13 source: the offscreen present path renders `layer.primitives`, so the custom shader canvas IS captured) with CPU render dumps and image statistics. This is the real app, the real render loop, the real message dispatch — with deterministic, replayable control.

## Architecture

### Activation and isolation

`photo --harness[=PORT]` (default port 7878, `0` = ephemeral) enables harness mode; a bare non-flag argument remains the optional image path. Harness mode defaults to **sandboxed storage**: a runtime override points `library::local_app_storage_dir()` and `repo::photo_repo_root()` at `<run-dir>/storage/`, so harness sessions can never pollute the real `%LOCALAPPDATA%/photo` library or the repo-local `decoded-cache/`/`local-edits/`/`edited/` (the v0.2.7 lesson, extended from tests to harness runs). `--harness-real-storage` opts out for sessions that must inspect the user's actual library. The overrides are `OnceLock`-set once in `main()` before the app starts; production behavior when unset is byte-identical to today.

### Control channel

`src/harness/server.rs` owns a `TcpListener` bound to `127.0.0.1:<port>`. Framing is JSON lines: one request object per line in, one response object per line out. A session token (random, written with the port to `<run-dir>/session.json`) must be presented as the first line of each connection; `harnessctl` reads that file automatically. Requests: `{"id": N, "cmd": "...", "params": {...}}`. Responses: `{"id": N, "ok": true, "data": {...}}` or `{"id": N, "ok": false, "error": {"code": "...", "message": "..."}}`. One request in flight at a time; the server loops on accept so a dropped connection doesn't kill the session. The channel reaches the app via `iced::stream::channel` subscription: the stream first emits `HarnessEvent::Connected(response_sender)`, then `HarnessEvent::Request(...)` per line; `update()` executes and replies through the stored sender.

### Command vocabulary (v1, as shipped)

Observation: `observe` (UI state + typed controls; screenshots are a separate orthogonal command — implementation refinement over the drafted optional-screenshot param), `screenshot` (window PNG → run dir; reports physical size, scale factor, and logical canvas size), `dump_render {source: current|original, max_dim?}` (CPU `render_edited_image` of the active image → PNG + full-resolution stats), `observe_library {offset, limit}`, `image_stats {path}` (stats for any PNG/JPEG on disk), `compare_images {path_a, path_b}` (per-channel mean/max abs diff, % differing pixels).

Actions (each maps to the exact messages real interaction produces): `open {path}` (the drag-drop/CLI open), `import_files {paths}` / `import_folder {path}` (the file-dialog completion path — dialog-opening clicks are refused), `set_slider {kind, value}` (drag semantics: two `SliderChanged` then `SliderReleased`, range-clamped, double-click-reset defused for consecutive calls), `reset_slider {kind}`, `click {control, value?}` (named buttons/toggles/selects: `save`, `back`, `rotate_cw`, `rotate_ccw`, `lens_correction`, `crop`, `crop_clear`, `reset_all`, `crop_aspect`, `lens_profile`; tab controls were dropped in review — the real UI has no tab buttons (navigation is `back`/Escape/`open`), and `click` refuses controls whose `enabled` is false so the vocabulary cannot exceed the honest affordances; collection ops deferred to a later protocol version), `key {name, mods}` (through the real `handle_key` dispatch — this is also how navigation `left`/`right`, undo/redo `z+ctrl`/`y+ctrl`, save `s+ctrl`, and zoom `f`/`1`/`=`/`-` are expressed; the drafted dedicated `zoom`/`next`/`prev`/`undo`/`redo`/`save_edited` commands were folded into `key`/`click` to keep one vocabulary per real input path), `set_crop {left, top, right, bottom}` (normalized; requires crop mode via `click crop`, emits the same committed-crop viewer event a drag produces; the drafted `toggle_crop` is `click crop`).

Session: `wait_idle {timeout_ms}` (responds when no detail load, EXIF read, local-edit persist, owed bake, or save is in flight — the anti-race primitive every script uses after `open`/`save`), `ping`, `quit`.

The `observe` control list is generated from the same enum/range sources the view uses (the pre-existing shared `slider_range`/`slider_step` functions), honoring the typed-controls principle: the agent discovers what it can do, it does not guess.

### Observation and statistics

`src/harness/stats.rs` computes, on CPU from RGBA pixels: per-channel mean and percentiles (p0.5/p1/p5/p50/p95/p99/p99.5), percent clipped at 0 and 255 per channel, a 64-bin luma histogram, and mean saturation. Stats attach to `dump_render`/`image_stats`/`compare_images` responses and are the primary "value range" evidence channel. Screenshots capture the full window (UI chrome included — slider positions and status text are part of what the user sees); the response reports the image-canvas bounds within the window so agents can reason about or crop the viewport region. GPU-vs-CPU tuning drift is assessed by comparing the screenshot's canvas region statistics against the CPU dump's statistics (exact pixel parity between paths is not asserted — sampling/scaling differ by design; statistical agreement is the honest v1 contract).

### Run artifacts and findings

Each harness launch creates `tmp/harness-runs/<run-id>/` (`run-id` = local timestamp + pid) containing: `session.json` (port + token), `manifest.json` (schema v1: runId, appVersion, startedAt/endedAt, stopReason: quit|disconnected|appClosed, artifacts list), `session.jsonl` (every request/response, artifacts by path — the replayable script), `artifacts/*.png`, and agent-authored `findings.jsonl`. The findings contract (documented in the guide, enforced by convention not code in v1) mirrors civ-engine's `ImprovementFinding`: `{schemaVersion: 1, id, title, severity: blocker|high|medium|low, category: valueRange|tuningDiff|parity|uiState|crash|ux, observed, expected, evidence: [{kind: screenshot|render|stats|compare|state, path?, note}], verificationStatus: unverified|verified|refuted, verificationMethod?: rerun|stats|screenshot|human, nextAction: proposalOnly|retuneMath|fixApp|addRegression|improveHarness|backlog}`. Committed docs get only durable outcomes (regression tests, tuning changes, thread REVIEW/devlog summaries); run dirs stay local and gitignored.

### The improvement loop (agent-side, documented in `docs/guides/agent-harness.md`)

1. Build and launch `photo --harness`, connect, `open` a fixture from `test_photos/`, `wait_idle`.
2. Observe: `screenshot` + `dump_render` + stats. Read the PNGs (multimodal), read the numbers.
3. Act like a user: adjust sliders, toggle lens correction, crop, rotate. Re-observe. Judge against intent and references (e.g. exported camera preview), using stats deltas as the measurable signal.
4. Record findings with evidence. Verify each finding by re-running its scenario (the session log replays via `harnessctl script`) before marking it `verified`.
5. Promote: verified tuning findings become `edit.rs` changes through the normal TDD/gates/review process, with the same scenario re-run before/after as the regression evidence; UX findings go to the thread/backlog. The loop repeats until observations stop yielding real findings.

Budgets live in the loop instructions (max actions, wall-clock, "stop when two consecutive passes yield no new findings"), not in-app.

## Module layout and boundaries

- `src/harness/mod.rs` — protocol types (requests, responses, observation/state/stats/manifest structs), serde codecs, schema version constant. JSON-stable; documented in `docs/api-reference.md`.
- `src/harness/server.rs` — TCP listener, token check, JSONL framing, subscription stream factory, session log + manifest writing. Only this file owns the socket and the run directory.
- `src/harness/stats.rs` — pure pixel-statistics functions (no I/O).
- `src/app/harness_exec.rs` — command → message/action translation and response assembly; the only bridge between harness types and `App`. Lives in `app/` because it orchestrates app state, honoring the existing boundary ("the app module owns all orchestration").
- `src/bin/harnessctl.rs` — std-only CLI client: reads `session.json`, sends one command (or a `script` of JSONL commands), prints the JSON response(s).
- Boundary rules for ARCHITECTURE.md: only `harness/server.rs` binds sockets and writes `tmp/harness-runs/`; only `app/harness_exec.rs` translates harness requests into app mutations; harness protocol changes bump the protocol schema version; the harness must never bypass `update()` to mutate state.

No new dependencies: serde/serde_json/tokio/image/log are already direct dependencies; `harnessctl` uses std only; the token uses existing entropy sources (no rand crate — hash of time+pid is sufficient for a localhost dev-tool token).

## Testing strategy (TDD)

- Protocol: round-trip serde tests for every command/response variant; unknown-command and malformed-JSON error paths; token rejection.
- Stats: exact-value tests on synthetic buffers (all-black, all-white, gradient, known-clip fractions); histogram bin edges; compare_images on crafted pairs.
- Command execution: `tests.rs`-idiom tests — construct `App`, feed `HarnessEvent::Request`s through `update()` inside the existing storage-isolation helpers, assert state mutations match the equivalent user actions (slider drag semantics including the first-change swallow, key dispatch, crop commit, controls list correctness, wait_idle resolution on load completion).
- Server: framing/token/accept-loop tests against a live localhost listener with a stub executor (no App, no GUI).
- Screenshot + full loop: not unit-testable without a window; covered by a live spike (screenshot content verified visually before building on it) and the end-to-end validation run, both recorded in the thread.

## Risks

- **Screenshot fidelity**: `iced_wgpu` 0.13.5's offscreen present includes custom primitives (source-verified), but empirical spike comes first; fallback is OS-level window capture via a helper, or CPU-dump-only observation (degraded but workable).
- **Slider drag-detection semantics**: the first `SliderChanged` of a drag is deliberately swallowed (`update.rs:293-307`); the harness emits the same two-change sequence a real drag produces, and the unit tests pin it.
- **Async races** (staged RAW loads, persist pipeline): `wait_idle` is the designed synchronization point; scripts that skip it are documented as nondeterministic.
- **Port conflicts / stale session files**: default port overridable, `0` = ephemeral; `session.json` carries pid so stale files are detectable.
- **Security**: localhost-only bind + per-run token + explicit opt-in flag; documented as a dev tool, not a production surface.

## Out of scope (v1)

OS-level input synthesis; multi-window; in-app LLM calls or auto-fix; pixel-exact GPU/CPU parity assertions; RAW-specific reference pipelines (the calibration harnesses under `PHOTO_RAW_TONE_COMPARE`/`PHOTO_RENDER_TUNING_GRID` stay as-is); Linux/macOS support (app is Windows-targeted today); findings schema enforcement in code (convention + guide in v1, promotable later exactly as civ-engine did in 1.4.0→1.6.0).
