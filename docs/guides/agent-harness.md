# The agent harness

`photo --harness` runs the real app without a window and lets an agent drive it the way a user does, over JSON lines on localhost, while it measures what the app shows. `harnessctl` is the client. This guide is the canonical description of both; `AGENTS.md` ("Agent harness") points here, and the design is in `docs/work/12_lightroom-workflow-restart/design.md` §8.7.

The harness is the real `PhotoApp` running inside `photo_ui::harness` (the same driver the UI tests use), rendering real GPU frames offscreen. Every action goes through the app's own input: pointer gestures on controls found by their accessible names, key presses, typed text. Nothing pokes the app's state directly. Reading goes through the app's and the engine's read-only accessors and the accessibility tree.

## Quick start

Build the release binaries once: `cargo build --release`.

Start a session from the checkout: `./target/release/photo --harness`. It listens on 127.0.0.1:7878 and prints its run directory; `--harness=0` picks a free port instead (the port is in `session.json`), and a path after the flags opens that photo at startup (`./target/release/photo --harness=0 test_photos/IMG_0001.CR3`).

Talk to it: `./target/release/harnessctl ping`, then for example `./target/release/harnessctl observe '{"controls": false}'`. On Windows the binaries are `photo.exe` and `harnessctl.exe`.

End it with `./target/release/harnessctl quit`, which finalises the run's manifest.

Storage is sandboxed by default: the catalog, its previews and the export folder live in `<run-dir>/storage/`, so a session never touches the real library. `--harness-real-storage` uses the user's own stores instead; use it only to look at the real library, never for experiments.

Local photo fixtures belong in `test_photos/` at the repository root, which git ignores. The RAW files there with the owner's XMP sidecars are the reference for tuning.

## The run directory

Each launch creates `tmp/harness-runs/<run-id>/` in the checkout (outside a checkout, `photo-harness-runs/` in the system temporary directory, which `session.json` then says). The run id is the UTC start time and the process id, so run directories sort by time.

`session.json` says how to reach the session and what it is: `port`, `token`, `pid`, `run_id`, `run_dir`, `started_at`, `storage_root`, `sandboxed`, `runs_location` (`checkout` or `system_temp`), and the protocol and app versions.

`manifest.json` is the run's record: start and end times, `stop_reason` (`null` while running, `quit` once a client quit, `startup_failed`), the storage root, the number of requests, and every artifact with its kind, size, request id and time. It is rewritten whenever an artifact is added, so a session that dies still lists what it made.

`session.jsonl` holds every request and response, one JSON object per line with a `kind` (`request`, `response`, `unparsed`, `connect`, `token`, `refused`, `disconnect`, `start`, `opened`, `start_failed`, `stop`) and `at_ms` since the start. It is the replayable record: `harnessctl script <run-dir>/session.jsonl` sends its requests again, in order. The token is never logged.

`artifacts/` holds the screenshots and renders, numbered in the order they were made (`0001-screenshot.png`).

`findings.jsonl` is the agent's own file, described under "Findings" below.

## harnessctl

