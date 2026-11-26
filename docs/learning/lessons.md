# Lessons

The one-line form of every lesson this repo has paid for and has NOT yet gated. Read this file at session start; it is short by construction, because every entry leaves as soon as its gate lands.

Each rule links into [lessons-evidence.md](lessons-evidence.md), which holds the war story and the anchor. Open that only when a rule is in doubt, or the work is in that area — it is not session-start reading.

A new lesson is an entry there plus one line here, and that line NAMES THE GATE it is waiting for. If you cannot name one, this is not the file: propose it in [canon-candidates.md](../../../fleet/canon-candidates.md) if it is fleet-wide, ask the owner about [local-rules.md](../policies/local-rules.md) if it is true only here, or drop it.

When the gate lands, prove it red by reintroducing the defect, record that in [gate-proofs.md](gate-proofs.md), and delete both halves in the same commit.

Code lessons need a real test id. Photo-processing lessons record the affected sample image or batch and the before-and-after pixel difference or quality metric.

## Rules

- On FAT and exFAT under macOS a file's identity (device and inode) changes when an empty file gets its first bytes, when it is renamed, and on exFAT when it is cut to nothing and rewritten, so identity checks that pass on APFS can fail to recognise a file there. — **gate:** a CI step on the macOS runner that runs `cargo test -p photo-export` with `TMPDIR` on a FAT32 and on an exFAT disk image, which would have caught `f4ed351`; the tests that show it on such a volume are `place::held::tests::a_held_file_is_recognised_through_its_first_bytes_and_a_rename` and `place::tests::the_guard_knows_a_file_by_identity_not_by_bytes_or_name` ([evidence](lessons-evidence.md#a-files-identity-changes-on-fat-and-exfat-under-macos))
