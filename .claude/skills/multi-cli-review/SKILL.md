---
name: multi-cli-review
description: Use when running the multi-CLI (Codex + Claude) adversarial code review on high-risk changes or full-codebase audits — routes to the fleet-canonical runbook (pins, commands, output extraction, failure modes) plus photo-specific notes.
---

# Multi-CLI review — photo stub

**Read the fleet-canonical runbook now:** `../loop-ops/docs/skills/multi-cli-review.md` — current review model pins (the fleet's single bump site), exact CLI commands, `-o` output extraction, Windows gotchas, and failure modes. Do not act from memory of an older per-repo copy of this skill.

photo-specific notes:

- Reviewer pin sites in scripts: NONE (verified 2026-07-10 — the repo has no `scripts/` directory and no hard-coded reviewer models outside historical docs). photo also pins no app-facing LLM models.
- Capture/artifact conventions: canonical defaults, no override — raw CLI captures under `tmp/review-runs/<objective>/<date>/<iteration_number>/` (never staged, cleaned up after synthesis); committed review artifacts (`docs/threads/current|done/<objective>/.../REVIEW.md`) follow AGENTS.md → Team of subagents / Code review.
