# Canon candidates

Lessons this repo retired that have no mechanical trigger and are not specific to photo. Staged here for the parent to promote into `../fleet/FLEET.md`; until then this file is the only copy, so it is complete.

Delete this file once the rules are in the constitution.

## A conclusion you disprove is amended in the same session that disproves it, in the file that carries it — a "Related" link pointing at a stale conclusion propagates the stale premise instead of flagging it

**From:** photo / `2026-04-23 - A closed debugging doc is read as a settled premise`

**Why it has no gate:** no mechanical trigger — nothing can detect that a document's conclusion is now wrong, and a next-session reader has no signal that a closed investigation was reopened elsewhere.

**Anchor:** `docs/debugging/2026-04-20-detail-load-latency.md` (added by 7e6990c, 2026-04-20 16:10) closed the Detail-latency topic naming exactly one remaining risk and saying nothing about the Library→Detail reopen path. The user re-reported the same symptom three days later; the second session's own record cites the 04-20 doc under "Relevant docs or notes" while stating that reopening still took a long time, and then had to re-derive that the reopen was still routed through the generic load path. The rule to correct superseded debugging docs was written into AGENTS.md (b4e23ec, 2026-04-23 13:15) 38 minutes before that second doc landed (0807793, 13:53) — and `git log -- docs/debugging/2026-04-20-detail-load-latency.md` still shows a single commit, so the rule was authored and then not applied to the very doc that prompted it.

## When a fix closes an invariant violation, audit the invariant rather than the diff: enumerate every site that produces or consumes the artifacts the invariant relates, before declaring it closed

**From:** photo / `2026-06-11 - Fixing an invariant for the path in your diff without auditing the sibling paths that share it`

**Why it has no gate:** the concrete corruption is gated (see `gate-proofs.md`), but the auditing discipline that would have found the sibling path in the first pass has no mechanical trigger — the failure is a review that stopped at the diff's boundary.

**Anchor:** v0.2.2 closed "session edit state must stay relative to its recorded base" for decodes whose base was the Original. The post-hoc reviewer found the identical corruption alive for bake-as-base reloads: edit over an existing bake → session-cache eviction → reopen loads the new bake → state re-applies and re-bakes, compounding S², S³ on every revisit, with exports corrupted identically. Finding 3-H1 in `docs/threads/done/library-offline-edits/2026-06-11/3/REVIEW.md`, plus MEDIUM 3-M2 for mainline identity re-bakes under the same predicate-reuse fix. The same shape recurred a month later: repo-root discovery existed twice, flagged "dedup is a tracked follow-up" since the 2026-06-10 split, and when a runtime sandbox override was added to one copy the other ignored it by construction (fix commit 36975dd).
