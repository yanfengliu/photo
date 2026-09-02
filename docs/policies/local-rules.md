# photo — local rules

Rules that apply to this repo and no other. The fleet constitution lives in AGENTS.md's FLEET-CANON block and is written by the sync; this file is the one the sync never opens.

Everything here has no mechanical trigger. Anything that could be gated is a gate instead, with its claim in the gate's own header and its mutation proof in `docs/learning/gate-proofs.md`.

## Colour science

Do not describe a tuning as "Lightroom-style" in a commit, devlog or comment without a published Adobe reference.

Lightroom's Basic-panel math is proprietary and has changed across Process Versions, so the claim is unfalsifiable and the next session inherits it as fact. Cite darktable or a specific paper instead: `basicadj.c` defines the power-law-in-linear-RGB contrast around middle gray 0.1842, and `toneequal.c` defines the Gaussian-in-EV tone bands. "Lightroom-feel" is a legitimate design goal and a legitimate word for one — it just is not a citation.

Photographer-facing contrast and tone controls operate in perceptual space, not linear.

A sigmoid pivot at linear 0.5 sits at L\* ≈ 76 — bright midtones, not middle gray — which is why a curve that is correct on paper feels wrong to a photographer. Apply the curve in gamma-2.2 perceptual space with the pivot at L_p 0.5. Tone-zone centers follow Lightroom's semantics in the same space: Blacks near-black only, Shadows a bell around L_p 0.32, Highlights a bell around L_p 0.72, Whites near-white only, so adjacent zones overlap smoothly without leaking into the midtones.

When a tone-zone test picks a sample pixel, put it inside the zone's peak.

A borderline pixel produces a small but technically passing response, which reads as "the slider works" while hiding a semantic mismatch about which zone it belongs to.

The tone bands are deliberately tighter than darktable's.

darktable's σ = √2 gives wide tails on purpose — smooth overlap is a feature there, not a leakage bug. This repo chose σ = 1 with 1.5 EV of per-band reach because at σ = √2 a single full-strength slider reads as a global exposure shift rather than targeted recovery. Porting more darktable math means deciding case by case which of its choices to keep, not assuming either answer.

## CPU/GPU parity

The CPU save path and the GPU preview share twelve editing sliders, lens correction, the clarity/dehaze blur and the vignette. Verify parity at three layers, because kernel parity alone is not enough:

1. The formula.
2. The sampling — bilinear against nearest. A Gaussian-matched CPU blur still produces stepwise artifacts if its lookup uses `x/4, y/4` integer indexing while the GPU sampler interpolates.
3. The UV the sampler receives — before or after distortion.

Watch for regime cliffs where an intermediate RGB value can go negative — Bradford CAT at tungsten temperatures is the known one — and silently short-circuit every later luminance-based stage. Clamp to non-negative at the source of the cliff rather than reasoning about each downstream branch.

## Persistence and caching

Undo and redo are session-scoped by product decision; cross-session undo is not part of the contract.

That is why this repo persists baked local image data rather than a log of edit operations, and why restart reopens from the baked local copy. Reversing that decision means changing the contract, not adding a file format.

A same-session cache captures its source validation in the same worker that starts reading the file, and a fast cache-hit path validates under a write-denying read handle.

Validating in a different task than the one that reads leaves a window where the file changes between the check and the read, and repeat opens serve pixels that no longer describe the file on disk.

Cache housekeeping is part of the hot path and is budgeted as such.

An unchanged hit stays on the metadata fast path; full-file verification runs only when metadata changed but content identity still matters; pruning is amortized across writes rather than rescanning the whole cache on every one. This is a performance contract, so it has no test: a gate here would either assert an implementation detail or measure wall-clock time on a shared machine. Reintroducing per-write pruning leaves the suite green — it is slow, not wrong.

## Loading

Full-quality RAW Detail decoding is expensive, so the load plan is chosen up front and the staged Detail-load state is modelled explicitly.

Show a fast embedded RAW preview first where one exists; keep the preview-to-full swap on the user's existing zoom and pan instead of inventing a resolution-based zoom correction; gate save until the full image and any required auto-lens metadata are ready.

## Test infrastructure

A process-wide override in a test — the repo-root override is the live example — is guarded by a mutex held across the whole override window, not just the write.

Rust's test harness runs tests in parallel by default, so an override that is merely set and unset lets two tests read each other's fake roots. `repo::with_test_photo_repo_root_override` holds `TEST_PHOTO_REPO_ROOT_GUARD` for the duration of the closure and recovers from poisoning. Per-thread state is the alternative when the value is only ever read on the calling thread — `library::TEST_APP_STORAGE_DIR` uses a thread-local for exactly that reason.

After any `cargo fix` run, re-verify with `cargo check --all-targets`.

`cargo fix` is not reliably multi-target-aware: after the main.rs module split it removed imports (`use repo::*;`, `std::io::{BufWriter, Write}`, `std::fs::File`) that only the `cfg(test)` target used, leaving the bin target green while `cargo test` failed with 17 resolution errors (E0425/E0433). Prefer `#[cfg(test)]`-gated imports for test-only names so the two targets cannot drift.
