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

None yet.
