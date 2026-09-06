# Agent Harness — Implementation Plan

Date: 2026-07-07. Executes `DESIGN.md` in this thread. TDD throughout: each step lands failing tests first, then the implementation, then the four gates locally (`cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`).

## Step 0 — Spike: screenshot fidelity (no commit)

Minimal wiring: `--harness` flag parsed, a temporary keybinding-free path that takes one `iced::window::screenshot` on a timer or trivial TCP trigger, PNG to `tmp/`. Open a test photo, verify the captured PNG shows the shader canvas (not blank) and UI chrome. This validates the design's core assumption before real work. Outcome recorded in the thread; if blank, stop and redesign observation per DESIGN.md fallback.

## Step 1 — Protocol types + stats (pure, no app coupling)

`src/harness/mod.rs`: `HarnessRequest`/`HarnessResponse`/command enum/params/`Observation`/`ControlSpec`/`ImageStatsReport`/`RunManifest` + serde. `src/harness/stats.rs`: percentiles, clip fractions, 64-bin luma histogram, mean saturation, image comparison. Tests: serde round-trips incl. unknown-command error mapping; exact stats on synthetic buffers (black/white/gradient/known-clip); compare on crafted pairs.

## Step 2 — Runtime storage sandbox

Runtime `OnceLock` overrides in `library.rs` (app storage dir) and `repo.rs` (photo repo root), set from `main()` only in harness mode, default = today's behavior. Tests: override set → both resolvers return sandbox paths (via a small indirection testable without touching the real `OnceLock` global — e.g. the resolver consults an injectable source in tests, mirroring the existing `#[cfg(test)]` isolation pattern).

## Step 3 — CLI parsing + harness activation

Parse `--harness[=PORT]`, `--harness-real-storage`, positional image path in `App::new`'s arg scan (hand-rolled, no new deps). Create run dir `tmp/harness-runs/<run-id>/` + `session.json` (port, token, pid) + initial `manifest.json`. Tests: arg-parse matrix; run-dir layout; manifest schema.

## Step 4 — Server + subscription wiring

`src/harness/server.rs`: accept loop, token first-line check, JSONL framing, `iced::stream::channel` factory emitting `HarnessEvent::Connected(sender)`/`Request(...)`/`Disconnected`; session.jsonl append; manifest finalization on quit/disconnect. Tests: localhost round-trip with a stub executor (bad token rejected, malformed JSON → error response, reconnect after drop works).

## Step 5 — Command execution bridge

`src/app/harness_exec.rs`: translate each command to the exact user-equivalent messages (slider two-change+release drag; key events through `handle_event`; crop-commit viewer event; named button messages), assemble `observe` (state + controls from shared `SliderKind::range()` — extract ranges from `view.rs` inline values into the shared source as part of this step), `wait_idle` waiter registry checked on relevant completion messages, `dump_render`/`screenshot` task chains (spawn_blocking PNG encode; `window::get_latest().and_then(window::screenshot)`), `observe_library`, `compare_images`, `quit`. Tests (the bulk): `tests.rs`-idiom coverage per command asserting parity with real-user message sequences, controls-list correctness, wait_idle resolution/timeout, response id fidelity, unknown control errors.

## Step 6 — harnessctl client

`src/bin/harnessctl.rs` (std-only): `harnessctl <cmd-json>`, `harnessctl script <file.jsonl>`, `--run-dir`/`--port` discovery via newest `tmp/harness-runs/*/session.json`. Prints response JSON lines to stdout, nonzero exit on `ok:false`/transport failure. Tests: arg parsing + session-file discovery (pure fns); transport covered by step-4 server tests + live validation.

## Step 7 — Live end-to-end validation

Build release, launch `photo --harness`, drive the real loop from this session: open `test_photos/` fixture → wait_idle → screenshot + dump_render + stats → set sliders (e.g. exposure +1.5, shadows +40) → re-observe → verify stats move as expected + read PNGs → save_edited → verify export exists in sandbox → quit → verify manifest finalized. Record a findings.jsonl exercising the contract. This is the `/verify` equivalent and the spike's full-loop successor.

## Step 8 — Docs + guide

`docs/guides/agent-harness.md` (canonical agent-facing guide: launch, protocol, vocabulary, loop, findings contract, budgets, replay). AGENTS.md pointer section. `docs/api-reference.md` harness protocol section. ARCHITECTURE.md component map + boundaries + data-flow; drift-log row; decisions.md entry (in-app TCP harness over OS automation; sandbox-by-default). Changelog v0.2.8 entry; devlog summary + detailed entry; Cargo.toml 0.2.7 → 0.2.8.

## Step 9 — Adversarial review + ship

In-process Workflow review (finder dimensions: correctness/concurrency, protocol/serde, app-boundary fidelity, security, docs accuracy; independent refuting verifiers against live code), fix real findings, iterate to nitpick-only; REVIEW.md per iteration under this thread. Full gates. Commit to main (explicit paths, no blanket add), push, move thread to `docs/threads/done/agent-harness/`.

## Deliberate non-goals

See DESIGN.md "Out of scope". Notably no new crate dependencies (skips the dependency-change protocol) and no findings-schema enforcement in code in v1.
