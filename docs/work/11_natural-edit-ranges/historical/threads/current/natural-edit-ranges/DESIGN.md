# Natural Edit Ranges — Design

Date: 2026-07-13. Objective: tune Photo's existing Lightroom-convention adjustment ranges against the supplied private RAW/JPEG corpus so ordinary and endpoint-recovery edits remain natural, continuous, and consistent between the real GPU preview and CPU export path.

## Evidence boundary

The five supplied Sony RAW files and paired XMP sidecars are private, gitignored calibration inputs. Their Basic-panel values are intent anchors, not pixel oracles: no same-RAW Lightroom exports are available, Photo and Adobe use different RAW profiles and white-balance stages, and Photo does not implement Adobe sharpening, noise reduction, Texture, HSL, or look profiles. Durable tests therefore pin mathematical and perceptual safety properties while harness artifacts remain local under `tmp/harness-runs/`.

The calibration set is `DSC01938.ARW` as the zero-edit anchor, `DSC01839.ARW` as a moderate outdoor recipe, `DSC09285.ARW` as a dark endpoint-recovery recipe, the Milky Way JPEG as a blacks/dehaze stress scene, and the carousel JPEG as a highlights/whites/skin stress scene. `DSC09287.ARW`, `DSC09613.ARW`, and the remaining JPEG scenes are holdouts.

## Naturalness contract

- Keep the public Exposure ±5 EV and all other slider ±100 ranges. The sidecars deliberately use Highlights −100 and Shadows +100, so endpoint values must stay reachable.
- Tone-zone combinations must remain monotone over the full display-referred input range. Increasing source luminance may never make output luminance decrease, and adjacent source tones must retain enough separation to avoid visible bands.
- Supplied reference-style recipes must produce continuous tones, plausible local contrast, and low unintended clipping across the calibration set, then generalize to the holdouts. Aggregate statistics guide the inspection but do not replace visual judgment.
- With lens correction off for calibration, GPU preview and CPU render must apply the same transfer-function and adjustment math. An identity render can hide paired conversion errors, so parity verification must include non-zero exposure and tone-zone edits. Lens-enabled interpolation-domain parity is tracked separately because it is independent of slider-range tuning.
- RAW decode must never enlarge a source or embedded preview merely because the GPU texture cap is larger. Calibration renders must measure the source pixels rather than an accidental upscale.
- Neutral edit state remains exact identity; Saturation −100 remains grayscale; existing direction and endpoint semantics for every adjustment remain intact.

## Verified starting findings

The first live harness pass applied the supplied endpoint-recovery pattern to `test_photos/test.jpg`: Exposure +0.30, Contrast +6, Highlights −100, Shadows +100, Whites +30, Blacks −30, Vibrance +15, Saturation −3. Luma p50 fell from 159 to 123 and the GPU screenshot plus CPU render showed severe posterized tone and color bands despite luma clipping remaining below 0.1%. The current 1.5 EV Gaussian tone bands make that composite curve non-monotone, so this is a value-response defect rather than an endpoint-range defect.

Prior harness evidence also verified that the RAW conversion helper asked `image::DynamicImage::thumbnail` for 16384×16384 unconditionally, which upscaled a roughly 60 MP RAW buffer to roughly 179 MP. Version 0.2.9 closed that prerequisite with a downscale-only guard, cache-contract invalidation, exact dimension regressions, and fresh harness proof at the source-sized 9504×6336 buffer.

A live code audit found and two fresh sessions verified a preview/export parity defect: sRGB GPU textures and the normal sRGB presentation surface perform hardware transfer conversion while `image.wgsl` also converted samples to linear and output back to sRGB manually. Version 0.2.10 removes the duplicate source/blur decodes, writes linear values to the sRGB target, and manually encodes only iced's non-sRGB fallback. Final release-harness run `20260714-163056Z-35340` closes the lens-off Exposure +1 bug class: GPU/CPU luma means are 199.54/199.12 with matching p50 205, channel means within 0.72, and saturation within 0.00023; the CPU render is byte-identical pre/post while the real GPU screenshot changes across 58.05% of pixels. The residual 0.00759 clipping-fraction gap is consistent with the different GPU bilinear versus CPU Triangle minification filters rather than a tone bias.

## Change shape

Close the prerequisites as coherent changes, each with tests, review, documentation, a patch-version bump, and a commit. Then retune only the shared slider-to-math mapping needed to restore monotonicity; both CPU and GPU consumers must continue to use that single mapping. Do not change UI ranges or add fixture-specific branches.

## Acceptance

The objective is complete when the decode and preview/export prerequisites are proven, the exact endpoint-recovery scenario is fixed by a fresh rerun, materially different RAW/JPEG calibration and holdout passes show no new substantive value-range/parity finding in two consecutive broadened scopes, all documented tests and four Rust gates pass, adversarial review converges to nits, owned processes are closed, and the resulting commits are pushed.
