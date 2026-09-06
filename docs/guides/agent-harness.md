# Agent Harness Guide

The agent harness lets an AI agent drive this app the way a user does — open photos, move sliders, crop, rotate, save — while inspecting the result both visually (real window screenshots, CPU render dumps) and mechanically (histograms, clipping fractions, percentiles, image diffs). It exists to close the edit-quality feedback loop: value-range problems and tuning-algorithm drift used to need a human looking at the screen and relaying "it still looks weird"; with the harness the agent sees and measures the render itself, decides the next adjustment, and iterates. Design and history: `docs/work/10_agent-harness/historical/threads/done/agent-harness/DESIGN.md` (civ-engine's visual-playtest / improvement-loop principles adapted to a desktop iced app).

## Quick start

```
cargo build --release
./target/release/photo.exe --harness                 # sandboxed storage, port 7878
./target/release/harnessctl.exe ping                 # run from the repo root
./target/release/harnessctl.exe open '{"path":"test_photos/<name>.jpg"}'
./target/release/harnessctl.exe wait_idle
./target/release/harnessctl.exe screenshot           # → run-dir artifacts/NNNN-screenshot.png
./target/release/harnessctl.exe set_slider '{"kind":"exposure","value":1.2}'
./target/release/harnessctl.exe dump_render '{"max_dim":1600}'
./target/release/harnessctl.exe quit
```

`--harness[=PORT]` accepts a port (`0` = ephemeral). Each launch creates `tmp/harness-runs/<run-id>/` containing `session.json` (port + token — `harnessctl` reads the newest automatically; `--run-dir` overrides), `manifest.json`, `session.jsonl` (every request/response — the replayable record), and `artifacts/`. Storage is **sandboxed by default**: `library.txt`, `collections.json`, `decoded-cache/`, `local-edits/`, and `edited/` all resolve into `<run-dir>/storage/`, so harness sessions can never pollute the real library. Pass `--harness-real-storage` only when the session must inspect the user's actual library. A bare path argument still opens that image at startup.

## Protocol

JSON lines over localhost TCP. First line of each connection is the session token from `session.json`. Requests are `{"id": N, "cmd": "...", "params": {...}}` (`params` optional when all parameters have defaults); responses are `{"id": N, "ok": true, "data": {...}}` or `{"id": N, "ok": false, "error": {"code", "message"}}`. Error codes: `bad_request`, `bad_token`, `invalid_params`, `unavailable`, `unsupported`, `io`, `timeout`, `quitting`, `internal`.

**Action responses mean "the gesture was performed", not "all resulting async work finished".** Decodes, EXIF reads, edit persists, and saves run asynchronously — call `wait_idle` before observing state you just changed and before reading files you just asked the app to write.

| Command | Params | What it does |
| --- | --- | --- |
| `ping` | — | Liveness + protocol/app version. |
| `observe` | — | Full UI state: tab, current image (path, load stage, logical/buffer dimensions, zoom), edit state (all 12 sliders, lens, rotation, crop, undo/redo), pending-work report, library count, collections, and the typed **controls list** (every control with kind, label, range/step/options, current value, enabled). |
| `observe_library` | `offset`, `limit` | Library page: paths, filenames, thumbnail presence and provenance (`original` / `persisted_local_edit`). |
| `screenshot` | — | Real window capture (GPU frame — exactly what the user sees, UI chrome included) → PNG in `artifacts/`; reports physical size, scale factor, and the logical canvas size. |
| `dump_render` | `source` (`current`/`original`), `max_dim` | CPU render of the loaded base through `edit::render_edited_image` — pixel-identical math to the export path. Stats are computed on the **full-resolution** render; the PNG may be downscaled to `max_dim` for cheap viewing. `original` renders identity edit state for before/after work. |
| `image_stats` | `path` | Stats for any PNG/JPEG on disk (relative paths resolve against the run dir). RAW files are not decodable here — dump or export first. |
| `compare_images` | `path_a`, `path_b` | Same-size comparison: per-channel mean/max absolute difference and the fraction of pixels differing beyond a ±2 tolerance. |
| `open` | `path` | Open an image into Detail (the drag-drop/CLI path). |
| `import_files` / `import_folder` | `paths` / `path` | Add to the library (the file-dialog completion path — the dialogs themselves cannot be driven; `add_folder`/`add_files` clicks are refused). |
| `set_slider` | `kind`, `value` | A real drag: two change events then a release, so drag detection and commit behave exactly as for a user. Values clamp to the widget range. Consecutive calls are safe — the harness clears the double-click-reset memory a rapid second release would otherwise trigger. |
| `reset_slider` | `kind` | Click the slider label (reset to 0 + commit). |
| `click` | `control`, `value?` | Named buttons/toggles/selects from the controls list: `save`, `back`, `rotate_cw`, `rotate_ccw`, `lens_correction`, `crop`, `crop_clear`, `reset_all`, `crop_aspect` (value `Freeform`/`Square`), `lens_profile` (value `Auto`/`None`/exact `"Maker Model"`). |
| `key` | `name`, `mods` | The real keyboard path: single characters or `escape`/`left`/`right`/`space`/`backspace`/`home`; mods `ctrl`/`shift`/`alt`. Navigation (`left`/`right`), undo/redo (`z+ctrl`/`y+ctrl`), save (`s+ctrl`), zoom (`f`, `1`, `=`, `-`). `o+ctrl` is refused (native dialog). |
| `set_crop` | `left`, `top`, `right`, `bottom` (normalized) | The crop drag-commit. Requires crop mode (`click crop` first), like a user. Committing exits crop mode. |
| `wait_idle` | `timeout_ms` (default 30000) | Responds when no detail load, EXIF read, edit persist, owed bake, or save is in flight. Import cache warming is deliberately excluded (background optimization). Timeout → `ok:false` with the pending report in the message. |
| `quit` | — | Finalize `manifest.json` (stop reason, artifact list), flush, exit the app. |

`harnessctl <cmd> [params-json]` sends one command; `harnessctl script <file.jsonl>` replays one command object per line (`#` comments allowed, ids auto-assigned) and stops on the first failure unless `--keep-going`. Exit codes: 0 ok, 1 a response was `ok:false`, 2 usage/transport.

## Reading the numbers

Stats are computed on sRGB-encoded 8-bit values (display-referred, matching what the user sees). Per channel and for Rec. 709 luma: mean, min/p1/p5/p50/p95/p99/max, and `clip_low_fraction`/`clip_high_fraction` (pixels at exactly 0/255 — the value-range red flags). Plus a 64-bin luma histogram and mean HSV saturation. Judging heuristics: rising `clip_high_fraction` under a positive exposure/contrast/whites move means blown highlights; p5 falling toward 0 with blacks/contrast means crushing shadows; `mean_saturation` tracks vibrance/saturation moves; p50 should hold roughly steady under contrast (it pivots around the midpoint). The GPU preview (screenshot) and CPU render (dump) share the same math through `edit::*_amount` mappings; statistical disagreement between the screenshot's canvas region and a dump at matching state is a parity finding worth filing — pixel-exact equality is not expected (scaling/sampling differ), statistical agreement is.

## The improvement loop

1. **Launch and anchor.** Build, launch `--harness`, `open` a fixture from `test_photos/`, `wait_idle`, then capture the anchor: `screenshot` + `dump_render {source: original}`. **Prefer the `.ARW` RAW fixtures — RAW editing is the user's primary real-world workflow** (staged loads, the default develop tone, full slider passes on developed bases); the JPGs are for quick mechanical checks. The `.xmp` sidecars next to the ARWs hold the user's real develop settings from their reference editor — when tuning slider math, treat them as the intent reference: apply comparable values through the harness and judge whether the app's rendering moves the way the reference editor's would (`test_photos/` is gitignored; fixtures and sidecars never enter the repo).
2. **Observe both ways.** Read the PNGs (multimodal) and the stats. Note what looks and measures wrong.
3. **Act like a user.** Adjust sliders / crop / lens via the real-dispatch commands. `wait_idle`, re-observe, `compare_images` against the anchor. Verify the numbers moved the way the intent said they should.
4. **Record findings.** Append one JSON object per finding to `<run-dir>/findings.jsonl`: `{"schema_version": 1, "id", "title", "severity": "blocker|high|medium|low", "category": "value_range|tuning_diff|parity|ui_state|crash|ux", "observed", "expected", "evidence": [{"kind": "screenshot|render|stats|compare|state", "path?", "note"}], "verification_status": "unverified|verified|refuted", "verification_method?": "rerun|stats|screenshot|human", "next_action": "proposal_only|retune_math|fix_app|add_regression|improve_harness|backlog"}`. A finding is a claim until verified: re-run its scenario (the `session.jsonl` you already produced is the script — convert the `req` payloads to a `harnessctl script` file) and only then set `verified`.
5. **Promote.** Verified `retune_math`/`fix_app` findings become normal TDD code changes with the full gates and review process; the same scenario re-run before/after (with `compare_images` + stats deltas) is the regression evidence. `add_regression` findings become unit tests; `ux`/`backlog` findings go to the thread docs. Run dirs stay local (gitignored) — commit only durable outcomes.
6. **Stop conditions.** Budget each session (e.g. max 40 actions or 15 minutes) and stop when two consecutive observe-act-measure passes yield no new findings — then write the summary.

The loop is deliberately proposal-only: the harness observes and acts in the app, but code changes always travel the ordinary verification path. Nothing about a harness session lowers the bar in AGENTS.md.

## Gotchas

- RAW loads are staged (embedded preview first, full develop later): `load_stage` tells you which pixels you are looking at; `wait_idle` before judging.
- `image_stats`/`compare_images` decode via the `image` crate: PNG/JPEG/etc., not RAW.
- The window must exist but need not be focused; don't type/click into the machine while a session runs — the app window is live.
- One client at a time; reconnect after a drop is supported (the session log keeps appending).
- `session.json` includes the pid — a stale run dir from a crashed app is detectable by checking whether that pid is alive.
- Abnormal exits leave `manifest.json` without `ended_at`/`stop_reason`; treat such runs as incomplete.
