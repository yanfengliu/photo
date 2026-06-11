# Lightroom Param Parity — Plan

See DESIGN.md for the mapping contract. Implementation order (TDD: each step's tests land before its code):

1. Tests first: new mapping-function tests in `edit.rs` (range anchors, contrast pivot slope + middle-gray invariance, saturation grayscale endpoint, temperature kelvin endpoints, all-extremes safety); update `app/tests.rs` range/step expectations and the kelvin-coverage test; update `edit.rs` tests that encode the old scales (e.g. the Bradford-cliff test moves temperature −60 → −100, same 3200 K scenario).
2. `edit.rs`: add the mapping functions; rewire `apply_all`; make `temperature_tint_matrix` consume `temperature_kelvin`/`tint_yd_shift`; update `EditState` field comments.
3. `viewer.rs`: uniform packing calls the mapping functions instead of `/100`; `image.wgsl` header comment updated.
4. `app/mod.rs`: `slider_range` (Exposure ±5, Temp/Tint ±100, Contrast/Saturation/Clarity/Dehaze ±100) and `slider_step` (Exposure 0.01, Temp/Tint 1.0).
5. Visual verification harness: a dev-only test (`#[ignore]`-gated or tmp-writing regular test? → regular test writing only when an env var is set, so CI stays clean) rendering `assets/test.jpg` at representative settings to `tmp/param-tuning/` with before/after metrics; inspect renders directly.
6. Gates (test/clippy/fmt/build + release build), version 0.2.0 (breaking: slider semantics), changelog entry, devlog entry, README check (feature table mentions sliders only generically — verify).
7. Multi-CLI review (Codex + Gemini + Claude) on the diff with the DESIGN.md contract in the prompt; synthesize to `2026-06-10/1/REVIEW.md`; fix; converge; move thread to done; push; CI green.

Risks: missed call sites that assume the old scales (mitigated by grepping every consumer of the renamed/rescaled fields; the mapping functions make stragglers compile-visible where signatures change, and tests pin the rest); LR-feel subjectivity (mitigated by anchoring to the validated April envelope and the rendered-grid inspection).
