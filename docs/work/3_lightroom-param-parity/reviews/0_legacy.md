# Multi-CLI Review — lightroom-param-parity, iteration 1

Date: 2026-06-10. Scope: diff `b75bf31..ff9e741` (v0.2.0, Lightroom-convention slider tuning). Reviewers: Codex (gpt-5.5, xhigh, read-only sandbox with live-tree access), Claude (claude-fable-5[1m], max effort, Read/Glob/Grep/git; also ran the full suite itself), Gemini (gemini-3.1-pro-preview, plan mode, diff-text only).

## Verdicts

- Codex: 1 HIGH + 1 MEDIUM, no issues in the scalar mappings themselves ("the live WGSL tone-zone constants match the Rust constants").
- Claude: no high-severity issues; CPU/GPU parity verified formula-by-formula and constant-by-constant against the live shader and CPU code; persistence-compatibility claim verified against the cache header format; 3 LOW + nits.
- Gemini: clean approval on all five focus areas.

## Findings and dispositions

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| Codex 1-H1 | HIGH | Neutral preview/export mismatch: `build_adjustment_uniforms` always computes `temperature_tint_matrix(0,0)`, the shader applies it unconditionally, but the CPU save path skips the stage at zero — and the matrix is not exact identity (daylight locus 6500K ≠ D65; measured blue scale 0.99867). Pre-existing hole, but load-bearing for this change's structural-parity claim | Fixed: `temperature_tint_matrix` returns exact identity when both sliders are zero; red-first regression `temperature_tint_matrix_is_exact_identity_at_neutral` (the red run reproduced Codex's matrix to 7 decimal places) |
| Codex 1-M1 | MEDIUM | Doc contradictions that endanger future parity maintenance: DESIGN.md claimed "the WGSL shader is unchanged" while the diff changes its tone-zone constants; `tone_zone_amount` rustdoc and the `apply_tone_zones` header still said ±2 EV per slider vs the implemented 1.5 EV per band / ±2 EV total | Fixed: DESIGN.md single-source paragraph rewritten to name the exact shader block that changes; both rustdoc comments corrected |
| Claude 1-L1 | LOW | `tint_mapping_preserves_validated_chromaticity_span` was nearly vacuous: the shared `approx()` tolerance (absolute 0.01) would accept an ~80% regression of the 0.012 target | Fixed: tolerance tightened to 1e-6 (matching the temperature test's tight ±1 K style) |
| Claude 1-L2 | LOW | Stale `tone_zone_amount` doc (±2 EV per band) | Fixed (same fix as Codex 1-M1) |
| Claude 1-L3 | LOW | Changelog migration record garbled: six sliders listed against five old ranges, implying Dehaze was ±60 | Fixed: ±50/±50/±50/±50/±60/±60 |
| Claude nit | NIT | Stale comment math in the band-isolation test (computed at the old 2.0 EV reach); DESIGN.md said `assets/test.jpg` (asset is repo-root `test.jpg`); amended April lesson said "below" where the new lesson sits above | All fixed |
| Claude observation | FOLLOW-UP→FIXED | Slider readout/text-prefill used `{:.1}`, so the new 0.01 exposure step wasn't representable and text-editing silently rounded | Fixed in this iteration: new `slider_value_label` (Exposure two decimals, ±100 sliders integers, Lightroom-style) used by both sites, with a unit test |

## Convergence assessment

The one substantive finding (1-H1) is a pre-existing defect surfaced by this change's parity focus, not a defect in the new mappings — all three reviewers independently confirmed the new scalar mappings, the shader/CPU constants, and the retuned tone-zone math are consistent (Claude verified summation order and every small constant; Gemini confirmed the σ/reach pairs; Codex confirmed live constants). All findings fixed and verified against the live tree; suite grew 315 → 317 (neutral-identity + label tests). Iteration 1 closes the review.

## Reviewer-noted non-defects worth keeping

- Claude confirmed the persistence-compatibility argument by checking the local-edits cache header contains no serialized `EditState` — slider-scale changes cannot corrupt baked edits.
- Claude verified the changelog's "315 tests" claim by running the suite itself; after this iteration's fixes the count is 317 (changelog updated).
