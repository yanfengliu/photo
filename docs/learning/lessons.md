# Lessons

The one-line form of every lesson this repo has paid for and has NOT yet gated. Read this file at session start; it is short by construction, because every entry leaves as soon as its gate lands.

Each rule links into [lessons-evidence.md](lessons-evidence.md), which holds the war story and the anchor. Open that only when a rule is in doubt, or the work is in that area — it is not session-start reading.

A new lesson is an entry there plus one line here, and that line NAMES THE GATE it is waiting for. If you cannot name one, this is not the file: propose it in [canon-candidates.md](../../../fleet/canon-candidates.md) if it is fleet-wide, ask the owner about [local-rules.md](../policies/local-rules.md) if it is true only here, or drop it.

When the gate lands, prove it red by reintroducing the defect, record that in [gate-proofs.md](gate-proofs.md), and delete both halves in the same commit.

Code lessons need a real test id. Photo-processing lessons record the affected sample image or batch and the before-and-after pixel difference or quality metric.

## Rules

- `cargo fmt` given `--manifest-path` checks only the root package, so a green local format gate can hide unformatted crates. — **gate:** CI's Format step, `cargo fmt --all --check`, which went red on `c59c2bf`; retire once it has also gone green on `main` ([evidence](lessons-evidence.md#cargo-fmt-given---manifest-path-checks-only-the-root-package))
