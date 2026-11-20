# Photo — local rules

Standing rules for this repository that have no mechanical trigger. Anything that can be checked by a test is a test instead. The first group was carried over from the previous app on 2026-11-19 because it is the owner's guidance and still applies; the second group is new with the restart.

## Colour science

Do not describe a tuning as "Lightroom-style" in a commit, devlog or comment without a published Adobe reference.

Adobe's Basic-panel math is proprietary and has changed across process versions, so the claim is unfalsifiable and the next session inherits it as fact. Cite what was actually used: a paper, a darktable source file, or one of Adobe's published references (the DNG specification, the DNG SDK, the XMP specification). "Familiar to Lightroom users" is a legitimate design goal for names, ranges and workflow; it is not a citation for math.

Photographer-facing contrast and tone controls operate in perceptual space, not linear.

A sigmoid pivot at linear 0.5 sits at L\* ≈ 76 — bright midtones, not middle grey — which is why a curve that is correct on paper feels wrong to a photographer. Apply the curve in gamma-2.2 perceptual space with the pivot at 0.5. Zone centres in the same space: Blacks near black only, Shadows a bell around 0.32, Highlights a bell around 0.72, Whites near white only, so adjacent zones overlap smoothly without leaking into the midtones.

When a tone-zone test picks a sample pixel, put it inside the zone's peak.

A borderline pixel produces a small but technically passing response, which reads as "the slider works" while hiding a mismatch about which zone it belongs to.

Watch for regime cliffs where an intermediate RGB value can go negative.

Chromatic adaptation at tungsten temperatures is the known one: a negative channel silently short-circuits every later luminance-based stage. Clamp to non-negative at the source of the cliff rather than reasoning about each downstream branch.

## Loading

Full-quality RAW decoding is expensive, so show the embedded preview first where one exists, and keep the user's zoom and pan across the swap to the developed image.

## Test infrastructure

A process-wide override in a test is guarded by a mutex held across the whole override window, not just the write. Rust runs tests in parallel, so an override that is merely set and unset lets two tests read each other's value. Per-thread state is the alternative when the value is only ever read on the calling thread.

After any `cargo fix` run, re-verify with `cargo check --all-targets`. `cargo fix` is not reliably multi-target-aware and can remove imports that only the test target uses.

## New with the restart

No library code computes a default storage location. Only `src/main.rs` may ask the platform where the user's catalog lives; every function below it takes explicit paths. This is what keeps tests away from real data, and a convenience helper that "just uses the default catalog" would quietly undo it.

Every adjustment exists once. If a second code path ever needs the result of an operator — a faster preview, a thumbnail, an export — it calls the same function or consumes a table generated from it. It does not reimplement it.

UI work reads the vendored egui source. egui's API changes substantially between minor versions and code written from memory compiles against the wrong one. The source is under `~/.cargo/registry/src/`.

Every interactive widget has an accessible name. It is how tests and the harness find controls, and `egui_kittest` fails on unnamed inputs.
