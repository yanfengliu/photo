# Gate proofs

Every lesson this repo retires leaves behind a gate, and each gate is made to go red by reintroducing the defect the lesson describes before the lesson's prose is deleted. This file records the mutation, the failure it produced, and the commit whose `docs/learning/lessons-evidence.md` still holds the full story.

A gate and the claim in its header can be wrong together and look exactly like a gate that is right. Auditing one means reaching what was believed and measured at the time, never the sentence the gate carries about itself.

The proofs for the previous code base are at `git show 94bd2cf:docs/learning/gate-proofs.md`; the gates they describe were removed with that code.

## `cargo fmt` given `--manifest-path` checks only the root package

**Gate:** CI's Format step, `cargo fmt --all --check` in `.github/workflows/ci.yml`.

**Red:** GitHub Actions run 44680100632 on `c59c2bf` (2026-11-19): the Format step failed on macOS and on Windows on eleven unformatted files in `crates/core`, after a local `cargo fmt --check --manifest-path <root>/Cargo.toml` had exited 0. The defect itself was the mutation; nothing had to be reintroduced.

**Green:** run 44746078151 on `main` at `11334af` (2026-11-20), Format passing on both systems, and every `main` run since.

**Story:** `git show 632d053:docs/learning/lessons-evidence.md`.
