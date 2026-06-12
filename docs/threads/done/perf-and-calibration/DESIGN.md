# Multi-image tone calibration + CPU render performance pass (perf-and-calibration)

Date: 2026-06-12. User request: "I have other images available in C:\Users\38909\Documents\images — use these to calibrate and see if you can speed up the app as much as you can while keeping functionalities."

## Calibration

The v0.2.5 tone constants (1.0 EV, contrast 65) were tuned on one dark ARW. The tuning harness gained a directory mode: per image, score a 6×7 (EV, contrast) grid against the embedded camera preview (luma percentile distance), then aggregate mean and worst-case per candidate over the set. Over 35 Sony ARWs (daylight + indoor), per-image optima split — daylight ~(0.85–1.0, 35–55), indoor (0.7–0.85, 55–85) — confirming the overfit. The aggregate optimum (0.85, 65) wins both mean (36.0 vs 46.6) and worst case (56.3 vs 80.1) against the old constants; (0.85, 75) ties the mean but clips more and is worse at the tail. Visual spot-checks on three extremes (bright daylight, worst-case indoor, the original dark scene) confirmed natural renders. Harness stats moved from sorting megapixel buffers to an exact 256-bin histogram (same percentile definition, O(n)).

## Performance (output-preserving by construction)

The full-resolution CPU render (`edit::render_edited_image`) sits on the persist path that fires at every slider release, plus saves, owed bakes, and thumbnail refreshes. Three changes, none of which can alter output beyond 1 ULP pre-quantization:

1. The quarter-res Gaussian blur atlas and its per-pixel bilinear sample run only when clarity or dehaze is non-zero; `apply_all` reads the sample only under those same conditions, so skipping is byte-identical.
2. `apply_tone_zones`, `apply_saturation`, and `apply_vibrance` early-return at zero amounts. The old zero paths computed `lum + (px − lum)·1` per pixel (plus `log2` + four `exp`s, or a `powf`) — identity only to within 1 ULP; the early return is the exact identity and is within the established CPU/GPU formula-level tolerance.
3. The main render loop is row-parallel via `std::thread::scope`: preallocated output, `ceil(height/threads)` rows per chunk, disjoint `chunks_mut` slices, per-pixel math untouched — byte-identical and deterministic.

Measured by the new manual `render_perf_probe` (24 MP synthetic, dev profile, like-for-like): exposure-only 9.02 s → 255 ms (~35×); clarity+dehaze 9.77 s → 2.07 s (~4.7×). Blur-stage parallelization is noted future work (it is the remaining serial portion of the heavy path).

## Out of scope / future

Per-camera tone tables; color-profile (saturation) matching; blur-pass parallelization; release-profile benchmarking (iced's test mocks only compile under debug assertions — timings are profile-relative and the structural gains carry).
