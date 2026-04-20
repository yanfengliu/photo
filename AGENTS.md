## Core rules

- Use test-driven development for behavior changes: write or update tests first, then make them pass. Test the contract, not the code: tests should focus exclusively on app experience and functionalities, not implementation details.
- For each desired change, make the change easy, then make the easy change.
- Before implementing a change, write a plan.
- Use a subagent to implement the plan such that the tests pass. For example, if the tech stack uses node, it should make sure `npx vitest run`, `npx tsc --noEmit`, and `npx vite build` pass.
- Use Codex and Gemini code reviewer subagents to independently review every change on the following aspects, in series:
  1. Design.
    - Can easily scale, generalize, debug, be understood and reasoned about, and stay lean.
  2. Test coverage.
  3. Correctness.
  4. Clean code, typing, efficiency, memory leaks.
    - No: god class, large files, duplicated logic, inconsistent implementations, violation of boundaries.
  5. Documentation.
    - Dev logs should be updated and maintained.
    - References to code should be up to date.
    - No outdated comments.
    - Learnings from debugging and friction points should be documented in `docs/learning/lessons.md`. The file should be actively maintained to not become long, tedious, or outdated.
- CRITICAL: Each round of review should be done by a new subagent in series. This means 5 steps * 2 reviewers = 10 reviews. Reviews might take a long time depending on the amount of changes you made. Be patient and wait for the result.
- After addressing review comments, ask the reviewer to verify that you have successfully done so. This is basically a second round of full review.
- Example commands to use Codex for code review:
  - `codex exec --sandbox read-only --ask-for-approval never --ephemeral "Review my code for bugs and security issues but do not make any edits"`
  - `codex exec --sandbox read-only --ask-for-approval never --ephemeral review uncommitted`
  - `codex exec --sandbox read-only --ask-for-approval never --ephemeral review base-branch main`
  - `codex exec --sandbox read-only --ask-for-approval never --ephemeral review commit <sha>`
- Example commands to use Gemini for code review:
  - `git diff [branch] | gemini -p "@src Review my code for bugs and security issues but do not make any edits" --model gemini-3-pro --thinking high` (Use the @ symbol within the prompt to include directory context for the best reasoning results).
- The reviewers should check `docs/learning/lessons.md`.
- Prefer small functions and files, reusable utilities, composition over inheritance, and dead-code cleanup.
- Do not change game mechanics or behavior unless explicitly asked.

## Command and git rules

- Never use compound shell commands. Do not chain commands with `&&`, `|`, or `;`.
- If multiple commands are needed, run them as separate sequential tool calls.
- Always run the full test suite, not a subset.
- Do not use worktrees or branches; work directly on `main`.
- For all git commands, always use `git -C <path> <command>`.
- Never use `cd ... && git ...`; that triggers the CLI security block.
- Commit durable docs you add if you are not planning to remove them.
- Commit as soon as you have a coherent, self-contained unit of change.

## Subagents

- If you dispatch a subagent that cannot read repository instructions on its own, include this file and any nested instruction files in its prompt.

## Project docs

- Read `docs/devlog/summary.md` and `docs/architecture/ARCHITECTURE.md` at session start.
- Key directories:
  - `src`: game code.
  - `docs`: architecture, devlogs, reviews.
  - `design`: game and mechanism notes.

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
- Clean up the stackdump files created during debugging after you are done, but keep the `.md` files.