`harnessctl <command> [params-json]` sends one command and prints the response as one JSON line. `harnessctl script <file.jsonl> [--keep-going]` sends one command object per line (`{"cmd": "key", "params": {"name": "3"}}`; ids are assigned; lines starting with `#` are comments; a session log's records other than requests are skipped) and stops at the first response that is not ok unless `--keep-going` is given.

It talks to the newest `session.json` under the checkout's `tmp/harness-runs/`; `--run-dir DIR` names a run directory (or a folder of them) instead.

Exit codes: 0 when every response was ok, 1 when a response was not ok, 2 for a usage error or a session that cannot be reached (a `session.json` can outlive its process).

## The protocol

A connection's first line is the session token from `session.json`. A wrong token is answered with a `bad_token` error and the connection is closed; the right one is not answered.

After it, each line is a request, `{"id": N, "cmd": "...", "params": {...}}`, answered by exactly one line: `{"id": N, "ok": true, "data": {...}}` or `{"id": N, "ok": false, "error": {"code": "...", "message": "..."}}`. `params` may be left out. Unknown parameters are refused, so a misspelt one is not silently ignored.

Error codes: `bad_request` (the line is not a request), `bad_token`, `invalid_params` (missing, mistyped or out of range; also an ambiguous control name), `not_found` (no such file or control), `unavailable` (not possible now: a disabled or off-screen control, no GPU, a dialog in the way, another client connected), `unsupported` (no such command, or the app lacks what it needs), `io`, `timeout`, `quitting` (lines that arrive after `quit`), `internal` (a failure inside the harness or the app, a panic included).

One client at a time; a second connection is answered `unavailable` and closed. Reconnecting is fine, and a client that has closed its connection makes way for the next at once, which is what lets `harnessctl` connect once per call.

An action's response means the gesture was performed, not that the work it started has finished: an import, a preview render or a scan runs on after the answer. `wait_idle` is how a client learns it has finished. Between requests the app keeps running frames whenever it asks for one (a worker finished, an animation, a toast due to go), so engine work progresses and the UI stays current.

An action's data can carry `warnings`: the UI was still changing when the action ended, or a control has no accessible name. The action was carried out either way.

## Commands

| Command | Parameters | What it does |
| --- | --- | --- |
| `ping` | | Protocol and app version, pid, run id and directory, uptime, whether storage is sandboxed, the GPU adapter (`gpu.available`, `gpu.adapter`), the window size and pixel density. |
| `observe` | `controls` (default true) | The module, view and zoom, whether a dialog is open, the layout and open panels, the source, filter, sort and photo counts, the selection, the active photo (id, file name, path, kind, rating, flag, label, size, and the load state of its thumbnail, standard preview and develop session), whether the engine is idle, pending work, tasks, toasts (`showing` now and the `recent` twenty), undo labels, the pointer, in Develop the canvas (`develop`: its zoom, `photo_rect`, the open `tool`, `crop_rect`, `selected_spot`, `selected_mask`, `mask_overlay`; see "The Develop tools"), and every control in the accessibility tree with role, name, value, number and range, enabled state, rectangle, toggled, selected, expanded and focused. |
| `observe_library` | `offset` (0), `limit` (100, at most 1000) | A page of the current photo list in grid order: index, id, file name, kind, rating, flag, label, `thumbnail_ready`, selected, active, virtual copy, edited, missing. |
| `screenshot` | | Renders the frame the user would see at the harness's pixel density and saves it as a PNG artifact; answers its path (relative to the run directory), pixel size, `pixels_per_point` and logical size. `unavailable` without a GPU. |
| `image_stats` | `path` | Statistics of a PNG or JPEG on disk (see below). A relative path is taken from the run directory, so a screenshot's `path` can be passed straight on. |
| `compare_images` | `path_a`, `path_b` | How two images of one size differ (see below). |
| `dump_render` | `source` (`current`, the default, or `original`), `max_dim` (2048, from 16 to 16384) | The active photo through the develop pipeline at full resolution: `current` with its settings as they are now (including an edit not yet committed in Develop), `original` with the defaults a new import of its kind gets. Answers the render's size and time, the statistics of the full render (see below), and a PNG artifact scaled down so its longer side is at most `max_dim` (never enlarged). `unavailable` when no photo is active. |
| `import_folder` | `path`, `recursive` (true), `timeout_ms` (120000) | Through the real Import dialog: opens it (its button, or ⇧⌘I when the panels are hidden), sets "Include subfolders", types the path into the folder field and presses Enter, waits for the scan, and presses `Import N Photos`. Answers what the scan found and what was ticked; with nothing to import it cancels the dialog and says why. |
| `open` | `path` | Opens the photo in the running app: imported in place (unless the catalog holds it), made active, alone in the selection, and shown, in the loupe from the Library and in Develop when that is the module. Answers the photo (`opened.id`, `file_name`, `path`, `imported`) and the `module` and `view` now showing. |
| `click` | `name`, `role` | Clicks the middle of the control with that accessible name. |
| `double_click` | `name`, `role` | Double-clicks it. |
| `key` | `name`, `mods` | Presses and releases a key with modifiers held: `{"name": "z", "mods": ["cmd", "shift"]}` or `"mods": "cmd+shift"`. |
| `type_text` | `text` | Types into whatever has keyboard focus (warns when nothing has). |
| `drag` | `from`, `to`, `mods` | Presses at `from`, moves in steps, releases at `to`, with the modifiers held from the press to the release. With `from` and `to` the same point it is a click there. |
| `hover` | `at` | Moves the pointer there. |
| `set_slider` | `name`, `value` | A real drag of the named slider ending in a release, so commit-on-release behaves as for a user. The value is held to the slider's range; the answer gives the value reached and whether it is exact (within half a step). |
| `reset_slider` | `name` | Double-clicks the slider, which puts a develop slider back to its default. |
| `wait_idle` | `timeout_ms` (30000, at most 600000) | Runs frames until the engine reports idle and the UI has come to rest; on timeout, `timeout` with the pending work in the message. |
| `quit` | | Finalises `manifest.json` and ends the session. |

Controls are found by accessible name, exactly as `observe` lists them (`"Import…"` has a real ellipsis). `role` is optional: `Button`, `CheckBox`, `TextInput`, `Slider`, `ListBoxOption` (a grid cell, named by its file name), `RadioButton` and so on, in any case and with or without underscores. When several controls share a name, an interactive one wins over text; a remaining tie is `invalid_params`, and giving `role` settles it. A name that is not on screen is `not_found`, with similar names suggested.

Keys are single characters (`"3"`, `"e"`, `"["`, `"\\"`) or names in any case: `escape`, `enter`, `tab`, `space`, `backspace`, `delete`, `home`, `end`, `page_up`, `page_down`, `left`, `right`, `up`, `down`, `f1` to `f35`. Modifiers are `cmd`, `ctrl`, `shift` and `alt`; `cmd` is the Command key on a Mac and Ctrl on Windows, and `ctrl` on a Mac is the Control key, which no Library shortcut uses. The Library's shortcuts are those of design §8.5: G grid, E loupe, C compare, N survey, 0 to 5 rating, P, X and U flags, 6 to 9 labels, arrows to move, ⇧⌘I import. D develops the active photo, and ⇧⌘E opens the Export dialog from either module.

Coordinates and rectangles are logical points from the top left of the window. A screenshot has `pixels_per_point` pixels per point; the harness runs a 1600 x 1000 point window at 1 pixel per point.

## The Develop tools

Crop, healing and masking are driven with the same commands, the way a user drives them. A key or a button of the tool strip under the histogram opens a tool: `r` or "Crop", `q` or "Healing", `m` or "Linear gradient mask", `m` with `shift` or "Radial gradient mask", `k` or "Brush mask". `enter` (or the panel's "Done") records what the tool changed as one history step, `escape` (or "Discard") drops it, and opening another tool or moving a panel's slider records the open tool's step first. `o` shows and hides the selected mask's red overlay, `x` swaps the crop's orientation, `delete` removes the selected spot or mask.

The tool's panel is under the strip, and its controls are found by name like any others: "Crop aspect", "Lock aspect", "Swap", "Crop Angle", "Straighten", "Rotate left", "Rotate right", "Flip horizontal", "Flip vertical", "Reset Crop"; "Heal", "Clone", "Spot Size", "Spot Feather", "Spot Opacity", "Delete Spot"; "Mask name", "Invert mask", "Show mask overlay", "Delete Mask", "Add", "Subtract", "Remove Shape", "Mask Amount", "Mask Exposure" and the other "Mask …" sliders, "Radial Feather", "Invert radial", "Brush Size", "Brush Feather", "Brush Flow". `observe` lists what is there.

Gestures on the photo are `drag`s aimed with `observe.develop`. `photo_rect` is the box the photo takes on the canvas, `[left, top, right, bottom]` in points, part of it perhaps out of view when zoomed in: the point at fractions (u, v) of the picture shown is at `left + u * width`, `top + v * height`. While the crop tool is open the canvas shows the whole frame straightened by the crop's angle, `photo_rect` is the box around it, and `crop_rect` is the crop rectangle: drag a corner or an edge of it to resize it and inside it to move it; a drag that starts outside it does nothing. With "Straighten" pressed, a drag along a line that should be level sets the angle. In the healing tool a click (a `drag` from a point to the same point) places a spot with the suggested source, and a real drag sizes one. In a mask tool a drag places a linear or radial gradient or paints with the brush; `"mods": ["alt"]` makes the brush rub out. `tool`, `selected_spot`, `selected_mask` and `mask_overlay` say what is open and chosen.

`dump_render` with `current` includes what an open tool has changed, before it is recorded, so a tool's effect is measured without committing it.

## The statistics

Every statistic is computed on 8-bit sRGB-encoded values, 0 to 255, over every pixel of the image. Alpha is ignored. Percentiles are nearest-rank: `p5` is the smallest value at or below which at least 5% of the pixels lie.

`dump_render` measures the same way: its statistics are of the full-resolution render in the 8-bit sRGB display encoding the user sees (the pipeline's 8-bit output, the same encoding `image_stats` reads from a PNG), not of the scaled-down PNG it saves and not of the pipeline's 16-bit export precision. So `image_stats` on its PNG will differ slightly from its own statistics wherever scaling averaged neighbouring pixels.

`channels.r`, `channels.g`, `channels.b` and `channels.luma` each give `mean`, `min`, `p1`, `p5`, `p50`, `p95`, `p99`, `max`, `clip_low_fraction` (the fraction of pixels at exactly 0) and `clip_high_fraction` (at exactly 255). Luma is Rec. 709 luma of the encoded values, Y' = 0.2126 R' + 0.7152 G' + 0.0722 B' (not linear luminance); its clip fractions count pixels black or white in every channel, and its percentiles are to a hundredth of a level.

`luma_histogram` has 64 bins of four levels each (bin `i` covers luma from `4i` to `4i + 4`, the last includes 255), as fractions of all pixels. `mean_saturation` is the mean HSV saturation, `(max - min) / max` per pixel.

`compare_images` gives per channel `mean_abs_diff` and `max_abs_diff`, `differing_pixels` and `differing_fraction` (pixels where some channel differs by more than 2 levels), and `identical`.

How to read them:

A rising `clip_high_fraction` under positive Exposure or Whites means highlights are blowing out; look at which channel clips first, since one channel at 255 while the others are not is colour clipping and shifts the hue.

`p5` falling toward 0, with `clip_low_fraction` rising, means shadows are being crushed. Expected under negative Blacks; a defect under positive Shadows.

A Highlights or Shadows move should change its own end of the distribution (`p95` and `p99`, or `p1` and `p5`) and leave `p50` nearly where it was; a large `p50` shift means the tone band is leaking into the midtones, which `docs/policies/local-rules.md` rules out.

`p99` close to `max` with a non-zero `clip_high_fraction` is a wall: values pile up against the top instead of rolling off. A histogram with mass in its last bin says the same; regular gaps between bins (combing) mean posterisation.

Channel means that differ on a neutral subject are a colour cast; the white balance moves them together.

`mean_saturation` should rise under Vibrance and Saturation and stay put under Exposure and Contrast; when it moves with tone controls, colour and tone are coupled.

`compare_images` answers whether a change had an effect (`differing_fraction` above zero) and whether something that should change nothing did (zero). A `max_abs_diff` of 1 or 2 is rounding noise and does not count as differing.

A screenshot is the whole window, interface included. The interface is a neutral dark grey (design §8.8), so its statistics are dominated by the chrome; judge the photo's own tones from `dump_render`, and compare `current` with `original` to see what the settings did.

## The improvement loop

The loop exists so the agent sees the result itself instead of asking a person how it looks. It is meant to make the agent harder to fool, not more confident.

Launch and anchor: build the release binaries, start `photo --harness=0` with the fixture (a file from `test_photos/`), `ping`, `wait_idle`, then take a screenshot and a `dump_render` with their statistics as the anchor every later observation is compared with. Note the run id.

Observe both ways: read the PNG (look at it at full size) and read the numbers (`observe`, `image_stats`). Eyes alone misjudge subtle tones and numbers alone miss layout and colour mistakes; a finding needs both to agree.

Act like a user: change things through the controls a user has (`click`, `key`, `set_slider`, `drag`, `type_text`), `wait_idle`, observe again, and `compare_images` against the anchor. Never reach around the UI.

Record findings in `<run-dir>/findings.jsonl` as they come, each with its evidence (see below). A finding is a claim until it is verified.

Verify each finding by re-running its scenario (`harnessctl script` with the requests that produced it, from `session.jsonl`) or by recomputing the statistics, and mark it `verified` or `refuted`.

Promote verified findings: a regression test in the crate that owns the behaviour comes first (TDD), then the fix through the normal gates and review, with the scenario re-run before and after as evidence. Tuning changes cite their real sources, never "Lightroom-style" (`docs/policies/local-rules.md`). UX findings go to the backlog when they are not fixed in the same change.

Stop when two consecutive passes find nothing new.

## Findings

One JSON object per line in `<run-dir>/findings.jsonl`:

```json
{"schema_version": 1, "id": "F1", "title": "Whites +60 blows the sky", "severity": "high", "category": "value_range", "observed": "clip_high_fraction rose from 0.002 to 0.081 with Whites at +60", "expected": "a highlight roll-off below 0.01", "evidence": [{"kind": "stats", "path": "artifacts/0004-screenshot.png", "note": "after set_slider Whites 60"}], "verification_status": "unverified", "next_action": "retune_math"}
```

`severity` is `blocker`, `high`, `medium` or `low`. `category` is `value_range` (clipping, crushing, ranges), `tuning_diff` (a control behaves unlike its reference), `parity` (two paths that should agree do not), `ui_state` (the UI shows something other than what the engine holds), `crash` or `ux`. `observed` and `expected` are sentences. `evidence` lists objects with `kind` (`screenshot`, `render`, `stats`, `compare`, `state`), an optional `path` relative to the run directory, and a `note`. `verification_status` is `unverified`, `verified` or `refuted`; `verification_method` (optional) is `rerun`, `stats`, `screenshot` or `human`. `next_action` is `proposal_only`, `retune_math`, `fix_app`, `add_regression`, `improve_harness` or `backlog`.

Run directories stay local and ignored. What lasts goes into the repository as regression tests, fixes and devlog lines.

## Gotchas

Always `wait_idle` after `import_folder`, `open`, and anything that renders; an answer only means the gesture was made.

`dump_render` renders on the session's own thread and blocks it: a full-resolution RAW render can take seconds, during which no other request is answered and no frame runs. The first render of a file also decodes it; later ones reuse the engine's cache of decoded files.

Time in the app passes only as frames run, a sixtieth of a second each. The harness runs frames between requests when the app asks for them, so a toast stays up for about 2.2 seconds of such frames; `observe` keeps the last twenty toasts in `toasts.recent` either way.

`observe` runs one frame to read the accessibility tree, so what it reports is that frame.

A control that is disabled, or whose middle lies outside the window (scrolled out of view), is `unavailable` to `click`. There is no scroll command: bring a lower Develop panel into view by collapsing the panels above it (click their headers, which are `DisclosureTriangle` controls named "Basic", "Tone Curve" and so on), as a user short of screen would.

No local photos? `cargo run -p photo-testkit --example write_fixtures -- 1500` writes three JPEGs, a Bayer DNG, a PNG and a 16-bit TIFF into a new folder under the system temporary directory and prints the folder; copy it into `test_photos/` to import it by a relative path. The synthetic DNG is exposed like a finished picture, not like a camera exposes, so it opens about a stop bright at the default settings.

`open` does not start the app again: the panels, the view's settings and the undo stack are as they were. A photo that was new to the catalog leaves the Previous Import showing, as every import does; one the catalog held but the list did not show is found in All Photographs with the filter cleared. (Until protocol version 2, `open` restarted the app and answered `relaunched`.)

`key` presses keys; it does not type. Text goes in with `type_text`, after a click has given a text field focus.

Screenshots do not show the pointer (hover highlights do show); `observe.pointer` says where it is.

Two clicks on one spot within the double-click time would merge into a double-click, so the harness lets enough simulated time pass between separate clicks, presses and drags; `double_click` and `reset_slider` mean it.

The harness renders on a hardware GPU when there is one (`ping.gpu.adapter` names it) and falls back to a software rasterizer; with no adapter at all, `screenshot` is `unavailable` and everything else still works.

Relative paths in `image_stats` and `compare_images` are taken from the run directory; relative photo and folder paths in `open` and `import_folder` from the checkout, so `test_photos/...` works as written.

A stale `session.json` can name a port nobody listens on any more; `harnessctl` then exits with 2. Start a new session, or pass `--run-dir` for the one meant.
