# photo — local rules

Rules that apply to this repo and no other. The fleet constitution's core lives in AGENTS.md's FLEET-CANON block and is written by the sync; the rest of it is in `../fleet/docs/skills/coordinator.md` and `../fleet/docs/work-rules.md`. This file is the one the sync never opens.

Everything here has no mechanical trigger. Anything that could be gated is a gate instead, with its claim in the gate's own header and its mutation proof in `docs/learning/gate-proofs.md`.

These are the owner's rules from before the 2026-11-19 restart that still bind the new code, in their original words. Sentences that described mechanisms the restart removed were cut, not reworded: the three-layer CPU/GPU parity check, baked-pixel persistence, the decoded-cache validation and pruning contracts, and the names of test helpers that no longer exist. The file as it stood is at `git show 94bd2cf:docs/policies/local-rules.md`. Nothing was added; only the owner adds a rule here.

## Colour science

Do not describe a tuning as "Lightroom-style" in a commit, devlog or comment without a published Adobe reference.

Lightroom's Basic-panel math is proprietary and has changed across Process Versions, so the claim is unfalsifiable and the next session inherits it as fact. Cite darktable or a specific paper instead: `basicadj.c` defines the power-law-in-linear-RGB contrast around middle gray 0.1842, and `toneequal.c` defines the Gaussian-in-EV tone bands. "Lightroom-feel" is a legitimate design goal and a legitimate word for one — it just is not a citation.

Photographer-facing contrast and tone controls operate in perceptual space, not linear.

A sigmoid pivot at linear 0.5 sits at L\* ≈ 76 — bright midtones, not middle gray — which is why a curve that is correct on paper feels wrong to a photographer. Apply the curve in gamma-2.2 perceptual space with the pivot at L_p 0.5. Tone-zone centers follow Lightroom's semantics in the same space: Blacks near-black only, Shadows a bell around L_p 0.32, Highlights a bell around L_p 0.72, Whites near-white only, so adjacent zones overlap smoothly without leaking into the midtones.

When a tone-zone test picks a sample pixel, put it inside the zone's peak.

A borderline pixel produces a small but technically passing response, which reads as "the slider works" while hiding a semantic mismatch about which zone it belongs to.

The tone bands are deliberately tighter than darktable's.

darktable's σ = √2 gives wide tails on purpose — smooth overlap is a feature there, not a leakage bug. This repo chose σ = 1 with 1.5 EV of per-band reach because at σ = √2 a single full-strength slider reads as a global exposure shift rather than targeted recovery. Porting more darktable math means deciding case by case which of its choices to keep, not assuming either answer.

Watch for regime cliffs where an intermediate RGB value can go negative — Bradford CAT at tungsten temperatures is the known one — and silently short-circuit every later luminance-based stage. Clamp to non-negative at the source of the cliff rather than reasoning about each downstream branch.

## Persistence

Undo and redo are session-scoped by product decision; cross-session undo is not part of the contract.

## Loading

Full-quality RAW decoding is expensive, so the load plan is chosen up front and the staged load state is modelled explicitly.

Show a fast embedded RAW preview first where one exists; keep the preview-to-full swap on the user's existing zoom and pan instead of inventing a resolution-based zoom correction.

## Test infrastructure

A process-wide override in a test is guarded by a mutex held across the whole override window, not just the write.

Rust's test harness runs tests in parallel by default, so an override that is merely set and unset lets two tests read each other's fake roots. Per-thread state is the alternative when the value is only ever read on the calling thread.

After any `cargo fix` run, re-verify with `cargo check --all-targets`.

`cargo fix` is not reliably multi-target-aware: it has removed imports that only the `cfg(test)` target used, leaving the bin target green while `cargo test` failed with resolution errors. Prefer `#[cfg(test)]`-gated imports for test-only names so the two targets cannot drift.
