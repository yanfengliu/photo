# RAW develop default tone (raw-default-tone) — investigation notes

Date: 2026-06-11. User report: "the way the app reads the RAW images seem to be duller than lightroom?"

## Diagnosis (code-level, confirmed)

The full RAW develop path is `RawDevelop::default().develop_intermediate(&rawimage)` (`src/decode.rs` `decode_raw_pixels`). rawler's default development performs demosaic, white balance, and the camera-matrix → sRGB conversion with gamma encoding — and nothing else. Lightroom's default rendering additionally applies a per-camera baseline exposure offset, a default contrast tone curve, and an Adobe camera profile, which is why the same ARW looks flatter/duller here. The contrast is extra visible in this app because the staged Detail load shows the camera's embedded JPEG preview first (which has the camera's tone curve baked in) and then "downgrades" the look when the linear-ish develop replaces it.

## Direction (not yet implemented)

Apply a default tone treatment in the RAW develop path (`decode.rs`, post-develop, pre-cache) so developed RAWs land near the embedded-preview look: a baseline S-curve (filmic/basecurve-style) plus a small black-point anchor, applied in a single pass over the developed buffer. Keep it deterministic and parameter-free in v1 (no per-camera tables). Because developed pixels persist in `decoded-cache/`, the cache contract version must bump so stale flat developments re-derive. Baked local edits are unaffected (pixels-as-committed stay as committed); new edit sessions start from the new base look.

Validation plan: side-by-side render of embedded preview vs developed output before/after the curve (the embedded camera JPEG is the reference look), plus the env-gated tuning-grid harness for slider interaction sanity, plus luminance-distribution assertions in tests (developed mean/contrast moves toward the embedded preview's).

## Validation assets

`C:\Users\38909\Documents\images\DSC01169.ARW` (80 MB Sony ARW, on local disk — usable even with the camera card unplugged) provides both the embedded camera-tone preview (reference look) and the rawler develop input for before/after comparison.

## Open questions for implementation

- Curve choice and strength: start with a darktable-basecurve-like sRGB S-curve; tune against a handful of the user's ARWs with the embedded preview as target.
- Whether highlights need a soft shoulder to avoid clipping after the curve (likely yes).
- Interaction with the Exposure slider's ±5 EV range: curve applies to the base, sliders stay relative — no mapping changes expected.
