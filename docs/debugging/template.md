# Debugging session — <short title>

Copy this file to `docs/debugging/<YYYY-MM-DD>-<topic>.md` and fill it in as you go. If a later session shows that this one did not fully solve the problem, come back and correct it.

## Symptom

What was observed, by whom, and how to see it again. Include the exact command, input file or UI steps.

## Expected

What should have happened instead, and how that is known (a contract, a test, a design section).

## Reproduction

The smallest reliable way to trigger it. Prefer a failing test.

## Hypotheses

| # | Hypothesis | Evidence for | Evidence against | Status |
| --- | --- | --- | --- | --- |
| 1 | | | | open |

## Investigation log

Timestamped notes: what was tried, what it showed. Keep dead ends; they stop the next session repeating them.

## Root cause

The actual defect, in one or two sentences, with the file and line.

## Fix

What changed and why that addresses the root cause rather than the symptom. Commit SHA.

## Verification

The test that now pins it (exact test id), and any before/after measurement or rendered comparison.

## Follow-ups

Anything left open, and whether it belongs in `docs/learning/lessons.md`.

## Cleanup

Temporary files, dumps and logs created during the session, and confirmation that they were removed.
