## Core rules

- Use test-driven development for behavior changes: write or update tests first, then make them pass. Test the contract, not the code: tests should focus exclusively on app experience and functionalities, not implementation details.
- For each desired change, make the change easy, then make the easy change.
- Before implementing a change, write a plan.
- Use a subagent to implement the plan such that the tests pass. For example, if the tech stack uses node, it should make sure `npx vitest run`, `npx tsc --noEmit`, and `npx vite build` pass.
- Use all of Codex / Gemini / Claude as code reviewer subagents to independently review every change on the following aspects:
  1. Design.
    - Can easily scale, generalize, debug, be understood and reasoned about, and stay lean.
  2. Test coverage.
  3. Correctness.
  4. Clean code, typing, efficiency, memory leaks.
    - No: god class, large files, duplicated logic, inconsistent implementations, violation of boundaries.
    - Prefer composition over inheritance.
    - Clean up dead code.
  5. Documentation.
    - Dev logs should be updated and maintained.
    - References to code should be up to date.
    - No outdated comments.
    - Learnings from debugging and friction points should be documented in `docs/learning/lessons.md`. The file should be actively maintained to not become long, tedious, or outdated.
- Reviews might take a long time depending on the amount of changes you made. Be patient and wait for the result.
- After addressing review comments, ask the reviewer to verify that you have successfully done so. This is basically a second round of full review.
- Example command to use Codex for code review:
  - `git diff [branch] | codex exec --model gpt-5.4 --model-reasoning-effort xhigh --sandbox read-only --ask-for-approval never --ephemeral "You are a senior code reviewer. Flag bugs, security issues, and performance concerns. Do NOT modify files or propose patches. Only return findings, explanations, and suggestions in plain text."`
- Example command to use Gemini for code review:
  - `git diff [branch] | gemini -p "@src You are a senior code reviewer. Flag bugs, security issues, and performance concerns. Do NOT modify files or propose patches. Only return findings, explanations, and suggestions in plain text." --model gemini-3-pro --thinking high` (Use the @ symbol within the prompt to include directory context for the best reasoning results).
- Example command to use Claude for code review:
  - `git diff [branch] | claude -p --append-system-prompt "You are a senior code reviewer. Flag bugs, security issues, and performance concerns. Do NOT modify files or propose patches. Only return findings, explanations, and suggestions in plain text." --allowedTools "Read,Bash(git diff *),Bash(git log *),Bash(git show *)"`
- The reviewers should check `docs/learning/lessons.md`.
- Do not change app mechanics or behavior unless explicitly asked.
- Run `cargo build --release` after your changes.

## Command and git rules

- Always run the full test suite, not a subset.
- Do not use worktrees or branches; work directly on `main`.
- Commit durable docs you add if you are not planning to remove them.
- CRITICAL: Commit as soon as you have a coherent, self-contained unit of change.

## Project docs

- Read `docs/devlog/summary.md` and `docs/architecture/ARCHITECTURE.md` at session start.

## Architecture

- Respect the boundaries documented there. If a boundary seems wrong, flag it instead of silently violating it.
- If architecture changes, update the relevant sections in `docs/architecture/ARCHITECTURE.md`, append a row to `docs/architecture/drift-log.md`, and mention the update in the devlog.
- Do not update `docs/architecture/ARCHITECTURE.md` for non-structural fixes, refactors, UI tweaks, or test-only work.
- Never delete a Key Architectural Decision in `docs/architecture/decisions.md`; add a newer decision that supersedes it.

## Devlog

- Detailed devlogs live under `docs/devlog/detailed/` as append-only files named `YYYY-MM-DD_YYYY-MM-DD.md` (e.g. `2026-04-07_2026-04-13.md`).
- Always append new entries to the latest detailed devlog (the file with the most recent `END_DATE`). When looking something up, start from the latest file and work backwards.
- Periodically archive: when the active file grows larger than 500 lines or a significant time boundary is reached, close it (freeze its `END_DATE` in the filename) and start a new file whose `START_DATE` is the next entry's date. Check if the start and end dates of all previous devlogs are still accurate.
- After every completed task, append a detailed entry with:
  - timestamp
  - action
  - code reviewer comments, broken down by AI provider and theme as stated above
  - result
  - reasoning
  - notes
- Keep `docs/devlog/summary.md` current after updating the detailed log. Always remove outdated info. Compact when it grows larger than 50 lines.
- If a subagent handles summary work, it should extract facts only and avoid interpretation.

## Debugging

- When debugging, use `docs/debugging/template.md` to record your process. Create a new file per debugging session and use it to iterate until you solve the problem.
- Clean up the stackdump files created during debugging after you are done.