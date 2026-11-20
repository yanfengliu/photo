# Review — raw-default-tone, 2026-06-12, iteration 1

Reviewers: Claude (fable-5, effort max, codebase-grounded), Gemini 3.1 Pro (plan mode). Codex quota-blocked until 2026-07-10 (documented since the library-offline-edits thread).

## Findings

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| Gemini 1-1 / Claude 1-2 | MEDIUM (perf) / MINOR | The per-pixel tone pass runs ~9 transcendental ops per pixel over up to 60 MP single-threaded — multi-second latency added to every uncached develop. Gemini suggested rayon; Claude observed the pipeline is strictly per-channel pointwise over a u8 domain, making a 256-entry LUT bitwise-exact. | Fixed with the LUT (no new dependency, exact by construction — the transform is channelwise and the input domain is quantized; verified by the unchanged unit/wiring tests). Both reviewers' framings agreed; the LUT strictly dominates parallelizing the float path. |
| Claude 1-1 | MINOR (test hygiene) | `raw_develop_applies_the_default_tone_curve` wrapped only the cache-dir override (`None` = production discovery), so each run deposited an orphaned decoded-cache entry for its temp DNG into the real repo's `decoded-cache/`, and it held no repo-root guard against concurrent override tests. Assertions were unaffected (fresh temp path → guaranteed miss; stored path-key verification prevents false passes). | Fixed: the decode is now additionally pinned with `with_test_photo_repo_root(Some(None), …)` (cache disabled), matching every other use of the wrapper. |
| Claude 1-2 (nit) | NIT | Harness output lands in repo-local `tmp/` which was not gitignored. | Fixed: `/tmp/` added to `.gitignore` (also covers the review-run capture convention). |

## Verified clean (both reviewers, Claude with call-site evidence)

Double application impossible: exactly one production call site, upstream of all caching; the persisted cache stores post-tone pixels and serves them verbatim; bakes, CPU save, and GPU preview all share the single toned in-memory base, so parity is structural. No path misses the tone: cache warming and the RAW-thumbnail no-embedded fallback flow through the toned closure (and the fallback matching camera-toned embedded neighbors is the desirable choice); embedded previews/thumbnails correctly keep the camera curve; previously-flat fallback thumbnails self-heal on next launch. The contract bump flows through the single constant with no test pinning version 5. Primitive numerics are safe at the edges (saturating cast, IEEE-correct identity check, clamp before encode). Blast radius contained (constants referenced only in decode.rs; fingerprints hash source bytes, never pixels; the in-memory session cache needs no versioning). The wiring test catches removal, identity-degradation, double application, and constant drift; the tuning harness is hermetic when its env var is unset.

## Outcome

Converged at iteration 1 with no correctness findings: the only fixes were a performance upgrade and test hygiene. 354 tests, clippy -D warnings, fmt --check, debug + release builds green. Tuning evidence (metrics + before/after/reference renders) recorded in the 2026-06-12 devlog entry; constants documented as single-reference-tuned v1.
