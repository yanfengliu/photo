# Lightroom Param Parity — Design

Date: 2026-06-10. Objective: align the 12 editing sliders with Adobe Lightroom's user-facing conventions — ranges, response strength, and result quality — without abandoning the citable math foundations (darktable `basicadj`/`toneequal`, Bradford CAT, CIE daylight locus) established in the 2026-04-24 audit. Per `docs/learning/lessons.md` (2026-04-24), Lightroom's internal math is proprietary, so "similar to Lightroom" is defined here as: (a) match Lightroom's publicly observable UI ranges, (b) match its publicly observable endpoint semantics (e.g. Saturation −100 is grayscale), (c) keep per-unit strength inside the envelope already validated as desirable in the April audit, spread across the wider ranges.

## Slider mapping table (the contract)

| Slider | New range (Lightroom UI convention) | Old range | Internal mapping (slider v, u = v/100) | Anchors |
| --- | --- | --- | --- | --- |
| Exposure | −5..+5 EV | −3..+3 | multiplier 2^v (unchanged) | LR exposure range is ±5 EV; EV is physically defined |
| Contrast | −100..+100 | −50..+50 | power-law exponent = 1 + 0.5·u (pivot at CIELab middle gray 0.1842) | darktable basicadj; ±100 reaches exactly the exponent envelope (0.5..1.5) validated at the old ±50; pivot slope = exponent in both linear and gamma space |
| Highlights / Shadows / Whites / Blacks | −100..+100 (unchanged) | same | u × 1.5 EV per Gaussian band (σ=1) at EV −1/−4/0/−7, summed correction clamped ±2 EV | darktable toneequal structure with two deliberate Lightroom-feel deviations found during render review: σ tightened from √2 to 1 so a full Highlights move stays targeted instead of reading as a global exposure cut (band weight two stops out drops 0.37→0.135), and per-band reach reduced from 2 EV to 1.5 EV so a single slider at ±100 recovers ~1.5 stops at its band center while combinations still reach the ±2 EV total clamp |
| Temperature | −100..+100 | −60..+60 | kelvin = 6500 + 33·v → 3200K..9800K | endpoint span preserved from the validated tungsten↔cloudy mapping; ±100 integer convention matches LR's rendered-image slider |
| Tint | −100..+100 | −60..+60 | yd chromaticity shift = 0.00012·v → ±0.012 | endpoint span preserved |
| Vibrance | −100..+100 (unchanged) | same | u with saturation-weighted attenuation (unchanged) | LR-style protect-saturated semantics already in place |
| Saturation | −100..+100 | −50..+50 | chroma scale t = 1 + u | LR endpoint semantics: −100 = exact grayscale, +100 = 2× chroma |
| Clarity | −100..+100 | −50..+50 | local-contrast gain a = 0.5·u (midtone-masked unsharp on the blur pre-pass) | gain envelope (±0.5) preserved from the validated ±50 |
| Dehaze | −100..+100 | −50..+50 | dark-channel strength a = 0.5·u (transmission floor 0.1 unchanged) | strength envelope preserved |

Steps: Exposure 0.01 (was 0.02), Temperature/Tint 1.0 (was 0.5), all others 1.0 (unchanged).

## Single source of truth

New public mapping functions in `edit.rs` (`contrast_amount`, `tone_zone_amount`, `vibrance_amount`, `saturation_amount`, `clarity_amount`, `dehaze_amount`, `temperature_kelvin`, `tint_yd_shift`) become the only place slider units convert to math amounts. Both consumers switch to them: `apply_all` (CPU save path) drops its inline `/100`, and `viewer.rs` uniform packing drops its `/100` block, so CPU/GPU parity is structural instead of duplicated. `temperature_tint_matrix` consumes `temperature_kelvin`/`tint_yd_shift` internally and returns exact identity at neutral (the daylight locus at 6500K is not exactly D65, and the GPU applies the matrix unconditionally while the CPU skips the stage at zero — review finding Codex 1-H1). The WGSL shader changes in exactly one block: the tone-zone Gaussian denominator (2σ² = 2) and per-band reach (1.5 EV), mirroring `edit.rs`; the slider scalings still arrive pre-scaled through `ScaledAmounts`.

## Why this is "desirable" and not destructive

The April audit tuned and validated the visual quality of: exponent ±0.5 contrast, ±0.5 clarity/dehaze gains, the 3200K..9800K white-balance span, the ±2 EV tone-zone cap, and the vibrance formula. This change re-labels that validated envelope onto Lightroom's scale so the slider extremes are strong-but-usable (Lightroom's own convention) instead of leaving headroom uses can't reach or, worse, doubling strengths into clipping. The two genuinely new behaviors are Saturation −100 = grayscale (t=0, the LR endpoint; previously unreachable) and Exposure to ±5 EV (pure EV math, no new failure modes; tone-zone clamp unaffected).

## Compatibility

Baked local edits store rendered pixels, not replayable operations (decision 2026-04-21), so existing `local-edits/` files keep rendering identically. Stored `EditState` values are never re-applied across sessions and the cache headers validate by generation + source metadata, not by state — no cache invalidation needed. In-session: slider positions are reinterpreted on the new scales, which is the intended breaking change (version bumps to 0.2.0).

## Verification

TDD invariants: range table exact; contrast middle-gray invariance for all amounts and identity at 0; saturation −100 → R=G=B exactly and +100 → 2× chroma within gamut; temperature ±100 endpoints 3200/9800 K with direction preserved; mapping functions are the values the GPU uniforms receive (parity test against `viewer.rs` packing); all-extremes `apply_all` stays finite and in-gamut. Visual gate: render the repo-root `test.jpg` through the CPU pipeline (which is shader-parity-tested) at a grid of representative settings, before vs after the rescale, write PNGs + per-image luminance/chroma metrics to `tmp/param-tuning/`, and inspect — slider values that used to be mid-range must produce visibly moderate results at the same physical strength (new value = old value × range ratio).
