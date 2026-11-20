# Lessons

Non-obvious failure modes worth preserving for future sessions. Re-reviewers read this alongside earlier `REVIEW.md` files. Lessons from before the 2026-11-19 restart were either retired into tests in the old code base or carried into `docs/policies/local-rules.md`; they are in git history before commit `94bd2cf`.

Every lesson starts with this evidence-anchor table. Without anchors a lesson is folklore.

| Field | Value |
| --- | --- |
| Surfaced by | path to the `REVIEW.md`, debug log, commit or conversation that exposed the failure |
| Reviewer findings | which reviewer flagged it, severity, finding id |
| Fix commit | short SHA of the commit that closed it |
| Test added | exact test id, or `n/a — process lesson` for review- or tooling-only lessons |
| Behavior delta | concrete before and after: what would have happened without the fix; for image-processing changes include the affected sample image and a before/after measurement |

Code lessons require a real test id. One concise entry per lesson.

## Active lessons

### Run `cargo fmt --check` from the workspace root, never with `--manifest-path`

| Field | Value |
| --- | --- |
| Surfaced by | GitHub Actions run 44680100632 on commit `c59c2bf`: the Format step failed on macOS and Windows 30 seconds after a push whose message said the format gate passed |
| Reviewer findings | None; CI caught it. No reviewer had looked at the commit |
| Fix commit | `5a64b64` |
| Test added | n/a — process lesson. The gate is CI's `cargo fmt --check`, which runs from the workspace root |
| Behavior delta | Before: `cargo fmt --check --manifest-path <root>/Cargo.toml` exited 0 while eleven files in `crates/core` were unformatted, so v0.3.0 was committed and pushed with a red format gate reported as green. After: the gate is run as plain `cargo fmt --check` with the workspace root as the working directory (or `cargo fmt -p <crate> -- --check` for one crate), which checks every member |

`cargo fmt` decides whether to format every workspace member by comparing the workspace root directory with the current directory. When `--manifest-path` is given it compares the root directory with the manifest *file* path instead, the comparison is always false, and it silently formats only the package that manifest belongs to. Passing `--manifest-path` to `cargo test`, `cargo clippy` and `cargo build` is harmless; passing it to `cargo fmt` turns the gate into a no-op for every crate under `crates/`.
