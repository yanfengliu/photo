- Add / update tests and make all tests pass with every change. Tests should reflect user behavior.

## Devlog System

This project uses a two-tier devlog for change tracking and agent context.

### Detailed Devlog (`docs/devlog-detailed.md`)
- Append-only log of every significant action, decision, and outcome.
- Each entry must include: timestamp, action taken, result, files modified, and reasoning.
- Format:
  ```
  ## [YYYY-MM-DD HH:MM] — [Short title]
  **Action:** What was done
  **Result:** What happened (success/failure/partial)
  **Files changed:** List of files touched
  **Reasoning:** Why this approach was chosen
  **Notes:** Edge cases, gotchas, or follow-ups
  ```
- This log is the source of truth. Never delete entries — only append corrections.

### Summary Devlog (`docs/devlog-summary.md`)
- Condensed view of project progress for agent context injection.
- Updated after every 5 detailed entries or at the end of a session.
- Each summary entry: one line per action, outcome only, no reasoning.
- Keep the summary under 80 lines. When it exceeds this, compress older entries into a "Prior work" section at the top.

### Devlog Rules
- **Always read `docs/devlog-summary.md` at session start** to understand current project state.
- **Always append to `docs/devlog-detailed.md`** after completing any task.
- After every 5 detailed entries, update `docs/devlog-summary.md`.
- If a subagent is available, delegate summarization to it. The summarizer should extract facts only — no interpretation, no editorializing.
- When compacting, always preserve the devlog file paths and the instruction to read the summary at session start.

## Agent Guidelines

### Decision-Making
- Before modifying existing code, read the relevant files and the devlog summary.
- If a prior attempt at the same task failed (check devlog), use a different approach.
- For ambiguous requirements, ask — do not assume.

### Safety and Guardrails
- Never modify `.env`, secrets, or credentials files.
- Never run destructive database commands without explicit confirmation.
- If unsure about a change's impact, write a test first.

### Subagent Usage
- Use subagents for scoped research tasks to avoid filling main context.
- Devlog summarization should be delegated to a subagent when possible.
- Subagents must write their findings to a file — do not rely on their output staying in context.
